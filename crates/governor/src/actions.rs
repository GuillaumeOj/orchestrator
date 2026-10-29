//! `GovernorAction` records (FR-809): every action with its reason, expected effect and undo.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::plan::AskDefault;
use crate::pressure::PressureState;
use crate::types::{ActionId, AgentId, AllocationId, EntryId, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionMode {
    /// Applied by the Governor.
    Auto,
    /// Proposed to the user, applied or skipped on answer or timeout (FR-810).
    Ask,
    /// Shadow mode: decided and logged, never enforced (FR-812).
    Shadow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    /// An ask waiting for the user or its timeout.
    Pending,
    /// An ask the user declined, or whose default is to skip.
    Skipped,
    /// Only logged (shadow mode, alerts).
    Logged,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    StateChange {
        from: PressureState,
        to: PressureState,
    },
    /// The live cap changed; agents above it pause at their turn boundary.
    LiveCap {
        cap: u32,
        live: u32,
    },
    /// A turn was queued (`entry` is `None` in shadow mode, where nothing is queued).
    Queue {
        entry: Option<EntryId>,
        agent: AgentId,
    },
    /// A queued turn was released.
    Release {
        entry: EntryId,
        agent: AgentId,
    },
    /// A tool call was held and denied ("paused by governor"); the turn ends.
    HoldToolCall {
        agent: AgentId,
    },
    /// Defer an allocation's autonomous work until the weekly reset (ask).
    DeferAfterReset {
        allocation: AllocationId,
    },
    /// Deferrals lifted (Catch-up releases the queue).
    ReleaseDeferrals {
        allocations: Vec<AllocationId>,
    },
    FiveHourGuard {
        active: bool,
    },
    Watchdog {
        agent: AgentId,
    },
    LimitReached {
        window: Window,
        resets_at: Timestamp,
    },
    LimitReset {
        window: Window,
    },
    /// Cancel the CLI's own auto-continue at the reset and reschedule (J9).
    CancelAutoContinue {
        agent: AgentId,
    },
    Sprint {
        extra_pp: f64,
        until: Timestamp,
    },
    PauseAll,
    ResumeAll,
    /// One agent paused by the user: its turns stop at the next tool boundary.
    PauseAgent {
        agent: AgentId,
    },
    ResumeAgent {
        agent: AgentId,
    },
    /// A queued turn dropped by the user.
    Cancel {
        entry: EntryId,
        agent: AgentId,
    },
    RunNow {
        entry: EntryId,
        agent: AgentId,
    },
    /// Usage crossed an alert level (FR-817).
    Alert {
        level_pct: f64,
    },
    /// Interactive and critical work used up the reserve (FR-807).
    ReserveExhausted,
    /// FR-819: the agent's next task runs on its role's fallback model.
    Downgrade {
        agent: AgentId,
        to: String,
        /// The role's model it leaves.
        #[serde(default)]
        from: Option<String>,
        /// `amber`, `red` or `model_window`: the role's condition that held.
        #[serde(default)]
        why: String,
    },
    /// FR-819: back to the role's model at the agent's next task.
    RestoreModel {
        agent: AgentId,
    },
}

impl ActionKind {
    /// The agent the action is about, if one.
    pub fn agent(&self) -> Option<&AgentId> {
        use ActionKind::*;
        match self {
            Queue { agent, .. }
            | Release { agent, .. }
            | HoldToolCall { agent, .. }
            | Watchdog { agent, .. }
            | CancelAutoContinue { agent, .. }
            | PauseAgent { agent, .. }
            | ResumeAgent { agent, .. }
            | Cancel { agent, .. }
            | RunNow { agent, .. }
            | Downgrade { agent, .. }
            | RestoreModel { agent, .. } => Some(agent),
            _ => None,
        }
    }
}

/// What undoing an action does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Undo {
    LiftDeferral {
        allocation: AllocationId,
    },
    EndSprint,
    ResumeAll,
    ResumeAgent {
        agent: AgentId,
    },
    /// The downgraded agent gets its role's model back at its next task.
    RestoreModel {
        agent: AgentId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ask {
    pub deadline: Timestamp,
    pub default: AskDefault,
}

/// `GovernorAction` (SPEC §7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorAction {
    pub id: ActionId,
    pub ts: Timestamp,
    pub state: PressureState,
    pub action: ActionKind,
    pub reason: String,
    pub expected_effect: String,
    pub mode: ActionMode,
    pub outcome: Outcome,
    #[serde(default)]
    pub ask: Option<Ask>,
    #[serde(default)]
    pub undo: Option<Undo>,
    #[serde(default)]
    pub undone_at: Option<Timestamp>,
}

impl GovernorAction {
    pub fn kind_name(&self) -> &'static str {
        match self.action {
            ActionKind::StateChange { .. } => "state_change",
            ActionKind::LiveCap { .. } => "live_cap",
            ActionKind::Queue { .. } => "queue",
            ActionKind::Release { .. } => "release",
            ActionKind::HoldToolCall { .. } => "hold_tool_call",
            ActionKind::DeferAfterReset { .. } => "defer_after_reset",
            ActionKind::ReleaseDeferrals { .. } => "release_deferrals",
            ActionKind::FiveHourGuard { .. } => "five_hour_guard",
            ActionKind::Watchdog { .. } => "watchdog",
            ActionKind::LimitReached { .. } => "limit_reached",
            ActionKind::LimitReset { .. } => "limit_reset",
            ActionKind::CancelAutoContinue { .. } => "cancel_auto_continue",
            ActionKind::Sprint { .. } => "sprint",
            ActionKind::PauseAll => "pause_all",
            ActionKind::ResumeAll => "resume_all",
            ActionKind::PauseAgent { .. } => "pause_agent",
            ActionKind::ResumeAgent { .. } => "resume_agent",
            ActionKind::Cancel { .. } => "cancel",
            ActionKind::RunNow { .. } => "run_now",
            ActionKind::Alert { .. } => "alert",
            ActionKind::ReserveExhausted => "reserve_exhausted",
            ActionKind::Downgrade { .. } => "downgrade",
            ActionKind::RestoreModel { .. } => "restore_model",
        }
    }
}
