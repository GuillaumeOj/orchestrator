//! Measuring the exit criterion (P4.5 spec §3.7): the week's starts from
//! the `intake.*` events, for the weekly report and `orchd intake`.

use std::collections::BTreeMap;

use anyhow::Result;
use governor::jiff::Timestamp;
use serde_json::{Value, json};

use crate::report::{IntakeCounts, UntrackedStart};
use crate::runtime::Runtime;

const KINDS: &[&str] = &["intake.started", "intake.planned_started", "intake.untracked_start", "intake.returned"];
const UNTRACKED_SHOWN: usize = 5;

/// The starts in the events (oldest first): an item given back no longer
/// counts as started.
pub fn counts(events: &[(String, Value)]) -> IntakeCounts {
    let mut c = IntakeCounts::default();
    let mut started: BTreeMap<String, i64> = BTreeMap::new();
    for (kind, d) in events {
        let item = d["item"].as_str().unwrap_or("").to_string();
        match kind.as_str() {
            "intake.started" => *started.entry(item).or_default() += 1,
            "intake.returned" => *started.entry(item).or_default() -= 1,
            "intake.planned_started" => c.planned += 1,
            "intake.untracked_start" => {
                c.untracked += 1;
                if c.untracked_items.len() < UNTRACKED_SHOWN {
                    c.untracked_items.push(UntrackedStart {
                        kind: d["kind"].as_str().unwrap_or("").into(),
                        reference: d["ref"].as_str().unwrap_or("").into(),
                    });
                }
            }
            _ => {}
        }
    }
    c.started = started.values().map(|n| (*n).max(0) as u32).sum();
    c
}

/// "started from intake: 9 of 10 (1 untracked: agent fix-ci-3)".
pub fn week_line(c: &IntakeCounts) -> String {
    let from = c.started + c.planned;
    let mut l = format!("started from intake: {from} of {}", from + c.untracked);
    if c.planned > 0 {
        l.push_str(&format!(" ({} planned with the Overwatch)", c.planned));
    }
    if c.untracked > 0 {
        let names: Vec<String> = c.untracked_items.iter().map(|u| format!("{} {}", u.kind, u.reference)).collect();
        let more = (c.untracked as usize).saturating_sub(names.len());
        let more = if more > 0 { format!(", +{more}") } else { String::new() };
        l.push_str(&format!(" ({} untracked: {}{more})", c.untracked, names.join(", ")));
    }
    l
}

/// `orchd intake`: one line per project, then the week's line.
pub fn render(r: &Value, tz: &governor::jiff::tz::TimeZone) -> String {
    let mut out = String::from("Intake\n");
    let now = r["now_ms"].as_u64().unwrap_or(0);
    for p in r["projects"].as_array().into_iter().flatten() {
        let s = |k: &str| p[k].as_str().unwrap_or("");
        out.push_str(&format!("  {} ({}): ", s("name"), s("project")));
        if s("status") == "none" {
            out.push_str("no source\n");
            continue;
        }
        out.push_str(&format!("{} · {}", s("label"), s("status")));
        if let Some(e) = p["error"]["code"].as_str() {
            out.push_str(&format!(" {e}: {}", p["error"]["message"].as_str().unwrap_or("")));
        }
        out.push_str(&format!(
            " · {} ready, {} more, {} hidden, {} started",
            p["count"].as_u64().unwrap_or(0),
            p["more"].as_u64().unwrap_or(0),
            p["hidden"].as_u64().unwrap_or(0),
            p["started"].as_u64().unwrap_or(0)
        ));
        if let Some(t) = p["refreshed_at_ms"].as_u64() {
            let mins = now.saturating_sub(t) / 60_000;
            let at = Timestamp::from_millisecond(t as i64).unwrap_or_default().to_zoned(tz.clone());
            out.push_str(&format!(" · updated {} ({mins} min ago)", at.strftime("%a %H:%M")));
        }
        out.push('\n');
    }
    if let Some(line) = r["week"]["line"].as_str() {
        out.push_str(&format!("This week: {line}\n"));
    }
    out
}

impl Runtime {
    /// The starts with `from_ms <= ts < to_ms`.
    pub(crate) fn intake_counts(&self, from_ms: u64, to_ms: u64) -> IntakeCounts {
        let events = self.db.events_of_kinds_in(from_ms, to_ms, KINDS).unwrap_or_default();
        counts(&events.into_iter().map(|(e, _)| (e.kind, e.detail)).collect::<Vec<_>>())
    }

    /// `orchd intake [--week]`: every project's source and counts; with
    /// `week`, this week's starts too.
    pub fn intake_report(&self, week: bool) -> Result<Value> {
        let view = self.intake_view();
        let started = self.db.intake_started()?;
        let names: BTreeMap<String, String> = self.db.projects(false)?.into_iter().map(|p| (p.id, p.name)).collect();
        let projects: Vec<Value> = view["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let id = s["project"].as_str().unwrap_or("");
                let mut v = s.clone();
                v["name"] = json!(names.get(id).cloned().unwrap_or_else(|| id.to_string()));
                v["started"] = json!(started.iter().filter(|r| r.project_id == id).count());
                v
            })
            .collect();
        let mut out = json!({"now_ms": orch_protocol::now_ms(), "projects": projects});
        if week {
            let plan = self.with_governor(|g| g.plan().clone());
            let ms = |t: Timestamp| t.as_millisecond().max(0) as u64;
            let c = self.intake_counts(ms(plan.week_start), ms(plan.reset_at));
            out["week"] = json!({"week_start_ms": ms(plan.week_start), "reset_at_ms": ms(plan.reset_at),
                                 "line": week_line(&c), "intake": c});
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_returned_item_does_not_count_and_untracked_are_named() {
        let ev = |k: &str, d: Value| (k.to_string(), d);
        let c = counts(&[
            ev("intake.started", json!({"item": "it_1"})),
            ev("intake.started", json!({"item": "it_2"})),
            ev("intake.returned", json!({"item": "it_2", "why": "run_deleted"})),
            ev("intake.planned_started", json!({"decision": "brief-3"})),
            ev("intake.untracked_start", json!({"kind": "agent", "ref": "fix-ci-3"})),
        ]);
        assert_eq!((c.started, c.planned, c.untracked), (1, 1, 1));
        assert_eq!(
            week_line(&c),
            "started from intake: 2 of 3 (1 planned with the Overwatch) (1 untracked: agent fix-ci-3)"
        );
        assert_eq!(week_line(&IntakeCounts::default()), "started from intake: 0 of 0");
    }
}
