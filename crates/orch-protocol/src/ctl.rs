//! Control requests from a client (the orchd CLI now, the Tauri UI later), and
//! the admission types the core reports.

use serde::{Deserialize, Serialize};

/// Which worktree to create (FR-105 branch naming).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorktreeKindSpec {
    /// `orch/solo/<slug>` in `<repo>.worktrees/solo/<slug>`.
    Solo { slug: String },
    /// `orch/<feature>/integration` in `<repo>.worktrees/<feature>/integration`.
    Integration { feature: String },
    /// `orch/<feature>/<role>-<n>` in `<repo>.worktrees/<feature>/<role>-<n>`.
    Team { feature: String, role: String, n: u32 },
}

/// What to start. Everything but `agent_id` and `repo` has a default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub agent_id: String,
    /// The project's main checkout.
    pub repo: String,
    /// Solo slug for the worktree; defaults to the agent id.
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// `manual | acceptEdits | auto | dontAsk | plan`; never `bypassPermissions`.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Role prompt appended after the ORCH preamble.
    #[serde(default)]
    pub prompt: Option<String>,
    /// `--strict-mcp-config` (saves ~25 MB per agent).
    #[serde(default)]
    pub strict_mcp: Option<bool>,
    /// Extra network allow-list entries for the sandbox.
    #[serde(default)]
    pub network_allow: Vec<String>,
    /// Extra `permissions.allow` rules (e.g. `Bash` to skip Bash prompts inside the sandbox).
    #[serde(default)]
    pub allow_rules: Vec<String>,
    /// Governor allocation this agent's work is charged to (default: the repo name).
    #[serde(default)]
    pub allocation: Option<String>,
    /// `low | normal | high | critical`.
    #[serde(default)]
    pub tier: Option<String>,
    /// First message, delivered through the Governor once the agent is ready.
    #[serde(default)]
    pub first_message: Option<String>,
}

/// The user's answer to an Inbox item (FR-705, FR-707).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxDecision {
    /// Allow / approve (`a`).
    Approve,
    /// Deny / reject (`r`); `message` goes back to the agent.
    Reject,
    /// Approve and create a scoped rule (FR-708); `scope` = project (default) / role / agent.
    AlwaysAllow,
    /// Answer with text (`⏎`): a denial with that message for a permission, a turn otherwise.
    Reply,
    /// Close without acting (alerts).
    Dismiss,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CtlRequest {
    Ping,
    CreateWorktree {
        repo: String,
        kind: WorktreeKindSpec,
        #[serde(default)]
        base: Option<String>,
    },
    Start {
        spec: Box<AgentSpec>,
    },
    /// Respawn a suspended agent with `--resume <session id>`.
    Resume {
        agent_id: String,
        #[serde(default)]
        message: Option<String>,
    },
    /// Stop the process, keep the session (FR-204).
    Suspend {
        agent_id: String,
    },
    /// FR-207 kill: process group + every process whose cwd is in the worktree.
    Kill {
        agent_id: String,
    },
    /// Esc to the PTY.
    Interrupt {
        agent_id: String,
    },
    /// Queue a message; the Governor admits it and it is delivered when the agent is idle.
    Deliver {
        agent_id: String,
        text: String,
    },
    /// Turns the connection into a raw byte stream to the agent's PTY.
    Attach {
        agent_id: String,
        rows: u16,
        cols: u16,
    },
    Resize {
        agent_id: String,
        rows: u16,
        cols: u16,
    },
    /// FR-811 Pause all: autonomous work holds at its next tool call.
    PauseAll,
    ResumeAll,
    /// One agent's autonomous work holds at its next tool call (until ResumeAgent).
    PauseAgent {
        agent_id: String,
    },
    ResumeAgent {
        agent_id: String,
    },
    /// FR-809 Run now / Start now on a queued entry.
    RunNow {
        entry: u64,
    },
    /// Drop a queued entry and its waiting turn.
    Cancel {
        entry: u64,
    },
    /// Undo a Governor action from its log.
    Undo {
        action: u64,
    },
    /// FR-811 Sprint mode: extra pp today (default 5).
    Sprint {
        #[serde(default)]
        pp: Option<f64>,
    },
    /// FR-810: answer a Governor ask (accept applies it).
    AnswerAsk {
        action: u64,
        accept: bool,
    },
    /// FR-812: enforce (`true`) or go back to shadow mode.
    Enforce {
        on: bool,
    },
    State,
    Usage,
    Governor,
    /// Pull transport: everything buffered for this agent since the last read.
    Read {
        agent_id: String,
    },
    // ---- Inbox and permission rules (FR-705, FR-707–709)
    Inbox,
    Decide {
        item_id: u64,
        decision: InboxDecision,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    Rules,
    AddRule {
        /// `project` / `role` / `agent`.
        scope: String,
        scope_id: String,
        pattern: String,
        /// `allow` / `deny` / `ask`.
        effect: String,
    },
    RevokeRule {
        id: u64,
    },
    Audit {
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Events {
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        since_ms: Option<u64>,
        #[serde(default)]
        limit: Option<usize>,
    },
    // ---- Projects and git (FR-101–111)
    ProjectAdd {
        path: String,
    },
    ProjectClone {
        url: String,
        parent: String,
        #[serde(default)]
        name: Option<String>,
    },
    Projects,
    /// Status, worktrees, branches (and PRs when `with_prs`).
    Project {
        project_id: String,
        #[serde(default)]
        with_prs: bool,
    },
    Fetch {
        project_id: String,
    },
    Diff {
        worktree: String,
        #[serde(default)]
        against: Option<String>,
    },
    RemoveWorktree {
        project_id: String,
        path: String,
        #[serde(default)]
        delete_branch: bool,
        /// Required to discard uncommitted or unmerged work.
        #[serde(default)]
        confirm_discard: bool,
    },
    RemovalPlan {
        project_id: String,
    },
    RemoveProject {
        project_id: String,
        /// Worktrees the user confirmed to discard even with unmerged work.
        #[serde(default)]
        discard: Vec<String>,
        #[serde(default)]
        keep_merged: bool,
        /// `delete` / `archive`.
        history: String,
    },
    RelocateProject {
        project_id: String,
        new_path: String,
    },
    /// Commit in an agent's worktree as the user would (same path as orch-mcp `commit`).
    Commit {
        agent_id: String,
        message: String,
        #[serde(default)]
        paths: Option<Vec<String>>,
    },
    Search {
        query: String,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    // ---- P2 teams. Teams and roles travel as `orch_team` JSON.
    /// Teams and roles (global + the project's), with validation and rules.
    Teams {
        #[serde(default)]
        project_id: Option<String>,
    },
    /// Saves the starter roles and presets that don't exist yet (FR-306).
    InstallPresets,
    TeamSave {
        team: serde_json::Value,
    },
    RoleSave {
        role: serde_json::Value,
    },
    TeamImport {
        json: String,
    },
    TeamExport {
        team_id: String,
        /// The project it's seen from (its own team first, else the global one).
        #[serde(default)]
        project_id: Option<String>,
    },
    Launch {
        project_id: String,
        team_id: String,
        slug: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        brief: Option<String>,
        #[serde(default)]
        base: Option<String>,
    },
    Runs,
    Run {
        run_id: String,
    },
    /// `pause` / `resume` / `stop`.
    RunAct {
        run_id: String,
        action: String,
    },
    Tasks {
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
    },
    TaskCreate {
        project_id: String,
        title: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        node: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        depends_on: Vec<u64>,
    },
    TaskUpdate {
        task_id: u64,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        node: Option<String>,
        #[serde(default)]
        note: Option<String>,
    },
    /// The user messages a node or agent of a run (FR-409).
    Message {
        run_id: String,
        to: String,
        body: String,
    },
    Timeline {
        run_id: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CtlReply {
    pub ok: bool,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl CtlReply {
    pub fn ok(data: serde_json::Value) -> Self {
        CtlReply { ok: true, data }
    }

    pub fn err(message: impl std::fmt::Display) -> Self {
        CtlReply { ok: false, data: serde_json::json!({ "error": message.to_string() }) }
    }
}

/// Where the Governor was asked (ADR-007 admission points).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionPoint {
    TurnStart,
    ToolCall,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub agent_id: String,
    pub point: AdmissionPoint,
    pub interactive: bool,
    #[serde(default)]
    pub self_started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionVerdict {
    Admit,
    Queue,
    Deny,
}

/// The Governor's decision as reported on the bus and in the core's log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub request: AdmissionRequest,
    pub ts_ms: u64,
    /// What the core enforced.
    pub verdict: AdmissionVerdict,
    /// What the policy wanted (differs in shadow mode).
    pub proposed: AdmissionVerdict,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub estimated_start_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctl_round_trip() {
        let r = CtlRequest::CreateWorktree {
            repo: "/r".into(),
            kind: WorktreeKindSpec::Team { feature: "f".into(), role: "backend".into(), n: 1 },
            base: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""cmd":"create_worktree""#), "{s}");
        assert_eq!(serde_json::from_str::<CtlRequest>(&s).unwrap(), r);
        let spec: AgentSpec = serde_json::from_str(r#"{"agent_id":"a","repo":"/r"}"#).unwrap();
        assert_eq!(spec.network_allow, Vec::<String>::new());
    }
}
