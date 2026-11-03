use super::*;
use crate::runtime::tests::{hook, insert_test_live, test_runtime};
use crate::store::{Admission, TeamSeat};
use governor::TurnKind;
use orch_store::DeliveryState;
use std::sync::Arc;

fn seat(rt: &Runtime) {
    rt.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            a.team = Some(TeamSeat {
                run_id: "r1".into(),
                node: "lead".into(),
                feature_id: "f1".into(),
                project_id: "p1".into(),
                task: None,
            });
            a.tracker.state = AgentState::Idle;
        })
        .unwrap();
}

fn message(rt: &Runtime, body: &str) -> u64 {
    rt.db
        .add_message(&Message {
            team_run_id: "r1".into(),
            from_node: "back".into(),
            from_agent: Some("back-1".into()),
            to_node: "lead".into(),
            to_agent: Some("a1".into()),
            kind: "report".into(),
            body: body.into(),
            r#ref: Value::Null,
            created_at_ms: 1,
            ..Default::default()
        })
        .unwrap()
}

fn msg(rt: &Runtime, id: u64) -> Message {
    rt.db.messages_by_id(&[id]).unwrap().remove(0)
}

fn rec(rt: &Runtime) -> AgentRecord {
    rt.store.read(|s| s.agents["a1"].clone())
}

fn turn(rt: &Runtime, id: &str) -> TurnRow {
    rt.store.read(|s| s.turns.iter().find(|t| t.id == id).cloned().unwrap())
}

fn events(rt: &Runtime, kind: &str) -> Vec<Value> {
    rt.db.flush().unwrap();
    if PLUMBING.contains(&kind) {
        // Kept per agent, off the run timeline.
        return rt.db.events(0, Some("a1"), Some(kind), 500).unwrap().into_iter().map(|e| e.detail).collect();
    }
    rt.db.run_events("r1", 500).unwrap_or_default().into_iter().filter(|e| e.kind == kind).map(|e| e.detail).collect()
}

// The watchdog end to end, on a live (sleeping) process: these need `openpty`.

/// A team agent with a live (sleeping) process and one pasted turn carrying
/// a message, pasted long enough ago for the watchdog.
fn pasted(d: &std::path::Path, rt: &Arc<Runtime>) -> (String, u64) {
    seat(rt);
    insert_test_live(rt, "a1", None, 0, d);
    let m = message(rt, "T-2 done");
    let mut t = TurnRow::queued("a1", "ORCH: 1 new message", TurnKind::Message, Admission::Admitted);
    t.messages = vec![m];
    t.status = TurnStatus::Delivering;
    t.delivered_at_ms = Some(1_000);
    let tid = t.id.clone();
    rt.store.write(|s| s.turns.push(t)).unwrap();
    rt.db.hand_messages(&[m], "a1", "turn", Some(&tid), 1_000).unwrap();
    (tid, m)
}

fn transcript(d: &std::path::Path, lines: &[String]) {
    let dir = d.join("projects").join("-w");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s.jsonl"), lines.join("\n")).unwrap();
}

/// What the PTY shows (the sleeping shell echoes what is typed).
fn type_into(rt: &Runtime, text: &str) {
    let l = rt.live("a1").unwrap();
    l.pty.write(text.as_bytes()).unwrap();
    let t = std::time::Instant::now();
    while !l.pty.output.tail_text().contains(text) && t.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

const NOW: u64 = 1_000_000;

#[test]
fn rules() {
    assert!(in_subagent(&HookPayload { agent_id: Some("sub-1".into()), ..Default::default() }));
    assert!(!in_subagent(&HookPayload::default()));
    assert!(!in_subagent(&HookPayload { agent_id: Some(String::new()), ..Default::default() }));
    let refs = referenced_files(&json!({"command": "cat \"/x/App Support/a1/messages/msg-4.md\" /x/messages/abc.txt"}));
    assert_eq!(refs, ["msg-4.md"]);
    assert_eq!(referenced_files(&json!({"file_path": "/a/messages/0123abcd.md"})), ["0123abcd.md"]);
    let short = Message { id: 4, body: "x".into(), via: Some("inject".into()), ..Default::default() };
    let long = Message { body: "y".repeat(crate::team::INLINE_INJECT + 1), ..short.clone() };
    let none = BTreeSet::new();
    assert!(fully_seen(&short, None, &none, false), "shown in full");
    assert!(!fully_seen(&long, None, &none, false), "its file not read");
    assert!(fully_seen(&long, None, &BTreeSet::from(["msg-4.md".to_string()]), false));
    assert!(fully_seen(&long, None, &none, true), "inbox shows it all");
    assert!(!fully_seen(&short, Some("0123abcd.md"), &none, false), "the paste's file not read");
    assert!(fully_seen(&short, Some("0123abcd.md"), &BTreeSet::from(["0123abcd.md".to_string()]), false));
    // The same body fits inline in a turn.
    assert!(fully_seen(&Message { via: Some("turn".into()), ..long }, None, &none, false));
    assert_eq!(last_lines("a\n\nb\nc\n", 2), "b\nc");
}

#[test]
fn a_long_message_goes_to_a_file_and_nothing_is_cut_or_dropped() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    let long = "z".repeat(crate::team::INLINE_TURN + 10);
    let m1 = message(&rt, "short one");
    let m2 = message(&rt, &long);
    let msgs = rt.db.messages_by_id(&[m1, m2]).unwrap();
    let r = rt.render_for("a1", &msgs, crate::team::MAX_BODY, crate::team::INLINE_TURN);
    assert_eq!(r.included, [m1, m2]);
    let path = rt.cfg.paths.agent_dir("a1").join("messages").join(format!("msg-{m2}.md"));
    assert_eq!(r.files, [(m2, path.clone())]);
    assert!(std::fs::read_to_string(&path).unwrap().contains(&long), "the full text");
    assert!(r.text.contains(&format!("full text: {}", path.display())));
    assert!(
        r.text.contains(&"z".repeat(crate::team::PREVIEW)) && !r.text.contains(&"z".repeat(crate::team::PREVIEW + 1))
    );
    // Past the budget: left for the next hand-over, never "(more…)".
    let many: Vec<u64> = (0..6).map(|_| message(&rt, &"w".repeat(2_000))).collect();
    let msgs = rt.db.messages_by_id(&many).unwrap();
    let r = rt.render_for("a1", &msgs, crate::team::MAX_BODY, crate::team::INLINE_TURN);
    assert_eq!(r.included.len(), 3);
    assert!(!r.text.contains("more"));
}

#[test]
fn a_long_message_is_read_only_once_its_file_was_read_else_handed_back_once() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    let long = "q".repeat(crate::team::INLINE_INJECT + 10);
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "go"})), None);
    let m = message(&rt, &long);
    let (ctx, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Edit"})), None);
    let ctx = ctx.unwrap().0.to_string();
    assert!(ctx.contains(&format!("msg-{m}.md")) && !ctx.contains(&long), "the path, not the whole text");
    rt.on_hook_ack("a1", &ack);
    rt.on_hook(&hook("Stop", json!({})), None);
    let back = msg(&rt, m);
    assert_eq!((back.delivery_state(), back.last_error.as_deref()), (DeliveryState::Queued, Some(NOT_FULLY_SEEN)));
    assert_eq!(back.attempts, 0, "not a redelivery for no ack");
    // The second time it is read with `truncated`, so it never loops.
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "go", "prompt_id": "p2"})), None);
    let (_, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Edit", "prompt_id": "p2"})), None);
    rt.on_hook_ack("a1", &ack);
    rt.on_hook(&hook("Stop", json!({"prompt_id": "p2"})), None);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Read);
    assert!(events(&rt, "read").iter().any(|e| e["truncated"] == true));

    // Read when the agent read its file during the turn.
    let m = message(&rt, &long);
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "go", "prompt_id": "p3"})), None);
    let (_, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Edit", "prompt_id": "p3"})), None);
    rt.on_hook_ack("a1", &ack);
    let path = rt.cfg.paths.agent_dir("a1").join("messages").join(format!("msg-{m}.md"));
    rt.on_hook(
        &hook("PostToolUse", json!({"tool_name": "Read", "prompt_id": "p3", "tool_input": {"file_path": path}})),
        None,
    );
    rt.on_hook(&hook("Stop", json!({"prompt_id": "p3"})), None);
    let r = msg(&rt, m);
    assert_eq!((r.delivery_state(), r.last_error), (DeliveryState::Read, None));
}

#[test]
fn inbox_never_says_more_and_reads_the_whole_message() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().role.orch_tools = vec!["inbox".into()]).unwrap();
    let long = "k".repeat(20_000);
    let ids: Vec<u64> = (0..3).map(|_| message(&rt, &long)).collect();
    let rec = rec(&rt);
    // Without a run the tool can't run here: render as inbox does.
    let r = rt.render_for("a1", &rt.db.messages_by_id(&ids).unwrap(), usize::MAX, 6_000);
    assert_eq!(r.included, ids, "every message, each in full or with its file");
    assert!(!r.text.contains("(more"));
    assert_eq!(r.files.len(), 3);
    let _ = rec;
}

#[test]
fn no_injection_into_a_subagent_and_a_lost_reply_gives_the_messages_back() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "go"})), None);
    let m = message(&rt, "T-2 done");
    let (out, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Bash", "agent_id": "sub-1"})), None);
    assert!(out.is_none() && ack.is_empty(), "a subagent's tool call gets nothing");
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Queued);
    // The reply could not be written: queued again at once.
    rt.serve_hook(&hook("PostToolUse", json!({"tool_name": "Bash"})), None, |r| {
        assert!(matches!(&r, Reply::Hook { ack, .. } if ack == &[m]));
        false
    });
    let q = msg(&rt, m);
    assert_eq!((q.delivery_state(), q.last_error.as_deref()), (DeliveryState::Queued, Some("write_failed")));
    // Written, confirmed by the next hook's `acks`.
    rt.serve_hook(&hook("PostToolUse", json!({"tool_name": "Bash"})), None, |_| true);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Handed);
    let mut next = hook("PreToolUse", json!({"tool_name": "Bash"}));
    next.acks = vec![m];
    rt.on_hook(&next, None);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Seen);
    // Another agent's ack changes nothing.
    rt.on_hook_ack("a2", &[m]);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Seen);
}

#[test]
fn a_restart_requeues_injected_messages_never_seen() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    let (seen, unseen) = (message(&rt, "a"), message(&rt, "b"));
    rt.db.hand_messages(&[seen, unseen], "a1", "inject", Some("p1"), 5).unwrap();
    rt.db.see_messages(&[seen], 6).unwrap();
    rt.requeue_injected_unseen(None, "restart");
    assert_eq!(msg(&rt, seen).delivery_state(), DeliveryState::Seen);
    let q = msg(&rt, unseen);
    assert_eq!((q.delivery_state(), q.last_error.as_deref(), q.attempts), (DeliveryState::Queued, Some("restart"), 1));
}

#[test]
fn watchdog_applies_a_lost_user_prompt_submit_from_the_transcript() {
    let (d, rt) = test_runtime(true);
    let (tid, m) = pasted(d.path(), &rt);
    rt.paste_watchdog(15_000);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivering, "not due yet (20 s)");
    transcript(d.path(), &[format!(r#"{{"type":"user","message":{{"content":"[t:{tid}] ORCH: 1 new message"}}}}"#)]);
    rt.paste_watchdog(NOW);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivered);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Seen);
    assert_eq!(rec(&rt).tracker.state, AgentState::Working, "the late TurnStarted");
    assert!(events(&rt, "recovered").iter().any(|e| e["how"] == "late_hook"));
    // The turn is over in the transcript too: the lost Stop is applied as well.
    let (d, rt) = test_runtime(true);
    let (tid, m) = pasted(d.path(), &rt);
    transcript(
        d.path(),
        &[
            format!(r#"{{"type":"user","message":{{"content":"[t:{tid}] hi"}}}}"#),
            r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#
                .into(),
        ],
    );
    rt.paste_watchdog(NOW);
    assert_eq!((turn(&rt, &tid).status, rec(&rt).tracker.state), (TurnStatus::Done, AgentState::Idle));
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Read);
}

#[test]
fn watchdog_presses_enter_again_when_the_paste_is_still_in_the_input_box() {
    let (d, rt) = test_runtime(true);
    let (tid, _) = pasted(d.path(), &rt);
    type_into(&rt, &format!("[t:{tid}] ORCH: 1 new message"));
    rt.paste_watchdog(NOW);
    let t = turn(&rt, &tid);
    assert_eq!(
        (t.status, t.enter_retried, t.delivered_at_ms, t.attempts),
        (TurnStatus::Delivering, true, Some(NOW), 0)
    );
    assert!(events(&rt, "redelivered").iter().any(|e| e["how"] == "enter" && e["attempt"] == 1));
}

#[test]
fn watchdog_sends_esc_once_and_requeues_only_when_idle_quiet_and_silent() {
    let (d, rt) = test_runtime(true);
    let (tid, m) = pasted(d.path(), &rt);
    let l = rt.live("a1").unwrap();
    // The user is typing: nothing.
    l.last_user_input_ms.store(NOW - 500, std::sync::atomic::Ordering::SeqCst);
    rt.paste_watchdog(NOW);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivering);
    l.last_user_input_ms.store(0, std::sync::atomic::Ordering::SeqCst);
    // Not idle (a turn of the user's runs): nothing.
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Working).unwrap();
    rt.paste_watchdog(NOW);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivering);
    // A hook came after the paste: something else is going on, nothing.
    rt.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            a.tracker.state = AgentState::Idle;
            a.last_event_at_ms = Some(2_000);
        })
        .unwrap();
    rt.paste_watchdog(NOW);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivering);
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().last_event_at_ms = None).unwrap();
    // Idle, quiet, silent, nothing on screen: Esc, back to the queue.
    rt.paste_watchdog(NOW);
    let t = turn(&rt, &tid);
    assert_eq!((t.status, t.attempts, t.admission), (TurnStatus::Queued, 1, Admission::Pending));
    let q = msg(&rt, m);
    assert_eq!((q.delivery_state(), q.attempts, q.last_error.as_deref()), (DeliveryState::Queued, 1, Some("no_ack")));
    assert!(l.ready_at_ms.load(std::sync::atomic::Ordering::SeqCst) > NOW, "ready_at restored");
    assert!(events(&rt, "redelivered").iter().any(|e| e["how"] == "esc" && e["attempt"] == 1));
}

#[test]
fn after_three_attempts_the_turn_fails_and_the_agent_takes_its_other_turns() {
    let (d, rt) = test_runtime(true);
    let (tid, m) = pasted(d.path(), &rt);
    rt.store.write(|s| s.turns.iter_mut().find(|t| t.id == tid).unwrap().attempts = 2).unwrap();
    let other = rt.deliver_kind("a1", "next", TurnKind::User).unwrap();
    rt.paste_watchdog(NOW);
    let t = turn(&rt, &tid);
    assert_eq!((t.status, t.attempts), (TurnStatus::Failed, 3));
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Failed);
    let ev = events(&rt, "delivery_failed");
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0]["blocker"].clone(), ev[0]["attempts"].clone()), (json!("no_ack"), json!(3)));
    assert_eq!(ev[0]["messages"], json!([m]));
    // Unblocked: input allowed again, the next queued turn is this agent's first.
    let l = rt.live("a1").unwrap();
    assert!(l.ready_at_ms.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(turn(&rt, &other).status, TurnStatus::Queued);
    assert!(rt.store.read(|s| !s.turns.iter().any(|t| t.status == TurnStatus::Delivering)));
}

#[test]
fn a_turn_the_cli_did_take_is_not_pasted_again() {
    let (d, rt) = test_runtime(true);
    let tid = stored_paste(&rt, 0).0.id;
    rt.store
        .write(|s| {
            let t = s.turns.iter_mut().find(|t| t.id == tid).unwrap();
            t.requeue();
            t.attempts = 1;
            t.admission = Admission::Admitted;
        })
        .unwrap();
    transcript(d.path(), &[format!(r#"{{"type":"user","message":{{"content":"[t:{tid}] hi"}}}}"#)]);
    let rec = rec(&rt);
    let t = turn(&rt, &tid);
    assert!(rt.already_in_transcript(&rec, &[t]));
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Delivered);
}

#[test]
fn the_spool_is_replayed_live_in_order_and_stale_events_are_dropped() {
    let (_d, rt) = test_runtime(true);
    seat(&rt);
    let dir = rt.cfg.paths.agent_dir("a1");
    std::fs::create_dir_all(&dir).unwrap();
    let m = message(&rt, "T-2 done");
    let tid = rt.deliver_messages_turn("a1", "ORCH: 1 new message", vec![m]).unwrap();
    rt.store.write(|s| s.turns.iter_mut().find(|t| t.id == tid).unwrap().status = TurnStatus::Delivering).unwrap();
    rt.db.hand_messages(&[m], "a1", "turn", Some(&tid), 5).unwrap();
    let spooled = |ts: u64, event: HookEvent, payload: Value| orch_protocol::SpoolRecord {
        v: 1,
        ts_ms: ts,
        agent_id: "a1".into(),
        event,
        why: "deadline".into(),
        payload,
    };
    let t0 = now_ms();
    for r in [
        spooled(
            t0,
            HookEvent::UserPromptSubmit,
            json!({"session_id": "s", "prompt_id": "p9", "prompt": format!("[t:{tid}] x")}),
        ),
        spooled(t0 + 1, HookEvent::Stop, json!({"session_id": "s"})),
    ] {
        spool::append(&dir, &r).unwrap();
    }
    // The next hook replays them first, in order: the turn started and ended.
    rt.on_hook(&hook("Notification", json!({"notification_type": "idle_prompt"})), None);
    assert_eq!(turn(&rt, &tid).status, TurnStatus::Done);
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Read);
    assert_eq!(rec(&rt).tracker.state, AgentState::Idle);
    assert!(!spool::spool_path(&dir).exists());

    // Older than what the core applied since: dropped, but a stale prompt
    // still marks its turn delivered.
    let tid2 = rt.deliver_kind("a1", "again", TurnKind::Message).unwrap();
    rt.store.write(|s| s.turns.iter_mut().find(|t| t.id == tid2).unwrap().status = TurnStatus::Delivering).unwrap();
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "live", "prompt_id": "p10"})), None);
    spool::append(&dir, &spooled(1, HookEvent::Stop, json!({"session_id": "s"}))).unwrap();
    spool::append(
        &dir,
        &spooled(2, HookEvent::UserPromptSubmit, json!({"session_id": "s", "prompt": format!("[t:{tid2}] y")})),
    )
    .unwrap();
    rt.handover_tick(now_ms());
    assert_eq!(turn(&rt, &tid2).status, TurnStatus::Delivered);
    assert_eq!(rec(&rt).tracker.state, AgentState::Working, "the stale Stop was dropped");
    assert!(!spool::spool_path(&dir).exists(), "the 5 s sweep took it");
}

// ---------------------------------------------------------------- without a PTY
// (the tests above that start a process need `openpty`)

fn delivering(id: &str, retried: bool) -> TurnRow {
    let mut t = TurnRow::queued("a1", "ORCH: hi", TurnKind::Message, Admission::Admitted);
    t.id = id.into();
    t.status = TurnStatus::Delivering;
    t.enter_retried = retried;
    t
}

#[test]
fn the_watchdog_rule_checks_the_transcript_then_the_input_box_then_escapes_only_when_safe() {
    let t = delivering("0000000a", false);
    let facts = |markers: &[&str], tail: &'static str, idle: bool, quiet: bool, silent: bool| PasteFacts {
        markers: markers.iter().map(|m| m.to_string()).collect(),
        turn_ended: false,
        tail,
        idle,
        quiet,
        silent,
    };
    // 1. The marker in the transcript wins over everything, Esc included.
    assert_eq!(
        next_step(std::slice::from_ref(&t), &facts(&["0000000a"], "[t:0000000a] ORCH", true, true, true)),
        Step::LateHook { seen: vec!["0000000a".into()], ended: false }
    );
    // 2. Still in the input box (as typed, split by redraws, or collapsed): Enter, once.
    for tail in ["> [t:0000000a] ORCH: hi", "> [t:0000 000a]", "> [Pasted text #1 +3 lines]"] {
        assert_eq!(next_step(std::slice::from_ref(&t), &facts(&[], tail, true, true, true)), Step::Enter, "{tail}");
    }
    let retried = delivering("0000000a", true);
    assert_eq!(next_step(std::slice::from_ref(&retried), &facts(&[], "> [t:0000000a]", true, true, true)), Step::Esc);
    // 3. Esc only when idle, the user quiet and no hook since the paste.
    assert_eq!(next_step(std::slice::from_ref(&t), &facts(&[], "Select an option", true, true, true)), Step::Esc);
    assert_eq!(next_step(std::slice::from_ref(&t), &facts(&[], "", false, true, true)), Step::Wait, "not idle");
    assert_eq!(next_step(std::slice::from_ref(&t), &facts(&[], "", true, false, true)), Step::Wait, "user typing");
    assert_eq!(next_step(std::slice::from_ref(&t), &facts(&[], "", true, true, false)), Step::Wait, "a hook came");
    // A slash command has no marker: found by its text.
    let mut c = delivering("0000000b", false);
    c.command = true;
    c.body = "/code-review high".into();
    assert_eq!(
        next_step(std::slice::from_ref(&c), &facts(&["0000000b"], "> /code-review high", true, true, true)),
        Step::Enter
    );
}

fn stored_paste(rt: &Runtime, attempts: u32) -> (TurnRow, u64) {
    seat(rt);
    let m = message(rt, "T-2 done");
    let mut t = TurnRow::queued("a1", "ORCH: 1 new message", TurnKind::Message, Admission::Admitted);
    t.messages = vec![m];
    t.status = TurnStatus::Delivering;
    t.delivered_at_ms = Some(1_000);
    t.attempts = attempts;
    rt.store.write(|s| s.turns.push(t.clone())).unwrap();
    rt.db.hand_messages(&[m], "a1", "turn", Some(&t.id), 1_000).unwrap();
    (t, m)
}

#[test]
fn a_paste_taken_back_is_queued_again_then_fails_at_its_third_attempt() {
    let (_d, rt) = test_runtime(true);
    let (t, m) = stored_paste(&rt, 0);
    rt.take_back_paste(&rec(&rt), None, std::slice::from_ref(&t), 1, "", NOW);
    let back = turn(&rt, &t.id);
    assert_eq!((back.status, back.attempts, back.admission), (TurnStatus::Queued, 1, Admission::Pending));
    let q = msg(&rt, m);
    assert_eq!((q.delivery_state(), q.attempts, q.last_error.as_deref()), (DeliveryState::Queued, 1, Some("no_ack")));
    assert!(events(&rt, "redelivered").iter().any(|e| e["how"] == "esc" && e["attempt"] == 1));

    let (t, m) = stored_paste(&rt, 2);
    let other = rt.deliver_kind("a1", "next", TurnKind::User).unwrap();
    rt.take_back_paste(&rec(&rt), None, std::slice::from_ref(&t), 3, "line 1\n\nmodal\n", NOW);
    assert_eq!((turn(&rt, &t.id).status, turn(&rt, &t.id).attempts), (TurnStatus::Failed, 3));
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Failed);
    let ev = events(&rt, "delivery_failed");
    assert_eq!(ev.len(), 1);
    assert_eq!(
        (ev[0]["blocker"].clone(), ev[0]["attempts"].clone(), ev[0]["messages"].clone()),
        (json!("no_ack"), json!(3), json!([m]))
    );
    assert_eq!(ev[0]["tail"], "line 1\nmodal");
    // Nothing is pasted any more: the agent's next turn goes out.
    assert!(rt.store.read(|s| !s.turns.iter().any(|t| t.status == TurnStatus::Delivering)));
    assert_eq!(turn(&rt, &other).status, TurnStatus::Queued);
}

#[test]
fn a_late_hook_from_the_transcript_starts_and_ends_the_turn() {
    let (_d, rt) = test_runtime(true);
    let (t, m) = stored_paste(&rt, 0);
    rt.late_turn_start(&rec(&rt), std::slice::from_ref(&t.id), false);
    assert_eq!((turn(&rt, &t.id).status, rec(&rt).tracker.state), (TurnStatus::Delivered, AgentState::Working));
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Seen);
    let (t, m) = stored_paste(&rt, 0);
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Idle).unwrap();
    rt.late_turn_start(&rec(&rt), std::slice::from_ref(&t.id), true);
    assert_eq!((turn(&rt, &t.id).status, rec(&rt).tracker.state), (TurnStatus::Done, AgentState::Idle));
    assert_eq!(msg(&rt, m).delivery_state(), DeliveryState::Read);
    assert!(events(&rt, "recovered").iter().any(|e| e["how"] == "late_hook"));
}

#[test]
fn a_turn_end_restarts_the_clock_of_a_paste_that_waits_without_a_process() {
    let (_d, rt) = test_runtime(true);
    let (t, _) = stored_paste(&rt, 0);
    rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "the user's own turn"})), None);
    rt.on_hook(&hook("Stop", json!({})), None);
    let after = turn(&rt, &t.id);
    assert_eq!(after.status, TurnStatus::Delivering);
    assert!(after.delivered_at_ms.unwrap() > 1_000);
}
