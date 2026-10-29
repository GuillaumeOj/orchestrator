//! The outcome digest (§9.6 level 4, P4.1 spec §3.4, D8, D11, D12): what
//! got done today, built from events (tasks done, merges, run phases, PRs
//! opened, a solo session's finished turn, the weekly report, a task-sync
//! conflict), never from the prompts sent. Each outcome's id is its event's.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use orch_store::Event;
use orch_team::RunState;
use serde::Serialize;
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// The events an outcome is made of.
pub const KINDS: [&str; 7] =
    ["task.done", "git.merge", "run.state", "run.pr_opened", "turn.done", "weekly_report", "task.sync_conflict"];

/// Settings key prefix of the per-day "seen" watermark (`digest.seen.2026-10-29`).
pub const SEEN_KEY: &str = "digest.seen";

/// One outcome (api-contract §2.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Outcome {
    pub id: String,
    /// task_done | merge_done | pr_opened | run_phase | run_done | turn_done | report_ready | sync_conflict
    pub kind: &'static str,
    pub at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// The row, written by the core (p4-design-notes §5).
    pub text: String,
    /// Kind-specific: task, branch, into, url, from, to, report, remote.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
    pub seen: bool,
}

/// What an event's outcome needs to name things.
#[derive(Default)]
pub struct Names {
    /// run id → (feature slug, project id)
    pub runs: BTreeMap<String, (String, String)>,
    /// agent id → project id (solo agents and members)
    pub projects: BTreeMap<String, String>,
    /// task id → (handle, title)
    pub tasks: BTreeMap<u64, (String, String)>,
}

/// "orch/checkout/back-1" → "back-1".
fn tail(branch: &str) -> &str {
    branch.rsplit('/').next().unwrap_or(branch)
}

/// "spec_gate" → "spec gate" (the run phase palette's label, lower case).
fn phase(s: &str) -> String {
    s.parse::<RunState>().map(|r| r.label().to_lowercase()).unwrap_or_else(|_| s.replace('_', " "))
}

/// "https://…/pull/12" → "#12".
fn pr_number(url: &str) -> String {
    url.rsplit('/')
        .next()
        .filter(|n| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty())
        .map(|n| format!(" #{n}"))
        .unwrap_or_default()
}

/// The outcome of one event, if it is one. `seen_up_to`: today's watermark.
pub fn outcome(ev: &Event, run: Option<&str>, names: &Names, seen_up_to: u64, tz: &TimeZone) -> Option<Outcome> {
    let d = &ev.detail;
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let run = run.map(str::to_string).or_else(|| d.get("run").and_then(Value::as_str).map(str::to_string));
    let (feature, run_project) = match run.as_deref().and_then(|r| names.runs.get(r)) {
        Some((f, p)) => (Some(f.clone()), Some(p.clone())),
        None => (None, None),
    };
    let agent = ev.agent_id.clone().or_else(|| d.get("agent").and_then(Value::as_str).map(str::to_string));
    let project = run_project
        .or_else(|| agent.as_deref().and_then(|a| names.projects.get(a)).cloned())
        .or_else(|| d.get("project").and_then(Value::as_str).map(str::to_string));
    let feat = feature.clone().unwrap_or_else(|| "the run".into());
    let mut extra = serde_json::Map::new();
    let (kind, text) = match ev.kind.as_str() {
        "task.done" => {
            let id = d.get("task").and_then(Value::as_u64)?;
            let (handle, title) = names.tasks.get(&id).cloned().unwrap_or_else(|| (format!("T-{id}"), String::new()));
            extra.insert("task".into(), json!(id));
            let text = if title.is_empty() { format!("{handle} done") } else { format!("{handle} done · {title}") };
            ("task_done", text)
        }
        "git.merge" => {
            // Only merges that happened: no conflict, nothing already merged, no error.
            if d["result"]["outcome"] != "merged" {
                return None;
            }
            let branch = s("branch");
            let into =
                d.get("into").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| "integration".into());
            extra.insert("branch".into(), json!(branch));
            extra.insert("into".into(), json!(into));
            ("merge_done", format!("Merged {} into {}", tail(&branch), tail(&into)))
        }
        "run.state" => {
            let (from, to) = (s("from"), s("to"));
            if from == to {
                return None;
            }
            extra.insert("from".into(), json!(from));
            extra.insert("to".into(), json!(to));
            if to == "done" {
                ("run_done", format!("{feat} done"))
            } else {
                ("run_phase", format!("{feat}: {} → {}", phase(&from), phase(&to)))
            }
        }
        "run.pr_opened" => {
            let url = s("url");
            extra.insert("url".into(), json!(url));
            let what = match (&feature, &agent) {
                (Some(f), _) => f.clone(),
                (None, Some(a)) => format!("{} › {a}", project.as_deref().unwrap_or("?")),
                (None, None) => "a branch".into(),
            };
            ("pr_opened", format!("PR opened · {what}{} ↗", pr_number(&url)))
        }
        "turn.done" => {
            let summary = s("summary");
            let line = Some(crate::attention::copy::cut(&summary, 200))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "finished a turn".into());
            let who = agent.clone().unwrap_or_default();
            ("turn_done", format!("{} › {who}: {line}", project.as_deref().unwrap_or("?")))
        }
        "weekly_report" => {
            let report = s("report");
            extra.insert("report".into(), json!(report));
            let week = report
                .strip_prefix("week-")
                .and_then(|n| n.parse::<i64>().ok())
                .and_then(|sec| jiff::Timestamp::from_second(sec).ok())
                .map(|t| t.to_zoned(tz.clone()).date().iso_week_date().week());
            let text = match week {
                Some(w) => format!("Weekly report for week {w} is ready"),
                None => "Weekly report is ready".into(),
            };
            ("report_ready", text)
        }
        "task.sync_conflict" => {
            let remote = s("remote");
            extra.insert("task".into(), d.get("task").cloned().unwrap_or(Value::Null));
            extra.insert("remote".into(), json!(remote));
            ("sync_conflict", format!("{} changed here and in {remote}: kept {remote}'s", s("handle")))
        }
        _ => return None,
    };
    Some(Outcome {
        id: format!("ev-{}", ev.id),
        kind,
        at_ms: ev.ts_ms,
        project,
        run,
        agent,
        feature,
        text,
        extra,
        seen: ev.id <= seen_up_to,
    })
}

/// Local midnight of `now_ms` and its date (the watermark's key).
pub fn today(now_ms: u64, tz: &TimeZone) -> (u64, String) {
    let z = jiff::Timestamp::from_millisecond(now_ms as i64).unwrap_or_default().to_zoned(tz.clone());
    let start = z.start_of_day().map(|d| d.timestamp().as_millisecond().max(0) as u64).unwrap_or(0);
    (start, z.date().to_string())
}

impl Runtime {
    /// Today's outcomes after event `after` (tests).
    #[cfg(test)]
    pub fn outcomes_since(&self, after: u64) -> Vec<Outcome> {
        let tz = TimeZone::system();
        let (midnight, date) = today(orch_protocol::now_ms(), &tz);
        let paths = self.db.projects(false).unwrap_or_default().into_iter().map(|p| (p.path, p.id)).collect();
        self.outcomes_of_day(midnight, &date, after, &tz, &BTreeMap::new(), &paths).0
    }

    /// The outcomes of the day starting at `midnight` (`date`) after event
    /// `after`, and the last event id read (any kind: nothing is read twice).
    /// `runs` (run → feature slug, project) and `paths` (main checkout →
    /// project) are the `now` publisher's; the store is asked only for the rest.
    pub(crate) fn outcomes_of_day(
        &self,
        midnight: u64,
        date: &str,
        after: u64,
        tz: &TimeZone,
        runs: &BTreeMap<String, (String, String)>,
        paths: &BTreeMap<std::path::PathBuf, String>,
    ) -> (Vec<Outcome>, u64) {
        let Ok((evs, last)) = self.db.events_of_kinds(midnight, after, &KINDS) else { return (vec![], after) };
        if evs.is_empty() {
            return (vec![], last);
        }
        let seen = self.digest_watermark(date);
        let names = self.names(&evs, runs, paths);
        (evs.iter().filter_map(|(e, r)| outcome(e, r.as_deref(), &names, seen, tz)).collect(), last)
    }

    /// The runs, agents' projects and tasks these events name.
    fn names(
        &self,
        evs: &[(Event, Option<String>)],
        runs: &BTreeMap<String, (String, String)>,
        paths: &BTreeMap<std::path::PathBuf, String>,
    ) -> Names {
        let mut n = Names { runs: runs.clone(), ..Default::default() };
        for (e, run) in evs {
            if let Some(r) = run.as_deref().filter(|r| !n.runs.contains_key(*r))
                && let Ok(Some(tr)) = self.db.run(r)
            {
                let f = self.db.feature(&tr.feature_id).ok().flatten();
                n.runs.insert(
                    r.to_string(),
                    (
                        f.as_ref().map(|f| f.slug.clone()).unwrap_or(tr.feature_id),
                        f.map(|f| f.project_id).unwrap_or_default(),
                    ),
                );
            }
            if let Some(id) = (e.kind == "task.done").then(|| e.detail["task"].as_u64()).flatten()
                && !n.tasks.contains_key(&id)
                && let Ok(Some(t)) = self.db.task(id)
            {
                n.tasks.insert(id, (t.handle(), t.title));
            }
        }
        n.projects = self.store.read(|s| {
            s.agents
                .values()
                .filter_map(|a| {
                    let p = a.team.as_ref().map(|t| t.project_id.clone()).or_else(|| paths.get(&a.repo).cloned())?;
                    Some((a.id.clone(), p))
                })
                .collect()
        });
        n
    }

    /// Today's "seen" watermark (an event id), 0 when none.
    fn digest_watermark(&self, date: &str) -> u64 {
        self.db.setting(&format!("{SEEN_KEY}.{date}")).ok().flatten().and_then(|v| v.as_u64()).unwrap_or(0)
    }

    /// `digest_seen {up_to}`: today's outcomes up to that one (`ev-N`) leave
    /// the Inbox digest; they still count in `done_today`.
    pub fn digest_seen(&self, up_to: &str) -> anyhow::Result<()> {
        let n: u64 = up_to.strip_prefix("ev-").unwrap_or(up_to).parse()?;
        let (_, date) = today(orch_protocol::now_ms(), &TimeZone::system());
        let key = format!("{SEEN_KEY}.{date}");
        if self.digest_watermark(&date) < n {
            self.db.set_setting(&key, &json!(n))?;
        }
        self.mark_seen(n);
        self.touch(orch_protocol::Topic::Inbox);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u64, kind: &str, agent: Option<&str>, detail: Value) -> Event {
        Event { id, ts_ms: 1_000 + id, agent_id: agent.map(str::to_string), kind: kind.into(), detail }
    }

    fn names() -> Names {
        let mut n = Names::default();
        n.runs.insert("run-1".into(), ("checkout".into(), "shop".into()));
        n.projects.insert("fix-feed".into(), "blog".into());
        n.tasks.insert(4, ("T-4".into(), "checkout API".into()));
        n
    }

    #[test]
    fn outcomes_from_run_events() {
        let (n, tz) = (names(), TimeZone::UTC);
        let run = Some("run-1");
        let out = |e: Event, r: Option<&str>| outcome(&e, r, &n, 3, &tz);
        let t = out(ev(1, "task.done", Some("back-1"), json!({"task": 4})), run).unwrap();
        assert_eq!(
            (t.kind, t.text.as_str(), t.feature.as_deref(), t.project.as_deref()),
            ("task_done", "T-4 done · checkout API", Some("checkout"), Some("shop"))
        );
        assert_eq!((t.id.as_str(), t.seen), ("ev-1", true));
        let m = out(
            ev(
                2,
                "git.merge",
                Some("lead-1"),
                json!({"branch": "orch/checkout/back-1", "result": {"outcome": "merged", "sha": "a"}}),
            ),
            run,
        )
        .unwrap();
        assert_eq!((m.kind, m.text.as_str()), ("merge_done", "Merged back-1 into integration"));
        for result in
            [json!({"outcome": "conflict", "files": []}), json!({"outcome": "up_to_date"}), json!({"error": "x"})]
        {
            assert!(
                out(ev(2, "git.merge", None, json!({"branch": "b", "result": result})), run).is_none(),
                "not a merge"
            );
        }
        let p = out(ev(5, "run.state", None, json!({"from": "planning", "to": "executing"})), run).unwrap();
        assert_eq!((p.kind, p.text.as_str(), p.seen), ("run_phase", "checkout: planning → executing", false));
        let g = out(ev(5, "run.state", None, json!({"from": "spec", "to": "spec_gate"})), run).unwrap();
        assert_eq!(g.text, "checkout: spec → spec gate");
        let d = out(ev(6, "run.state", None, json!({"from": "pr_gate", "to": "done"})), run).unwrap();
        assert_eq!((d.kind, d.text.as_str()), ("run_done", "checkout done"));
        let pr =
            out(ev(7, "run.pr_opened", Some("lead-1"), json!({"url": "https://github.com/o/r/pull/12"})), run).unwrap();
        assert_eq!((pr.kind, pr.text.as_str()), ("pr_opened", "PR opened · checkout #12 ↗"));
        assert_eq!(pr.extra["url"], "https://github.com/o/r/pull/12");
        let solo = out(ev(8, "run.pr_opened", Some("fix-feed"), json!({"url": "https://x/pull/3"})), None).unwrap();
        assert_eq!(solo.text, "PR opened · blog › fix-feed #3 ↗");
        let turn = out(
            ev(9, "turn.done", Some("fix-feed"), json!({"summary": "\nFixed the feed pagination\nand more"})),
            None,
        )
        .unwrap();
        assert_eq!((turn.kind, turn.text.as_str()), ("turn_done", "blog › fix-feed: Fixed the feed pagination"));
        // 2026-10-26 00:00 UTC is in ISO week 44.
        let w = out(ev(10, "weekly_report", None, json!({"report": "week-1792972800"})), None).unwrap();
        assert_eq!((w.kind, w.text.as_str()), ("report_ready", "Weekly report for week 44 is ready"));
        let c = out(
            ev(
                11,
                "task.sync_conflict",
                None,
                json!({"task": 9, "handle": "ENG-140", "remote": "Linear", "project": "shop"}),
            ),
            None,
        )
        .unwrap();
        assert_eq!(
            (c.kind, c.text.as_str(), c.project.as_deref()),
            ("sync_conflict", "ENG-140 changed here and in Linear: kept Linear's", Some("shop"))
        );
        assert!(out(ev(12, "hook.Stop", None, json!({})), None).is_none());
        let v = serde_json::to_value(&pr).unwrap();
        assert_eq!(v["url"], "https://github.com/o/r/pull/12", "kind fields are flat");
        assert_eq!(v["feature"], "checkout");
    }

    #[test]
    fn today_is_local() {
        let tz = TimeZone::fixed(jiff::tz::offset(-5));
        // 2026-10-29 03:00 UTC is 28 Oct 22:00 at UTC-5.
        let (start, date) = today(1_793_242_800_000, &tz);
        assert_eq!(date, "2026-10-28");
        assert_eq!(start, 1_793_163_600_000);
    }
}
