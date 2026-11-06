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

use agent_runtime::runtime::DEFAULT_ALLOCATION;

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
    /// Which check `problem` comes from (set iff `problem` is).
    pub problem_kind: Option<ProblemKind>,
    /// The login could not be checked (a slow or failed `claude auth status`):
    /// shown, not blocking.
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    NotFound,
    Version,
    Login,
    Plan,
}

impl ClaudeCheck {
    fn fail(&mut self, kind: ProblemKind, problem: String) {
        self.problem = Some(problem);
        self.problem_kind = Some(kind);
    }

    /// Agents may start: a supported version, logged in with Pro/Max.
    pub fn ready(&self) -> Result<()> {
        match &self.problem {
            Some(p) => Err(anyhow!("{p}")),
            None => Ok(()),
        }
    }
}

/// The start-up runtime's version warnings (`claude 2.1.330 is newer than…`,
/// `claude: claude 2.1.200 is older than…`) are fixed at start: `state()`
/// drops them and shows these, from the current check, so they follow an
/// in-app update.
pub fn is_runtime_version_warning(w: &str) -> bool {
    (w.starts_with("claude ") && w.contains(" is newer than the last tested version"))
        || (w.starts_with("claude: ") && w.contains(" is older than the supported minimum"))
}

pub fn version_warnings(c: Option<&ClaudeCheck>) -> Vec<String> {
    let Some(c) = c else { return vec![] };
    let (Some(v), range) = (&c.version, preflight::SUPPORTED_CLAUDE) else { return vec![] };
    if c.untested {
        vec![format!("claude {v} is newer than the last tested version: run the contract tests (R2)")]
    } else if c.problem_kind == Some(ProblemKind::Version) {
        vec![format!("claude: claude {v} is older than the supported minimum {}", range.min)]
    } else {
        vec![]
    }
}

/// `claude --version` and `claude auth status --json` (deadline 15 s).
pub fn check_claude(path: &Path) -> ClaudeCheck {
    let mut c = ClaudeCheck { path: path.to_path_buf(), ..Default::default() };
    match preflight::claude_version(path) {
        Err(e) => {
            c.fail(
                ProblemKind::NotFound,
                format!(
                    "Claude Code not found ({e:#}). Install it (https://claude.com/claude-code) or set ORCH_CLAUDE, then restart the app."
                ),
            );
            return c;
        }
        Ok(v) => {
            c.version = Some(v.to_string());
            match preflight::SUPPORTED_CLAUDE.check(v) {
                Ok(VersionCheck::Supported(_)) => {}
                Ok(VersionCheck::Untested(_)) => c.untested = true,
                Err(e) => c.fail(
                    ProblemKind::Version,
                    format!("{e:#}: update Claude Code (Settings › Claude Code, or `claude update`)."),
                ),
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
        c.fail(ProblemKind::Login, "Claude Code is not logged in: run `claude` in a terminal and /login with your Claude account, then restart the app.".into());
    } else if c.auth_method.as_deref() != Some("claude.ai") || !matches!(c.subscription.as_deref(), Some("pro" | "max"))
    {
        let problem = format!(
            "Claude Code is logged in with {} ({}), not a Pro/Max subscription: agents would not run on the plan (NFR-10). Log in with your Claude account (/login).",
            c.auth_method.as_deref().unwrap_or("?"),
            c.subscription.as_deref().unwrap_or("no subscription")
        );
        c.fail(ProblemKind::Plan, problem);
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

impl AllocationForm {
    /// The rules `PlanForm::apply` and `project_allocation_set` share. A
    /// weight-0 row is dropped, so only a kept row's cap matters.
    fn check(&self) -> Result<()> {
        if !(self.weight.is_finite() && self.weight >= 0.0) {
            bail!("weight of {} {}: use 0 or more", self.id, self.weight);
        }
        if let Some(c) = self.hard_cap_pct.filter(|c| self.weight > 0.0 && !(0.0..=100.0).contains(c)) {
            bail!("hard cap of {} {c}%: use 0–100 %, or leave it empty", self.id);
        }
        Ok(())
    }
}

impl AllocationForm {
    /// `project`'s row: its id is the allocation id, its name the target.
    fn of_project(p: &Project, weight: f64, tier: Tier, hard_cap_pct: Option<f64>) -> Self {
        AllocationForm { id: p.id.clone(), target: p.name.clone(), weight, tier, hard_cap_pct }
    }
}

/// Where a new allocation goes among `ids`: before `default`, which stays last.
fn before_default<'a>(mut ids: impl Iterator<Item = &'a str>) -> Option<usize> {
    ids.position(|id| id == DEFAULT_ALLOCATION)
}

impl From<&BudgetAllocation> for AllocationForm {
    fn from(a: &BudgetAllocation) -> Self {
        AllocationForm {
            id: a.id.0.clone(),
            target: a.target.clone(),
            weight: a.weight,
            tier: a.tier,
            hard_cap_pct: a.hard_cap_pct,
        }
    }
}

impl From<&AllocationForm> for BudgetAllocation {
    fn from(a: &AllocationForm) -> Self {
        BudgetAllocation {
            id: AllocationId(a.id.clone()),
            target: a.target.clone(),
            weight: a.weight,
            tier: a.tier,
            hard_cap_pct: a.hard_cap_pct,
        }
    }
}

/// A project's allocation as Project settings › Allocation shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectAllocationView {
    /// The project id (= its allocation id).
    pub project: String,
    pub name: String,
    /// A plan was saved (`PLAN_KEY`). Without one, the allocations are the
    /// split `PlanForm::from_plan` proposes.
    pub plan_confirmed: bool,
    /// None: not in the plan (charged to `default`, other sessions).
    pub allocation: Option<AllocationLine>,
    /// The plan's other allocations, in plan order, `default` last.
    pub others: Vec<AllocationLine>,
    /// Where `allocation` sits among all the plan's allocations, in plan
    /// order (the UI inserts it into `others` there); None with no allocation.
    pub own_index: Option<usize>,
    /// Weight `Add` pre-fills: mean of the project allocations (not the
    /// features'), rounded, ≥ 1.
    pub proposed_weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AllocationLine {
    #[serde(flatten)]
    pub row: AllocationForm,
    /// Usage charged to it this week, pp of the week.
    pub used_pct: f64,
}

/// The project's row as Project settings › Allocation saves it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AllocationEdit {
    pub weight: f64,
    pub tier: Tier,
    #[serde(default)]
    pub hard_cap_pct: Option<f64>,
}

fn allocation_view(
    project: &Project,
    projects: &[Project],
    plan_confirmed: bool,
    plan: &WeeklyPlan,
    used: impl Fn(&AllocationId) -> f64,
) -> ProjectAllocationView {
    let line = |a: &BudgetAllocation| AllocationLine { row: a.into(), used_pct: used(&a.id) };
    let is_default = |a: &&BudgetAllocation| a.id.0 == DEFAULT_ALLOCATION;
    let own = plan.allocations.iter().position(|a| a.id.0 == project.id);
    let mut others: Vec<AllocationLine> =
        plan.allocations.iter().filter(|a| a.id.0 != project.id && !is_default(a)).map(line).collect();
    others.extend(plan.allocations.iter().filter(is_default).map(line));
    let weights: Vec<f64> = plan
        .allocations
        .iter()
        .filter(|a| !is_default(a) && projects.iter().any(|p| p.id == a.id.0))
        .map(|a| a.weight)
        .collect();
    let proposed_weight =
        if weights.is_empty() { 1.0 } else { (weights.iter().sum::<f64>() / weights.len() as f64).round().max(1.0) };
    ProjectAllocationView {
        project: project.id.clone(),
        name: project.name.clone(),
        plan_confirmed,
        allocation: own.map(|i| line(&plan.allocations[i])),
        others,
        own_index: own,
        proposed_weight,
    }
}

/// `plan` with `project`'s allocation replaced, added (before `default`) or,
/// with `row` None, removed. The other allocations are untouched.
fn with_project_allocation(mut plan: WeeklyPlan, project: &str, row: Option<&AllocationForm>) -> WeeklyPlan {
    let at = plan.allocations.iter().position(|a| a.id.0 == project);
    match (at, row) {
        (Some(i), Some(r)) => plan.allocations[i] = r.into(),
        (Some(i), None) => {
            plan.allocations.remove(i);
        }
        (None, Some(r)) => {
            let at = before_default(plan.allocations.iter().map(|a| a.id.0.as_str()));
            plan.allocations.insert(at.unwrap_or(plan.allocations.len()), r.into());
        }
        (None, None) => {}
    }
    plan
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
    /// The form for `plan`. Before a plan is `confirmed`, one with only the
    /// default allocation proposes an even split over the projects (J0 step
    /// 4). Else the plan's allocations, plus each project it doesn't have at
    /// weight 0 (dropped by `apply` unless given a weight), before `default`:
    /// a confirmed plan whose projects were all taken out stays that way.
    pub fn from_plan(plan: &WeeklyPlan, projects: &[Project], confirmed: bool) -> PlanForm {
        let blocks = &plan.calendar.blocks;
        let first = blocks.first();
        let mut days: Vec<String> =
            DAYS.iter().filter(|(_, d)| blocks.iter().any(|b| b.day == *d)).map(|(n, _)| n.to_string()).collect();
        if days.is_empty() {
            days = DAYS[..5].iter().map(|(n, _)| n.to_string()).collect();
        }
        let only_default = plan.allocations.len() == 1 && plan.allocations[0].id.0 == DEFAULT_ALLOCATION;
        let allocations = if only_default && !confirmed && !projects.is_empty() {
            projects.iter().map(|p| AllocationForm::of_project(p, 1.0, Tier::Normal, None)).collect()
        } else {
            let mut rows: Vec<AllocationForm> = plan.allocations.iter().map(AllocationForm::from).collect();
            let missing: Vec<AllocationForm> = projects
                .iter()
                .filter(|p| !rows.iter().any(|a| a.id == p.id))
                .map(|p| AllocationForm::of_project(p, 0.0, Tier::Normal, None))
                .collect();
            let at = before_default(rows.iter().map(|a| a.id.as_str())).unwrap_or(rows.len());
            rows.splice(at..at, missing);
            rows
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
        for a in &self.allocations {
            a.check()?;
            if a.weight == 0.0 {
                continue; // not in the plan: charged to `default`
            }
            if allocations.iter().any(|x| x.id.0 == a.id) {
                bail!("allocation {} twice", a.id);
            }
            allocations.push(a.into());
        }
        if !allocations.iter().any(|a| a.id.0 == DEFAULT_ALLOCATION) {
            let w = if allocations.is_empty() { 100.0 } else { 1.0 };
            allocations.push(BudgetAllocation {
                id: AllocationId(DEFAULT_ALLOCATION.into()),
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
            "claude": self.claude_update.view(),
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
        let confirmed = self.settings.get(PLAN_KEY).is_some();
        Ok(rt.with_governor(|g| {
            let mut f = PlanForm::from_plan(g.plan(), &projects, confirmed);
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
        if let Some(cap) = form.off_hours_live_cap {
            agent_runtime::live_cap::check_off_hours(cap)?;
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

    /// Project settings › Allocation: `project`'s allocation in the current
    /// plan, or in the split proposed while no plan is confirmed.
    pub fn project_allocation(&self, project: &str) -> Result<ProjectAllocationView> {
        let p = self.project(project)?;
        let rt = self.runtime()?;
        let confirmed = self.settings.get(PLAN_KEY).is_some();
        let projects = self.projects();
        rt.with_governor(|g| {
            let plan = if confirmed {
                g.plan().clone()
            } else {
                PlanForm::from_plan(g.plan(), &projects, false).apply(g.plan())?
            };
            Ok(allocation_view(&p, &projects, confirmed, &plan, |id| g.allocation_usage_pct(id)))
        })
    }

    /// Replaces, adds or (`form` None or weight 0) removes only `project`'s
    /// allocation in the current plan; the week's usage is replayed into it.
    pub fn project_allocation_set(
        &self,
        project: &str,
        form: Option<&AllocationEdit>,
    ) -> Result<ProjectAllocationView> {
        let p = self.project(project)?;
        if self.settings.get(PLAN_KEY).is_none() {
            bail!("set up the weekly plan first");
        }
        if p.id == DEFAULT_ALLOCATION {
            // Its id is the other sessions' allocation: editing it here would edit (or drop) that one.
            bail!("project {} shares its id with the other sessions' allocation: edit it in the weekly plan", p.id);
        }
        let row =
            form.filter(|f| f.weight != 0.0).map(|f| AllocationForm::of_project(&p, f.weight, f.tier, f.hard_cap_pct));
        if let Some(r) = &row {
            r.check()?;
        }
        let rt = self.runtime()?;
        let plan = rt.with_governor(|g| g.plan().clone());
        let next = with_project_allocation(plan.clone(), &p.id, row.as_ref());
        if next != plan {
            rt.replan(next)?;
        }
        self.project_allocation(project)
    }

    /// Settings › Pacing policy (api-contract `pacing_settings`).
    pub fn pacing_settings(&self) -> Result<agent_runtime::live_cap::PacingSettings> {
        Ok(self.runtime()?.pacing_settings())
    }

    /// Sets the base and the off-hours live caps (absent = unchanged); they
    /// apply now. Returns the new view.
    pub fn set_pacing_settings(
        &self,
        live_cap: Option<u32>,
        off_hours_live_cap: Option<u32>,
    ) -> Result<agent_runtime::live_cap::PacingSettings> {
        let rt = self.runtime()?;
        rt.set_live_caps(live_cap, off_hours_live_cap)?;
        Ok(rt.pacing_settings())
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
        assert_eq!(c.problem_kind, Some(ProblemKind::Login));
        let mut c = ClaudeCheck::default();
        apply_auth(&mut c, &json!({"loggedIn": true, "authMethod": "api_key"}));
        assert!(c.ready().unwrap_err().to_string().contains("not a Pro/Max"));
        assert_eq!(c.problem_kind, Some(ProblemKind::Plan));
        // A version problem is kept (the first thing to fix).
        let mut c =
            ClaudeCheck { problem: Some("old".into()), problem_kind: Some(ProblemKind::Version), ..Default::default() };
        apply_auth(&mut c, &json!({"loggedIn": false}));
        assert_eq!((c.problem.as_deref(), c.problem_kind), (Some("old"), Some(ProblemKind::Version)));
        let c = check_claude(Path::new("/no/such/claude"));
        assert!(c.ready().unwrap_err().to_string().contains("not found"));
        assert_eq!(c.problem_kind, Some(ProblemKind::NotFound));
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
        assert!(c.ready().is_ok() && c.note.is_some() && c.problem_kind.is_none(), "{c:?}");
        // Too old: the fix is the update, no restart needed.
        let old = d.path().join("old-claude");
        std::fs::write(&old, "#!/bin/sh\necho '2.1.200 (Claude Code)'\n").unwrap();
        std::fs::set_permissions(&old, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let c = check_claude(&old);
        assert_eq!(c.problem_kind, Some(ProblemKind::Version));
        let p = c.problem.unwrap();
        assert!(
            p.ends_with("update Claude Code (Settings › Claude Code, or `claude update`).") && !p.contains("restart"),
            "{p}"
        );
    }

    #[test]
    fn version_warnings_follow_the_current_check() {
        let runtime = "claude 2.9.0 is newer than the last tested version: run the contract tests (R2)";
        let old = "claude: claude 2.1.200 is older than the supported minimum 2.1.302";
        assert!(is_runtime_version_warning(runtime) && is_runtime_version_warning(old));
        assert!(!is_runtime_version_warning("claude: no claude at /x"));
        assert!(!is_runtime_version_warning("helpers: claude is newer"));
        let check = |v: &str, untested, kind| ClaudeCheck {
            version: Some(v.into()),
            untested,
            problem_kind: kind,
            ..Default::default()
        };
        assert_eq!(version_warnings(None), Vec::<String>::new());
        assert_eq!(version_warnings(Some(&check("2.9.0", true, None))), [runtime]);
        assert_eq!(version_warnings(Some(&check("2.1.200", false, Some(ProblemKind::Version)))), [old]);
        // After an in-app update to a supported version: no warning.
        assert!(version_warnings(Some(&check("2.1.302", false, None))).is_empty());
        // And to another untested one: the new version.
        assert!(version_warnings(Some(&check("2.9.1", true, None)))[0].starts_with("claude 2.9.1 is newer"));
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
        let f = PlanForm::from_plan(&plan, &[project("shop"), project("api")], false);
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
        let back = PlanForm::from_plan(&p, &[project("shop"), project("api")], false);
        assert_eq!(back.allocations.len(), 3, "every project already in the plan");
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
        assert!(bad(|x| x.allocations[0].weight = -1.0));
    }

    #[test]
    fn plan_form_lists_projects_missing_from_the_plan_at_weight_0() {
        let now: governor::jiff::Timestamp = "2026-10-21T09:00:00Z".parse().unwrap();
        let plan = agent_runtime::runtime::default_plan(now, true);
        let p = PlanForm::from_plan(&plan, &[project("shop")], false).apply(&plan).unwrap();
        let f = PlanForm::from_plan(&p, &[project("shop"), project("new")], false);
        let rows = |f: &PlanForm| f.allocations.iter().map(|a| (a.id.clone(), a.weight)).collect::<Vec<_>>();
        assert_eq!(rows(&f), [("shop".into(), 1.0), ("new".into(), 0.0), ("default".into(), 1.0)]);
        let new = &f.allocations[1];
        assert_eq!((new.tier, new.hard_cap_pct, new.target.as_str()), (Tier::Normal, None, "new"));
        assert_eq!(f.apply(&p).unwrap().allocations, p.allocations, "a weight-0 row is not added");
        let mut f2 = f.clone();
        f2.allocations[1].weight = 2.0;
        let ids: Vec<String> = f2.apply(&p).unwrap().allocations.iter().map(|a| a.id.0.clone()).collect();
        assert_eq!(ids, ["shop", "new", "default"]);
        // A confirmed plan whose projects were all taken out: no split proposed.
        let f = PlanForm::from_plan(&plan, &[project("shop")], true);
        assert_eq!(rows(&f), [("shop".into(), 0.0), ("default".into(), 100.0)]);
    }

    #[test]
    fn one_projects_allocation_is_replaced_added_or_removed() {
        let now: governor::jiff::Timestamp = "2026-10-21T09:00:00Z".parse().unwrap();
        let plan = agent_runtime::runtime::default_plan(now, true);
        let p = PlanForm::from_plan(&plan, &[project("shop")], false).apply(&plan).unwrap();
        let row = |id: &str, weight| AllocationForm {
            id: id.into(),
            target: id.into(),
            weight,
            tier: Tier::High,
            hard_cap_pct: None,
        };
        let ids = |p: &WeeklyPlan| p.allocations.iter().map(|a| (a.id.0.clone(), a.weight)).collect::<Vec<_>>();
        let added = with_project_allocation(p.clone(), "api", Some(&row("api", 4.0)));
        assert_eq!(ids(&added), [("shop".into(), 1.0), ("api".into(), 4.0), ("default".into(), 1.0)]);
        let changed = with_project_allocation(added.clone(), "shop", Some(&row("shop", 7.0)));
        assert_eq!(ids(&changed), [("shop".into(), 7.0), ("api".into(), 4.0), ("default".into(), 1.0)]);
        let removed = with_project_allocation(changed, "api", None);
        assert_eq!(ids(&removed), [("shop".into(), 7.0), ("default".into(), 1.0)]);
        assert_eq!(removed.calendar, p.calendar);
        assert_eq!(with_project_allocation(p.clone(), "api", None), p);
    }
}
