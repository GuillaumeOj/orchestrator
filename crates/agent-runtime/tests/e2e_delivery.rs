//! fix-team-getting-stucked §3.8: one end-to-end test per failure mode F1–F9
//! (decisions.md §2) on the fake CLI that misbehaves (tests/fake_claude.sh,
//! its switches are sentinel files in the agent's worktree). Each test shows
//! the message reached the model (its marker is on the fake's screen and the
//! turn is done) or a `delivery` decision was raised within the spec's bound,
//! and that the agent takes the next message afterwards.
//!
//! The bounds are config, not sleeps: `delivery.ack_secs` is 1 s, and the
//! 15 min / 10 min / 2 min rules are driven with a later `now`
//! (`Runtime::watchdog`, `lift_limits`) or an older message.

use agent_runtime::handover::{ACK_KEY, INVARIANT_KEY, SLOT_WAIT_KEY};
use orch_protocol::{CtlRequest, now_ms};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

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

/// F1: a dialog eats every key until Esc. The watchdog sends Esc once and
/// puts the turn back; the paste then goes through.
#[test]
fn f1_a_modal_that_eats_keys_gets_an_esc_and_the_turn_is_pasted_again() {
    let core = core_with_agent("orchd1b", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-modal", "");
    let turn = deliver(&core, "a", "second");
    wait_for("an Esc and a new attempt", 30, || {
        core.rt.paste_watchdog(now_ms());
        events(&core, "a", "redelivered").iter().any(|e| e["how"] == "esc")
    });
    unset(&core, "a", ".fake-modal");
    until_done(&core, &turn, 20);
    seen_by_model(&core, &screens, "a", &turn);
    next_message_arrives(&core, &screens, "a");
}

/// F1 (the hook is lost): the CLI took the paste but its UserPromptSubmit
/// and Stop never reached the core. The marker is in the transcript: the
/// watchdog applies them late, no second paste.
#[test]
fn f1_a_lost_hook_is_applied_late_from_the_transcript() {
    let core = core_with_agent("orchd1c", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-drop-hooks", "");
    let turn = deliver(&core, "a", "second");
    until_done(&core, &turn, 20);
    seen_by_model(&core, &screens, "a", &turn);
    let ev = events(&core, "a", "recovered");
    assert!(ev.iter().any(|e| e["how"] == "late_hook"), "{ev:?}");
    assert!(events(&core, "a", "redelivered").is_empty(), "no second paste");
    next_message_arrives(&core, &screens, "a");
}

/// F1 (bound): a paste nobody ever acknowledges fails after 3 attempts with a
/// `delivery` decision naming the cause, and the agent takes its next turn.
#[test]
fn f1_three_unacknowledged_pastes_raise_a_delivery_decision() {
    let core = core_with_agent("orchd1d", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-modal", "");
    let turn = deliver(&core, "a", "second");
    wait_for("the turn to fail", 60, || {
        core.rt.paste_watchdog(now_ms());
        turn_status(&core, &turn) == "failed"
    });
    let delivery = items(&core, "delivery");
    assert_eq!(delivery.len(), 1, "{delivery:?}");
    assert_eq!(delivery[0]["payload"]["blocker"], "no_ack", "{}", delivery[0]);
    assert!(!events(&core, "a", "delivery_failed").is_empty());
    unset(&core, "a", ".fake-modal");
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

/// F2: a rate limit blocks the agent; once the limit is over (the plan reads
/// under 100 % again) it is idle and takes messages.
#[test]
fn f2_a_rate_limited_agent_is_idle_again_when_the_limit_is_over() {
    let core = core_with_agent("orchd2b", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-stop-failure", "rate_limit");
    deliver(&core, "a", "second");
    wait_for("blocked by the limit", 20, || core.state_of("a") == "blocked_by_limit");
    // Housekeeping lifts it a minute after the hit; ask for that minute.
    wait_for("idle again", 20, || {
        core.rt.lift_limits(now_ms() + 61_000);
        core.state_of("a") == "idle"
    });
    next_message_arrives(&core, &screens, "a");
}

/// F2: a Stop with a background task leaves the agent idle, not working:
/// a message is delivered (Claude Code queues input while shells run).
#[test]
fn f2_background_tasks_do_not_hold_a_message() {
    let core = core_with_agent("orchd2c", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-bg", "");
    let turn = deliver(&core, "a", "second");
    until_done(&core, &turn, 20);
    wait_for("idle with a background task", 10, || core.state_of("a") == "idle");
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

/// F3 (bound): at most 2 automatic restarts an hour; the third crash is a
/// `startup` decision (`failed: crash`), and the user's resume works.
#[test]
fn f3_an_agent_that_keeps_crashing_raises_a_decision_after_two_restarts() {
    let core = core_with_agent("orchd3b", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-crash-always", "");
    deliver(&core, "a", "second");
    wait_for("a crash decision", 90, || items(&core, "startup").iter().any(|i| i["payload"]["failed"] == "crash"));
    let spawns = core.rt.db.events(0, Some("a"), Some("spawned"), 20).unwrap().len();
    assert!((3..=4).contains(&spawns), "the start and two restarts, no loop: {spawns}");
    unset(&core, "a", ".fake-crash-always");
    request(&core, CtlRequest::Resume { agent_id: "a".into(), message: Some("back again".into()) });
    wait_for("the resumed agent on the screen", 30, || screens.read(&core.sock, "a").contains("back again"));
}

/// F4: the Stop never reached the core (late past the CLI's deadline), so the
/// agent looks `Working`. The watchdog reads the transcript, finds the turn
/// ended and applies the Stop (15 min in production).
#[test]
fn f4_a_lost_stop_is_applied_from_the_transcript() {
    let core = core_with_agent("orchd4", |_| {});
    let screens = Screens::default();
    set(&core, "a", ".fake-drop-stop", "");
    let turn = deliver(&core, "a", "second");
    seen_by_model(&core, &screens, "a", &turn);
    wait_for("working with no Stop", 10, || core.state_of("a") == "working");
    core.rt.watchdog(now_ms() + 16 * 60_000);
    wait_for("idle after the late Stop", 10, || core.state_of("a") == "idle");
    let ev = events(&core, "a", "recovered");
    assert!(ev.iter().any(|e| e["from_state"] == "working_silent" && e["how"] == "late_hook"), "{ev:?}");
    next_message_arrives(&core, &screens, "a");
}

/// F5: an autonomous turn held by the Governor (the 5 h guard) no longer
/// blocks the user's answer queued behind it.
#[test]
fn f5_a_held_turn_does_not_block_the_one_behind_it() {
    let core = core_with_agent("orchd5", |c| c.shadow = false);
    let screens = Screens::default();
    // The plan reads 90 %: the 5 h guard holds autonomous work.
    set(&core, "a", ".fake-usage", "90");
    let warm = deliver(&core, "a", "read the usage");
    until_done(&core, &warm, 20);
    core.rt
        .store
        .write(|s| {
            let t = agent_runtime::store::TurnRow::queued(
                "a",
                "background chore",
                governor::TurnKind::Task,
                agent_runtime::store::Admission::Pending,
            );
            s.turns.push(t);
        })
        .unwrap();
    let answer = deliver(&core, "a", "the user answers");
    until_done(&core, &answer, 20);
    seen_by_model(&core, &screens, "a", &answer);
    assert!(
        !screens.read(&core.sock, "a").contains("background chore"),
        "the held task did not run first: the answer went out ahead of it"
    );
    unset(&core, "a", ".fake-usage");
}

// ---- Teams: F6–F9 and the delivery invariant ---------------------------

struct Team {
    run: String,
    lead: String,
}

/// A bugfix-squad run with its lead started and idle.
fn launch_lead(core: &Core) -> Team {
    let p = request(core, CtlRequest::ProjectAdd { path: core.repo.display().to_string() });
    let project_id = p["id"].as_str().unwrap().to_string();
    request(core, CtlRequest::InstallPresets);
    let r = request(
        core,
        CtlRequest::Launch {
            project_id,
            team_id: "bugfix-squad".into(),
            slug: "fx".into(),
            title: Some("Fix X".into()),
            brief: Some("Fix the X bug.".into()),
            base: None,
        },
    );
    let lead = "fx-lead-1";
    wait_for("the lead's first turn", 30, || agent(&core.sock, lead)["turns"] == 1 && core.state_of(lead) == "idle");
    Team { run: r["id"].as_str().unwrap().to_string(), lead: lead.into() }
}

/// The user's message to a node of the run.
fn tell(core: &Core, team: &Team, to: &str, body: &str) {
    request(core, CtlRequest::Message { run_id: team.run.clone(), to: to.into(), body: body.into() });
}

/// Unread messages of the agent's node.
fn unread(core: &Core, team: &Team, node: &str, agent: &str) -> usize {
    core.rt.db.messages(&team.run, Some((node, Some(agent))), true).unwrap().len()
}

/// The run's timeline events of a kind.
fn run_events(core: &Core, team: &Team, kind: &str) -> Vec<Value> {
    core.rt.db.flush().unwrap();
    core.rt.db.run_events(&team.run, 1000).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.detail).collect()
}

/// F7: a message too long for the paste is written to a file and shown cut.
/// It is read only when the model read the file; one that did not is handed
/// back once, then marked read with `truncated: true`, never in a loop.
#[test]
fn f7_a_long_message_is_read_only_when_the_model_saw_all_of_it() {
    let core = Core::start("orchd7", |_| {});
    let team = launch_lead(&core);
    let screens = Screens::default();
    let long = format!("Q{}", "x".repeat(5_000));
    // The model opens the file: read at the end of that turn.
    set(&core, &team.lead, ".fake-read-full", "");
    tell(&core, &team, "lead", &long);
    wait_for("the cut message in the pane", 30, || {
        core.rt.team_tick();
        screens.read(&core.sock, &team.lead).contains("full text:")
    });
    wait_for("the message read", 30, || {
        core.rt.team_tick();
        unread(&core, &team, "lead", &team.lead) == 0
    });
    assert!(run_events(&core, &team, "read").iter().all(|e| e["truncated"] != true), "read in full, not truncated");
    // The model does not: handed back once, then read as truncated.
    unset(&core, &team.lead, ".fake-read-full");
    tell(&core, &team, "lead", &long);
    wait_for("read as truncated", 60, || {
        core.rt.team_tick();
        unread(&core, &team, "lead", &team.lead) == 0
            && run_events(&core, &team, "read").iter().any(|e| e["truncated"] == true)
    });
    wait_for("idle, nothing looping", 10, || core.state_of(&team.lead) == "idle");
    // The lead takes the next message.
    tell(&core, &team, "lead", "short follow-up");
    wait_for("the follow-up", 30, || {
        core.rt.team_tick();
        screens.read(&core.sock, &team.lead).contains("short follow-up")
    });
}

/// F8: a message that arrives during a turn is injected at the next tool call
/// and is seen only once the reply was written; the model gets it once. A
/// tool call of a subagent gets nothing: the message waits for the main
/// agent and arrives as its own turn.
#[test]
fn f8_an_injected_message_is_seen_once_and_never_goes_into_a_subagent() {
    let core = Core::start("orchd8", |_| {});
    let team = launch_lead(&core);
    let screens = Screens::default();
    let lead = team.lead.as_str();
    let busy = |note: &str| {
        set(&core, lead, ".fake-delay-post", "3");
        let turn = deliver(&core, lead, "work on it");
        wait_for("the lead working", 20, || core.state_of(lead) == "working");
        tell(&core, &team, "lead", note);
        turn
    };
    // (a) The main agent's tool call carries it.
    let turn = busy("mid-turn note");
    wait_for("the note in the tool reply", 30, || {
        screens.read(&core.sock, lead).contains("POST[") && {
            let s = screens.read(&core.sock, lead);
            s.lines().any(|l| l.contains("POST[") && l.contains("mid-turn note"))
        }
    });
    until_done(&core, &turn, 30);
    wait_for("the note read", 30, || {
        core.rt.team_tick();
        unread(&core, &team, "lead", lead) == 0
    });
    wait_for("idle", 10, || core.state_of(lead) == "idle");
    let s = screens.read(&core.sock, lead);
    assert_eq!(s.matches("mid-turn note").count(), 1, "shown once, not pasted again: {s}");
    // (b) A subagent's tool call gets nothing.
    set(&core, lead, ".fake-subagent", "");
    let turn = busy("subagent note");
    until_done(&core, &turn, 30);
    wait_for("the note as its own turn", 60, || {
        core.rt.team_tick();
        screens.read(&core.sock, lead).lines().any(|l| l.starts_with("DONE[") && l.contains("subagent note"))
    });
    assert!(
        !screens.read(&core.sock, lead).lines().any(|l| l.contains("POST[") && l.contains("subagent note")),
        "no injection into a subagent"
    );
    wait_for("the note read", 30, || {
        core.rt.team_tick();
        unread(&core, &team, "lead", lead) == 0
    });
}

/// F9: a panic in a step of the team tick is an event, and delivery goes on.
#[test]
fn f9_a_panic_in_the_team_tick_does_not_stop_delivery() {
    let core = Core::start("orchd9", |_| {});
    let team = launch_lead(&core);
    let screens = Screens::default();
    core.rt.inject_tick_fault(Some("dispatch"));
    tell(&core, &team, "lead", "still delivered");
    wait_for("the message in the pane", 30, || {
        core.rt.team_tick();
        screens.read(&core.sock, &team.lead).contains("still delivered")
    });
    let panics = run_events(&core, &team, "tick_panic");
    assert!(panics.iter().any(|e| e["step"] == "dispatch" && e["run"] == team.run.as_str()), "{panics:?}");
    // The next ticks keep running every step.
    core.rt.inject_tick_fault(None);
    tell(&core, &team, "lead", "and after the fault");
    wait_for("the next message", 30, || {
        core.rt.team_tick();
        screens.read(&core.sock, &team.lead).contains("and after the fault")
    });
}

/// Exit criterion 2: a message unread over the limit (10 min in production,
/// `delivery.invariant_secs`) while its target is not working raises one
/// `delivery` decision naming the cause. When the message is finally seen the
/// decision closes by itself.
#[test]
fn the_delivery_invariant_names_the_cause_of_a_message_left_unread() {
    let core = Core::start("orchinv", |_| {});
    // The watchdog stays away until the test lets it act: the paste is never acknowledged.
    core.rt.db.set_setting(ACK_KEY, &json!(3600)).unwrap();
    core.rt.db.set_setting(INVARIANT_KEY, &json!(2)).unwrap();
    let team = launch_lead(&core);
    let screens = Screens::default();
    set(&core, &team.lead, ".fake-modal", "");
    tell(&core, &team, "lead", "are you there?");
    wait_for("the paste in flight", 30, || {
        core.rt.team_tick();
        request(&core, CtlRequest::State)["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["agent_id"] == team.lead.as_str() && t["status"] == "delivering")
    });
    std::thread::sleep(Duration::from_secs(3));
    wait_for("a delivery decision", 30, || {
        core.rt.team_tick();
        !items(&core, "delivery").is_empty()
    });
    let item = items(&core, "delivery").remove(0);
    assert_eq!(item["payload"]["blocker"], "no_ack", "{item}");
    assert_eq!(item["agent_id"], team.lead.as_str());
    // One decision per episode, however many ticks.
    for _ in 0..3 {
        core.rt.team_tick();
    }
    assert_eq!(items(&core, "delivery").len(), 1);
    // The cause goes away: the watchdog acts, the message arrives, the decision closes.
    unset(&core, &team.lead, ".fake-modal");
    core.rt.db.set_setting(ACK_KEY, &json!(1)).unwrap();
    wait_for("the message seen", 40, || {
        core.rt.paste_watchdog(now_ms());
        core.rt.team_tick();
        screens.read(&core.sock, &team.lead).contains("are you there?") && unread(&core, &team, "lead", &team.lead) == 0
    });
    wait_for("the decision closed", 20, || {
        core.rt.team_tick();
        items(&core, "delivery").iter().all(|i| i["status"] != "pending")
    });
}

/// Runs one line as the agent's turn and waits until it is done and the agent
/// idle again.
fn call(core: &Core, id: &str, line: &str) {
    let turn = deliver(core, id, line);
    wait_for(line, 40, || turn_status(core, &turn) == "done" && core.state_of(id) == "idle");
}

/// F6: the live agents fill the cap while a member with unread messages has
/// no process. After the wait (2 min in production, `delivery.slot_wait_secs`)
/// the core suspends an idle member of the run that has nothing unread, and
/// the waiting member starts and gets its message.
#[test]
fn f6_a_member_waiting_for_a_slot_gets_one_from_an_idle_colleague() {
    let core = Core::start("orchd6", |c| {
        c.shadow = false;
        c.base_cap = 1;
    });
    core.rt.db.set_setting(SLOT_WAIT_KEY, &json!(1)).unwrap();
    let team = launch_lead(&core);
    let screens = Screens::default();
    let lead = team.lead.as_str();
    let lead_wt = worktree(&core, lead);
    std::fs::create_dir_all(lead_wt.join(".orchestrator/features/fx")).unwrap();
    std::fs::write(lead_wt.join(".orchestrator/features/fx/spec.md"), "# Fix X\n").unwrap();
    call(&core, lead, r#"ORCHTOOL commit {"message":"spec for fx"}"#);
    call(&core, lead, r#"ORCHTOOL request_spec_approval {"summary":"two tasks"}"#);
    let gate =
        items(&core, "gate").into_iter().find(|i| i["payload"]["action"] == "spec_approval").expect("the spec gate");
    request(
        &core,
        CtlRequest::Decide {
            item_id: gate["id"].as_u64().unwrap(),
            decision: orch_protocol::InboxDecision::Approve,
            scope: None,
            message: None,
        },
    );
    wait_for("the approval delivered", 30, || screens.read(&core.sock, lead).contains("approved: proceed"));
    // The first specialist takes the only slot; the second is held for it.
    call(&core, lead, r#"ORCHTOOL create_task {"title":"UI","node":"front","body":"ui"}"#);
    wait_for("front's first turn", 40, || {
        agent(&core.sock, "fx-front-1")["turns"] == 1 && core.state_of("fx-front-1") == "idle"
    });
    call(&core, lead, r#"ORCHTOOL create_task {"title":"API","node":"back","body":"api"}"#);
    tell(&core, &team, "back", "please look at this");
    wait_for("back's message in its pane", 60, || {
        core.rt.team_tick();
        core.rt.housekeeping();
        screens.read(&core.sock, "fx-back-1").contains("please look at this")
    });
    assert!(
        !events(&core, "fx-front-1", "slot_freed").is_empty() || !run_events(&core, &team, "slot_freed").is_empty()
    );
    assert_eq!(unread(&core, &team, "back", "fx-back-1"), 0, "read once it was in the model's turn");
}
