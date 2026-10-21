//! Agent state machine driven by hook events (SPEC §6.2, §6.4, §9.5). Pure.

use std::collections::BTreeSet;

use orch_protocol::{HookEvent, HookPayload};
use serde::{Deserialize, Serialize};

/// Internal states of §6.2 / §9.5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Configured, never spawned (or waiting for the Governor to admit its first turn).
    #[default]
    Created,
    /// Process spawned, no SessionStart yet (trust dialog, startup).
    Starting,
    Working,
    WaitingForYou,
    WaitingGate,
    Queued,
    BlockedByLimit,
    Idle,
    Suspended,
    Archived,
    Error,
    Killed,
}

impl AgentState {
    pub fn is_live(self) -> bool {
        matches!(
            self,
            AgentState::Starting
                | AgentState::Working
                | AgentState::WaitingForYou
                | AgentState::WaitingGate
                | AgentState::Queued
                | AgentState::BlockedByLimit
                | AgentState::Idle
        )
    }

    /// §9.5 UI label.
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Created => "Created",
            AgentState::Starting => "Starting",
            AgentState::Working => "Working",
            AgentState::WaitingForYou => "Waiting for you",
            AgentState::WaitingGate => "At gate",
            AgentState::Queued => "Queued",
            AgentState::BlockedByLimit => "Blocked by limit",
            AgentState::Idle => "Idle",
            AgentState::Suspended => "Suspended",
            AgentState::Archived => "Archived",
            AgentState::Error => "Error",
            AgentState::Killed => "Killed",
        }
    }
}

/// What the core intended when it stopped the process. SessionEnd's `reason`
/// is `other` both on crash and on suspend, so the core's intent decides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopIntent {
    #[default]
    None,
    Suspend,
    Kill,
}

/// Who started the running turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    /// Delivered by the app (carries a turn marker).
    App,
    /// Typed by the user in the pane: never blocked.
    User,
    /// `<task-notification>`: started by the CLI for finished background work (FR-410).
    SelfStarted,
}

/// The machine's memory of one agent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tracker {
    pub state: AgentState,
    pub session_id: Option<String>,
    /// `prompt_id` of the running turn.
    pub prompt_id: Option<String>,
    pub turn_origin: Option<TurnOrigin>,
    pub current_tool: Option<String>,
    /// Running background tasks reported by the last Stop.
    pub background_tasks: usize,
    /// Subagents between SubagentStart and SubagentStop.
    pub subagents: BTreeSet<String>,
    pub stop_intent: StopIntent,
    pub last_error: Option<String>,
    /// Messages queued by the CLI during a turn (same prompt_id).
    pub queued_messages: u32,
    pub turns: u32,
}

/// Side effects the runtime must carry out after a transition.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// SessionStart: input may be sent after the start delay.
    Ready,
    /// A new turn started.
    TurnStarted { origin: TurnOrigin, markers: Vec<String> },
    /// UserPromptSubmit with the running turn's `prompt_id`: a queued message, not a turn.
    QueuedMessage,
    /// The turn ended and the agent is idle (deliver queued work, tell the Governor).
    TurnEnded,
    /// The turn ended but background work runs on and will start a turn by itself.
    TurnEndedWithBackground(usize),
    /// StopFailure `rate_limit` (J9).
    RateLimited { message: Option<String> },
    /// StopFailure with another error.
    Failed { error: String },
    /// A permission dialog is up in the pane.
    NeedsUser,
}

/// Applies one hook event. Returns the effects, in order.
pub fn apply(t: &mut Tracker, event: &HookEvent, p: &HookPayload) -> Vec<Effect> {
    use AgentState as S;
    let mut fx = vec![];
    if let Some(sid) = &p.session_id {
        t.session_id = Some(sid.clone());
    }
    match event {
        HookEvent::SessionStart => {
            // `compact` restarts the session in place; the turn goes on.
            if p.source.as_deref() != Some("compact") {
                t.state = S::Idle;
                t.prompt_id = None;
                t.turn_origin = None;
                t.current_tool = None;
                fx.push(Effect::Ready);
            }
        }
        HookEvent::UserPromptSubmit => {
            let same_turn = t.prompt_id.is_some() && t.prompt_id == p.prompt_id && is_busy(t.state);
            if same_turn {
                t.queued_messages += 1;
                fx.push(Effect::QueuedMessage);
            } else {
                let markers = crate::delivery::find_markers(p.prompt.as_deref().unwrap_or(""));
                let origin = if p.is_task_notification() {
                    TurnOrigin::SelfStarted
                } else if !markers.is_empty() {
                    TurnOrigin::App
                } else {
                    TurnOrigin::User
                };
                t.state = S::Working;
                t.prompt_id = p.prompt_id.clone();
                t.turn_origin = Some(origin);
                t.current_tool = None;
                t.turns += 1;
                fx.push(Effect::TurnStarted { origin, markers });
            }
        }
        HookEvent::PreToolUse => {
            t.current_tool = p.tool_name.clone();
            if !matches!(t.state, S::WaitingGate | S::Queued) {
                t.state = S::Working;
            }
        }
        HookEvent::PermissionRequest => {
            t.state = S::WaitingForYou;
            fx.push(Effect::NeedsUser);
        }
        HookEvent::PostToolUse | HookEvent::PostToolUseFailure => {
            t.current_tool = None;
            if t.state.is_live() && t.state != S::Starting {
                t.state = S::Working;
            }
        }
        // `idle_prompt` comes ~60 s after Stop: nothing new.
        HookEvent::Notification => {
            if p.notification_type.as_deref() == Some("permission_prompt") {
                t.state = S::WaitingForYou;
                fx.push(Effect::NeedsUser);
            }
        }
        HookEvent::Stop => {
            t.current_tool = None;
            t.prompt_id = None;
            let bg = p.running_background_tasks();
            t.background_tasks = bg;
            if bg > 0 {
                // Not idle: background work will start a new turn by itself.
                t.state = S::Working;
                fx.push(Effect::TurnEndedWithBackground(bg));
            } else {
                t.state = S::Idle;
                fx.push(Effect::TurnEnded);
            }
            t.turn_origin = None;
        }
        HookEvent::StopFailure => {
            t.current_tool = None;
            t.prompt_id = None;
            t.turn_origin = None;
            let err = p.error.clone().unwrap_or_default();
            t.last_error = Some(err.clone());
            if err == "rate_limit" {
                t.state = S::BlockedByLimit;
                fx.push(Effect::RateLimited { message: p.last_assistant_message.clone() });
            } else {
                t.state = S::Error;
                fx.push(Effect::Failed { error: err });
            }
        }
        HookEvent::SubagentStart => {
            if let Some(id) = &p.agent_id {
                t.subagents.insert(id.clone());
            }
        }
        HookEvent::SubagentStop => {
            // Internal helpers (empty agent_type, no SubagentStart) carry no state.
            if !p.is_internal_subagent() {
                if let Some(id) = &p.agent_id {
                    t.subagents.remove(id);
                }
            }
        }
        // SessionEnd's reason can't tell a crash from a suspend: the process
        // exit (with the core's intent) decides, see `on_exit`.
        HookEvent::SessionEnd
        | HookEvent::PreCompact
        | HookEvent::PostCompact
        | HookEvent::PostToolBatch
        | HookEvent::PermissionDenied
        | HookEvent::Other(_) => {}
    }
    fx
}

fn is_busy(s: AgentState) -> bool {
    matches!(s, AgentState::Working | AgentState::WaitingForYou | AgentState::WaitingGate | AgentState::Queued)
}

/// The process exited. The core's own intent decides the state.
pub fn on_exit(t: &mut Tracker) {
    t.state = match t.stop_intent {
        StopIntent::Suspend => AgentState::Suspended,
        StopIntent::Kill => AgentState::Killed,
        StopIntent::None if t.state == AgentState::Archived => AgentState::Archived,
        StopIntent::None => AgentState::Error,
    };
    t.stop_intent = StopIntent::None;
    t.current_tool = None;
    t.prompt_id = None;
    t.turn_origin = None;
    t.subagents.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(t: &mut Tracker, name: &str, extra: serde_json::Value) -> Vec<Effect> {
        let mut v = json!({"hook_event_name": name, "session_id": "s1"});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        apply(t, &HookEvent::from(name.to_string()), &HookPayload::parse(&v))
    }

    #[test]
    fn basic_turn() {
        let mut t = Tracker { state: AgentState::Starting, ..Default::default() };
        assert_eq!(ev(&mut t, "SessionStart", json!({"source":"startup"})), [Effect::Ready]);
        assert_eq!(t.state, AgentState::Idle);
        assert_eq!(t.session_id.as_deref(), Some("s1"));
        let fx = ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1","prompt":"[t:0123abcd] hi"}));
        assert_eq!(fx, [Effect::TurnStarted { origin: TurnOrigin::App, markers: vec!["0123abcd".into()] }]);
        assert_eq!(t.state, AgentState::Working);
        ev(&mut t, "PreToolUse", json!({"prompt_id":"p1","tool_name":"Bash"}));
        assert_eq!(t.current_tool.as_deref(), Some("Bash"));
        ev(&mut t, "PostToolUse", json!({"prompt_id":"p1","tool_name":"Bash"}));
        assert_eq!(t.current_tool, None);
        assert_eq!(ev(&mut t, "Stop", json!({"prompt_id":"p1","background_tasks":[]})), [Effect::TurnEnded]);
        assert_eq!(t.state, AgentState::Idle);
        assert_eq!(t.turns, 1);
    }

    #[test]
    fn queued_message_same_prompt_id_is_not_a_new_turn() {
        let mut t = Tracker::default();
        ev(&mut t, "SessionStart", json!({}));
        ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1","prompt":"go"}));
        assert_eq!(t.turn_origin, Some(TurnOrigin::User));
        assert_eq!(
            ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1","prompt":"also this"})),
            [Effect::QueuedMessage]
        );
        assert_eq!(t.turns, 1);
        assert_eq!(t.queued_messages, 1);
        // A different prompt_id after the Stop is a new turn.
        ev(&mut t, "Stop", json!({}));
        let fx = ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p2","prompt":"next"}));
        assert!(matches!(fx[0], Effect::TurnStarted { origin: TurnOrigin::User, .. }));
        assert_eq!(t.turns, 2);
    }

    #[test]
    fn stop_with_background_tasks_is_not_idle_and_notification_is_self_started() {
        let mut t = Tracker::default();
        ev(&mut t, "SessionStart", json!({}));
        ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1","prompt":"spawn a subagent"}));
        ev(&mut t, "SubagentStart", json!({"agent_id":"sa1","agent_type":"general-purpose"}));
        assert_eq!(t.subagents.len(), 1);
        let fx = ev(&mut t, "Stop", json!({"background_tasks":[{"id":"sa1","type":"subagent","status":"running"}]}));
        assert_eq!(fx, [Effect::TurnEndedWithBackground(1)]);
        assert_eq!(t.state, AgentState::Working);
        assert_eq!(t.background_tasks, 1);
        // Internal SubagentStop: no state.
        ev(&mut t, "SubagentStop", json!({"agent_id":"x","agent_type":""}));
        assert_eq!(t.subagents.len(), 1);
        ev(&mut t, "SubagentStop", json!({"agent_id":"sa1","agent_type":"general-purpose"}));
        assert!(t.subagents.is_empty());
        let fx = ev(
            &mut t,
            "UserPromptSubmit",
            json!({"prompt_id":"p2","prompt":"<task-notification>\n<status>completed</status>"}),
        );
        assert!(matches!(fx[0], Effect::TurnStarted { origin: TurnOrigin::SelfStarted, .. }));
        assert_eq!(ev(&mut t, "Stop", json!({"background_tasks":[]})), [Effect::TurnEnded]);
        assert_eq!(t.state, AgentState::Idle);
    }

    #[test]
    fn stop_failure() {
        let mut t = Tracker::default();
        ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1"}));
        let fx = ev(
            &mut t,
            "StopFailure",
            json!({"error":"rate_limit","last_assistant_message":"You've hit your session limit"}),
        );
        assert_eq!(fx, [Effect::RateLimited { message: Some("You've hit your session limit".into()) }]);
        assert_eq!(t.state, AgentState::BlockedByLimit);
        ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p2"}));
        let fx = ev(&mut t, "StopFailure", json!({"error":"model_not_found"}));
        assert_eq!(fx, [Effect::Failed { error: "model_not_found".into() }]);
        assert_eq!(t.state, AgentState::Error);
    }

    #[test]
    fn permission_prompts_and_compaction() {
        let mut t = Tracker::default();
        ev(&mut t, "UserPromptSubmit", json!({"prompt_id":"p1"}));
        assert_eq!(ev(&mut t, "PermissionRequest", json!({"tool_name":"Write"})), [Effect::NeedsUser]);
        assert_eq!(t.state, AgentState::WaitingForYou);
        ev(&mut t, "PostToolUse", json!({}));
        assert_eq!(t.state, AgentState::Working);
        ev(&mut t, "Notification", json!({"notification_type":"permission_prompt"}));
        assert_eq!(t.state, AgentState::WaitingForYou);
        ev(&mut t, "PostToolUse", json!({}));
        // /compact restarts the session in place: SessionStart source=compact keeps the turn.
        assert!(ev(&mut t, "SessionStart", json!({"source":"compact"})).is_empty());
        assert_eq!(t.state, AgentState::Working);
        ev(&mut t, "Stop", json!({}));
        ev(&mut t, "Notification", json!({"notification_type":"idle_prompt"}));
        assert_eq!(t.state, AgentState::Idle);
    }

    #[test]
    fn session_end_is_ignored_exit_uses_intent() {
        let mut t = Tracker::default();
        ev(&mut t, "SessionStart", json!({}));
        ev(&mut t, "SessionEnd", json!({"reason":"other"}));
        assert_eq!(t.state, AgentState::Idle);
        t.stop_intent = StopIntent::Suspend;
        on_exit(&mut t);
        assert_eq!(t.state, AgentState::Suspended);
        t.state = AgentState::Idle;
        on_exit(&mut t);
        assert_eq!(t.state, AgentState::Error, "an exit nobody asked for is an error");
        t.stop_intent = StopIntent::Kill;
        on_exit(&mut t);
        assert_eq!(t.state, AgentState::Killed);
    }

    #[test]
    fn replays_recovery_sample_payloads() {
        // The scrubbed payloads recorded in the P0 recovery spike, in order.
        let samples: serde_json::Value =
            serde_json::from_str(include_str!("../../../spikes/recovery/samples/hook-payloads.json")).unwrap();
        let mut t = Tracker { state: AgentState::Idle, ..Default::default() };
        let order = [
            "PostToolUse (Agent tool, async_launched)",
            "SubagentStart (Agent tool, async)",
            "Stop (with a running background subagent)",
            "SubagentStop (general-purpose)",
            "UserPromptSubmit (task-notification turn started by the CLI)",
        ];
        for key in order {
            let v = &samples[key];
            assert!(!v.is_null(), "{key}");
            let name = v["hook_event_name"].as_str().unwrap().to_string();
            apply(&mut t, &HookEvent::from(name), &HookPayload::parse(v));
        }
        assert_eq!(t.state, AgentState::Working);
        assert_eq!(t.turn_origin, Some(TurnOrigin::SelfStarted));
        assert!(t.subagents.is_empty());
    }
}
