//! The run report (P4.0 exit criterion, D7): what a team run (or a time
//! range) cost the user in prompts, pane interventions and waiting, from the
//! events, the audit log and the Inbox rows. Read only.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Store;

/// The 2026-10-27 team run the P4.0 exit run is compared with (brief 2026-10-28).
pub const BASELINE: Baseline =
    Baseline { date: "2026-10-27", prompts: 29, interventions: 6, waiting_ms: 5 * 3_600_000 };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Baseline {
    pub date: &'static str,
    pub prompts: u64,
    pub interventions: u64,
    /// About 5 h of agent time waiting.
    pub waiting_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportQuery {
    #[serde(default)]
    pub team_run_id: Option<String>,
    #[serde(default)]
    pub since_ms: Option<u64>,
    #[serde(default)]
    pub until_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAgent {
    pub id: String,
    pub role: String,
    /// From its last spawn's `--permission-mode` (`default` without one).
    pub permission_mode: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub total: u64,
    pub by: BTreeMap<String, u64>,
}

impl Counts {
    fn add(&mut self, key: &str) {
        self.total += 1;
        *self.by.entry(key.to_string()).or_default() += 1;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    pub agents: Vec<ReportAgent>,
    /// PermissionRequest hooks, by tool.
    pub prompts_by_tool: Counts,
    /// Permission answers (audit rows), by decider: `user`, `pane`, `rule:<id>`, `expired`, `core`.
    pub prompts_by_decider: Counts,
    /// `pane_intervention` events, by cause.
    pub interventions: Counts,
    /// Open → decided of the agents' permission, question, gate and startup
    /// items (parked time included; still open ones up to `until`).
    pub waiting_ms: u64,
    pub startup_blocked: u64,
    pub restarted_fresh: u64,
    /// Effective memory-pressure changes (core-wide, in the range).
    pub pressure_changes: u64,
    /// Agents suspended for memory pressure.
    pub pressure_suspends: u64,
    pub since_ms: u64,
    pub until_ms: u64,
}

/// `--permission-mode <m>` in a spawn's args.
fn permission_mode(args: &Value) -> String {
    let a: Vec<&str> = args.as_array().map(|v| v.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    a.windows(2).find(|w| w[0] == "--permission-mode").map(|w| w[1].to_string()).unwrap_or_else(|| "default".into())
}

fn hours(ms: u64) -> String {
    format!("{:.1} h", ms as f64 / 3_600_000.0)
}

fn by(c: &Counts) -> String {
    if c.by.is_empty() {
        return "-".into();
    }
    let mut v: Vec<(&String, &u64)> = c.by.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    v.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
}

impl RunReport {
    /// A short text table (`orchd report`), with the baseline next to the
    /// numbers when asked.
    pub fn render(&self, baseline: bool) -> String {
        let mut out = String::new();
        out.push_str("agents:\n");
        for a in &self.agents {
            out.push_str(&format!("  {:<28} {:<12} mode {}\n", a.id, a.role, a.permission_mode));
        }
        let line = |name: &str, now: String, base: String, detail: String| {
            let now = if baseline { format!("{now:<10} (baseline {base})") } else { now };
            format!("{name:<22} {now}  {detail}\n")
        };
        out.push_str(&line(
            "permission prompts",
            self.prompts_by_tool.total.to_string(),
            BASELINE.prompts.to_string(),
            format!("by tool: {}", by(&self.prompts_by_tool)),
        ));
        out.push_str(&line("", String::new(), String::new(), format!("by decider: {}", by(&self.prompts_by_decider))));
        out.push_str(&line(
            "pane interventions",
            self.interventions.total.to_string(),
            BASELINE.interventions.to_string(),
            format!("by cause: {}", by(&self.interventions)),
        ));
        out.push_str(&line("waiting", hours(self.waiting_ms), hours(BASELINE.waiting_ms), String::new()));
        out.push_str(&format!(
            "startups blocked {}, fresh restarts {}, pressure changes {}, pressure suspends {}\n",
            self.startup_blocked, self.restarted_fresh, self.pressure_changes, self.pressure_suspends
        ));
        if baseline {
            out.push_str(&format!("baseline: the {} team run\n", BASELINE.date));
        }
        out
    }
}

impl Store {
    pub fn run_report(&self, q: &ReportQuery, now_ms: u64) -> Result<RunReport> {
        let (since, until) = (q.since_ms.unwrap_or(0), q.until_ms.unwrap_or(now_ms));
        let run = q.team_run_id.clone();
        self.read(move |c| {
            // The agents: a run's seats, or every agent.
            let mut agents: Vec<(String, String)> = vec![];
            let mut st = c.prepare("SELECT id, data FROM agents ORDER BY id")?;
            for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (id, data) = row?;
                let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
                if run.as_deref().is_some_and(|r| data["team"]["run_id"].as_str() != Some(r)) {
                    continue;
                }
                agents.push((id, data["role"]["name"].as_str().unwrap_or("").to_string()));
            }
            let ids: BTreeSet<&str> = agents.iter().map(|(id, _)| id.as_str()).collect();
            let mine = |a: &Option<String>| a.as_deref().is_some_and(|a| ids.contains(a));
            let mut rep = RunReport { since_ms: since, until_ms: until, ..Default::default() };

            // Events in the range.
            let mut spawned: BTreeMap<String, Value> = BTreeMap::new();
            let mut st = c.prepare(
                "SELECT agent_id, kind, detail FROM events WHERE ts_ms >= ?1 AND ts_ms < ?2
                 AND kind IN ('PermissionRequest', 'pane_intervention', 'restarted_fresh', 'memory_pressure',
                              'auto_suspend', 'spawned')
                 ORDER BY ts_ms, id",
            )?;
            let rows = st.query_map(params![since as i64, until as i64], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for row in rows {
                let (agent, kind, detail) = row?;
                let mut d: Value = serde_json::from_str(&detail).unwrap_or(Value::Null);
                if let Some(p) = d.get("blob").and_then(Value::as_str) {
                    d = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
                }
                if kind == "memory_pressure" {
                    rep.pressure_changes += 1;
                    continue;
                }
                if !mine(&agent) {
                    continue;
                }
                let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("?").to_string();
                match kind.as_str() {
                    "PermissionRequest" => rep.prompts_by_tool.add(&s("tool")),
                    "pane_intervention" => rep.interventions.add(&s("waited_on")),
                    "restarted_fresh" => rep.restarted_fresh += 1,
                    "auto_suspend" if d["why"] == "memory_pressure" => rep.pressure_suspends += 1,
                    "spawned" => {
                        spawned.insert(agent.unwrap_or_default(), d["args"].clone());
                    }
                    _ => {}
                }
            }
            // Spawned before the range: its last spawn still says how it runs.
            for (id, _) in &agents {
                if !spawned.contains_key(id) {
                    let last: Option<String> = c
                        .query_row(
                            "SELECT detail FROM events WHERE agent_id=?1 AND kind='spawned' ORDER BY ts_ms DESC, id DESC LIMIT 1",
                            [id],
                            |r| r.get(0),
                        )
                        .ok();
                    if let Some(d) = last.and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                        spawned.insert(id.clone(), d["args"].clone());
                    }
                }
            }
            rep.agents = agents
                .iter()
                .map(|(id, role)| ReportAgent {
                    id: id.clone(),
                    role: role.clone(),
                    permission_mode: spawned.get(id).map(permission_mode).unwrap_or_else(|| "default".into()),
                })
                .collect();

            // Who answered each permission prompt.
            let mut st = c.prepare(
                "SELECT agent_id, actor FROM audit_log WHERE action='permission' AND ts_ms >= ?1 AND ts_ms < ?2",
            )?;
            for row in st.query_map(params![since as i64, until as i64], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)))? {
                let (agent, actor) = row?;
                if mine(&agent) {
                    rep.prompts_by_decider.add(&actor);
                }
            }

            // Waiting on the user, and starts that were blocked.
            let mut st = c.prepare(
                "SELECT agent_id, kind, created_at_ms, decided_at_ms FROM inbox_items
                 WHERE kind IN ('permission', 'question', 'gate', 'startup') AND created_at_ms >= ?1 AND created_at_ms < ?2",
            )?;
            let rows = st.query_map(params![since as i64, until as i64], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<i64>>(3)?))
            })?;
            for row in rows {
                let (agent, kind, created, decided) = row?;
                if !mine(&agent) {
                    continue;
                }
                let end = decided.map_or(until, |d| (d as u64).min(until));
                rep.waiting_ms += end.saturating_sub(created as u64);
                if kind == "startup" {
                    rep.startup_blocked += 1;
                }
            }
            Ok(rep)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_rows::upsert_inbox;
    use serde_json::json;

    /// A report over a fixture: two agents of run `r1`, one of another run.
    #[test]
    fn a_report_counts_prompts_interventions_and_waiting_for_its_run() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("s.db")).unwrap();
        s.write(|tx| {
            for (id, run, role) in [("f-back-1", "r1", "back"), ("f-lead-1", "r1", "lead"), ("g-back-1", "r2", "back")] {
                tx.execute(
                    "INSERT INTO agents(id, repo, worktree, session_id, state, created_at_ms, data) VALUES (?1,'/r','/w','s','idle',0,?2)",
                    params![id, json!({"role": {"name": role}, "team": {"run_id": run}}).to_string()],
                )?;
            }
            let ev = |ts: i64, agent: &str, kind: &str, d: Value| {
                tx.execute(
                    "INSERT INTO events(ts_ms, agent_id, kind, detail) VALUES (?1,?2,?3,?4)",
                    params![ts, agent, kind, d.to_string()],
                )
            };
            ev(5, "f-back-1", "spawned", json!({"args": ["--permission-mode", "auto", "--resume", "x"]}))?;
            ev(10, "f-back-1", "PermissionRequest", json!({"tool": "Bash"}))?;
            ev(11, "f-back-1", "PermissionRequest", json!({"tool": "Bash"}))?;
            ev(12, "f-lead-1", "PermissionRequest", json!({"tool": "Read"}))?;
            ev(13, "g-back-1", "PermissionRequest", json!({"tool": "Bash"}))?;
            ev(14, "f-back-1", "pane_intervention", json!({"waited_on": "permission"}))?;
            ev(15, "f-lead-1", "pane_intervention", json!({"waited_on": "idle_with_task"}))?;
            ev(16, "f-back-1", "restarted_fresh", json!({}))?;
            ev(17, "f-back-1", "auto_suspend", json!({"why": "memory_pressure"}))?;
            ev(18, "f-lead-1", "auto_suspend", json!({"why": "idle"}))?;
            tx.execute("INSERT INTO events(ts_ms, agent_id, kind, detail) VALUES (19, NULL, 'memory_pressure', '{}')", [])?;
            for (actor, agent) in [("rule:3", "f-back-1"), ("expired", "f-back-1"), ("user", "f-back-1"), ("pane", "f-lead-1"), ("user", "g-back-1")] {
                tx.execute(
                    "INSERT INTO audit_log(ts_ms, actor, agent_id, action, target, decision, detail) VALUES (20,?1,?2,'permission','Bash','allow','{}')",
                    params![actor, agent],
                )?;
            }
            let item = |id: u64, agent: &str, kind: &str, created: u64, decided: Option<u64>| {
                let mut i = crate::InboxItem::new(id, kind, Some(agent), "x".into());
                i.created_at_ms = created;
                i.decided_at_ms = decided;
                i.status = if decided.is_some() { crate::InboxStatus::Decided } else { crate::InboxStatus::Parked };
                upsert_inbox(tx, &i)
            };
            item(1, "f-back-1", "permission", 100, Some(400))?; // 300 ms
            item(2, "f-back-1", "permission", 500, None)?; // parked, open until `until`: 500 ms
            item(3, "f-lead-1", "startup", 900, Some(950))?; // 50 ms
            item(4, "f-lead-1", "info", 0, Some(5_000))?; // not a wait
            item(5, "g-back-1", "question", 0, Some(900))?; // another run
            Ok(())
        })
        .unwrap();
        let r = s
            .run_report(&ReportQuery { team_run_id: Some("r1".into()), since_ms: None, until_ms: Some(1_000) }, 0)
            .unwrap();
        let ids: Vec<(&str, &str, &str)> =
            r.agents.iter().map(|a| (a.id.as_str(), a.role.as_str(), a.permission_mode.as_str())).collect();
        assert_eq!(ids, [("f-back-1", "back", "auto"), ("f-lead-1", "lead", "default")]);
        assert_eq!((r.prompts_by_tool.total, r.prompts_by_tool.by["Bash"], r.prompts_by_tool.by["Read"]), (3, 2, 1));
        assert_eq!(r.prompts_by_decider.total, 4);
        assert_eq!(
            (r.prompts_by_decider.by["expired"], r.prompts_by_decider.by["user"], r.prompts_by_decider.by["pane"]),
            (1, 1, 1)
        );
        assert_eq!((r.interventions.total, r.interventions.by["permission"]), (2, 1));
        assert_eq!(r.waiting_ms, 300 + 500 + 50);
        assert_eq!((r.startup_blocked, r.restarted_fresh, r.pressure_changes, r.pressure_suspends), (1, 1, 1, 1));
        let text = r.render(true);
        assert!(text.contains("permission prompts     3") && text.contains("(baseline 29)"), "{text}");
        assert!(
            text.contains("by tool: Bash 2, Read 1") && text.contains("by cause: idle_with_task 1, permission 1"),
            "{text}"
        );
        assert!(text.contains("baseline: the 2026-10-27 team run"), "{text}");
        assert!(!r.render(false).contains("baseline"));
        // A time range alone covers every agent; one ending early cuts the events.
        let all = s.run_report(&ReportQuery { until_ms: Some(12), ..Default::default() }, 0).unwrap();
        assert_eq!((all.agents.len(), all.prompts_by_tool.total), (3, 2));
    }
}
