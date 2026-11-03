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
        None => Err(format!(
            "{LIVE_CAP_KEY} {v} is outside {LIVE_CAP_MIN}–{LIVE_CAP_MAX}; using {DEFAULT_LIVE_CAP}"
        )),
    }
}

/// Both values checked before anything is written (api-contract).
pub fn check(live_cap: Option<u32>, off_hours: Option<u32>) -> Result<()> {
    if let Some(n) = live_cap.filter(|n| !(LIVE_CAP_MIN..=LIVE_CAP_MAX).contains(n)) {
        bail!("live agents {n}: use {LIVE_CAP_MIN}–{LIVE_CAP_MAX}");
    }
    if let Some(n) = off_hours.filter(|n| *n > OFF_HOURS_LIVE_CAP_MAX) {
        bail!("off-hours live agents {n}: use 0–{OFF_HOURS_LIVE_CAP_MAX}");
    }
    Ok(())
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
        if let Some(n) = live_cap {
            self.db.set_setting(LIVE_CAP_KEY, &json!(n))?;
            self.set_base_cap(n);
        }
        if let Some(n) = off_hours {
            self.update_policy(|p| p.off_hours_live_cap = n)?;
        }
        // Reads the base and the stored policy. Failed: nothing changes.
        if let Err(e) = self.replan(plan) {
            if live_cap.is_some() {
                self.set_base_cap(base);
                let _ = self.db.set_setting(LIVE_CAP_KEY, &json!(base));
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
