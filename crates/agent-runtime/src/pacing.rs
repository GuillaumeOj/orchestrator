//! P3 pacing in the runtime: the model a turn runs on and the per-role
//! downgrade at task boundaries (FR-815, FR-819), work scheduled off-hours
//! (FR-816, decision D4) and the power assertion that keeps the Mac awake
//! for it on AC.

use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use governor::AgentId;
use governor::jiff::Timestamp;
use orch_protocol::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::AgentRecord;

/// Settings key: tasks and team runs the user scheduled off-hours.
pub const OFF_HOURS_KEY: &str = "governor.off_hours";
/// How often the power source is read while off-hours work waits.
pub const POWER_EVERY_MS: u64 = 60 * 1000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OffHoursMarks {
    #[serde(default)]
    pub tasks: BTreeSet<u64>,
    #[serde(default)]
    pub runs: BTreeSet<String>,
}

/// What the power side last saw (`app_state.power`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PowerState {
    pub on_ac: Option<bool>,
    pub assertion_held: bool,
    pub off_hours_pending: usize,
}

impl Runtime {
    /// The model an agent's next turn runs on: a downgrade in force, else its role's.
    pub(crate) fn turn_model(&self, rec: &AgentRecord) -> Option<String> {
        let o = self.with_governor(|g| g.model_override(&AgentId(rec.id.clone())).map(|o| o.to.clone()));
        o.or_else(|| rec.role.model.clone())
    }

    /// The agent header's note while a downgrade is in force ("downgraded (Red)").
    pub fn model_note(&self, id: &str) -> Option<String> {
        self.with_governor(|g| g.model_override(&AgentId(id.into())).map(|o| o.note()))
    }

    /// FR-819 at a task boundary (the agent's next turn is a new task): the
    /// Governor decides the model of the task; an agent running another one
    /// is suspended while idle (never mid-turn) and resumed with `--model`
    /// by the pump. True while the switch is under way (the task waits).
    pub(crate) fn task_boundary(&self, rec: &AgentRecord) -> bool {
        let agent = AgentId(rec.id.clone());
        let rule = rec.role.downgrade.as_ref();
        if rule.is_none() && self.with_governor(|g| g.model_override(&agent).is_none()) {
            return false;
        }
        let model = rec.role.model.as_deref();
        // A look first: the write (and its carry save) only when something changes.
        let settled = self.with_governor(|g| g.task_boundary_settled(&agent, model, rule, Timestamp::now()));
        let want = match settled {
            Some(m) => m,
            None => self.governor_act(|g, now| (g.task_boundary_model(&agent, model, rule, now), Vec::new())),
        }
        .or_else(|| rec.role.model.clone());
        let Some(l) = self.live(&rec.id) else { return false };
        if l.model == want {
            return false;
        }
        if rec.tracker.state == AgentState::Idle {
            let from = l.model.clone();
            match self.suspend(&rec.id) {
                Ok(_) => self.event(&rec.id, "model_switch", json!({"from": from, "to": want})),
                Err(e) => {
                    // The task runs on the current model rather than wait for
                    // a switch that keeps failing (and log it at every pump).
                    self.event(&rec.id, "model_switch_failed", json!(e.to_string()));
                    return false;
                }
            }
        }
        true
    }

    /// The marks, read from the store once and kept in memory (written by
    /// [`Runtime::mark_off_hours`], the only writer).
    pub fn off_hours_marks(&self) -> OffHoursMarks {
        if let Some(m) = self.house().off_hours_marks.clone() {
            return m;
        }
        let m: OffHoursMarks = self
            .db
            .setting(OFF_HOURS_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        self.house().off_hours_marks = Some(m.clone());
        m
    }

    /// Marks (or clears) a task or a team run for off-hours.
    pub(crate) fn mark_off_hours(&self, task: Option<u64>, run: Option<&str>, on: bool) -> Result<()> {
        let mut m = self.off_hours_marks();
        if let Some(t) = task {
            if on {
                m.tasks.insert(t)
            } else {
                m.tasks.remove(&t)
            };
        }
        if let Some(r) = run {
            if on {
                m.runs.insert(r.to_string())
            } else {
                m.runs.remove(r)
            };
        }
        self.db.set_setting(OFF_HOURS_KEY, &serde_json::to_value(&m)?)?;
        self.house().off_hours_marks = Some(m);
        Ok(())
    }

    /// Before an app turn is decided: a team agent on a task (or run) marked
    /// off-hours has its work scheduled off-hours again (the Governor clears
    /// the mark when a scheduled turn ends).
    pub(crate) fn sync_off_hours(&self, rec: &AgentRecord) {
        let m = self.off_hours_marks();
        if m.tasks.is_empty() && m.runs.is_empty() {
            return;
        }
        let marked = match &rec.team {
            Some(seat) => m.runs.contains(&seat.run_id) || seat.task.is_some_and(|t| m.tasks.contains(&t)),
            // An agent outside a team: a marked open task assigned to it.
            None => m.tasks.iter().any(|t| {
                self.db.task(*t).ok().flatten().is_some_and(|t| {
                    t.agent_id.as_deref() == Some(rec.id.as_str()) && t.status != orch_team::TaskStatus::Done
                })
            }),
        };
        if marked {
            let agent = AgentId(rec.id.clone());
            let _ = self.governor_act(|g, now| (g.set_off_hours(&agent, true, now), Vec::new()));
        }
    }

    /// `queue_off_hours`: mark a queued entry or a task (its next turns) for
    /// off-hours, or clear it. Returns when it may start.
    pub fn queue_off_hours(&self, entry: Option<u64>, task: Option<u64>, off: bool) -> Result<Timestamp> {
        let mut agents = vec![];
        if let Some(e) = entry {
            let a = self
                .with_governor(|g| g.queue().iter().find(|q| q.id.0 == e).map(|q| q.agent.clone()))
                .ok_or_else(|| anyhow!("no queued entry {e}"))?;
            if !off {
                // Clear its task's mark too, or the next turn would be scheduled again.
                let t = self.store.read(|s| s.agents.get(&a.0).and_then(|r| r.team.as_ref()?.task));
                if let Some(t) = t {
                    self.mark_off_hours(Some(t), None, false)?;
                }
            }
            agents.push(a);
        }
        if let Some(t) = task {
            let tk = self.db.task(t)?.ok_or_else(|| anyhow!("no task {t}"))?;
            if off && self.with_governor(|g| g.plan().off_hours_allowance_pct) <= 0.0 {
                bail!("{}", governor::GovernorError::NoOffHoursAllowance);
            }
            self.mark_off_hours(Some(t), None, off)?;
            agents.extend(tk.agent_id.map(AgentId));
        }
        if entry.is_none() && task.is_none() {
            bail!("queue_off_hours needs an entry or a task");
        }
        for a in &agents {
            self.governor_act(|g, now| (g.set_off_hours(a, off, now), Vec::new())).map_err(|e| anyhow!("{e}"))?;
        }
        // Clearing may let the work run now.
        self.governor_act(|g, now| ((), g.tick(now)));
        self.power_tick(true);
        Ok(self.with_governor(|g| g.next_off_hours(Timestamp::now())))
    }

    /// FR-816: holds the power assertion while off-hours work is queued or
    /// running and the Mac is on AC; releases it otherwise. On battery, one
    /// Inbox alert per night. `force` reads the power source now.
    pub fn power_tick(&self, force: bool) {
        let pending = self.with_governor(|g| g.off_hours_pending());
        let held = self.cfg.power.held();
        let mut on_ac = self.house().power.on_ac;
        if pending > 0 {
            let due = {
                let mut h = self.house();
                let d = force || now_ms().saturating_sub(h.last_power_ms) >= POWER_EVERY_MS || h.power.on_ac.is_none();
                if d {
                    h.last_power_ms = now_ms();
                }
                d
            };
            if due {
                on_ac = Some(self.cfg.conditions.battery().is_none());
            }
        }
        let want = pending > 0 && on_ac == Some(true);
        if want && !held {
            match self.cfg.power.acquire() {
                Ok(()) => self.event("-", "power_assertion", json!({"held": true, "pending": pending})),
                Err(e) => self.event("-", "power_assertion_failed", json!(e.to_string())),
            }
        } else if !want && held {
            self.cfg.power.release();
            self.event("-", "power_assertion", json!({"held": false, "pending": pending, "on_ac": on_ac}));
        }
        if pending > 0 && on_ac == Some(false) {
            // One per night: an evening and the morning after are the same night.
            let night = Timestamp::now()
                .checked_sub(governor::jiff::SignedDuration::from_hours(12))
                .unwrap_or(Timestamp::now())
                .to_zoned(governor::jiff::tz::TimeZone::system())
                .date()
                .to_string();
            let first = self.house().battery_alerted.as_deref() != Some(night.as_str());
            if first {
                let body = "Not on AC power\nOff-hours work won't run while the Mac sleeps. Plug it in to keep it awake for the queued work.".to_string();
                let payload = json!({"alert": "on_battery", "on_battery": true, "queued": pending});
                if self.add_item(None, "alert", body, payload, 0).is_ok() {
                    self.house().battery_alerted = Some(night);
                }
            }
        }
        let power = PowerState { on_ac, assertion_held: self.cfg.power.held(), off_hours_pending: pending };
        let changed = std::mem::replace(&mut self.house().power, power.clone()) != power;
        if changed {
            self.touch("power");
            self.touch("banners");
        }
    }

    /// `app_state.power`, and the `not_on_ac` banner while off-hours work waits on battery.
    pub fn power_json(&self) -> (Value, Option<Value>) {
        let p = self.house().power.clone();
        let banner = (p.off_hours_pending > 0 && p.on_ac == Some(false)).then(|| json!({"kind": "not_on_ac"}));
        (json!(p), banner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use governor::{Downgrade, DowngradeWhen, QueueReason, Tier, TurnKind, Verdict};
    use orch_protocol::StatusMsg;

    use super::*;
    use crate::power::{FakePower, PowerAssertion};
    use crate::runtime::tests::{test_agent, test_cfg};
    use crate::state::TurnOrigin;

    #[derive(Debug, Default)]
    struct Battery(AtomicBool);

    impl orch_git::fetch::Conditions for Battery {
        fn battery(&self) -> Option<u8> {
            self.0.load(Ordering::SeqCst).then_some(80)
        }
        fn online(&self) -> bool {
            true
        }
    }

    fn agent(rt: &Runtime, d: &std::path::Path, id: &str, model: &str, downgrade: Option<Downgrade>) {
        let mut a = test_agent(d);
        a.id = id.into();
        a.role.model = Some(model.into());
        a.role.downgrade = downgrade;
        rt.store.write(|s| s.agents.insert(id.into(), a)).unwrap();
    }

    /// Spec §6 item 7: a synthetic 95 % opus window from the statusline
    /// queues opus turns with "model window"; a role with a downgrade
    /// switches to sonnet at its next task and runs.
    #[test]
    fn a_tight_opus_window_queues_opus_and_downgrades_at_the_next_task() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
        let rule = Downgrade { to: "sonnet".into(), when: DowngradeWhen::ModelWindow };
        agent(&rt, d.path(), "a1", "opus", None);
        agent(&rt, d.path(), "a2", "opus", Some(rule));
        let reset = rt.with_governor(|g| g.plan().reset_at.as_second());
        rt.on_status(&StatusMsg {
            agent_id: "a1".into(),
            ts_ms: now_ms(),
            payload: json!({"session_id": "s", "model": {"id": "claude-opus-4-1"}, "cost": {"total_cost_usd": 0.5},
                "rate_limits": {"seven_day": {"used_percentage": 40, "resets_at": reset},
                    "seven_day_opus": {"used_percentage": 95, "resets_at": reset}}}),
        });
        let w = rt.with_governor(|g| g.snapshot(Timestamp::now()).model_windows);
        assert_eq!((w[0].name.as_str(), w[0].used_pct), ("seven_day_opus", 95.0));
        rt.db.flush().unwrap();
        let rows = rt.db.usage_since(0, Some(orch_store::UsageSource::Statusline)).unwrap();
        assert_eq!(rows[0].model_windows.len(), 1, "stored for the replay (migration 8)");

        let rec1 = rt.store.read(|s| s.agents["a1"].clone());
        let d1 = rt.decide_turn(&rec1, TurnKind::Task, TurnOrigin::App, false);
        assert!(
            matches!(&d1.effective, Verdict::Queue { reason: QueueReason::ModelWindow { model, .. }, .. } if model == "opus"),
            "{d1:?}"
        );

        // a2's next task: the pump reaches its task boundary first.
        rt.deliver_kind("a2", "ORCH: task T-1", TurnKind::Task).unwrap();
        rt.pump();
        let rec2 = rt.store.read(|s| s.agents["a2"].clone());
        assert_eq!(rt.turn_model(&rec2).as_deref(), Some("sonnet"));
        assert_eq!(rt.model_note("a2").as_deref(), Some("downgraded (model window)"));
        let d2 = rt.decide_turn(&rec2, TurnKind::Task, TurnOrigin::App, false);
        assert_eq!(d2.effective, Verdict::Admit, "the task runs on sonnet");
        assert!(rt.with_governor(|g| g.actions().iter().any(|a| a.kind_name() == "downgrade")));
        assert_eq!(rt.model_note("a1"), None);
    }

    /// Spec §6 item 8: a queued entry marked "Run off-hours" waits for its
    /// window; the power assertion is held on AC while it waits and let go
    /// on battery (banner, one alert) and once nothing waits.
    #[test]
    fn off_hours_work_waits_and_holds_the_power_assertion_on_ac() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let power = Arc::new(FakePower::default());
        let battery = Arc::new(Battery::default());
        let mut cfg = test_cfg(d.path(), false);
        cfg.power = power.clone();
        cfg.conditions = battery.clone();
        let rt = Runtime::new(cfg).unwrap();
        agent(&rt, d.path(), "a1", "sonnet", None);
        let rec = rt.store.read(|s| s.agents["a1"].clone());

        // Queue a1's turn (paused), then mark the entry off-hours.
        rt.pause_all();
        let Verdict::Queue { entry: Some(e), .. } =
            rt.decide_turn(&rec, TurnKind::Task, TurnOrigin::App, false).effective
        else {
            panic!("queued while paused")
        };
        let err = rt.queue_off_hours(Some(e.0), None, true).unwrap_err().to_string();
        assert!(err.contains("off-hours allowance"), "{err}");
        let plan = rt.with_governor(|g| g.plan().clone());
        rt.replan(governor::WeeklyPlan { off_hours_allowance_pct: 5.0, ..plan }).unwrap();
        let starts = rt.queue_off_hours(Some(e.0), None, true).unwrap();
        // The test plan works all day until 23:59:59 of a zone where it is now about noon.
        let now = Timestamp::now();
        assert!(starts > now && starts.as_second() - now.as_second() <= 13 * 3600, "{starts}");
        rt.resume_all();
        let reason = rt.with_governor(|g| g.queue()[0].reason.clone());
        assert_eq!(reason, QueueReason::ScheduledOffHours { starts_at: starts });

        // On AC: held.
        rt.power_tick(true);
        assert!(power.held() && power.acquired.load(Ordering::SeqCst) == 1);
        let (p, banner) = rt.power_json();
        assert_eq!(
            (p["on_ac"].clone(), p["assertion_held"].clone(), p["off_hours_pending"].clone()),
            (json!(true), json!(true), json!(1))
        );
        assert!(banner.is_none());

        // On battery: released, the banner, one Inbox alert per night.
        battery.0.store(true, Ordering::SeqCst);
        rt.power_tick(true);
        rt.power_tick(true);
        assert!(!power.held() && power.released.load(Ordering::SeqCst) == 1);
        assert_eq!(rt.power_json().1, Some(json!({"kind": "not_on_ac"})));
        let alerts = rt.store.read(|s| s.inbox.iter().filter(|i| i.body.starts_with("Not on AC power")).count());
        assert_eq!(alerts, 1);

        // Back on AC, then cleared: nothing waits, nothing held.
        battery.0.store(false, Ordering::SeqCst);
        rt.power_tick(true);
        assert!(power.held());
        rt.queue_off_hours(Some(e.0), None, false).unwrap();
        assert!(!power.held() && power.released.load(Ordering::SeqCst) == 2);
        assert!(rt.power_json().1.is_none());
        let tier = rt.with_governor(|g| g.queue().first().map(|q| q.tier));
        assert!(tier.is_none_or(|t| t == Tier::Normal));

        // A task (of an agent outside a team) marked off-hours: every turn of it waits,
        // and cancelling its queued work lets the assertion go.
        let mut t = orch_store::Task::new("p", "local", "Long refactor", 1);
        t.agent_id = Some("a1".into());
        let tid = rt.db.upsert_task(&t).unwrap();
        rt.queue_off_hours(None, Some(tid), true).unwrap();
        for _ in 0..2 {
            let Verdict::Queue { entry: Some(e), reason, .. } =
                rt.decide_turn(&rec, TurnKind::Task, TurnOrigin::App, false).effective
            else {
                panic!("waits")
            };
            assert!(matches!(reason, QueueReason::ScheduledOffHours { .. }), "{reason:?}");
            rt.power_tick(true);
            assert!(power.held());
            rt.cancel(e.0).unwrap();
            rt.power_tick(true);
            assert!(!power.held(), "nothing waits once it is cancelled");
        }
        // A deleted agent leaves no mark behind.
        let a1 = AgentId("a1".into());
        rt.governor_act(|g, now| (g.set_off_hours(&a1, true, now).unwrap(), Vec::new()));
        rt.governor_forget("a1");
        rt.with_governor(|g| assert!(g.off_hours_agents().is_empty()));
    }
}
