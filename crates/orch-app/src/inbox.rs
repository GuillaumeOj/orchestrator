//! Inbox (FR-707, §9.6): everything that needs the user, in attention
//! ladder order. Level 1 blocking (permission prompts held for the Inbox,
//! gates: spec approval and pull requests, questions from team agents, agents
//! waiting for you, trust dialogs), level 2 errors (crashes, API errors,
//! runtime alerts), level 3 budget (the Governor's pending ask), level 4 info
//! (finished turns, task-sync notes; a silent digest). Within a level, oldest
//! first.

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InboxItem {
    /// Stable across refreshes and restarts: `inbox-3` (a permission, the store's
    /// id), `gate-5`, `q-6`, `wait-a1`, `alert-7`, `ask-12`, `info-<turn>`.
    pub id: String,
    pub level: u8,
    /// permission / gate / question / trust / error / alert / budget / info
    pub kind: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub detail: String,
    /// Monospace preview (a command, the pane's last lines).
    pub preview: Option<String>,
    pub created_ms: u64,
    /// Keys this item answers to, besides j/k/x (FR-707).
    pub actions: Vec<&'static str>,
    /// A gate's or a team question's details for the preview (`action`:
    /// `spec_approval` with `docs`, `summary`, `feature`; `push_pr` with
    /// `title`, `body`, `branch`, `base`; `ask_user` with `question`, `options`).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub payload: Value,
    /// `open`, or `parked`: a permission whose hold expired; the agent's turn
    /// ended and it resumes on the answer (FR-705).
    pub status: &'static str,
}

/// What the Inbox needs to know about an agent.
#[derive(Clone, Debug, Default)]
pub struct AgentFacts {
    pub id: String,
    pub state: String,
    pub since_ms: u64,
    pub trust_dialog: bool,
    pub last_error: Option<String>,
    pub tail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub id: u64,
    /// The store item's kind: `gate` and `question` are level 1, `budget`
    /// (the Governor's warnings) level 3, `info` level 4, the rest level 2.
    pub kind: String,
    pub agent_id: Option<String>,
    pub body: String,
    pub created_ms: u64,
    pub payload: Value,
    /// A permission whose hold expired (FR-705).
    pub parked: bool,
}

#[derive(Clone, Debug)]
pub struct Ask {
    pub id: u64,
    pub reason: String,
    pub effect: String,
    pub created_ms: u64,
    pub deadline_ms: i64,
    pub default_apply: bool,
}

#[derive(Clone, Debug)]
pub struct Done {
    pub turn_id: String,
    pub agent_id: String,
    pub body: String,
    pub at_ms: u64,
}

#[derive(Default)]
pub struct Inputs<'a> {
    pub agents: &'a [AgentFacts],
    pub alerts: &'a [Alert],
    pub ask: Option<Ask>,
    pub done: &'a [Done],
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

pub fn build(i: &Inputs) -> Vec<InboxItem> {
    let mut out = vec![];
    for a in i.agents {
        // Its prompt is a permission card already (held or parked): not listed twice.
        let held = i.alerts.iter().any(|al| al.kind == "permission" && al.agent_id.as_deref() == Some(a.id.as_str()));
        if a.trust_dialog {
            out.push(InboxItem {
                id: format!("trust-{}", a.id),
                level: 1,
                kind: "trust".into(),
                agent_id: Some(a.id.clone()),
                title: "Claude Code asks to trust this folder".into(),
                detail: format!("{}: answer it in the pane (the app never does)", a.id),
                preview: a.tail.as_deref().map(|t| last_lines(t, 6)),
                created_ms: a.since_ms,
                actions: vec!["o"],
                payload: Value::Null,
                status: "open",
            });
        } else if a.state == "waiting_for_you" && !held {
            out.push(InboxItem {
                id: format!("wait-{}", a.id),
                level: 1,
                kind: "question".into(),
                agent_id: Some(a.id.clone()),
                title: format!("{} is waiting for you", a.id),
                detail: "A prompt or question in its pane".into(),
                preview: a.tail.as_deref().map(|t| last_lines(t, 8)),
                created_ms: a.since_ms,
                actions: vec!["enter", "o"],
                payload: Value::Null,
                status: "open",
            });
        } else if a.state == "error" {
            out.push(InboxItem {
                id: format!("error-{}", a.id),
                level: 2,
                kind: "error".into(),
                agent_id: Some(a.id.clone()),
                title: format!("{} stopped with an error", a.id),
                detail: a.last_error.clone().unwrap_or_else(|| "Its session is saved; resume it from its pane.".into()),
                preview: None,
                created_ms: a.since_ms,
                actions: vec!["o", "r"],
                payload: Value::Null,
                status: "open",
            });
        }
    }
    for al in i.alerts {
        let title = al.body.lines().next().unwrap_or("").to_string();
        let s = |k: &str| al.payload.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let item = match (al.kind.as_str(), s("action").as_str()) {
            // A start that never happened (FR-415): open the pane, restart fresh, or dismiss.
            ("startup", _) => {
                let mut actions = vec!["o", "restart_fresh", "dismiss"];
                if al.payload.get("failed").is_some_and(Value::is_string) {
                    actions.push("delete");
                }
                InboxItem {
                    id: ui_id("startup", al.id),
                    level: 2,
                    kind: "startup".into(),
                    agent_id: al.agent_id.clone(),
                    title,
                    detail: match al.payload.get("failed").and_then(Value::as_str) {
                        Some(f) => match al.payload.get("exit_status").and_then(Value::as_u64) {
                            Some(st) => format!("Its {f} start exited at once (status {st})"),
                            None => format!("Its {f} start exited at once"),
                        },
                        None => "No SessionStart yet: a prompt or dialog may be waiting in its pane".into(),
                    },
                    preview: Some(last_lines(&s("tail"), 8)).filter(|t| !t.is_empty()),
                    created_ms: al.created_ms,
                    actions,
                    payload: al.payload.clone(),
                    status: "open",
                }
            }
            // A permission prompt (FR-705), held for the Inbox or parked.
            ("permission", _) => {
                let p = &al.payload;
                let tool = s("tool");
                let summary = Some(s("summary")).filter(|v| !v.is_empty()).unwrap_or_else(|| al.body.clone());
                let agent = al.agent_id.clone().unwrap_or_default();
                let mut actions = vec!["a", "r", "enter", "o"];
                if !al.parked {
                    actions.push("in_pane");
                }
                if p.get("always").is_some_and(|v| v.is_string()) && p.get("ask_rule").is_none_or(Value::is_null) {
                    actions.push("always");
                }
                InboxItem {
                    id: ui_id(&al.kind, al.id),
                    level: 1,
                    kind: "permission".into(),
                    agent_id: al.agent_id.clone(),
                    title: format!("{tool}: {}", p.get("description").and_then(Value::as_str).unwrap_or(&summary)),
                    detail: if al.parked {
                        format!("{agent} stopped and resumes on your answer ({tool})")
                    } else {
                        format!("{agent} asks to run {tool}")
                    },
                    preview: Some(summary),
                    created_ms: al.created_ms,
                    actions,
                    payload: al.payload.clone(),
                    status: if al.parked { "parked" } else { "open" },
                }
            }
            // Gates (FR-704): decided through the runtime, which acts on them.
            ("gate", action) => InboxItem {
                id: ui_id(&al.kind, al.id),
                level: 1,
                kind: "gate".into(),
                agent_id: al.agent_id.clone(),
                title,
                detail: match action {
                    "spec_approval" => format!(
                        "Spec gate: approve the feature docs so the lead can create tasks and the team can start ({})",
                        s("branch")
                    ),
                    "push_pr" => format!("Push {} and open a pull request into {}", s("branch"), s("base")),
                    "restart_gate" => {
                        format!("Still waiting when the core restarted: {}. Approve to let it proceed", s("detail"))
                    }
                    _ => al.body.clone(),
                },
                preview: match action {
                    "spec_approval" => Some(s("summary")),
                    "push_pr" => Some(s("body")).filter(|b| !b.is_empty()),
                    "restart_gate" => Some(s("detail")).filter(|d| !d.is_empty()),
                    _ => None,
                },
                created_ms: al.created_ms,
                actions: if action == "restart_gate" { vec!["a", "r", "o"] } else { vec!["a", "r", "enter", "o"] },
                payload: al.payload.clone(),
                status: "open",
            },
            // A team agent's question (`ask_user`): the reply becomes its next turn.
            ("question", _) => InboxItem {
                id: ui_id(&al.kind, al.id),
                level: 1,
                kind: "question".into(),
                agent_id: al.agent_id.clone(),
                title,
                detail: s("question"),
                preview: None,
                created_ms: al.created_ms,
                actions: vec!["enter", "r", "o"],
                payload: al.payload.clone(),
                status: "open",
            },
            // FR-505: a comment on a published Notion page (level 3); `open` → the page.
            ("notion", _) => InboxItem {
                id: format!("alert-{}", al.id),
                level: 3,
                kind: "question".into(),
                agent_id: None,
                title,
                detail: al.payload["text"].as_str().unwrap_or(&al.body).to_string(),
                preview: None,
                created_ms: al.created_ms,
                // `o` opens the page, `r` dismisses (any action but open/reconnect dismisses an alert).
                actions: vec!["o", "r"],
                payload: al.payload.clone(),
                status: "open",
            },
            (kind, _) => InboxItem {
                id: ui_id(kind, al.id),
                level: match kind {
                    "budget" => 3,
                    "info" => 4,
                    _ => 2,
                },
                // Still an alert to dismiss: `budget` items are asks (Apply/Skip).
                kind: if kind == "info" { "info".into() } else { "alert".into() },
                agent_id: al.agent_id.clone(),
                title,
                detail: al.body.clone(),
                preview: None,
                created_ms: al.created_ms,
                actions: if al.payload["alert"] == "mcp_needs_auth" { vec!["a", "o", "r"] } else { vec!["a", "o"] },
                // `{"alert": "mcp_needs_auth", "server", "name", "roles"}` (FR-604).
                payload: if al.payload.is_object() { al.payload.clone() } else { Value::Null },
                status: "open",
            },
        };
        out.push(item);
    }
    if let Some(a) = &i.ask {
        out.push(InboxItem {
            id: format!("ask-{}", a.id),
            level: 3,
            kind: "budget".into(),
            agent_id: None,
            title: a.reason.clone(),
            detail: format!(
                "{} · default: {} at the deadline",
                a.effect,
                if a.default_apply { "apply" } else { "skip" }
            ),
            preview: None,
            created_ms: a.created_ms,
            actions: vec!["a", "r"],
            payload: Value::Null,
            status: "open",
        });
    }
    for d in i.done {
        out.push(InboxItem {
            id: format!("info-{}", d.turn_id),
            level: 4,
            kind: "info".into(),
            agent_id: Some(d.agent_id.clone()),
            title: format!("{} finished a turn", d.agent_id),
            detail: d.body.chars().take(120).collect(),
            preview: None,
            created_ms: d.at_ms,
            actions: vec!["o"],
            payload: Value::Null,
            status: "open",
        });
    }
    out.sort_by(|a, b| a.level.cmp(&b.level).then(a.created_ms.cmp(&b.created_ms)).then(a.id.cmp(&b.id)));
    out
}

/// The listed id of a store Inbox item (as [`build`] names it).
pub fn ui_id(kind: &str, id: u64) -> String {
    agent_runtime::attention::decision_id(kind, id)
}

/// An `inbox_outcome` event for the UI (api-contract §3): the runtime's
/// outcome with the item's listed id.
pub fn outcome_event(v: &Value) -> Value {
    let kind = v["kind"].as_str().unwrap_or("");
    serde_json::json!({
        "item": ui_id(kind, v["item"].as_u64().unwrap_or(0)),
        "decided_by": v["decided_by"], "effect": v["effect"], "at_ms": v["at_ms"],
    })
}

/// The badge counts levels 1–3 (§9.6); level 4 stays in the digest.
pub fn badge(items: &[InboxItem]) -> usize {
    items.iter().filter(|i| i.level <= 3).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, state: &str, since: u64) -> AgentFacts {
        AgentFacts { id: id.into(), state: state.into(), since_ms: since, ..Default::default() }
    }

    #[test]
    fn ladder_order_and_badge() {
        let agents = [
            agent("front-1", "waiting_for_you", 40), // held for the Inbox: not listed twice
            agent("solo", "waiting_for_you", 10),
            agent("ci-1", "error", 5),
            agent("back-1", "working", 1),
        ];
        let alert = |id, kind: &str, body: &str, created_ms| Alert {
            id,
            kind: kind.into(),
            agent_id: None,
            body: body.into(),
            created_ms,
            payload: Value::Null,
            parked: false,
        };
        let perm = Alert {
            agent_id: Some("front-1".into()),
            payload: serde_json::json!({"tool": "Bash", "summary": "pnpm add x", "description": "Add the SDK",
                                        "tool_input": {"command": "pnpm add x"}, "always": "Bash(pnpm add x)"}),
            ..alert(1, "permission", "front-1: Bash: pnpm add x", 50)
        };
        let alerts = [perm, alert(7, "alert", "x: API error", 1), alert(8, "budget", "reserve used up", 0)];
        let done = [Done { turn_id: "t1".into(), agent_id: "back-1".into(), body: "go".into(), at_ms: 0 }];
        let items = build(&Inputs {
            agents: &agents,
            alerts: &alerts,
            ask: Some(Ask {
                id: 12,
                reason: "Defer infra".into(),
                effect: "saves 6 pp".into(),
                created_ms: 0,
                deadline_ms: 0,
                default_apply: true,
            }),
            done: &done,
        });
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["wait-solo", "inbox-1", "alert-7", "error-ci-1", "alert-8", "ask-12", "info-t1"]);
        assert_eq!((items[4].level, items[4].kind.as_str()), (3, "alert"), "a budget warning, dismissed like an alert");
        assert_eq!(badge(&items), 6);
        assert_eq!(items[1].preview.as_deref(), Some("pnpm add x"));
        assert_eq!(items[1].title, "Bash: Add the SDK");
        assert_eq!((items[1].status, items[1].payload["tool_input"]["command"].as_str()), ("open", Some("pnpm add x")));
        assert_eq!(items[1].actions, ["a", "r", "enter", "o", "in_pane", "always"]);
    }

    #[test]
    fn a_parked_permission_has_no_pane_answer_and_no_wait_card() {
        let perm = |id, parked, payload| Alert {
            id,
            kind: "permission".into(),
            agent_id: Some("a1".into()),
            body: "a1: Bash: ls".into(),
            created_ms: id,
            payload,
            parked,
        };
        let ask = serde_json::json!({"tool": "Bash", "summary": "ls", "ask_rule": 3, "always": null});
        let alerts = [
            perm(1, true, serde_json::json!({"tool": "Bash", "summary": "ls", "always": "Bash(ls)"})),
            perm(2, false, ask),
        ];
        let items =
            build(&Inputs { agents: &[agent("a1", "waiting_for_you", 0)], alerts: &alerts, ..Default::default() });
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["inbox-1", "inbox-2"], "no wait-a1 card");
        assert_eq!((items[0].status, items[0].actions.clone()), ("parked", vec!["a", "r", "enter", "o", "always"]));
        assert!(items[0].detail.contains("resumes on your answer"));
        assert_eq!(items[1].actions, ["a", "r", "enter", "o", "in_pane"], "an ask rule sent it: no always");
        let ev = outcome_event(&serde_json::json!({"item": 2, "kind": "permission", "decided_by": "pane",
                                                   "effect": "in_pane", "at_ms": 5}));
        assert_eq!(ev, serde_json::json!({"item": "inbox-2", "decided_by": "pane", "effect": "in_pane", "at_ms": 5}));
    }

    #[test]
    fn gates_questions_and_info_from_the_runtime() {
        let mk = |id, kind: &str, payload: Value| Alert {
            id,
            kind: kind.into(),
            agent_id: Some("fx-lead-1".into()),
            body: format!("item {id}"),
            created_ms: id,
            payload,
            parked: false,
        };
        let alerts = [
            mk(3, "info", Value::Null),
            mk(
                7,
                "gate",
                serde_json::json!({"action": "restart_gate", "agent": "fx-lead-1", "detail": "git push", "restarted": true}),
            ),
            mk(
                4,
                "gate",
                serde_json::json!({"action": "push_pr", "branch": "orch/fx/integration", "base": "main", "title": "Fix X", "body": "b"}),
            ),
            mk(
                5,
                "gate",
                serde_json::json!({"action": "spec_approval", "summary": "two tasks", "branch": "orch/fx/integration", "docs": ["spec.md"]}),
            ),
            mk(6, "question", serde_json::json!({"action": "ask_user", "question": "Keep v1?"})),
        ];
        let items = build(&Inputs { alerts: &alerts, ..Default::default() });
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["gate-4", "gate-5", "q-6", "gate-7", "alert-3"]);
        assert!(items[3].detail.contains("git push") && !items[3].detail.contains("Spec gate"));
        assert_eq!(items[3].actions, ["a", "r", "o"]);
        assert_eq!((items[0].level, items[0].kind.as_str()), (1, "gate"));
        assert!(items[0].detail.contains("orch/fx/integration") && items[0].detail.contains("main"));
        assert_eq!(items[1].preview.as_deref(), Some("two tasks"));
        assert_eq!(items[1].payload["docs"][0], "spec.md");
        assert_eq!((items[2].kind.as_str(), items[2].detail.as_str()), ("question", "Keep v1?"));
        assert_eq!((items[4].level, items[4].kind.as_str()), (4, "info"));
        assert_eq!(badge(&items), 4);
    }

    #[test]
    fn startup_cards_offer_a_fresh_restart_and_delete_once_failed() {
        let st = |id, payload| Alert {
            id,
            kind: "startup".into(),
            agent_id: Some("a1".into()),
            body: "a1 did not start".into(),
            created_ms: id,
            payload,
            parked: false,
        };
        let alerts = [
            st(1, serde_json::json!({"waiting_on": "startup", "mode": "new", "tail": "x\nAccept the terms?\n"})),
            st(2, serde_json::json!({"waiting_on": "startup", "failed": "fresh", "exit_status": 1, "tail": ""})),
        ];
        let items = build(&Inputs { alerts: &alerts, ..Default::default() });
        assert_eq!((items[0].id.as_str(), items[0].level, items[0].kind.as_str()), ("startup-1", 2, "startup"));
        assert_eq!(items[0].actions, ["o", "restart_fresh", "dismiss"]);
        assert_eq!(items[0].preview.as_deref(), Some("x\nAccept the terms?"));
        assert_eq!(items[1].actions, ["o", "restart_fresh", "dismiss", "delete"]);
        assert!(items[1].detail.contains("fresh start exited at once"));
        assert_eq!(ui_id("startup", 2), "startup-2");
    }

    #[test]
    fn trust_dialog_wins_over_waiting() {
        let mut a = agent("a1", "starting", 3);
        a.trust_dialog = true;
        a.tail = Some("\n\nQuick safety check\n\n 1. Yes, I trust this folder\n".into());
        let items = build(&Inputs { agents: &[a], ..Default::default() });
        assert_eq!(items[0].kind, "trust");
        assert_eq!(items[0].preview.as_deref(), Some("Quick safety check\n 1. Yes, I trust this folder"));
    }
}
