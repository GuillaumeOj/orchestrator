#![doc = include_str!("../README.md")]

pub mod actions;
pub mod calendar;
pub mod estimator;
mod governor;
pub mod models;
pub mod plan;
pub mod pressure;
pub mod queue;
pub mod types;

pub use actions::{ActionKind, ActionMode, Ask, GovernorAction, Outcome, Undo};
pub use calendar::{CalendarError, Curve, Day, WorkBlock, WorkingCalendar};
pub use estimator::{Calibration, Calibrator, FAMILIES, FamilyCalibrator, MIN_FAMILY_SAMPLES, ModelK, family_index};
pub use governor::{
    Carry, CostClass, Decision, DenyReason, EstimatorState, ForecastSnapshot, Governor, GovernorError, LimitResponse,
    LiveRecord, LiveTurn, NextActionPreview, Release, SprintRecord, Stats, ToolCallRequest, TurnRequest, Verdict,
};
pub use models::{Downgrade, DowngradeWhen, ModelOverride, ModelWindow, ModelWindowReading, ModelWindowState};
pub use plan::{AskDefault, AutoContinue, BudgetAllocation, LiveCaps, PacingPolicy, Thresholds, WeeklyPlan};
pub use pressure::{PressureInputs, PressureState, StateMachine};
pub use queue::{QueueEntry, QueueReason, TurnCeiling, TurnKind};
pub use types::{ActionId, AgentId, AllocationId, EntryId, Provenance, Tier, UsageSample, Window, WindowReading};

/// Re-export so callers use the same time types.
pub use jiff;
