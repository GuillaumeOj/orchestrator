//! A team node compiled into what the agent runtime needs (FR-308): the
//! role prompt with the team context and, for the lead, the protocol of
//! FR-401; the orchestrator tools the node may call; the Claude Code tools it
//! may not use. The runtime turns a [`NodeConfig`] into its own `Role`.

use governor::Tier;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::model::{ReviewMode, RoleDef, Team, TurnCeiling};
use crate::rules;

/// The orchestrator's own MCP server name (tools are `mcp__orch__<tool>`).
pub const ORCH_SERVER: &str = "orch";

/// Orchestrator MCP tools every team node has.
pub const COMMON_TOOLS: &[&str] =
    &["commit", "send_message", "inbox", "list_tasks", "handoff", "request_review", "ask_user"];
/// Lead only (FR-401: the protocol is enforced by tool availability).
pub const LEAD_TOOLS: &[&str] = &[
    "create_task",
    "update_task",
    "request_spec_approval",
    "merge",
    "request_pr",
    "team_status",
    "nudge",
    "resume_agent",
    "decide_plan",
];
/// Specialists only.
pub const SPECIALIST_TOOLS: &[&str] =
    &["update_task", "report_done", "sync_integration", "submit_plan", "submit_review"];
/// A solo session (P1).
pub const SOLO_TOOLS: &[&str] = &["commit", "request_pr"];
/// Every orchestrator tool, in the order orch-mcp lists them.
pub const ALL_TOOLS: &[&str] = &[
    "commit",
    "request_pr",
    "send_message",
    "inbox",
    "list_tasks",
    "handoff",
    "request_review",
    "ask_user",
    "create_task",
    "update_task",
    "request_spec_approval",
    "merge",
    "report_done",
    "team_status",
    "nudge",
    "resume_agent",
    "decide_plan",
    "sync_integration",
    "submit_plan",
    "submit_review",
];

pub fn tools_for(is_lead: bool) -> Vec<String> {
    let extra = if is_lead { LEAD_TOOLS } else { SPECIALIST_TOOLS };
    COMMON_TOOLS.iter().chain(extra).map(|s| s.to_string()).collect()
}

/// The feature a team works on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureCtx {
    pub slug: String,
    pub title: String,
    /// Usually `main`.
    pub base: String,
}

impl FeatureCtx {
    pub fn integration_branch(&self) -> String {
        format!("orch/{}/integration", self.slug)
    }
    pub fn docs_dir(&self) -> String {
        format!(".orchestrator/features/{}/", self.slug)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeConfig {
    pub node: String,
    pub role_id: String,
    pub role_name: String,
    pub is_lead: bool,
    pub color: Option<u8>,
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `--permission-mode`.
    pub permission_mode: String,
    pub strict_mcp: bool,
    pub mcp_servers: Map<String, Value>,
    /// Registry ids, resolved by the runtime (it has the registry).
    #[serde(default)]
    pub mcp_refs: Vec<String>,
    pub network_allow: Vec<String>,
    /// Claude Code tools denied in the generated settings (`permissions.deny`).
    pub deny_tools: Vec<String>,
    /// Orchestrator MCP tools this node may call.
    pub orch_tools: Vec<String>,
    pub tier: Tier,
    pub max_concurrency: u32,
    pub turn_ceiling: TurnCeiling,
    /// FR-819 (the role's; applied by the runtime at task boundaries).
    #[serde(default)]
    pub downgrade: Option<governor::Downgrade>,
}

pub fn compile_node(team: &Team, node_id: &str, roles: &[RoleDef], f: &FeatureCtx) -> Result<NodeConfig, String> {
    let node = team.node(node_id).ok_or_else(|| format!("no node {node_id} in team {}", team.id))?;
    let role = roles
        .iter()
        .find(|r| r.id == node.role_id)
        .ok_or_else(|| format!("node {node_id} uses the unknown role {}", node.role_id))?;
    let o = &node.overrides;
    let mut deny = Vec::new();
    if !role.tools.edit {
        deny.extend(["Edit", "Write", "NotebookEdit"].map(String::from));
    }
    if !role.tools.bash {
        deny.push("Bash".into());
    }
    if !role.tools.web {
        deny.extend(["WebFetch", "WebSearch"].map(String::from));
    }
    // `/code-review` runs its reviewers as subagents: the owner's self-review
    // and the reviewer node's review alike.
    if !role.subagents_allowed && team.review == ReviewMode::None {
        deny.push("Agent".into());
    }
    let mut prompt = role.prompt.trim().to_string();
    if let Some(extra) = o.prompt_extra.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }
    prompt.push_str("\n\n");
    prompt.push_str(&team_context(team, node_id, roles, f));
    prompt.push_str("\n\n");
    prompt.push_str(&if node.is_lead { lead_protocol(team, f) } else { specialist_protocol(team, node_id, f) });
    Ok(NodeConfig {
        node: node.id.clone(),
        role_id: role.id.clone(),
        role_name: role.name.clone(),
        is_lead: node.is_lead,
        color: role.color,
        prompt,
        model: o.model.clone().or_else(|| role.model.clone()),
        effort: o.effort.clone().or_else(|| role.effort.clone()),
        permission_mode: o.permission_profile.unwrap_or(role.permission_profile).permission_mode().into(),
        strict_mcp: role.strict_mcp,
        mcp_servers: role.mcp_servers.clone(),
        mcp_refs: role.mcp_refs.clone(),
        network_allow: role.network_allow.clone(),
        deny_tools: deny,
        orch_tools: tools_for(node.is_lead),
        // The lead keeps the team moving: its turns are short (review, merge,
        // answer) and must never wait behind the specialists it unblocks.
        tier: if node.is_lead { Tier::Critical } else { o.budget_priority.unwrap_or(role.budget_priority) },
        max_concurrency: node.max_concurrency.max(1),
        turn_ceiling: role.turn_ceiling,
        downgrade: role.downgrade.clone(),
    })
}

fn role_name<'a>(roles: &'a [RoleDef], id: &'a str) -> &'a str {
    roles.iter().find(|r| r.id == id).map(|r| r.name.as_str()).unwrap_or(id)
}

fn team_context(team: &Team, me: &str, roles: &[RoleDef], f: &FeatureCtx) -> String {
    let mut s = format!(
        "## Team\nFeature: {} ({}). Integration branch: {}. Feature docs: {} on that branch.\nYou are node `{me}`. \
         Nodes:",
        f.title,
        f.slug,
        f.integration_branch(),
        f.docs_dir()
    );
    for n in &team.nodes {
        let lead = if n.is_lead { ", lead" } else { "" };
        let you = if n.id == me { ", you" } else { "" };
        s.push_str(&format!("\n- `{}` ({}{lead}{you})", n.id, role_name(roles, &n.role_id)));
    }
    s.push_str("\nRules:");
    for (n, line) in rules::rules_text(team) {
        s.push_str(&format!("\n- `{n}`: {line}"));
    }
    s
}

fn lead_protocol(team: &Team, f: &FeatureCtx) -> String {
    let specialists: Vec<&str> = team.nodes.iter().filter(|n| !n.is_lead).map(|n| n.id.as_str()).collect();
    let docs = f.docs_dir();
    format!(
        "## Protocol (follow it in order)\n\
         1. Understand the brief you are given and the code.\n\
         2. Write `{docs}spec.md` (and `ux-brief.md`, `api-contract.md` when the team has design or both backend and \
         frontend work) in your worktree, and commit them with mcp__orch__commit.\n\
         3. Call mcp__orch__request_spec_approval with a short summary, then end your turn. The orchestrator tells \
         you when the user approves, or what to change. You cannot create tasks before the approval.\n\
         4. Decompose the work into tasks with mcp__orch__create_task, each assigned to one node ({}) with its \
         dependencies (`depends_on`). Split by module ownership so branches don't conflict.\n\
         5. Track: every report, message and user answer wakes you as a new turn (`ORCH:` notes). Never poll and \
         never wait inside a turn: when nothing is left to do now, end your turn. Answer with \
         mcp__orch__send_message. When you are told a member is stuck, unblock it: mcp__orch__team_status shows \
         everyone, mcp__orch__nudge tells a member what to do, mcp__orch__resume_agent restarts a suspended one.\n\
         6. {} Then merge it into {} with mcp__orch__merge. On a conflict the owner is told to sync and fix; merge \
         again when it reports. A task created with `plan_first` starts with a plan: approve it or send it back \
         with mcp__orch__decide_plan.\n\
         7. When every task is done and the integration branch passes its tests, call mcp__orch__request_pr. The \
         user approves the pull request; the orchestrator pushes and opens it. Once the PR is open the feature is \
         not done: you are told when CI fails or the PR falls behind or conflicts with its base. Fix it: tasks as \
         usual for CI; mcp__orch__rebase_base (once every task is merged) when it is behind or conflicts. Then \
         mcp__orch__request_pr again to push. The feature is done when the PR is merged.\n\
         You keep the team moving so the user only talks to you: settle what you can yourself, and send the user \
         only real decisions (product, scope, security, money) with mcp__orch__ask_user. Never ask the user a \
         question in plain text at the end of a turn: nobody reads it there. You never push and never merge with \
         git yourself.",
        if specialists.is_empty() { "yourself".to_string() } else { specialists.join(", ") },
        review_step(team.review),
        f.integration_branch()
    )
}

fn review_step(mode: ReviewMode) -> &'static str {
    match mode {
        ReviewMode::SelfReview => {
            "A task reported done has been reviewed by its owner (`/code-review` with fixes, then `/simplify`); the \
             report lists what was fixed and what was left. Decide each finding left: accept it (say why), send it \
             back with mcp__orch__nudge, or, only for a product, scope or security call, ask the user (one \
             mcp__orch__ask_user per task)."
        }
        ReviewMode::Node => {
            "A task reported done is reviewed by the team's reviewer; its findings come with the report. Decide each \
             open finding: accept it (say why), send it back with mcp__orch__nudge, or, only for a product, scope or \
             security call, ask the user (one mcp__orch__ask_user per task)."
        }
        ReviewMode::None => "When a node reports a task done, review its branch.",
    }
}

fn specialist_protocol(team: &Team, me: &str, f: &FeatureCtx) -> String {
    let up = rules::reports_to(team, me).unwrap_or_else(|| "the lead".into());
    format!(
        "## How you work\n\
         - Tasks arrive as turns from the orchestrator (`ORCH: task #…`). Work only on that task, in your own \
         worktree; your branch is based on {}.\n\
         - Commit with mcp__orch__commit (not git commit). To bring in the latest integration branch, call \
         mcp__orch__sync_integration; on a conflict, fix the files and commit.\n\
         - Talk to the team with mcp__orch__send_message, hand artifacts with mcp__orch__handoff, ask reviews with \
         mcp__orch__request_review (allowed recipients are in the rules above). Ask the user with \
         mcp__orch__ask_user only when `{up}` can't answer, never in plain text at the end of a turn.\n\
         - When the task is done (tests pass, everything committed), call mcp__orch__report_done with a short \
         summary for `{up}`, then end your turn.{}\n\
         - A task that says \"plan first\": read what you need, then call mcp__orch__submit_plan and end your turn. \
         Edits are refused until `{up}` approves the plan.\n\
         - Asked to review a branch: read it (`git diff`), run `/code-review high --max-findings all` on it, then \
         call mcp__orch__submit_review with what you found. You don't change another node's branch.",
        f.integration_branch(),
        match team.review {
            ReviewMode::SelfReview => {
                " The orchestrator then has you review your own branch (`/code-review`, then `/simplify`): when it \
                 asks, commit the fixes and call mcp__orch__report_done again with what you fixed and what you left."
            }
            ReviewMode::Node => " Your branch is then reviewed by the team's reviewer; you get its findings.",
            ReviewMode::None => "",
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn ctx() -> FeatureCtx {
        FeatureCtx { slug: "checkout-v2".into(), title: "Checkout v2".into(), base: "main".into() }
    }

    #[test]
    fn lead_and_specialist_configs() {
        let (roles, t) = presets::full_stack();
        let lead = compile_node(&t, "lead", &roles, &ctx()).unwrap();
        assert!(lead.is_lead);
        assert_eq!(lead.model.as_deref(), Some("opus"));
        assert_eq!(lead.tier, Tier::Critical, "the lead is never queued behind its team");
        assert!(lead.orch_tools.contains(&"merge".to_string()));
        assert!(lead.orch_tools.contains(&"request_spec_approval".to_string()));
        assert!(!lead.orch_tools.contains(&"report_done".to_string()));
        assert!(lead.prompt.contains("mcp__orch__request_spec_approval"));
        assert!(lead.prompt.contains("orch/checkout-v2/integration"));
        assert!(lead.prompt.contains("back, front, ui, ux"));

        let back = compile_node(&t, "back", &roles, &ctx()).unwrap();
        assert!(!back.is_lead);
        assert_eq!(back.permission_mode, "auto");
        assert!(back.orch_tools.contains(&"report_done".to_string()));
        assert!(!back.deny_tools.contains(&"Agent".to_string()), "/code-review runs subagents");
        let mut no_review = t.clone();
        no_review.review = crate::model::ReviewMode::None;
        assert!(compile_node(&no_review, "back", &roles, &ctx()).unwrap().deny_tools.contains(&"Agent".to_string()));
        assert!(!back.orch_tools.contains(&"merge".to_string()));
        assert!(back.deny_tools.contains(&"WebFetch".to_string()));
        assert!(back.prompt.contains("You are node `back`"));
        assert!(back.prompt.contains("summary for `lead`"));
    }

    #[test]
    fn overrides_win() {
        let (roles, mut t) = presets::full_stack();
        let n = t.nodes.iter_mut().find(|n| n.id == "back").unwrap();
        n.overrides.model = Some("haiku".into());
        n.overrides.budget_priority = Some(Tier::Low);
        n.overrides.permission_profile = Some(crate::model::PermissionProfile::Strict);
        n.overrides.prompt_extra = Some("Prefer SQL migrations.".into());
        let c = compile_node(&t, "back", &roles, &ctx()).unwrap();
        assert_eq!(c.model.as_deref(), Some("haiku"));
        assert_eq!(c.tier, Tier::Low);
        assert_eq!(c.permission_mode, "manual");
        assert!(c.prompt.contains("Prefer SQL migrations."));
    }

    #[test]
    fn errors() {
        let (roles, t) = presets::full_stack();
        assert!(compile_node(&t, "ghost", &roles, &ctx()).is_err());
        assert!(compile_node(&t, "back", &[], &ctx()).is_err());
    }

    #[test]
    fn orch_mcp_serves_exactly_these_tools() {
        let served: Vec<String> =
            orch_mcp::tools().as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(served, ALL_TOOLS);
        assert_eq!(orch_mcp::SOLO_TOOLS, SOLO_TOOLS);
    }

    #[test]
    fn tool_sets_are_known() {
        for t in COMMON_TOOLS.iter().chain(LEAD_TOOLS).chain(SPECIALIST_TOOLS).chain(SOLO_TOOLS) {
            assert!(ALL_TOOLS.contains(t), "{t}");
        }
    }
}
