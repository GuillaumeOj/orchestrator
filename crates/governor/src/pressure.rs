//! Pressure states with hysteresis and minimum dwell (ADR-007, FR-804).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::plan::Thresholds;
use crate::types::add_hours;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureState {
    Green,
    Amber,
    Red,
    CatchUp,
}

impl std::fmt::Display for PressureState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PressureState::Green => "green",
            PressureState::Amber => "amber",
            PressureState::Red => "red",
            PressureState::CatchUp => "catch-up",
        })
    }
}

/// What the state machine looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressureInputs {
    /// `e = W − plan(t)`, pp.
    pub error_pp: f64,
    /// Projected time W reaches 100 %, `None` if not before the reset.
    pub eta: Option<Timestamp>,
    pub reset_at: Timestamp,
    /// Weighted hours left in the week.
    pub hours_left: f64,
}

/// The state machine. Every transition waits for the minimum dwell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateMachine {
    pub state: PressureState,
    pub since: Timestamp,
}

impl StateMachine {
    pub fn new(now: Timestamp) -> Self {
        StateMachine { state: PressureState::Green, since: now }
    }

    /// Earliest time the state may change.
    pub fn dwell_end(&self, t: &Thresholds) -> Timestamp {
        add_hours(self.since, t.min_dwell_min as f64 / 60.0)
    }

    /// The state the inputs call for, ignoring dwell.
    pub fn target(&self, i: &PressureInputs, t: &Thresholds) -> PressureState {
        let red_line = add_hours(i.reset_at, -t.red_eta_margin_h);
        let red_enter = i.error_pp > t.red_enter || i.eta.is_some_and(|eta| eta < red_line);
        let red_exit_ok = i.error_pp < t.red_exit && i.eta.is_none_or(|eta| eta >= i.reset_at);
        let catch_enter = i.error_pp < t.catchup_enter && i.hours_left < t.catchup_hours_left;
        let fresh = || {
            if catch_enter {
                PressureState::CatchUp
            } else if i.error_pp > t.amber_enter {
                PressureState::Amber
            } else {
                PressureState::Green
            }
        };
        match self.state {
            PressureState::Red if !red_exit_ok => PressureState::Red,
            PressureState::Red => fresh(),
            _ if red_enter => PressureState::Red,
            PressureState::CatchUp if i.error_pp <= t.catchup_exit => PressureState::CatchUp,
            PressureState::Amber if !catch_enter && i.error_pp >= t.amber_exit => PressureState::Amber,
            _ => fresh(),
        }
    }

    /// Apply the dwell rule; returns `(from, to)` on a transition.
    pub fn step(
        &mut self,
        i: &PressureInputs,
        t: &Thresholds,
        now: Timestamp,
    ) -> Option<(PressureState, PressureState)> {
        if now < self.dwell_end(t) {
            return None;
        }
        let next = self.target(i, t);
        if next == self.state {
            return None;
        }
        let from = self.state;
        self.state = next;
        self.since = now;
        Some((from, next))
    }

    /// Force a state (new week).
    pub fn reset(&mut self, now: Timestamp) {
        self.state = PressureState::Green;
        self.since = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(m: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + m * 60).unwrap()
    }

    fn inputs(e: f64) -> PressureInputs {
        PressureInputs { error_pp: e, eta: None, reset_at: ts(100_000), hours_left: 40.0 }
    }

    fn at(state: PressureState) -> StateMachine {
        StateMachine { state, since: ts(0) }
    }

    #[test]
    fn green_to_amber_and_back_with_hysteresis() {
        let t = Thresholds::default();
        let mut sm = at(PressureState::Green);
        assert_eq!(sm.step(&inputs(3.5), &t, ts(31)), Some((PressureState::Green, PressureState::Amber)));
        // Below enter but above exit: stays Amber.
        assert_eq!(sm.step(&inputs(2.0), &t, ts(120)), None);
        assert_eq!(sm.step(&inputs(0.5), &t, ts(121)), Some((PressureState::Amber, PressureState::Green)));
    }

    #[test]
    fn dwell_blocks_early_transitions() {
        let t = Thresholds::default();
        let mut sm = at(PressureState::Green);
        assert_eq!(sm.step(&inputs(20.0), &t, ts(29)), None);
        assert_eq!(sm.step(&inputs(20.0), &t, ts(30)), Some((PressureState::Green, PressureState::Red)));
        assert_eq!(sm.step(&inputs(-20.0), &t, ts(59)), None);
    }

    #[test]
    fn red_enters_on_error_or_eta_and_needs_both_to_exit() {
        let t = Thresholds::default();
        let sm = at(PressureState::Green);
        assert_eq!(sm.target(&inputs(8.5), &t), PressureState::Red);
        let mut i = inputs(0.0);
        i.eta = Some(ts(100_000 - 300)); // 5 h before reset
        assert_eq!(sm.target(&i, &t), PressureState::Red);
        i.eta = Some(ts(100_000 - 200)); // 3h20 before reset: not < reset − 4h
        assert_eq!(sm.target(&i, &t), PressureState::Green);

        let red = at(PressureState::Red);
        assert_eq!(red.target(&inputs(6.0), &t), PressureState::Red);
        assert_eq!(red.target(&i, &t), PressureState::Red, "eta before reset keeps Red");
        assert_eq!(red.target(&inputs(4.0), &t), PressureState::Amber);
        assert_eq!(red.target(&inputs(2.0), &t), PressureState::Green);
    }

    #[test]
    fn catch_up_needs_few_hours_left() {
        let t = Thresholds::default();
        let sm = at(PressureState::Green);
        assert_eq!(sm.target(&inputs(-12.0), &t), PressureState::Green);
        let mut i = inputs(-12.0);
        i.hours_left = 20.0;
        assert_eq!(sm.target(&i, &t), PressureState::CatchUp);
        let cu = at(PressureState::CatchUp);
        i.error_pp = -6.0;
        assert_eq!(cu.target(&i, &t), PressureState::CatchUp);
        i.error_pp = -4.0;
        assert_eq!(cu.target(&i, &t), PressureState::Green);
        i.error_pp = 9.0;
        assert_eq!(cu.target(&i, &t), PressureState::Red);
    }
}
