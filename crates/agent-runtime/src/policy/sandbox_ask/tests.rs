//! `request_sandbox_access`, the `sandbox` card and its decisions.

use orch_protocol::{InboxDecision, McpMsg};
use orch_store::InboxStatus;
use serde_json::{Value, json};

use super::*;
use crate::runtime::tests::{hook, insert_test_live, test_runtime};

/// Agent `a1` (role `back`) in a registered project.
fn setup() -> (tempfile::TempDir, tempfile::TempDir, std::sync::Arc<Runtime>, orch_store::Project) {
    let (d, rt) = test_runtime(true);
    let (r, repo) = orch_git::fixtures::scratch_repo();
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    rt.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            (a.repo, a.worktree) = (repo.clone(), repo.clone());
            a.role.name = "back".into();
        })
        .unwrap();
    (d, r, rt, p)
}

fn ask(rt: &Runtime, args: Value) -> (bool, String, Value) {
    rt.on_mcp(
        &McpMsg { agent_id: "a1".into(), tool: "request_sandbox_access".into(), arguments: args, deadline_ms: 10_000 },
        None,
    )
}

fn item(rt: &Runtime, id: u64) -> InboxItem {
    rt.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned()).unwrap()
}

fn turns(rt: &Runtime) -> Vec<String> {
    rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == "a1").map(|t| t.body.clone()).collect())
}

fn project_text(rt: &Runtime, p: &orch_store::Project) -> String {
    std::fs::read_to_string(rt.project_policy_path(p)).unwrap_or_default()
}

#[test]
fn labels_and_the_approved_message() {
    assert_eq!(label(SandboxKind::Network, Some("api.github.com")), "network api.github.com");
    assert_eq!(label(SandboxKind::LocalPorts, None), "local ports");
    assert_eq!(label(SandboxKind::UnixSocket, Some("/tmp/x.sock")), "unix socket /tmp/x.sock");
    assert_eq!(
        approved_message(&["network a.example".into()]),
        "ORCH: approved: network a.example is now allowed by the sandbox. Retry."
    );
    assert_eq!(
        approved_message(&["network a.example".into(), "local ports".into()]),
        "ORCH: approved: network a.example, local ports are now allowed by the sandbox. Retry."
    );
    assert_eq!(label_of(&json!({"kind": "network", "value": "a.example"})), "network a.example");
    assert_eq!(label_of(&json!({"kind": "local_ports", "value": null})), "local ports");
}

#[test]
fn a_request_is_validated_then_parks_on_a_sandbox_card() {
    let (_d, _r, rt, p) = setup();
    // Never allowed, bad input: a tool error, no card.
    let (err, text, _) = ask(&rt, json!({"kind": "write", "value": "~/.ssh", "why": "keys"}));
    assert!(err && text.contains("credentials"), "{text}");
    let (err, text, _) = ask(&rt, json!({"kind": "teleport", "why": "x"}));
    assert!(err && text.contains("kind"), "{text}");
    let (err, text, _) = ask(&rt, json!({"kind": "network", "value": "a.example"}));
    assert!(err && text.contains("why"), "{text}");
    let long = "x".repeat(WHY_MAX + 1);
    assert!(ask(&rt, json!({"kind": "network", "value": "a.example", "why": long})).0);
    assert!(rt.inbox().is_empty());

    let (err, text, data) = ask(&rt, json!({"kind": "network", "value": "API.github.com", "why": "release check"}));
    assert!(!err, "{text}");
    assert_eq!(text, ASKED);
    let id = data["item"].as_u64().unwrap();
    let i = item(&rt, id);
    assert_eq!((i.kind.as_str(), i.status, i.priority), (KIND, InboxStatus::Parked, PRIORITY_BLOCKING));
    assert_eq!(
        i.payload,
        json!({"kind": "network", "value": "api.github.com", "why": "release check", "project": p.id,
               "role": "back", "agent": "a1"})
    );
    assert!(rt.is_parked("a1"), "its Stop parks it like a parked permission");
    // The same request again: no second card.
    let (_, text, data) = ask(&rt, json!({"kind": "network", "value": "api.github.com", "why": "again"}));
    assert_eq!((text.as_str(), data["item"].as_u64()), (ALREADY_ASKED, Some(id)));
    assert_eq!(rt.inbox().len(), 1);
    // A card of its own in the decisions, never the Overwatch's.
    assert_eq!(crate::decision::decision_id(KIND, id), format!("sandbox-{id}"));
    let e = rt.decide_as(id, InboxDecision::Approve, None, None, crate::overwatch::ID).unwrap_err();
    assert!(e.to_string().contains("only the user"), "{e}");
    assert!(rt.db.audit_log(None, 10).unwrap().iter().any(|e| e.action == "sandbox_request"));
}

#[test]
fn approve_adds_to_the_project_file_and_restarts_with_the_message() {
    let (_d, _r, rt, p) = setup();
    let id = ask(&rt, json!({"kind": "network", "value": "api.github.com", "why": "release check"})).2["item"]
        .as_u64()
        .unwrap();
    // A scope that isn't one is refused before anything is claimed.
    assert!(rt.decide(id, InboxDecision::Approve, Some("galaxy"), None).is_err());
    assert!(item(&rt, id).open());
    let r = rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(r["effect"], "allowed");
    assert_eq!((r["outcome"]["created"].as_bool(), r["outcome"]["scope"].as_str()), (Some(true), Some("project")));
    assert_eq!(r["outcome"]["entry"]["file"], format!("project:{}", p.id));
    assert_eq!(r["outcome"]["entry"]["id"], "s1");
    let text = project_text(&rt, &p);
    assert!(text.contains("network = \"api.github.com\"") && text.contains("note = \"release check\""), "{text}");
    assert!(!text.contains("role ="), "project scope: every role");
    let i = item(&rt, id);
    assert_eq!(i.payload["outcome"], r["outcome"]);
    assert_eq!((i.status, i.decided_by.as_deref()), (InboxStatus::Sent, Some("user")));
    // Not running: the message resumes it on the new settings.
    assert_eq!(turns(&rt), ["ORCH: approved: network api.github.com is now allowed by the sandbox. Retry."]);
    let log = rt.db.audit_log(None, 10).unwrap();
    assert!(log.iter().any(|e| e.action == KIND && e.decision == "approve" && e.actor == "user"));
    assert!(log.iter().any(|e| e.action == "sandbox_add" && e.actor == "user"));
    // Now in its sandbox: asked again, it is already allowed.
    let (_, text, _) = ask(&rt, json!({"kind": "network", "value": "api.github.com", "why": "again"}));
    assert_eq!(text, ALREADY_ALLOWED);
    // A role host is allowed already too.
    assert_eq!(ask(&rt, json!({"kind": "network", "value": "crates.io", "why": "deps"})).1, ALREADY_ALLOWED);
}

#[test]
fn approve_globally_or_for_the_role_and_a_duplicate_is_not_created() {
    let (_d, _r, rt, p) = setup();
    let id = ask(&rt, json!({"kind": "local_ports", "why": "dev server"})).2["item"].as_u64().unwrap();
    let r = rt.decide(id, InboxDecision::Approve, Some("role"), None).unwrap();
    assert_eq!(r["outcome"]["entry"]["role"], "back");
    assert!(project_text(&rt, &p).contains("role = \"back\""));
    let id = ask(&rt, json!({"kind": "network", "value": "g.example", "why": "x"})).2["item"].as_u64().unwrap();
    let r = rt.decide(id, InboxDecision::Approve, Some("global"), None).unwrap();
    assert_eq!(r["outcome"]["entry"]["file"], "global");
    assert!(std::fs::read_to_string(rt.global_policy_path()).unwrap().contains("g.example"));
    // Added by hand meanwhile: the Approve finds the entry (no Undo for it).
    let id = ask(&rt, json!({"kind": "network", "value": "h.example", "why": "x"})).2["item"].as_u64().unwrap();
    rt.sandbox_add("project", Some(&p.id), "network", Some("h.example"), None, None).unwrap();
    let r = rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(r["outcome"]["created"], false);
}

#[test]
fn reject_and_reply_tell_the_agent_and_write_nothing() {
    let (_d, _r, rt, p) = setup();
    let a = ask(&rt, json!({"kind": "network", "value": "a.example", "why": "x"})).2["item"].as_u64().unwrap();
    let b = ask(&rt, json!({"kind": "unix_socket", "value": "/tmp/t.sock", "why": "x"})).2["item"].as_u64().unwrap();
    assert_eq!(rt.decide(a, InboxDecision::Reject, None, None).unwrap()["effect"], "denied");
    rt.decide(b, InboxDecision::Reply, None, Some("use the TCP port")).unwrap();
    assert_eq!(
        turns(&rt),
        [
            "ORCH: denied: sandbox access to network a.example.",
            "ORCH: denied: sandbox access to unix socket /tmp/t.sock. The user said: use the TCP port",
        ]
    );
    assert!(project_text(&rt, &p).is_empty());
    assert!(!rt.is_parked("a1"));
    assert!(rt.decide(a, InboxDecision::Approve, None, None).is_err(), "decided once");
}

#[test]
fn a_value_invalid_at_decision_time_refuses_approve() {
    let (_d, _r, rt, p) = setup();
    // Valid when asked; the guard says otherwise now.
    let payload =
        json!({"kind": "write", "value": "~/.ssh", "why": "x", "project": p.id, "role": "back", "agent": "a1"});
    let id = rt.add_item(Some("a1"), KIND, "a1: sandbox access".into(), payload, PRIORITY_BLOCKING).unwrap();
    let e = rt.decide(id, InboxDecision::Approve, None, None).unwrap_err();
    assert!(e.to_string().contains("credentials"), "{e}");
    let i = item(&rt, id);
    assert!(i.open());
    assert!(i.payload["invalid"].as_str().unwrap().contains("credentials"));
    // Deny still works.
    rt.decide(id, InboxDecision::Reject, None, None).unwrap();
}

#[test]
fn a_running_agent_restarts_at_its_stop_once_for_every_approval() {
    let (d, _r, rt, p) = setup();
    insert_test_live(&rt, "a1", None, 0, d.path());
    std::fs::create_dir_all(rt.cfg.paths.agent_dir("a1")).unwrap();
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Working).unwrap();
    let a = ask(&rt, json!({"kind": "network", "value": "a.example", "why": "x"})).2["item"].as_u64().unwrap();
    let b = ask(&rt, json!({"kind": "local_ports", "why": "x"})).2["item"].as_u64().unwrap();
    rt.decide(a, InboxDecision::Approve, None, None).unwrap();
    rt.decide(b, InboxDecision::Approve, None, None).unwrap();
    // Still in its turn: nothing interrupts it.
    assert!(turns(&rt).is_empty());
    assert_eq!(rt.house().sandbox_restart["a1"], ["network a.example", "local ports"]);
    // In its sandbox now, not in the settings it runs on: it restarts to get it.
    let (_, text, _) = ask(&rt, json!({"kind": "network", "value": "a.example", "why": "x"}));
    assert_eq!(text, format!("{ALREADY_ALLOWED}{RESTART_TO_APPLY}"));
    rt.on_hook(&hook("Stop", json!({})), None);
    assert_eq!(
        turns(&rt),
        ["ORCH: approved: network a.example, local ports are now allowed by the sandbox. Retry."],
        "one restart, one message"
    );
    assert!(rt.house().sandbox_restart.is_empty());
    let ev = rt.db.events(0, Some("a1"), Some("sandbox_restart"), 5).unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].detail["restarted"], true);
    assert!(project_text(&rt, &p).contains("local_ports = true"));
}

#[test]
fn an_approval_survives_a_core_restart_before_the_agents_stop() {
    let (d, _r, rt, _p) = setup();
    insert_test_live(&rt, "a1", None, 0, d.path());
    std::fs::create_dir_all(rt.cfg.paths.agent_dir("a1")).unwrap();
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Working).unwrap();
    let a = ask(&rt, json!({"kind": "network", "value": "a.example", "why": "x"})).2["item"].as_u64().unwrap();
    let b = ask(&rt, json!({"kind": "local_ports", "why": "x"})).2["item"].as_u64().unwrap();
    rt.decide(a, InboxDecision::Approve, None, None).unwrap();
    rt.decide(b, InboxDecision::Approve, None, None).unwrap();
    // Kept on the decided items, not only in memory.
    for (id, what) in [(a, "network a.example"), (b, "local ports")] {
        let p = item(&rt, id).payload;
        assert_eq!((p["restart_pending"].clone(), p["approved"].as_str()), (json!(true), Some(what)));
    }
    // The core restarted: the in-memory list is gone, the items are not.
    rt.house().sandbox_restart.clear();
    rt.sandbox_restarts_after_restart();
    assert_eq!(rt.house().sandbox_restart["a1"], ["network a.example", "local ports"]);
    assert!(turns(&rt).is_empty(), "still in its turn: it waits for its Stop");
    rt.on_hook(&hook("Stop", json!({})), None);
    assert_eq!(turns(&rt), ["ORCH: approved: network a.example, local ports are now allowed by the sandbox. Retry."]);
    assert_eq!(item(&rt, a).payload["restart_pending"], false);
    assert_eq!(item(&rt, b).payload["restart_pending"], false);
    // Done: another start does nothing.
    rt.sandbox_restarts_after_restart();
    assert!(rt.house().sandbox_restart.is_empty());

    // An agent that is gone: the flag is cleared, nobody is told.
    let c = ask(&rt, json!({"kind": "network", "value": "c.example", "why": "x"})).2["item"].as_u64().unwrap();
    // (The restart above stopped its process: a new one, in a turn again.)
    insert_test_live(&rt, "a1", None, 0, d.path());
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Working).unwrap();
    rt.decide(c, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(item(&rt, c).payload["restart_pending"], true);
    rt.house().sandbox_restart.clear();
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Archived).unwrap();
    let before = turns(&rt).len();
    rt.sandbox_restarts_after_restart();
    assert_eq!((turns(&rt).len(), item(&rt, c).payload["restart_pending"].clone()), (before, json!(false)));
}
