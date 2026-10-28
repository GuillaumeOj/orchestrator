//! First run (J0): Claude Code's version and login (NFR-7, FR-1201), workspace
//! trust per project, the editor command, the kitty.conf import, and the first
//! weekly plan (FR-802). Everything is read only towards Claude Code's own
//! files (FR-1206): trust is read from `~/.claude.json`, never written.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use agent_runtime::preflight::{self, VersionCheck};
use anyhow::{Result, anyhow, bail};
use governor::jiff::civil::Time;
use governor::{AllocationId, BudgetAllocation, Day, Tier, WeeklyPlan, WorkBlock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LockOk;
use crate::core::Core;
use crate::kitty;
use crate::services::Project;

/// Settings keys (under `ui.` in the store).
pub const FIRST_RUN_KEY: &str = "first_run_ms";
pub const KITTY_KEY: &str = "kitty.conf";
pub const EDITOR_KEY: &str = "editor.command";
/// When the user confirmed a weekly plan (J0 step 4).
pub const PLAN_KEY: &str = "plan.confirmed_ms";
/// The first-run checklist was closed.
pub const DONE_KEY: &str = "setup.done";

/// What `claude` reports at start (J0 step 1).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ClaudeCheck {
    pub path: PathBuf,
    pub version: Option<String>,
    /// Newer than the last tested version (NFR-7: runs, with a warning).
    pub untested: bool,
    pub logged_in: Option<bool>,
    pub auth_method: Option<String>,
    pub subscription: Option<String>,
    /// What is wrong, with the fix (the app is read-only until it is fixed).
    pub problem: Option<String>,
    /// The login could not be checked (a slow or failed `claude auth status`):
    /// shown, not blocking.
    pub note: Option<String>,
}

impl ClaudeCheck {
    /// Agents may start: a supported version, logged in with Pro/Max.
    pub fn ready(&self) -> Result<()> {
        match &self.problem {
            Some(p) => Err(anyhow!("{p}")),
            None => Ok(()),
        }
    }
}

/// `claude --version` and `claude auth status --json` (deadline 15 s).
pub fn check_claude(path: &Path) -> ClaudeCheck {
    let mut c = ClaudeCheck { path: path.to_path_buf(), ..Default::default() };
    match preflight::claude_version(path) {
        Err(e) => {
            c.problem = Some(format!(
                "Claude Code not found ({e:#}). Install it (https://claude.com/claude-code) or set ORCH_CLAUDE, then restart the app."
            ));
            return c;
        }
        Ok(v) => {
            c.version = Some(v.to_string());
            match preflight::SUPPORTED_CLAUDE.check(v) {
                Ok(VersionCheck::Supported(_)) => {}
                Ok(VersionCheck::Untested(_)) => c.untested = true,
                Err(e) => c.problem = Some(format!("{e:#}: update Claude Code (claude update), then restart the app.")),
            }
        }
    }
    match auth_status(path) {
        Ok(v) => apply_auth(&mut c, &v),
        Err(e) => c.note = Some(format!("the login could not be checked (claude auth status: {e:#})")),
    }
    c
}

fn auth_status(claude: &Path) -> Result<Value> {
    let mut cmd = Command::new(claude);
    cmd.args(["auth", "status", "--json"]).env("DISABLE_AUTOUPDATER", "1").env_remove("ANTHROPIC_API_KEY");
    let out = orch_git::git::run_with_deadline(cmd, Duration::from_secs(15))?;
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// `loggedIn`, `authMethod` (`claude.ai` for a subscription), `subscriptionType`.
fn apply_auth(c: &mut ClaudeCheck, v: &Value) {
    c.logged_in = v["loggedIn"].as_bool();
    c.auth_method = v["authMethod"].as_str().map(str::to_string);
    c.subscription = v["subscriptionType"].as_str().map(str::to_string);
    if c.problem.is_some() {
        return;
    }
    if c.logged_in != Some(true) {
        c.problem = Some("Claude Code is not logged in: run `claude` in a terminal and /login with your Claude account, then restart the app.".into());
    } else if c.auth_method.as_deref() != Some("claude.ai") || !matches!(c.subscription.as_deref(), Some("pro" | "max"))
    {
        c.problem = Some(format!(
            "Claude Code is logged in with {} ({}), not a Pro/Max subscription: agents would not run on the plan (NFR-10). Log in with your Claude account (/login).",
            c.auth_method.as_deref().unwrap_or("?"),
            c.subscription.as_deref().unwrap_or("no subscription")
        ));
    }
}

/// Claude Code's workspace trust for `repo` (keyed by the main checkout),
/// read from `~/.claude.json` (re-read when its size or mtime changes; never written).
pub fn trusted(repo: &Path) -> Option<bool> {
    trusted_in(&agent_runtime::paths::home_dir().join(".claude.json"), repo)
}

fn trusted_in(file: &Path, repo: &Path) -> Option<bool> {
    type Stamp = (PathBuf, u64, Option<std::time::SystemTime>);
    static CACHE: std::sync::Mutex<Option<(Stamp, Value)>> = std::sync::Mutex::new(None);
    let meta = std::fs::metadata(file).ok()?;
    let stamp: Stamp = (file.to_path_buf(), meta.len(), meta.modified().ok());
    let mut c = CACHE.lock_ok();
    if c.as_ref().is_none_or(|(s, _)| *s != stamp) {
        let v: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        // Keep only the trust flags: the file also holds account details.
        let trust: serde_json::Map<String, Value> = v["projects"]
            .as_object()
            .map(|m| m.iter().map(|(k, p)| (k.clone(), p["hasTrustDialogAccepted"].clone())).collect())
            .unwrap_or_default();
        *c = Some((stamp, Value::Object(trust)));
    }
    let trust = &c.as_ref()?.1;
    Some(trust[repo.to_string_lossy().as_ref()].as_bool().unwrap_or(false))
}

/// The weekly plan as the first-run form and the planner edit it (FR-802):
/// working days and hours, reserve, one weighted allocation per project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanForm {
    /// `mon`…`sun`.
    pub days: Vec<String>,
    /// `HH:MM`.
    pub start: String,
    pub end: String,
    pub reserve_pct: f64,
    /// % of the week agents may use outside working hours (FR-816; 0 = none).
    #[serde(default)]
    pub off_hours_allowance_pct: f64,
    pub allocations: Vec<AllocationForm>,
    /// FR-814: convert each model family's cost with its own fitted `k`
    /// (`PacingPolicy::per_model_weights`). Absent = unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_model_weights: Option<bool>,
    /// FR-816: live agents for work scheduled off-hours (`PacingPolicy::off_hours_live_cap`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_hours_live_cap: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AllocationForm {
    /// The project id (agents of the project are charged to it).
    pub id: String,
    pub target: String,
    pub weight: f64,
    pub tier: Tier,
    /// FR-811: the allocation stops for the week at this % of the week (none: no cap).
    #[serde(default)]
    pub hard_cap_pct: Option<f64>,
}

const DAYS: [(&str, Day); 7] = [
    ("mon", Day::Mon),
    ("tue", Day::Tue),
    ("wed", Day::Wed),
    ("thu", Day::Thu),
    ("fri", Day::Fri),
    ("sat", Day::Sat),
    ("sun", Day::Sun),
];

fn hhmm(t: Time) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

fn parse_time(s: &str) -> Result<Time> {
    let (h, m) = s.trim().split_once(':').ok_or_else(|| anyhow!("time {s:?}: use HH:MM"))?;
    Ok(Time::new(h.parse()?, m.parse()?, 0, 0)?)
}

impl PlanForm {
    /// The form for `plan`. A plan with only the default allocation (never
    /// edited) proposes an even split over the projects (J0 step 4).
    pub fn from_plan(plan: &WeeklyPlan, projects: &[Project]) -> PlanForm {
        let blocks = &plan.calendar.blocks;
        let first = blocks.first();
        let mut days: Vec<String> =
            DAYS.iter().filter(|(_, d)| blocks.iter().any(|b| b.day == *d)).map(|(n, _)| n.to_string()).collect();
        if days.is_empty() {
            days = DAYS[..5].iter().map(|(n, _)| n.to_string()).collect();
        }
        let only_default =
            plan.allocations.len() == 1 && plan.allocations[0].id.0 == agent_runtime::runtime::DEFAULT_ALLOCATION;
        let allocations = if only_default && !projects.is_empty() {
            projects
                .iter()
                .map(|p| AllocationForm {
                    id: p.id.clone(),
                    target: p.name.clone(),
                    weight: 1.0,
                    tier: Tier::Normal,
                    hard_cap_pct: None,
                })
                .collect()
        } else {
            plan.allocations
                .iter()
                .map(|a| AllocationForm {
                    id: a.id.0.clone(),
                    target: a.target.clone(),
                    weight: a.weight,
                    tier: a.tier,
                    hard_cap_pct: a.hard_cap_pct,
                })
                .collect()
        };
        PlanForm {
            days,
            start: first.map(|b| hhmm(b.start)).unwrap_or_else(|| "09:00".into()),
            end: first.map(|b| hhmm(b.end)).unwrap_or_else(|| "19:00".into()),
            reserve_pct: plan.reserve_pct,
            off_hours_allowance_pct: plan.off_hours_allowance_pct,
            allocations,
            per_model_weights: None,
            off_hours_live_cap: None,
        }
    }

    /// `plan` with this form's calendar, reserve and allocations. Agents not
    /// in an allocation are charged to `default`, so it is always kept.
    pub fn apply(&self, plan: &WeeklyPlan) -> Result<WeeklyPlan> {
        let (start, end) = (parse_time(&self.start)?, parse_time(&self.end)?);
        if end <= start {
            bail!("the working day ends before it starts");
        }
        if !(0.0..=50.0).contains(&self.reserve_pct) {
            bail!("reserve {}%: use 0–50 %", self.reserve_pct);
        }
        if !(0.0..=100.0).contains(&self.off_hours_allowance_pct) {
            bail!("off-hours allowance {}%: use 0–100 %", self.off_hours_allowance_pct);
        }
        let mut blocks = vec![];
        for d in &self.days {
            let (_, day) = DAYS.iter().find(|(n, _)| n == d).ok_or_else(|| anyhow!("unknown day {d:?}"))?;
            blocks.push(WorkBlock { day: *day, start, end, weight: 1.0 });
        }
        if blocks.is_empty() {
            bail!("pick at least one working day");
        }
        let mut allocations: Vec<BudgetAllocation> = vec![];
        for a in self.allocations.iter().filter(|a| a.weight > 0.0) {
            if allocations.iter().any(|x| x.id.0 == a.id) {
                bail!("allocation {} twice", a.id);
            }
            if let Some(c) = a.hard_cap_pct.filter(|c| !(0.0..=100.0).contains(c)) {
                bail!("hard cap of {} {c}%: use 0–100 %, or leave it empty", a.id);
            }
            allocations.push(BudgetAllocation {
                id: AllocationId(a.id.clone()),
                target: a.target.clone(),
                weight: a.weight,
                tier: a.tier,
                hard_cap_pct: a.hard_cap_pct,
            });
        }
        if !allocations.iter().any(|a| a.id.0 == agent_runtime::runtime::DEFAULT_ALLOCATION) {
            let w = if allocations.is_empty() { 100.0 } else { 1.0 };
            allocations.push(BudgetAllocation {
                id: AllocationId(agent_runtime::runtime::DEFAULT_ALLOCATION.into()),
                target: "other sessions".into(),
                weight: w,
                tier: Tier::Normal,
                hard_cap_pct: None,
            });
        }
        let mut p = plan.clone();
        p.calendar.blocks = blocks;
        p.reserve_pct = self.reserve_pct;
        p.off_hours_allowance_pct = self.off_hours_allowance_pct;
        p.allocations = allocations;
        Ok(p)
    }
}

impl Core {
    /// The first-run checklist (J0) as the UI shows it (`projects`: the
    /// registered ones, already read by the caller).
    pub fn setup_view(&self, projects: &[Project]) -> Value {
        let trust: Vec<Value> = projects.iter().map(|p| json!({"id": p.id, "trusted": trusted(&p.path)})).collect();
        let kitty = self.kitty.lock_ok();
        json!({
            "done": self.settings.get(DONE_KEY).is_some(),
            "claude": self.claude_check(),
            "trust": trust,
            "kitty": {"sources": kitty.sources, "font": kitty.font_family, "mapped": kitty.mapped.len(), "unmapped": kitty.unmapped.len(), "editor": kitty.editor},
            "editor": *self.editor_template.lock_ok(),
            "plan_confirmed": self.settings.get(PLAN_KEY).is_some(),
        })
    }

    /// J0 step 3: the editor command template (`{file}`, `{line}`, `{socket}`).
    pub fn set_editor(&self, template: &str) -> Result<()> {
        let t = template.trim();
        if t.is_empty() {
            bail!("the editor command is empty");
        }
        self.settings.set(EDITOR_KEY, json!(t))?;
        *self.editor_template.lock_ok() = t.to_string();
        Ok(())
    }

    /// J0 step 3: (re)imports a kitty.conf, read only, and remembers its path.
    pub fn import_kitty(&self, path: &Path) -> Result<kitty::KittyImport> {
        if !path.is_file() {
            bail!("no kitty.conf at {}", path.display());
        }
        let k = kitty::import(path);
        self.settings.set(KITTY_KEY, json!(path))?;
        *self.kitty.lock_ok() = k.clone();
        Ok(k)
    }

    pub fn plan_form(&self) -> Result<PlanForm> {
        let rt = self.runtime()?;
        let projects = self.projects();
        Ok(rt.with_governor(|g| {
            let mut f = PlanForm::from_plan(g.plan(), &projects);
            f.per_model_weights = Some(g.policy().per_model_weights);
            f.off_hours_live_cap = Some(g.policy().off_hours_live_cap);
            f
        }))
    }

    /// J0 step 4 / the planner's Edit plan: the plan of the current week.
    pub fn set_plan(&self, form: &PlanForm) -> Result<()> {
        let rt = self.runtime()?;
        let plan = form.apply(&rt.with_governor(|g| g.plan().clone()))?;
        // Pacing options live in the policy; the replan below rebuilds the Governor with it.
        if let Some(cap) = form.off_hours_live_cap.filter(|c| *c > 20) {
            bail!("off-hours live cap {cap}: use 0–20");
        }
        if form.per_model_weights.is_some() || form.off_hours_live_cap.is_some() {
            rt.update_policy(|p| {
                if let Some(on) = form.per_model_weights {
                    p.per_model_weights = on;
                }
                if let Some(cap) = form.off_hours_live_cap {
                    p.off_hours_live_cap = cap;
                }
            })?;
        }
        rt.replan(plan)?;
        self.settings.set(PLAN_KEY, json!(orch_protocol::now_ms()))?;
        Ok(())
    }

    /// FR-812: leave (or go back to) shadow mode. Takes effect now.
    pub fn set_enforce(&self, enforce: bool) -> Result<()> {
        // The runtime stores the choice (`governor.enforce`), like `orchd enforce`.
        self.runtime()?.set_shadow(!enforce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_days(p: &WeeklyPlan) -> Vec<&'static str> {
        p.calendar
            .blocks
            .iter()
            .map(|b| DAYS.iter().find(|(_, x)| *x == b.day).map(|(n, _)| *n).unwrap_or("?"))
            .collect()
    }

    fn project(id: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: PathBuf::from("/x").join(id),
            default_branch: "main".into(),
            last_fetch_at_ms: None,
            last_fetch_error: None,
        }
    }

    #[test]
    fn auth_status_decides_read_only_mode() {
        let mut c = ClaudeCheck::default();
        apply_auth(&mut c, &json!({"loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max"}));
        assert!(c.ready().is_ok(), "{c:?}");
        let mut c = ClaudeCheck::default();
        apply_auth(&mut c, &json!({"loggedIn": false}));
        assert!(c.ready().unwrap_err().to_string().contains("not logged in"));
        let mut c = ClaudeCheck::default();
        apply_auth(&mut c, &json!({"loggedIn": true, "authMethod": "api_key"}));
        assert!(c.ready().unwrap_err().to_string().contains("not a Pro/Max"));
        // A version problem is kept (the first thing to fix).
        let mut c = ClaudeCheck { problem: Some("old".into()), ..Default::default() };
        apply_auth(&mut c, &json!({"loggedIn": false}));
        assert_eq!(c.problem.as_deref(), Some("old"));
        assert!(check_claude(Path::new("/no/such/claude")).ready().unwrap_err().to_string().contains("not found"));
        // A login that could not be checked is noted, not blocking.
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("claude");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1\" = --version ] && echo '2.1.302 (Claude Code)' && exit 0\necho garbage\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let c = check_claude(&fake);
        assert!(c.ready().is_ok() && c.note.is_some(), "{c:?}");
    }

    #[test]
    fn trust_is_read_from_the_claude_json_projects() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("claude.json");
        std::fs::write(&f, r#"{"oauthAccount": {"emailAddress": "x"}, "projects": {"/r/a": {"hasTrustDialogAccepted": true}, "/r/b": {}}}"#).unwrap();
        assert_eq!(trusted_in(&f, Path::new("/r/a")), Some(true));
        assert_eq!(trusted_in(&f, Path::new("/r/b")), Some(false));
        assert_eq!(trusted_in(&f, Path::new("/r/c")), Some(false));
        assert_eq!(trusted_in(&d.path().join("missing.json"), Path::new("/r/a")), None);
    }

    #[test]
    fn plan_form_even_split_then_round_trip() {
        let now: governor::jiff::Timestamp = "2026-10-21T09:00:00Z".parse().unwrap();
        let plan = agent_runtime::runtime::default_plan(now, true);
        let f = PlanForm::from_plan(&plan, &[project("shop"), project("api")]);
        assert_eq!(f.days, ["mon", "tue", "wed", "thu", "fri"]);
        assert_eq!((f.start.as_str(), f.end.as_str()), ("09:00", "19:00"));
        assert_eq!(
            f.allocations.iter().map(|a| (a.id.as_str(), a.weight)).collect::<Vec<_>>(),
            [("shop", 1.0), ("api", 1.0)]
        );
        let mut f2 = f.clone();
        f2.days = vec!["mon".into(), "sat".into()];
        f2.start = "10:00".into();
        f2.allocations[1].weight = 3.0;
        f2.allocations[1].hard_cap_pct = Some(12.0);
        f2.off_hours_allowance_pct = 5.0;
        let p = f2.apply(&plan).unwrap();
        assert_eq!(plan_days(&p), ["mon", "sat"]);
        assert_eq!((p.allocations[1].hard_cap_pct, p.off_hours_allowance_pct), (Some(12.0), 5.0));
        assert_eq!(p.allocations.len(), 3, "default kept for other sessions");
        assert_eq!((p.week_start, p.reset_at), (plan.week_start, plan.reset_at), "same week");
        let back = PlanForm::from_plan(&p, &[]);
        assert_eq!(back.allocations.len(), 3);
        assert_eq!(back.start, "10:00");
        assert_eq!((back.allocations[1].hard_cap_pct, back.off_hours_allowance_pct), (Some(12.0), 5.0));
        governor::Governor::new(p, governor::PacingPolicy::default(), now).expect("a valid plan");
        let bad = |m: fn(&mut PlanForm)| {
            let mut x = f.clone();
            m(&mut x);
            x.apply(&plan).is_err()
        };
        assert!(bad(|x| x.days.clear()));
        assert!(bad(|x| x.end = "08:00".into()));
        assert!(bad(|x| x.reserve_pct = 80.0));
        assert!(bad(|x| x.off_hours_allowance_pct = 120.0));
        assert!(bad(|x| x.allocations[0].hard_cap_pct = Some(-1.0)));
        assert!(bad(|x| x.days = vec!["funday".into()]));
    }
}
