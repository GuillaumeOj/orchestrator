//! Roles, nodes, edges and teams (SPEC §7: Role, Team, AgentNode, Edge).

use std::fmt;

use governor::Tier;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Where a role or team lives (FR-306: global or per project).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Global,
    Project(String),
}

impl Scope {
    /// The project it belongs to; `None` for global.
    pub fn project(&self) -> Option<&str> {
        match self {
            Scope::Global => None,
            Scope::Project(p) => Some(p),
        }
    }
}

/// Permission profile of a role (FR-706). Mapped to `--permission-mode`:
/// strict → `manual`, gated → `acceptEdits`, autonomous → `auto` (the
/// default: Claude Code's classifier lets routine commands run and stops
/// only risky ones; the worktree sandbox (FR-701) and the core-only push,
/// merge and PR gates hold either way, and an agent stopped on a prompt
/// would stall its team).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionProfile {
    Strict,
    Gated,
    #[default]
    Autonomous,
}

impl PermissionProfile {
    pub fn permission_mode(self) -> &'static str {
        match self {
            PermissionProfile::Strict => "manual",
            PermissionProfile::Gated => "acceptEdits",
            PermissionProfile::Autonomous => "auto",
        }
    }
}

/// Claude Code tool families a role may use (the canvas inspector's checkboxes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    #[serde(default = "yes")]
    pub edit: bool,
    #[serde(default = "yes")]
    pub bash: bool,
    #[serde(default)]
    pub web: bool,
}

fn yes() -> bool {
    true
}

impl Default for Tools {
    fn default() -> Self {
        Tools { edit: true, bash: true, web: false }
    }
}

/// Per-turn ceiling (FR-407): the watchdog interrupts a turn past either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnCeiling {
    /// Percentage points of the week.
    #[serde(default)]
    pub weekly_pp: Option<f64>,
    #[serde(default)]
    pub minutes: Option<u32>,
}

/// A reusable agent definition (SPEC §7 Role, FR-303).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoleDef {
    /// Stable id, e.g. `backend`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub scope: Scope,
    /// Material glyph or a letter; the UI falls back to the name's initial.
    #[serde(default)]
    pub icon: Option<String>,
    /// `color.role.1…8`.
    #[serde(default)]
    pub color: Option<u8>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub tools: Tools,
    /// `mcpServers` entries added to the agent's `--mcp-config` (Penpot, Figma…).
    #[serde(default)]
    pub mcp_servers: Map<String, Value>,
    /// MCP registry ids (FR-601), resolved at compile time (project override
    /// > global) into `mcpServers`.
    #[serde(default)]
    pub mcp_refs: Vec<String>,
    /// `--strict-mcp-config`: ignore the user's own MCP servers.
    #[serde(default)]
    pub strict_mcp: bool,
    #[serde(default)]
    pub permission_profile: PermissionProfile,
    /// Only `worktree` exists (ADR-009); kept for the inspector and later profiles.
    #[serde(default = "worktree")]
    pub sandbox_profile: String,
    /// Extra domains on top of the package registries (ADR-009 allow-list).
    #[serde(default)]
    pub network_allow: Vec<String>,
    #[serde(default)]
    pub turn_ceiling: TurnCeiling,
    /// Async subagents (Claude Code's Agent tool, FR-410).
    #[serde(default)]
    pub subagents_allowed: bool,
    #[serde(default = "normal")]
    pub budget_priority: Tier,
    #[serde(default = "one")]
    pub version: u32,
    /// The starter it was installed from (FR-306 starter upgrades).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<StarterOrigin>,
}

fn worktree() -> String {
    "worktree".into()
}
fn normal() -> Tier {
    Tier::Normal
}
fn one() -> u32 {
    1
}

impl RoleDef {
    pub fn new(id: &str, name: &str) -> RoleDef {
        RoleDef {
            id: id.into(),
            name: name.into(),
            scope: Scope::Global,
            icon: None,
            color: None,
            prompt: String::new(),
            model: None,
            effort: None,
            tools: Tools::default(),
            mcp_servers: Map::new(),
            mcp_refs: vec![],
            strict_mcp: false,
            permission_profile: PermissionProfile::Autonomous,
            sandbox_profile: worktree(),
            network_allow: vec![],
            turn_ceiling: TurnCeiling::default(),
            subagents_allowed: false,
            budget_priority: Tier::Normal,
            version: 1,
            origin: None,
        }
    }
}

/// Fields a node may override on its role (FR-303 "overrides").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<PermissionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_priority: Option<Tier>,
    /// Appended to the role's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_extra: Option<String>,
}

/// A node on the canvas: one Role in the team, started as up to
/// `max_concurrency` agents. Its id names the agents' worktrees
/// (`orch/<feature>/<node>-<n>`), so it is a git-safe slug.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentNode {
    pub id: String,
    pub role_id: String,
    #[serde(default)]
    pub overrides: RoleOverrides,
    #[serde(default)]
    pub position: (f64, f64),
    #[serde(default = "one")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub is_lead: bool,
}

/// Edge types (FR-304).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// A → B: A sends reports to B; B can assign tasks to A.
    ReportsTo,
    /// A → B: A can send artifacts to B.
    HandsOffTo,
    /// A → B: A can request reviews from B; B can reply.
    Reviews,
    /// Both ways.
    CanMessage,
}

impl EdgeKind {
    pub const ALL: [EdgeKind; 4] = [EdgeKind::ReportsTo, EdgeKind::HandsOffTo, EdgeKind::Reviews, EdgeKind::CanMessage];

    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::ReportsTo => "reports-to",
            EdgeKind::HandsOffTo => "hands-off-to",
            EdgeKind::Reviews => "reviews",
            EdgeKind::CanMessage => "can-message",
        }
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

impl Edge {
    pub fn new(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge { from: from.into(), to: to.into(), kind }
    }
}

/// How a task is reviewed before the lead merges it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewMode {
    /// The owner runs `/code-review` (with fixes) then `/simplify` on its
    /// branch, and reports what it fixed and what it left to the lead.
    #[default]
    #[serde(rename = "self")]
    SelfReview,
    /// The node a `reviews` edge points to reviews the branch.
    Node,
    /// The lead reviews (as before).
    None,
}

/// A graph of Roles, authored on the canvas (SPEC §7 Team).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub nodes: Vec<AgentNode>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub review: ReviewMode,
    /// Shown in the presets row of the canvas and the Launch dialog (FR-306).
    #[serde(default)]
    pub preset: bool,
    /// The starter it was installed from (FR-306 starter upgrades).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<StarterOrigin>,
}

/// Where an installed starter role or team came from: the starter's id, the
/// starter version installed and the fingerprint of what was installed, so
/// an unmodified copy can be upgraded in place and a modified one is left alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterOrigin {
    pub starter: String,
    pub version: u32,
    pub fingerprint: String,
}

impl Team {
    /// The roles its nodes use, by id: what a snapshot or an export holds.
    pub fn used_roles(&self, roles: &[RoleDef]) -> Vec<RoleDef> {
        let mut v: Vec<RoleDef> =
            roles.iter().filter(|r| self.nodes.iter().any(|n| n.role_id == r.id)).cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn node(&self, id: &str) -> Option<&AgentNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn lead(&self) -> Option<&AgentNode> {
        self.nodes.iter().find(|n| n.is_lead)
    }

    pub fn is_lead(&self, id: &str) -> bool {
        self.node(id).is_some_and(|n| n.is_lead)
    }
}

/// A git-safe slug (the same rule as worktree names, orch-git `valid_id`).
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !s.starts_with('-')
}
