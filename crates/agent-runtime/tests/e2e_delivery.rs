//! fix-team-getting-stucked §3.8, the smoke tests: the chaos fake CLI
//! (tests/fake_claude.sh, its switches are sentinel files in the agent's
//! worktree) on a real PTY and the real orch-hook, for CI. The full F1–F9
//! suite is in process, in `src/chaos.rs` (it runs anywhere, no PTY or socket).

use agent_runtime::handover::ACK_KEY;
use orch_protocol::{CtlRequest, now_ms};
use serde_json::{Value, json};
use std::path::PathBuf;

mod common;
use common::{Core, Screens, agent, wait_for};

/// The agent's worktree: where the fake looks for its switches.
fn worktree(core: &Core, id: &str) -> PathBuf {
    PathBuf::from(agent(&core.sock, id)["worktree"].as_str().expect("a worktree"))
}

/// Sets a switch of the fake (`content` is its parameter, if it has one).
fn set(core: &Core, id: &str, switch: &str, content: &str) {
    std::fs::write(worktree(core, id).join(switch), content).unwrap();
}

fn unset(core: &Core, id: &str, switch: &str) {
    let _ = std::fs::remove_file(worktree(core, id).join(switch));
}

/// A user message for the agent; its turn id.
fn deliver(core: &Core, id: &str, text: &str) -> String {
    let r = request(core, CtlRequest::Deliver { agent_id: id.into(), text: text.into() });
    r["turn"].as_str().expect("a turn").to_string()
}

fn request(core: &Core, req: CtlRequest) -> Value {
    let r = agent_runtime::server::request(&core.sock, req).unwrap();
    assert!(r.ok, "{r:?}");
    r.data
}

fn turn_status(core: &Core, turn: &str) -> String {
    let s = request(core, CtlRequest::State);
    let t = s["turns"].as_array().unwrap().iter().find(|t| t["id"] == turn).cloned().unwrap_or(Value::Null);
    t["status"].as_str().unwrap_or("").to_string()
}

fn events(core: &Core, id: &str, kind: &str) -> Vec<Value> {
    core.rt.db.flush().unwrap();
    core.rt.db.events(0, Some(id), Some(kind), 100).unwrap().into_iter().map(|e| e.detail).collect()
}

/// The Inbox items of a kind (`delivery`, `startup`).
fn items(core: &Core, kind: &str) -> Vec<Value> {
    request(core, CtlRequest::Inbox).as_array().unwrap().iter().filter(|i| i["kind"] == kind).cloned().collect()
}

/// A core whose paste watchdog acts after 1 s, with agent `a` started.
fn core_with_agent(prefix: &str, tweak: impl FnOnce(&mut agent_runtime::runtime::RuntimeConfig)) -> Core {
    let core = Core::start(prefix, tweak);
    core.rt.db.set_setting(ACK_KEY, &json!(1)).unwrap();
    core.start_agent("a");
    core
}

/// Waits for the turn, running the paste watchdog the way the 5 s
/// housekeeping does, without waiting for its tick.
fn until_done(core: &Core, turn: &str, secs: u64) {
    wait_for(&format!("turn {turn} to be done"), secs, || {
        core.rt.paste_watchdog(now_ms());
        turn_status(core, turn) == "done"
    });
}

/// The marker of `turn` is on the fake's screen: the model saw the message.
fn seen_by_model(core: &Core, screens: &Screens, id: &str, turn: &str) {
    let marker = format!("[t:{turn}]");
    wait_for(&format!("{marker} on the screen"), 10, || screens.read(&core.sock, id).contains(&marker));
}

/// The agent can receive the next message.
fn next_message_arrives(core: &Core, screens: &Screens, id: &str) {
    let turn = deliver(core, id, "and the next one");
    until_done(core, &turn, 20);
    seen_by_model(core, screens, id, &turn);
    wait_for("idle again", 10, || core.state_of(id) == "idle");
}

/// F1: the Enter was lost, the text sits in the input box. The watchdog
/// (20 s in production) sends Enter once more.
#[test]
fn f1_a_swallowed_enter_is_sent_again() {
    let core = core_with_agent("orchd1a", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-swallow-enter", "");
    let turn = deliver(&core, "a", "second");
    until_done(&core, &turn, 20);
    seen_by_model(&core, &screens, "a", &turn);
    let ev = events(&core, "a", "redelivered");
    assert!(ev.iter().any(|e| e["how"] == "enter"), "{ev:?}");
    next_message_arrives(&core, &screens, "a");
}

/// F2: an API error (the process lives) leaves the agent idle with one
/// Continue turn; a second failure on that turn raises a decision, no loop.
#[test]
fn f2_a_server_error_gets_one_continue_and_a_second_failure_a_decision() {
    let core = core_with_agent("orchd2a", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-stop-failure", "server_error");
    let turn = deliver(&core, "a", "second");
    wait_for("the Continue turn on the screen", 20, || {
        screens.read(&core.sock, "a").contains("your last turn failed (server_error)")
    });
    wait_for("idle", 10, || core.state_of("a") == "idle");
    assert!(["done", "failed"].contains(&turn_status(&core, &turn).as_str()));
    assert!(events(&core, "a", "recovered").iter().any(|e| e["from_state"] == "error"));
    // The same error on the Continue turn: a decision instead of another.
    set(&core, "a", ".fake-stop-failure-sticky", "server_error");
    deliver(&core, "a", "third");
    wait_for("a decision", 30, || !items(&core, "delivery").is_empty());
    wait_for("idle", 10, || core.state_of("a") == "idle");
    assert!(!events(&core, "a", "continue_failed").is_empty(), "the second failure stops the chain");
    unset(&core, "a", ".fake-stop-failure-sticky");
    next_message_arrives(&core, &screens, "a");
}

/// F3: the process dies mid-turn. The agent restarts with `--resume` and the
/// turn is pasted again, no app restart.
#[test]
fn f3_a_crashed_agent_is_resumed_and_gets_its_message() {
    let core = core_with_agent("orchd3a", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-exit-mid-turn", "");
    let turn = deliver(&core, "a", "second");
    until_done(&core, &turn, 40);
    seen_by_model(&core, &screens, "a", &turn);
    let ev = events(&core, "a", "recovered");
    assert!(ev.iter().any(|e| e["from_state"] == "crashed" && e["how"] == "resume"), "{ev:?}");
    next_message_arrives(&core, &screens, "a");
}
