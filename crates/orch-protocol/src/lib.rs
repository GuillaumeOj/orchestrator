#![doc = include_str!("../README.md")]

pub mod bus;
pub mod ctl;
pub mod hook;
pub mod spool;
pub mod status;

use serde::{Deserialize, Serialize};

pub use bus::{
    BusError, MAX_SOCKET_PATH, bind_listener, check_socket_path, connect, is_listening, read_line, write_line,
};
pub use ctl::{AgentSpec, CtlReply, CtlRequest, InboxDecision, WorktreeKindSpec};
pub use hook::{HookEvent, HookOutput, HookPayload, PermissionBehavior, ToolDecision};
pub use spool::SpoolRecord;
pub use status::{RateLimits, StatusSample, WindowSample};

/// Version of the bus protocol. A peer that sees another version answers
/// [`Reply::Error`] and the helpers fall back as if the core were down.
pub const PROTOCOL_VERSION: u32 = 1;

/// Environment variables the core sets for every agent (and so for its hooks,
/// statusline and MCP helpers, which inherit the agent's environment).
pub mod env {
    pub const AGENT_ID: &str = "ORCH_AGENT_ID";
    pub const SOCKET: &str = "ORCH_SOCK";
    pub const AGENT_DIR: &str = "ORCH_AGENT_DIR";
    /// orch-hook's own deadline for PreToolUse/PermissionRequest, in ms.
    pub const HOOK_DEADLINE_MS: &str = "ORCH_HOOK_DEADLINE_MS";
    /// The user's own statusline command, which orch-status runs after forwarding.
    pub const USER_STATUSLINE: &str = "ORCH_USER_STATUSLINE";
}

/// Default orch-hook deadline for the two events that wait on the core. The
/// configured hook `timeout` is 600 s; hook timeouts fail open, so orch-hook
/// must always answer well before it (FR-806).
pub const DEFAULT_HOOK_DEADLINE_MS: u64 = 540_000;

/// How long orch-mcp waits for a tool call (a commit, or an Inbox approval
/// when a rule says "ask"). Below Claude Code's own MCP tool timeout.
pub const DEFAULT_MCP_DEADLINE_MS: u64 = 300_000;

/// Connect timeout for every helper (FR-806: ≤ 1 s).
pub const CONNECT_TIMEOUT_MS: u64 = 1_000;

/// One request on the bus: a single JSON line, answered by a single JSON line
/// (except [`CtlRequest::Attach`], which turns the connection into a raw byte stream).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    #[serde(flatten)]
    pub msg: Msg,
}

impl Envelope {
    pub fn new(msg: Msg) -> Self {
        Envelope { v: PROTOCOL_VERSION, msg }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Msg {
    /// From orch-hook: one hook invocation.
    Hook(HookMsg),
    /// From orch-status: one statusline render.
    Status(StatusMsg),
    /// From a control client (orchd CLI, later the UI).
    Ctl(CtlRequest),
    /// From orch-mcp: one orchestrator tool call by an agent.
    Mcp(McpMsg),
}

/// An orchestrator MCP tool call (`commit`, `request_pr`…), executed by the
/// core outside the agent's sandbox, subject to permission rules (ADR-009).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpMsg {
    pub agent_id: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// How long orch-mcp waits for the answer, in ms.
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookMsg {
    pub agent_id: String,
    /// `hook_event_name` from the payload, parsed.
    pub event: HookEvent,
    /// Milliseconds since the epoch when orch-hook started.
    pub ts_ms: u64,
    /// How long orch-hook will wait for the answer (0 = it does not wait).
    pub deadline_ms: u64,
    /// The raw hook payload, untouched.
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusMsg {
    pub agent_id: String,
    pub ts_ms: u64,
    /// The raw statusline JSON, untouched.
    pub payload: serde_json::Value,
}

/// A reply line: `{"v":1,"reply":…}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyEnvelope {
    pub v: u32,
    #[serde(flatten)]
    pub reply: Reply,
}

impl ReplyEnvelope {
    pub fn new(reply: Reply) -> Self {
        ReplyEnvelope { v: PROTOCOL_VERSION, reply }
    }
}

/// The core's answer to a [`Msg`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    /// For a hook: the JSON to print on stdout (`None` = print nothing, exit 0).
    Hook {
        output: Option<HookOutput>,
    },
    /// For a statusline sample or anything that needs no answer.
    Ack,
    Ctl(CtlReply),
    /// For an MCP tool call: text for the model, `is_error` marks a failure.
    Mcp {
        is_error: bool,
        text: String,
        #[serde(default)]
        data: serde_json::Value,
    },
    Error {
        message: String,
    },
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip_is_flat_and_versioned() {
        let e = Envelope::new(Msg::Status(StatusMsg {
            agent_id: "a1".into(),
            ts_ms: 5,
            payload: serde_json::json!({"x": 1}),
        }));
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""v":1"#) && s.contains(r#""type":"status""#), "{s}");
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn mcp_round_trip() {
        let e = Envelope::new(Msg::Mcp(McpMsg {
            agent_id: "a1".into(),
            tool: "commit".into(),
            arguments: serde_json::json!({"message": "fix"}),
            deadline_ms: 1000,
        }));
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""type":"mcp""#), "{s}");
        assert_eq!(serde_json::from_str::<Envelope>(&s).unwrap(), e);
        let r = Reply::Mcp { is_error: false, text: "ok".into(), data: serde_json::Value::Null };
        assert_eq!(serde_json::from_str::<Reply>(&serde_json::to_string(&r).unwrap()).unwrap(), r);
    }

    #[test]
    fn hook_reply_round_trip() {
        let r = Reply::Hook { output: Some(HookOutput::pre_tool_use(ToolDecision::Deny, "ORCH: no")) };
        let s = serde_json::to_string(&r).unwrap();
        let back: Reply = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}
