//! Row types for SPEC §7 entities owned by this crate. Governor entities
//! (WeeklyPlan, PacingPolicy, QueueEntry, GovernorAction, Calibration,
//! ForecastSnapshot) are stored as the `governor` crate's own serde types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// A registered project (FR-101, FR-109).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// The main checkout (canonical path).
    pub path: PathBuf,
    pub default_branch: String,
    pub remotes: Vec<Remote>,
    /// `<parent>/<repo>.worktrees` unless changed per project.
    pub worktree_root: PathBuf,
    /// `{"type": "local"}` until task backends exist (P2).
    pub task_backend: serde_json::Value,
    pub track_orchestrator_dir: bool,
    /// Per-project settings (FR-109): base branch, setup script, copy_env opt-in, budget share…
    pub settings: serde_json::Value,
    pub created_at_ms: u64,
    /// Drives FR-103 (fetch only projects active in the last 7 days).
    pub last_active_at_ms: u64,
    pub last_fetch_at_ms: Option<u64>,
    pub last_fetch_error: Option<String>,
    /// J11 "archive" choice: hidden, history kept.
    pub archived: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeRowState {
    /// Creation started (add + lock + row, each idempotent): the reconciler finishes it.
    Creating,
    Ready,
    Removed,
}

impl WorktreeRowState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeRowState::Creating => "creating",
            WorktreeRowState::Ready => "ready",
            WorktreeRowState::Removed => "removed",
        }
    }

    pub fn parse(s: &str) -> WorktreeRowState {
        match s {
            "creating" => WorktreeRowState::Creating,
            "removed" => WorktreeRowState::Removed,
            _ => WorktreeRowState::Ready,
        }
    }
}

/// A worktree the app created (FR-105). Keyed by its path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorktreeRow {
    pub path: PathBuf,
    /// The project's main checkout.
    pub repo: PathBuf,
    pub branch: String,
    pub base: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub state: WorktreeRowState,
    pub created_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    #[default]
    Pending,
    /// Approved / rejected / replied: see `decision`.
    Decided,
    /// Nobody answered in time; the default applied (or the pane took over).
    Expired,
    /// The hook's connection closed (Esc in the pane) or the core restarted.
    Interrupted,
    Dismissed,
    /// Still open: a permission whose hold expired; the agent's turn ended
    /// and it resumes on the answer (FR-705).
    Parked,
}

impl InboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            InboxStatus::Pending => "pending",
            InboxStatus::Decided => "decided",
            InboxStatus::Expired => "expired",
            InboxStatus::Interrupted => "interrupted",
            InboxStatus::Dismissed => "dismissed",
            InboxStatus::Parked => "parked",
        }
    }

    pub fn parse(s: &str) -> InboxStatus {
        match s {
            "decided" => InboxStatus::Decided,
            "expired" => InboxStatus::Expired,
            "interrupted" => InboxStatus::Interrupted,
            "dismissed" => InboxStatus::Dismissed,
            "parked" => InboxStatus::Parked,
            _ => InboxStatus::Pending,
        }
    }
}

/// An Inbox item (FR-707). `kind`: gate / question / permission / budget / alert / planner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: u64,
    pub kind: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    /// One line for the list.
    pub body: String,
    /// Kind-specific detail (tool, input, diff summary…).
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Attention ladder (§9.6): higher first.
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub status: InboxStatus,
    #[serde(default)]
    pub stale: bool,
    pub created_at_ms: u64,
    #[serde(default)]
    pub decided_at_ms: Option<u64>,
    /// `allow` / `deny` / `always_allow` / reply text…
    #[serde(default)]
    pub decision: Option<String>,
    /// Who closed it (FR-709): `user`, `pane`, `rule:<id>`, `expired`, `core`.
    #[serde(default)]
    pub decided_by: Option<String>,
}

impl InboxItem {
    pub fn new(id: u64, kind: &str, agent_id: Option<&str>, body: String) -> InboxItem {
        InboxItem {
            id,
            kind: kind.into(),
            agent_id: agent_id.map(str::to_string),
            project_id: None,
            body,
            payload: serde_json::Value::Null,
            priority: 0,
            status: InboxStatus::Pending,
            stale: false,
            created_at_ms: crate::now_ms(),
            decided_at_ms: None,
            decision: None,
            decided_by: None,
        }
    }

    pub fn pending(&self) -> bool {
        self.status == InboxStatus::Pending
    }

    /// Still waiting on the user: pending, or parked (FR-705).
    pub fn open(&self) -> bool {
        matches!(self.status, InboxStatus::Pending | InboxStatus::Parked)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Project,
    Role,
    Agent,
}

impl RuleScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleScope::Project => "project",
            RuleScope::Role => "role",
            RuleScope::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<RuleScope> {
        Some(match s {
            "project" => RuleScope::Project,
            "role" => RuleScope::Role,
            "agent" => RuleScope::Agent,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
    /// Always ask in the Inbox (overrides a broader allow).
    Ask,
}

impl RuleEffect {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleEffect::Allow => "allow",
            RuleEffect::Deny => "deny",
            RuleEffect::Ask => "ask",
        }
    }

    pub fn parse(s: &str) -> Option<RuleEffect> {
        Some(match s {
            "allow" => RuleEffect::Allow,
            "deny" => RuleEffect::Deny,
            "ask" => RuleEffect::Ask,
            _ => return None,
        })
    }
}

/// A scoped permission rule (FR-708), visible and revocable.
/// `scope_id` is the project id, the role name or the agent id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub id: u64,
    pub scope: RuleScope,
    pub scope_id: String,
    /// Claude Code rule syntax: `Tool`, `Tool(exact)` or `Tool(prefix:*)`.
    pub pattern: String,
    pub effect: RuleEffect,
    #[serde(default)]
    pub source_inbox_item_id: Option<u64>,
    pub created_at_ms: u64,
    #[serde(default)]
    pub revoked_at_ms: Option<u64>,
}

/// Where a usage sample came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Statusline,
    Otel,
    Estimate,
}

impl UsageSource {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageSource::Statusline => "statusline",
            UsageSource::Otel => "otel",
            UsageSource::Estimate => "estimate",
        }
    }
}

/// A §7 UsageSample row. Statusline rows carry the windows (and feed the
/// Governor back at start); OTLP rows carry tokens and cost per API request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: u64,
    pub ts_ms: u64,
    /// `None` = unattributed.
    pub agent_id: Option<String>,
    pub turn_id: Option<String>,
    pub model: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    /// Cumulative session cost (statusline) or this request's cost (OTLP).
    pub cost_estimate: Option<f64>,
    /// Cost since the previous sample of the same agent (what the Governor ingests).
    pub cost_delta: Option<f64>,
    pub five_hour_pct: Option<f64>,
    pub five_hour_resets_at: Option<i64>,
    pub weekly_pct: Option<f64>,
    pub weekly_resets_at: Option<i64>,
    pub model_weekly_pct: Option<f64>,
    /// `observed` / `estimated`.
    pub provenance: String,
    pub source: Option<UsageSource>,
    /// The Governor's cost class when recorded (`autonomous` / `critical` /
    /// `interactive`), so the week can be replayed after a restart.
    pub class: Option<String>,
    pub allocation: Option<String>,
    /// Per-model weekly windows of the response (FR-815), statusline rows.
    pub model_windows: Vec<governor::ModelWindowReading>,
}

/// OTLP token and cost totals of one agent (`None` = unattributed).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageTotal {
    pub agent_id: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// A terminal pane to restore after a restart (cwd only, NFR-2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terminal {
    pub id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub cwd: PathBuf,
    pub shell: String,
    #[serde(default)]
    pub layout: serde_json::Value,
    pub created_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Proposed by the app (prefilled), not confirmed by the user yet.
    Draft,
    Active,
    Closed,
}

impl PlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanStatus::Draft => "draft",
            PlanStatus::Active => "active",
            PlanStatus::Closed => "closed",
        }
    }
}
