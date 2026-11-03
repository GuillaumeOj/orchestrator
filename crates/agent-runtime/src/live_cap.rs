//! The base live cap (FR-203) and the off-hours live cap (FR-816), set in
//! Settings › Pacing policy. The base drives both the Governor's `LiveCaps`
//! (per pressure state) and the process cap under memory pressure; changing
//! it rebuilds the Governor at once.

use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use governor::jiff::Timestamp;
use serde::Serialize;
use serde_json::json;

use crate::resources::{self, PressureLevel};
use crate::runtime::Runtime;

/// Settings key of the base live cap (a number). `orchd serve --cap` wins for its run.
pub const LIVE_CAP_KEY: &str = "governor.live_cap";
pub const DEFAULT_LIVE_CAP: u32 = 3;
pub const LIVE_CAP_MIN: u32 = 1;
pub const LIVE_CAP_MAX: u32 = 20;
pub const OFF_HOURS_LIVE_CAP_MIN: u32 = 0;
pub const OFF_HOURS_LIVE_CAP_MAX: u32 = 20;

/// One change at a time: each reads the current values, then rebuilds.
static SETTING: Mutex<()> = Mutex::new(());

/// The stored base cap: the default when none is stored; `Err` (a warning
/// for the log) when the stored value is not a whole number in 1–20.
pub fn stored(db: &orch_store::Store) -> Result<u32, String> {
    let v = match db.setting(LIVE_CAP_KEY) {
        Ok(Some(v)) if !v.is_null() => v,
        Ok(_) => return Ok(DEFAULT_LIVE_CAP),
        Err(e) => return Err(format!("{LIVE_CAP_KEY}: {e:#}; using {DEFAULT_LIVE_CAP}")),
    };
    match v.as_u64().filter(|n| (LIVE_CAP_MIN as u64..=LIVE_CAP_MAX as u64).contains(n)) {
        Some(n) => Ok(n as u32),
        None => Err(format!("{LIVE_CAP_KEY} {v} is outside {LIVE_CAP_MIN}–{LIVE_CAP_MAX}; using {DEFAULT_LIVE_CAP}")),
    }
}

/// The base live cap: 1–20 (Settings, `orchd serve --cap`).
pub fn check_live_cap(n: u32) -> Result<()> {
    if !(LIVE_CAP_MIN..=LIVE_CAP_MAX).contains(&n) {
        bail!("live agents {n}: use {LIVE_CAP_MIN}–{LIVE_CAP_MAX}");
    }
    Ok(())
}

/// The off-hours live cap: 0–20 (Settings, and the Week planner's `plan_set`).
pub fn check_off_hours(n: u32) -> Result<()> {
    if !(OFF_HOURS_LIVE_CAP_MIN..=OFF_HOURS_LIVE_CAP_MAX).contains(&n) {
        bail!("off-hours live agents {n}: use {OFF_HOURS_LIVE_CAP_MIN}–{OFF_HOURS_LIVE_CAP_MAX}");
    }
    Ok(())
}

/// Both values checked before anything is written (api-contract).
pub fn check(live_cap: Option<u32>, off_hours: Option<u32>) -> Result<()> {
    live_cap.map_or(Ok(()), check_live_cap)?;
    off_hours.map_or(Ok(()), check_off_hours)
}

/// The caps a base gives, by the core's own formulas.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DerivedCaps {
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    pub catch_up: u32,
    pub memory_normal: u32,
    pub memory_warn: u32,
    pub memory_critical: u32,
}

impl DerivedCaps {
    pub fn from_base(base: u32) -> DerivedCaps {
        let g = governor::LiveCaps::from_base(base);
        let mem = |l| resources::effective_cap(base, l);
        DerivedCaps {
            green: g.green,
            amber: g.amber,
            red: g.red,
            catch_up: g.catch_up,
            memory_normal: mem(PressureLevel::Normal),
            memory_warn: mem(PressureLevel::Warn),
            memory_critical: mem(PressureLevel::Critical),
        }
    }
}

/// The cap in effect now.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapNow {
    pub state: governor::PressureState,
    /// The Governor's cap now (block weight and 5h guard included).
    pub governor_cap: u32,
    pub live_autonomous: u32,
    pub pressure: PressureLevel,
    /// The process cap now ([`Runtime::live_cap`]).
    pub process_cap: u32,
}

/// Settings › Pacing policy (api-contract `PacingSettings`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PacingSettings {
    pub live_cap: u32,
    pub live_cap_default: u32,
    pub live_cap_min: u32,
    pub live_cap_max: u32,
    pub off_hours_live_cap: u32,
    pub off_hours_live_cap_default: u32,
    pub off_hours_live_cap_min: u32,
    pub off_hours_live_cap_max: u32,
    /// From the current plan: 0 = agents don't run off-hours at all.
    pub off_hours_allowance_pct: f64,
    /// The Governor's cap is logged, not enforced (the process cap applies anyway).
    pub shadow: bool,
    pub derived: DerivedCaps,
    pub now: CapNow,
}

impl Runtime {
    pub fn pacing_settings(&self) -> PacingSettings {
        let base = self.base_cap();
        let (snap, off_hours, allowance, shadow) = self.with_governor(|g| {
            let p = g.plan();
            (g.snapshot(Timestamp::now()), g.policy().off_hours_live_cap, p.off_hours_allowance_pct, p.shadow_mode)
        });
        PacingSettings {
            live_cap: base,
            live_cap_default: DEFAULT_LIVE_CAP,
            live_cap_min: LIVE_CAP_MIN,
            live_cap_max: LIVE_CAP_MAX,
            off_hours_live_cap: off_hours,
            off_hours_live_cap_default: governor::PacingPolicy::default().off_hours_live_cap,
            off_hours_live_cap_min: OFF_HOURS_LIVE_CAP_MIN,
            off_hours_live_cap_max: OFF_HOURS_LIVE_CAP_MAX,
            off_hours_allowance_pct: allowance,
            shadow,
            derived: DerivedCaps::from_base(base),
            now: CapNow {
                state: snap.state,
                governor_cap: snap.live_cap,
                live_autonomous: snap.live_autonomous,
                pressure: self.memory_pressure(),
                process_cap: self.live_cap(),
            },
        }
    }

    /// Settings › Pacing policy: the base live cap (stored as
    /// `governor.live_cap`) and the off-hours one (the policy's). Applies now:
    /// the Governor is rebuilt once as by [`Runtime::replan`] (the week's
    /// accounting, queue, pauses and sprints carry over) and the process slots
    /// are re-synced, so work held for a slot starts if the cap went up.
    /// Lowering never stops a working agent: its next turns queue, and idle
    /// agents above the cap are suspended by housekeeping. `None`, or the
    /// current value, leaves a cap as it is.
    pub fn set_live_caps(&self, live_cap: Option<u32>, off_hours: Option<u32>) -> Result<()> {
        check(live_cap, off_hours)?;
        let _one = SETTING.lock().unwrap_or_else(|e| e.into_inner());
        let (base, old_off, plan) =
            self.with_governor(|g| (self.base_cap(), g.policy().off_hours_live_cap, g.plan().clone()));
        // The cap in effect already (`--cap` gave it): nothing to apply, but
        // the setting is the user's choice for the next start.
        if let Some(n) = live_cap.filter(|n| *n == base) {
            if self.db.setting(LIVE_CAP_KEY)?.and_then(|v| v.as_u64()) != Some(n as u64) {
                self.db.set_setting(LIVE_CAP_KEY, &json!(n))?;
            }
        }
        let live_cap = live_cap.filter(|n| *n != base);
        let off_hours = off_hours.filter(|n| *n != old_off);
        if live_cap.is_none() && off_hours.is_none() {
            return Ok(());
        }
        let now = Timestamp::now();
        if plan.reset_at <= now || plan.week_start > now {
            // The week is just rolling over (`tick` moves the Governor on).
            return Err(anyhow!("the plan week is changing: try again in a moment"));
        }
        // The stored value as it was (none, or one `--cap` overrides): a
        // failed change puts it back, never the base in effect.
        let old_stored = match live_cap {
            Some(_) => Some(self.db.setting(LIVE_CAP_KEY)?.unwrap_or(serde_json::Value::Null)),
            None => None,
        };
        // Reads the base and the stored policy. Failed at any step: nothing changes.
        let applied = (|| {
            if let Some(n) = live_cap {
                self.db.set_setting(LIVE_CAP_KEY, &json!(n))?;
                self.set_base_cap(n);
            }
            if let Some(n) = off_hours {
                self.update_policy(|p| p.off_hours_live_cap = n)?;
            }
            self.replan(plan)
        })();
        if let Err(e) = applied {
            if let Some(old) = &old_stored {
                self.set_base_cap(base);
                let _ = self.db.set_setting(LIVE_CAP_KEY, old);
            }
            if off_hours.is_some() {
                let _ = self.update_policy(|p| p.off_hours_live_cap = old_off);
            }
            return Err(e);
        }
        if let Some(n) = live_cap {
            self.event("-", "live_cap_set", json!({"from": base, "to": n, "by": "user"}));
        }
        if let Some(n) = off_hours {
            self.event("-", "off_hours_live_cap_set", json!({"from": old_off, "to": n, "by": "user"}));
        }
        self.sync_process_slots();
        self.touch(orch_protocol::Topic::Usage);
        self.touch(orch_protocol::Topic::Planner);
        // Sent now, not at the next delivery pass.
        self.now_pump(false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::{hook, insert_test_live, test_agent, test_cfg, test_runtime};
    use crate::state::TurnOrigin;
    use governor::{AgentId, LiveCaps, QueueReason, TurnKind};

    fn events(rt: &Runtime, kind: &str) -> Vec<serde_json::Value> {
        rt.db.flush().unwrap();
        rt.db.events(0, None, Some(kind), 10).unwrap().into_iter().map(|e| e.detail).collect()
    }

    #[test]
    fn the_setting_is_read_at_start_and_cap_overrides_it() {
        let d = tempfile::Builder::new().prefix("orchlc").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let start = |cap: Option<u32>| {
            let mut cfg = test_cfg(d.path(), true);
            cfg.base_cap = cap;
            Runtime::new(cfg).unwrap()
        };
        // Nothing stored: 3.
        let rt = start(None);
        assert_eq!((rt.base_cap(), rt.live_cap()), (3, 3));
        rt.db.set_setting(LIVE_CAP_KEY, &json!(6)).unwrap();
        rt.db.flush().unwrap();
        drop(rt);
        // The setting, in both caps.
        let rt = start(None);
        assert_eq!((rt.base_cap(), rt.live_cap()), (6, 6));
        assert_eq!(rt.with_governor(|g| g.policy().live_caps.clone()), LiveCaps::from_base(6));
        drop(rt);
        // `--cap` wins for its run, and is not stored.
        let rt = start(Some(2));
        assert_eq!((rt.base_cap(), rt.pacing_settings().live_cap), (2, 2));
        assert_eq!(rt.with_governor(|g| g.policy().live_caps.green), 2);
        assert_eq!(rt.db.setting(LIVE_CAP_KEY).unwrap(), Some(json!(6)));
        // Set to the `--cap` value: no change now, but stored for the next start.
        rt.set_live_caps(Some(2), None).unwrap();
        assert_eq!(rt.db.setting(LIVE_CAP_KEY).unwrap(), Some(json!(2)));
        assert!(events(&rt, "live_cap_set").is_empty());
        drop(rt);
        // A bad `--cap` fails the start.
        let mut cfg = test_cfg(d.path(), true);
        cfg.base_cap = Some(21);
        assert_eq!(Runtime::new(cfg).err().unwrap().to_string(), "live agents 21: use 1–20");
        // A stored value out of range: ignored with a warning.
        for bad in [json!(0), json!(25), json!("lots")] {
            let rt = start(None);
            rt.db.set_setting(LIVE_CAP_KEY, &bad).unwrap();
            rt.db.flush().unwrap();
            drop(rt);
            let rt = start(None);
            assert_eq!(rt.base_cap(), DEFAULT_LIVE_CAP, "{bad}");
            assert!(rt.warnings.iter().any(|w| w.contains(LIVE_CAP_KEY)), "{:?}", rt.warnings);
        }
    }

    #[test]
    fn a_change_applies_now_and_logs_once() {
        let (_d, rt) = test_runtime(true);
        let caps = |rt: &Runtime| rt.with_governor(|g| (g.policy().live_caps.clone(), g.policy().off_hours_live_cap));
        rt.set_live_caps(Some(6), Some(2)).unwrap();
        assert_eq!(caps(&rt), (LiveCaps::from_base(6), 2));
        assert_eq!(rt.live_cap(), 6);
        let v = rt.pacing_settings();
        assert_eq!((v.live_cap, v.off_hours_live_cap, v.derived.catch_up, v.now.process_cap), (6, 2, 8, 6));
        let state = rt.with_governor(|g| g.snapshot(Timestamp::now()).state);
        assert_eq!(v.now.governor_cap, LiveCaps::from_base(6).for_state(state), "the snapshot follows");
        assert_eq!(events(&rt, "live_cap_set"), vec![json!({"from": 3, "to": 6, "by": "user"})]);
        assert_eq!(events(&rt, "off_hours_live_cap_set"), vec![json!({"from": 1, "to": 2, "by": "user"})]);
        // Stored: the next start keeps both.
        assert_eq!(rt.db.setting(LIVE_CAP_KEY).unwrap(), Some(json!(6)));
        assert_eq!(rt.db.policy(crate::runtime::POLICY_ID).unwrap().unwrap().off_hours_live_cap, 2);
        // The same values: a no-op (no event).
        rt.set_live_caps(Some(6), Some(2)).unwrap();
        rt.set_live_caps(None, None).unwrap();
        assert_eq!(events(&rt, "live_cap_set").len(), 1);
        // Refused: nothing changes, the other field included.
        for (cap, off, err) in
            [(Some(0), Some(3), "live agents 0: use 1–20"), (Some(4), Some(25), "off-hours live agents 25: use 0–20")]
        {
            assert_eq!(rt.set_live_caps(cap, off).unwrap_err().to_string(), err);
            assert_eq!(caps(&rt), (LiveCaps::from_base(6), 2));
        }
        assert_eq!(events(&rt, "live_cap_set").len(), 1);
    }

    #[test]
    fn work_held_for_a_process_slot_starts_when_the_cap_goes_up() {
        let (d, rt) = test_runtime(true);
        rt.set_live_caps(Some(1), None).unwrap();
        // Another agent holds the only slot; a1's spawn waits for one.
        insert_test_live(&rt, "a2", None, 0, d.path());
        let a1 = AgentId("a1".into());
        rt.governor_act(|g, now| (g.hold_spawn(&a1, &"default".into(), governor::Tier::Normal, 1, 1, now), Vec::new()));
        rt.sync_process_slots();
        let queued =
            |rt: &Runtime| rt.with_governor(|g| g.queue().iter().map(|e| e.reason.clone()).collect::<Vec<_>>());
        assert_eq!(queued(&rt), vec![QueueReason::MemoryPressure { cap: 1 }]);
        rt.set_live_caps(Some(2), None).unwrap();
        assert!(queued(&rt).is_empty(), "released by the higher cap");
    }

    #[test]
    fn lowering_never_stops_a_working_agent() {
        let (d, rt) = test_runtime(false);
        let mut a2 = test_agent(d.path());
        a2.id = "a2".into();
        rt.store.write(|s| s.agents.insert("a2".into(), a2)).unwrap();
        // a1 runs an app turn.
        let rec = rt.store.read(|s| s.agents["a1"].clone());
        assert!(rt.decide_turn(&rec, TurnKind::Message, TurnOrigin::App, false).effective.is_admit());
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None);
        rt.set_live_caps(Some(1), None).unwrap();
        // Still running, still counted, its tools still admitted.
        assert!(rt.with_governor(|g| g.is_live(&AgentId("a1".into()))));
        assert!(rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), None).is_none(), "tool admitted");
        // Another agent's turn now waits for the lower cap.
        let rec = rt.store.read(|s| s.agents["a2"].clone());
        let d = rt.decide_turn(&rec, TurnKind::Message, TurnOrigin::App, false);
        assert!(!d.effective.is_admit());
        let reason = rt.with_governor(|g| g.queue().iter().find(|e| e.agent.0 == "a2").map(|e| e.reason.clone()));
        assert!(matches!(reason, Some(QueueReason::LiveCap { cap: 1, .. })), "{reason:?}");
    }

    #[test]
    fn derived_caps_follow_the_core_formulas() {
        let d = DerivedCaps::from_base(3);
        assert_eq!(
            (d.green, d.amber, d.red, d.catch_up, d.memory_normal, d.memory_warn, d.memory_critical),
            (3, 2, 1, 5, 3, 2, 1)
        );
        let d = DerivedCaps::from_base(1);
        assert_eq!((d.green, d.amber, d.red, d.catch_up, d.memory_warn), (1, 1, 1, 3, 1));
        let d = DerivedCaps::from_base(7);
        assert_eq!((d.amber, d.catch_up, d.memory_warn), (4, 9, 6));
    }

    #[test]
    fn bounds_and_their_texts() {
        assert!(check(Some(1), Some(0)).is_ok());
        assert!(check(Some(20), Some(20)).is_ok());
        assert!(check(None, None).is_ok());
        assert_eq!(check(Some(0), None).unwrap_err().to_string(), "live agents 0: use 1–20");
        assert_eq!(check(Some(21), None).unwrap_err().to_string(), "live agents 21: use 1–20");
        assert_eq!(check(None, Some(25)).unwrap_err().to_string(), "off-hours live agents 25: use 0–20");
        // The base is checked first.
        assert_eq!(check(Some(0), Some(25)).unwrap_err().to_string(), "live agents 0: use 1–20");
    }
}
