//! P3 pacing additions: per-model weekly windows (FR-815) and the opt-in
//! per-role model downgrade (FR-819).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::types::ActionId;

/// An extra statusline `rate_limits.<name>` window beyond `five_hour` and
/// `seven_day` (e.g. `seven_day_opus`), as one response reported it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelWindowReading {
    pub name: String,
    pub used_pct: f64,
    pub resets_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelWindowState {
    Green,
    Amber,
    Red,
    Exhausted,
}

impl ModelWindowState {
    /// Turns of that model wait (FR-815).
    pub fn tight(self) -> bool {
        matches!(self, ModelWindowState::Red | ModelWindowState::Exhausted)
    }
}

/// A model window as the planner shows it (`snapshot.model_windows`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelWindow {
    pub name: String,
    /// `opus`, `sonnet`, `haiku`, `fable` or `other` (from the name).
    pub family: String,
    pub used_pct: f64,
    pub resets_at: Timestamp,
    pub state: ModelWindowState,
}

/// When a role switches to its fallback model (FR-819).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DowngradeWhen {
    /// The week is Amber or Red.
    Amber,
    Red,
    /// The window of the role's model is Red or exhausted.
    ModelWindow,
}

impl DowngradeWhen {
    pub fn label(self) -> &'static str {
        match self {
            DowngradeWhen::Amber => "Amber",
            DowngradeWhen::Red => "Red",
            DowngradeWhen::ModelWindow => "model window",
        }
    }
}

/// `RoleDef.downgrade`: off unless set; never set on Lead or Reviewer starters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Downgrade {
    pub to: String,
    pub when: DowngradeWhen,
}

/// A downgrade in force for one agent, applied at its task boundaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelOverride {
    pub to: String,
    pub from: Option<String>,
    pub when: DowngradeWhen,
    pub since: Timestamp,
    /// The `downgrade` action (undo restores the model at the next task).
    pub action: ActionId,
    /// Undone: the role's model comes back at the agent's next task, not
    /// mid-task.
    #[serde(default)]
    pub restore_pending: bool,
}

impl ModelOverride {
    /// The agent header's note: "downgraded (Red)".
    pub fn note(&self) -> String {
        format!("downgraded ({})", self.when.label())
    }
}
