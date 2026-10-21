#![doc = include_str!("../README.md")]

pub mod actions;
pub mod calendar;
pub mod estimator;
mod governor;
pub mod plan;
pub mod pressure;
pub mod queue;
pub mod types;

pub use actions::{ActionKind, ActionMode, Ask, GovernorAction, Outcome, Undo};
pub use calendar::{CalendarError, Curve, Day, WorkBlock, WorkingCalendar};
pub use estimator::{Calibration, Calibrator};
pub use governor::{
    Decision, DenyReason, ForecastSnapshot, Governor, GovernorError, LimitResponse, NextActionPreview, Release, Stats,
    ToolCallRequest, TurnRequest, Verdict,
};
pub use plan::{AskDefault, AutoContinue, BudgetAllocation, LiveCaps, PacingPolicy, Thresholds, WeeklyPlan};
pub use pressure::{PressureInputs, PressureState, StateMachine};
pub use queue::{QueueEntry, QueueReason, TurnCeiling, TurnKind};
pub use types::{ActionId, AgentId, AllocationId, EntryId, Provenance, Tier, UsageSample, Window, WindowReading};

/// Re-export so callers use the same time types.
pub use jiff;
