//! Claude Code hook events: names, the payload fields the core reads, and the
//! output shapes verified in the P0 spikes (Claude Code 2.1.302).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `hook_event_name`. Unknown names are kept, so a newer CLI never breaks parsing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PermissionRequest,
    PermissionDenied,
    PostToolUse,
    PostToolUseFailure,
    PostToolBatch,
    Notification,
    Stop,
    StopFailure,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PostCompact,
    SessionEnd,
    Other(String),
}

impl HookEvent {
    /// Every event the generated settings register (SPEC §6.3, plus PostCompact
    /// which the recovery harness also registered).
    pub const REGISTERED: [HookEvent; 15] = [
        HookEvent::SessionStart,
        HookEvent::UserPromptSubmit,
        HookEvent::PreToolUse,
        HookEvent::PermissionRequest,
        HookEvent::PostToolUse,
        HookEvent::PostToolUseFailure,
        HookEvent::PostToolBatch,
        HookEvent::Notification,
        HookEvent::Stop,
        HookEvent::StopFailure,
        HookEvent::SubagentStart,
        HookEvent::SubagentStop,
        HookEvent::PreCompact,
        HookEvent::PostCompact,
        HookEvent::SessionEnd,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PermissionRequest => "PermissionRequest",
            HookEvent::PermissionDenied => "PermissionDenied",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PostToolUseFailure => "PostToolUseFailure",
            HookEvent::PostToolBatch => "PostToolBatch",
            HookEvent::Notification => "Notification",
            HookEvent::Stop => "Stop",
            HookEvent::StopFailure => "StopFailure",
            HookEvent::SubagentStart => "SubagentStart",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::PostCompact => "PostCompact",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::Other(s) => s,
        }
    }

    /// The only events whose answer matters, so the only ones that wait on the
    /// core and that fail closed (FR-806). Every other event exits 0 at once
    /// when the core is unreachable.
    pub fn waits_for_core(&self) -> bool {
        matches!(self, HookEvent::PreToolUse | HookEvent::PermissionRequest)
    }
}

impl From<String> for HookEvent {
    fn from(s: String) -> Self {
        match s.as_str() {
            "SessionStart" => HookEvent::SessionStart,
            "UserPromptSubmit" => HookEvent::UserPromptSubmit,
            "PreToolUse" => HookEvent::PreToolUse,
            "PermissionRequest" => HookEvent::PermissionRequest,
            "PermissionDenied" => HookEvent::PermissionDenied,
            "PostToolUse" => HookEvent::PostToolUse,
            "PostToolUseFailure" => HookEvent::PostToolUseFailure,
            "PostToolBatch" => HookEvent::PostToolBatch,
            "Notification" => HookEvent::Notification,
            "Stop" => HookEvent::Stop,
            "StopFailure" => HookEvent::StopFailure,
            "SubagentStart" => HookEvent::SubagentStart,
            "SubagentStop" => HookEvent::SubagentStop,
            "PreCompact" => HookEvent::PreCompact,
            "PostCompact" => HookEvent::PostCompact,
            "SessionEnd" => HookEvent::SessionEnd,
            _ => HookEvent::Other(s),
        }
    }
}

impl From<HookEvent> for String {
    fn from(e: HookEvent) -> Self {
        e.as_str().to_string()
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry of `background_tasks` in Stop/SubagentStop payloads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTask {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
}

/// The payload fields the core uses. Every field is optional: payloads differ
/// per event and per CLI version, and the raw JSON is always kept beside this.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HookPayload {
    #[serde(default)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Id of the running turn. A UserPromptSubmit with the running turn's
    /// `prompt_id` is a message queued during that turn, not a new turn.
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    // SessionStart / SessionEnd
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    // UserPromptSubmit
    #[serde(default)]
    pub prompt: Option<String>,
    // Tool events
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub tool_response: Option<Value>,
    // Stop / SubagentStop / StopFailure
    #[serde(default)]
    pub stop_hook_active: Option<bool>,
    #[serde(default)]
    pub background_tasks: Option<Vec<BackgroundTask>>,
    #[serde(default)]
    pub last_assistant_message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    // Subagents
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub agent_transcript_path: Option<String>,
    // Notification
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    // PreCompact / PostCompact
    #[serde(default)]
    pub trigger: Option<String>,
}

impl HookPayload {
    /// Parses the fields the core uses; never fails (unknown or mistyped
    /// fields degrade to `None` one by one rather than losing the event).
    pub fn parse(v: &Value) -> HookPayload {
        serde_json::from_value(v.clone()).unwrap_or_else(|_| {
            let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            HookPayload {
                hook_event_name: s("hook_event_name"),
                session_id: s("session_id"),
                transcript_path: s("transcript_path"),
                cwd: s("cwd"),
                prompt_id: s("prompt_id"),
                prompt: s("prompt"),
                tool_name: s("tool_name"),
                tool_use_id: s("tool_use_id"),
                error: s("error"),
                ..Default::default()
            }
        })
    }

    /// Background shells or async subagents still running at this Stop: the
    /// agent is not idle, they will start a new turn by themselves (§6.2).
    pub fn running_background_tasks(&self) -> usize {
        self.background_tasks.as_ref().map_or(0, |b| b.iter().filter(|t| t.status == "running").count())
    }

    /// A UserPromptSubmit the CLI started itself for finished background work (FR-410).
    pub fn is_task_notification(&self) -> bool {
        self.prompt.as_deref().is_some_and(|p| p.trim_start().starts_with("<task-notification>"))
    }

    /// SubagentStop with an empty `agent_type` comes from internal helpers and
    /// compaction and carries no state.
    pub fn is_internal_subagent(&self) -> bool {
        self.agent_type.as_deref().unwrap_or("").is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
}

/// The JSON a hook prints on stdout. Built only through the constructors,
/// which produce the shapes verified in P0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HookOutput(pub Value);

/// The reason orch-hook gives when it cannot get an answer from the core.
pub const UNAVAILABLE_REASON: &str =
    "ORCH: orchestrator unavailable; tool not run. Wait for the orchestrator, do not retry in a loop.";

impl HookOutput {
    /// `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":…,"permissionDecisionReason":…}}`
    pub fn pre_tool_use(decision: ToolDecision, reason: &str) -> Self {
        HookOutput(json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }}))
    }

    /// `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":…}}}`
    pub fn permission_request(behavior: PermissionBehavior, message: Option<&str>) -> Self {
        let mut decision = json!({"behavior": behavior});
        if let Some(m) = message {
            decision["message"] = json!(m);
        }
        HookOutput(json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}))
    }

    /// Context for a busy agent, prefixed `ORCH:` (needs the preamble).
    pub fn additional_context(event: &HookEvent, text: &str) -> Self {
        let text = if text.starts_with("ORCH:") { text.to_string() } else { format!("ORCH: {text}") };
        HookOutput(json!({"hookSpecificOutput": {"hookEventName": event.as_str(), "additionalContext": text}}))
    }

    /// Keep an agent working: `{"decision":"block","reason":…}` on Stop.
    pub fn stop_block(reason: &str) -> Self {
        HookOutput(json!({"decision": "block", "reason": reason}))
    }

    /// What orch-hook answers when the core is unreachable: deny for the two
    /// events that gate tools, nothing for every other event (FR-806).
    pub fn fail_closed(event: &HookEvent) -> Option<Self> {
        match event {
            HookEvent::PreToolUse => Some(Self::pre_tool_use(ToolDecision::Deny, UNAVAILABLE_REASON)),
            HookEvent::PermissionRequest => {
                Some(Self::permission_request(PermissionBehavior::Deny, Some(UNAVAILABLE_REASON)))
            }
            _ => None,
        }
    }

    pub fn to_json_string(&self) -> String {
        self.0.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_round_trip_and_unknowns_survive() {
        for e in HookEvent::REGISTERED {
            assert_eq!(HookEvent::from(e.as_str().to_string()), e);
        }
        let e: HookEvent = serde_json::from_str(r#""CwdChanged""#).unwrap();
        assert_eq!(e, HookEvent::Other("CwdChanged".into()));
        assert_eq!(serde_json::to_string(&e).unwrap(), r#""CwdChanged""#);
    }

    #[test]
    fn only_tool_events_wait() {
        let waiting: Vec<_> = HookEvent::REGISTERED.iter().filter(|e| e.waits_for_core()).collect();
        assert_eq!(waiting, [&HookEvent::PreToolUse, &HookEvent::PermissionRequest]);
    }

    #[test]
    fn output_shapes_match_p0() {
        let o = HookOutput::pre_tool_use(ToolDecision::Deny, "r");
        assert_eq!(
            o.0,
            json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"r"}})
        );
        let o = HookOutput::permission_request(PermissionBehavior::Allow, None);
        assert_eq!(
            o.0,
            json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}})
        );
        assert_eq!(HookOutput::stop_block("next").0, json!({"decision":"block","reason":"next"}));
        let c = HookOutput::additional_context(&HookEvent::PostToolUse, "2 new messages");
        assert_eq!(c.0["hookSpecificOutput"]["additionalContext"], "ORCH: 2 new messages");
    }

    #[test]
    fn fail_closed_denies_only_tools() {
        let d = HookOutput::fail_closed(&HookEvent::PreToolUse).unwrap();
        assert_eq!(d.0["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(
            d.0["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .starts_with("ORCH: orchestrator unavailable")
        );
        let p = HookOutput::fail_closed(&HookEvent::PermissionRequest).unwrap();
        assert_eq!(p.0["hookSpecificOutput"]["decision"]["behavior"], "deny");
        for e in HookEvent::REGISTERED.iter().filter(|e| !e.waits_for_core()) {
            assert!(HookOutput::fail_closed(e).is_none(), "{e}");
        }
    }

    #[test]
    fn payload_helpers() {
        let stop = json!({"hook_event_name":"Stop","prompt_id":"p","background_tasks":[
            {"id":"a","type":"subagent","status":"running"},{"id":"b","type":"shell","status":"completed"}]});
        let p = HookPayload::parse(&stop);
        assert_eq!(p.running_background_tasks(), 1);
        let n = HookPayload::parse(&json!({"prompt":"<task-notification>\n<task-id>x</task-id>"}));
        assert!(n.is_task_notification());
        let s = HookPayload::parse(&json!({"hook_event_name":"SubagentStop","agent_type":""}));
        assert!(s.is_internal_subagent());
        // A mistyped field does not lose the event.
        let bad = HookPayload::parse(&json!({"hook_event_name":"Stop","background_tasks":"oops","session_id":"s"}));
        assert_eq!(bad.session_id.as_deref(), Some("s"));
    }
}
