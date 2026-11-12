//! The Overwatch's tools (spec §3.4–3.6): decide_permission, answer_question
//! and its grounding, annotate, reprioritize, the unsticking tools, post,
//! chat_send / chat_discard, world / inspect, the stuck events.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use governor::{AgentId, Tier, TurnRequest};
use orch_protocol::{HookPayload, InboxDecision, now_ms};
use serde_json::{Value, json};

use super::tests::{allowed, at, blocking, held, manual, payload, t0};
use super::*;
use crate::inbox::PRIORITY_BLOCKING;
use crate::runtime::Runtime;

/// Like `manual()` but enforcing, so the Governor queues instead of admitting.
fn enforcing() -> (tempfile::TempDir, Arc<Runtime>) {
    let (d, rt) = crate::runtime::tests::test_runtime(false);
    rt.overwatch.manual.store(true, Ordering::SeqCst);
    at(&rt, t0());
    (d, rt)
}

pub(super) fn call(rt: &Runtime, tool: &str, args: Value) -> (bool, Value) {
    let rec = rt.store.read(|s| s.agents["a1"].clone());
    let (err, _text, data) = rt.overwatch_tool(&rec, tool, &args);
    (err, data)
}

/// An open permission the Overwatch may decide (effect overwatch, a minute to go).
pub(super) fn deciding_item(rt: &Runtime, cmd: &str) -> u64 {
    rt.add_item(
        Some("a1"),
        "permission",
        format!("a1: Bash: {cmd}"),
        json!({"tool": "Bash", "tool_input": {"command": cmd}, "cwd": "/w", "effect": "overwatch",
               "deciding_until_ms": now_ms() + 60_000, "always": format!("Bash({cmd})"), "project_id": "p1",
               "role": "solo", "tool_use_id": "tu-1"}),
        PRIORITY_BLOCKING,
    )
    .unwrap()
}

fn item_of(rt: &Runtime, n: u64) -> orch_store::InboxItem {
    rt.store.read(|s| s.inbox.iter().find(|i| i.id == n).cloned().unwrap())
}

fn settle(rt: &Runtime) -> Vec<orch_store::ChatEntry> {
    rt.now_pump(true);
    rt.db.chat_day(&rt.overwatch_today().1).unwrap()
}

#[test]
fn decide_permission_settles_within_the_window_and_audits_as_the_overwatch() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let n = deciding_item(&rt, "pnpm add zod");
    let (err, v) = call(
        &rt,
        "decide_permission",
        json!({"decision": format!("inbox-{n}"), "effect": "allow",
                                                        "reason": "routine install inside the task"}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["line"]["by"], "overwatch");
    let i = item_of(&rt, n);
    assert_eq!((i.decided_by.as_deref(), i.decision.as_deref()), (Some("overwatch"), Some("approve")));
    // FR-709: the audit names the Overwatch.
    let log = rt.db.audit_log(Some("a1"), 5).unwrap();
    assert!(log.iter().any(|e| e.actor == "overwatch" && e.decision == "approve"), "{log:?}");
    // The chat line: by overwatch, its reason, and the undo it offers.
    let entry = settle(&rt).into_iter().find(|e| e.decision.as_deref() == Some(&format!("inbox-{n}"))).unwrap();
    let line = entry.line.unwrap();
    assert_eq!((line["verb"].as_str(), line["by"].as_str()), (Some("allowed"), Some("overwatch")));
    assert_eq!(line["by_detail"], "routine install inside the task");
    assert_eq!(line["undo"]["label"], "Tell the agent");
    // Decided once: a second call is stale.
    let (err, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "deny", "reason": "x"}));
    assert!(err);
    assert_eq!(v["code"], "stale");
}

#[test]
fn the_user_deciding_while_it_decides_settles_it_and_the_overwatch_is_stale() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (n, h) = held(&rt, "pnpm add zod");
    assert!(payload(&rt, n)["deciding_until_ms"].is_u64(), "still deciding");
    // The user's card acts at once, inside the Overwatch's window.
    rt.decide(n, InboxDecision::Approve, None, None).unwrap();
    assert!(allowed(h.join().unwrap()));
    assert_eq!(item_of(&rt, n).decided_by.as_deref(), Some("user"));
    let (err, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "deny", "reason": "x"}));
    assert!(err);
    assert_eq!(v["code"], "stale", "{v}");
}

#[test]
fn decide_permission_refuses_what_the_core_keeps_and_hands_it_over() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let n = deciding_item(&rt, "git reset --hard origin/main");
    assert!(payload(&rt, n)["deciding_until_ms"].is_u64());
    let (err, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "allow", "reason": "x"}));
    assert!(err);
    assert_eq!(v["code"], "not_allowed", "{v}");
    // Deciding ended at once (F1): it is the user's, and the tool says so.
    assert!(payload(&rt, n).get("deciding_until_ms").is_none());
    let (_, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "too_late");
    assert_eq!(item_of(&rt, n).status, orch_store::InboxStatus::Pending);
}

#[test]
fn decide_permission_needs_effect_overwatch_and_a_decision_that_exists() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let user = rt
        .add_item(
            Some("a1"),
            "permission",
            "a1: Bash: make".into(),
            json!({"tool": "Bash", "tool_input": {"command": "make"}, "effect": "user"}),
            30,
        )
        .unwrap();
    let (_, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{user}"), "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "not_allowed", "{v}");
    let (_, v) = call(&rt, "decide_permission", json!({"decision": "inbox-9999", "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "stale");
    let (_, v) = call(&rt, "decide_permission", json!({"decision": "q-1", "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "bad_request");
    let n = deciding_item(&rt, "pnpm add zod");
    let (_, v) =
        call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "maybe", "reason": "x"}));
    assert_eq!(v["code"], "bad_request");
}

#[test]
fn a_held_hook_gets_the_overwatchs_words_when_it_denies() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (id, h) = held(&rt, "pnpm add left-pad");
    let (err, v) = call(
        &rt,
        "decide_permission",
        json!({"decision": format!("inbox-{id}"), "effect": "deny",
                                                        "reason": "not in this task's scope"}),
    );
    assert!(!err, "{v}");
    let out = h.join().unwrap().expect("a hook answer").0.to_string();
    assert!(out.contains("deny") && out.contains("the Overwatch denied this: not in this task's scope"), "{out}");
    // The line: denied by the Overwatch, with "Allow instead".
    let entry = settle(&rt).into_iter().find(|e| e.decision.as_deref() == Some(&format!("inbox-{id}"))).unwrap();
    let line = entry.line.unwrap();
    assert_eq!((line["verb"].as_str(), line["undo"]["label"].as_str()), (Some("denied"), Some("Allow instead")));
    let _ = allowed(None);
}

// ------------------------------------------------------------ questions

fn question(rt: &Runtime, questions: Option<Value>) -> u64 {
    rt.add_item(
        Some("a1"),
        "question",
        "a1: Which pagination?".into(),
        json!({"action": "ask_user", "question": "Which pagination?", "questions": questions}),
        20,
    )
    .unwrap()
}

/// `a1` is solo in `repo`: its docs are the project's docs/.
fn docs(rt: &Runtime) -> std::path::PathBuf {
    let repo = rt.store.read(|s| s.agents["a1"].repo.clone());
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/api.md"), "# API\n\n## 3. Pagination\nCursor based.\n").unwrap();
    repo
}

fn texts(rt: &Runtime) -> Vec<String> {
    rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == "a1").map(|t| t.body.clone()).collect())
}

#[test]
fn answer_question_needs_a_source_the_core_can_check() {
    let (_d, rt) = manual();
    let repo = docs(&rt);
    let q = question(&rt, None);
    let id = format!("q-{q}");
    let args = |sources: Value| json!({"decision": id, "answers": [{"text": "Cursor based."}], "sources": sources});
    for bad in [
        json!([]),
        json!(["docs/missing.md"]),
        json!(["../outside.md"]),
        json!(["docs/api.md", "nope/x.md"]),
        json!(["docs/api.md#9"]),
        json!(["docs/api.md#Nothing here"]),
    ] {
        let (err, v) = call(&rt, "answer_question", args(bad.clone()));
        assert!(err, "{bad}");
        assert_eq!(v["code"], "ungrounded", "{bad}: {v}");
    }
    // A file outside docs/ (the repo's README) is not a source either.
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    let (_, v) = call(&rt, "answer_question", args(json!(["README.md"])));
    assert_eq!(v["code"], "ungrounded");
    assert!(texts(&rt).is_empty(), "nothing was sent");
    assert!(item_of(&rt, q).open());
    // A real one: the agent hears who answered and from what.
    let (err, v) = call(&rt, "answer_question", args(json!(["docs/api.md#3"])));
    assert!(!err, "{v}");
    let i = item_of(&rt, q);
    assert_eq!(i.decided_by.as_deref(), Some("overwatch"));
    let t = texts(&rt);
    assert_eq!(t.len(), 1);
    assert!(
        t[0].starts_with("ORCH: the Overwatch answered your question (source: docs/api.md#3).\nQuestion: Which pagination?\nAnswer: Cursor based."),
        "{}",
        t[0]
    );
    // The line shows the source, and offers the correction.
    let entry = settle(&rt).into_iter().find(|e| e.decision.as_deref() == Some(&id)).unwrap();
    let line = entry.line.unwrap();
    assert_eq!(line["sources"], json!(["docs/api.md#3"]));
    assert_eq!((line["by"].as_str(), line["undo"]["label"].as_str()), (Some("overwatch"), Some("Correct the answer")));
    // A closed question is stale.
    let (_, v) = call(&rt, "answer_question", args(json!(["docs/api.md"])));
    assert_eq!(v["code"], "stale");
}

#[test]
fn a_multi_question_decision_needs_an_answer_per_question() {
    let (_d, rt) = manual();
    docs(&rt);
    let q = question(
        &rt,
        Some(json!([{"question": "Pagination?", "options": ["cursor", "offset"]}, {"question": "Format?"}])),
    );
    let id = format!("q-{q}");
    let src = json!(["docs/api.md"]);
    let (_, v) = call(&rt, "answer_question", json!({"decision": id, "answers": [{"text": "cursor"}], "sources": src}));
    assert_eq!(v["code"], "bad_request", "{v}");
    let (_, v) = call(
        &rt,
        "answer_question",
        json!({"decision": id, "sources": src,
        "answers": [{"question_index": 0, "text": "cursor"}, {"question_index": 0, "text": "again"}]}),
    );
    assert_eq!(v["code"], "bad_request", "twice the same question: {v}");
    let (err, v) = call(
        &rt,
        "answer_question",
        json!({"decision": id, "sources": src,
        "answers": [{"question_index": 1, "text": "json"}, {"question_index": 0, "text": "cursor"}]}),
    );
    assert!(!err, "{v}");
    let t = texts(&rt).pop().unwrap();
    assert!(t.contains("Answer: 1. cursor\n2. json"), "{t}");
}

#[test]
fn a_chat_source_is_a_question_or_gate_the_user_answered() {
    let (_d, rt) = manual();
    docs(&rt);
    // The user answered an earlier question; its entry is the source.
    let old = question(&rt, None);
    rt.decide(old, InboxDecision::Reply, None, Some("cursor, always")).unwrap();
    let entries = settle(&rt);
    let answered = entries.iter().find(|e| e.decision.as_deref() == Some(&format!("q-{old}"))).unwrap().id();
    let other =
        rt.db.chat_day(&rt.overwatch_today().1).unwrap().into_iter().find(|e| e.kind == "morning").map(|e| e.id());
    let q = question(&rt, None);
    let ask = |src: String| {
        call(
            &rt,
            "answer_question",
            json!({"decision": format!("q-{q}"), "answers": [{"text": "cursor"}], "sources": [src]}),
        )
    };
    for bad in
        [format!("chat:{}", other.unwrap_or("c-999".into())), "chat:c-999".to_string(), "chat:nonsense".to_string()]
    {
        let (err, v) = ask(bad.clone());
        assert!(err && v["code"] == "ungrounded", "{bad}: {v}");
    }
    let (err, v) = ask(format!("chat:{answered}"));
    assert!(!err, "{v}");
    assert_eq!(v["line"]["sources"], json!([format!("chat:{answered}")]));
}

// ------------------------------------------------------------ annotate

#[test]
fn annotate_lives_on_the_card_one_per_decision_and_hands_a_permission_over() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let n = deciding_item(&rt, "pnpm add zod");
    let id = format!("inbox-{n}");
    rt.now_pump(true);
    let (err, v) = call(
        &rt,
        "annotate",
        json!({"decision": id, "note": "Not in the allow list; allow it for this project.",
                                                "recommend": "always", "sources": ["chat:c-7"]}),
    );
    assert!(!err, "{v}");
    // On the card, with its recommendation; the permission is the user's now.
    let d = card(&rt, n);
    let a = d.annotation.clone().expect("annotated");
    assert_eq!(
        (a["note"].as_str(), a["recommend"].as_str()),
        (Some("Not in the allow list; allow it for this project."), Some("always"))
    );
    assert_eq!(a["sources"], json!(["chat:c-7"]));
    assert_eq!((d.waits_on, d.deciding_until_ms), ("user", None));
    // A later call replaces it.
    call(&rt, "annotate", json!({"decision": id, "note": "Changed my mind.", "recommend": "approve"}));
    assert_eq!(card(&rt, n).annotation.unwrap()["note"], "Changed my mind.");
    // Limits: 280 characters, a recommendation that is one of the card's actions.
    let (_, v) = call(&rt, "annotate", json!({"decision": id, "note": "x".repeat(281)}));
    assert_eq!(v["code"], "bad_request");
    let (_, v) = call(&rt, "annotate", json!({"decision": id, "note": "y", "recommend": "launch"}));
    assert_eq!(v["code"], "bad_request");
    let (_, v) = call(&rt, "annotate", json!({"decision": "inbox-9999", "note": "y"}));
    assert_eq!(v["code"], "stale");
    // After hand-over the Overwatch can no longer decide it.
    let (_, v) = call(&rt, "decide_permission", json!({"decision": id, "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "too_late");
}

fn card(rt: &Runtime, n: u64) -> crate::decision::Decision {
    rt.now_pump(true);
    let want = crate::decision::decision_id("permission", n);
    rt.open_decisions().into_iter().find(|d| d.id == want).expect("open card")
}

#[test]
fn a_gate_can_only_be_annotated() {
    let (_d, rt) = manual();
    let g = rt
        .add_item(
            Some("a1"),
            "gate",
            "a1: spec".into(),
            json!({"action": "spec", "feature": "checkout", "summary": "The spec"}),
            20,
        )
        .unwrap();
    let (err, v) = call(
        &rt,
        "annotate",
        json!({"decision": format!("gate-{g}"), "note": "Scope looks right.", "recommend": "approve"}),
    );
    assert!(!err, "{v}");
    rt.now_pump(true);
    let d = rt.open_decisions().into_iter().find(|d| d.id == format!("gate-{g}")).unwrap();
    assert_eq!(d.annotation.unwrap()["recommend"], "approve");
    // None of the decide tools reach it.
    let (_, v) =
        call(&rt, "decide_permission", json!({"decision": format!("gate-{g}"), "effect": "allow", "reason": "x"}));
    assert_eq!(v["code"], "bad_request");
    let (_, v) = call(
        &rt,
        "answer_question",
        json!({"decision": format!("gate-{g}"), "answers": [{"text": "x"}], "sources": ["a"]}),
    );
    assert_eq!(v["code"], "bad_request");
    assert!(item_of(&rt, g).open());
}

// ------------------------------------------------------------ reprioritize, unsticking, post

fn queue_one(rt: &Runtime, tier: Tier) -> u64 {
    rt.governor_act(|g, now| {
        g.pause_all(now);
        g.request_turn(TurnRequest::task("a1", "default", tier), now);
        (g.queue().iter().find(|e| e.agent == AgentId::from("a1")).map(|e| e.id.0).unwrap(), Vec::new())
    })
}

#[test]
fn reprioritize_moves_a_queued_entry_within_its_allocation_and_can_be_restored() {
    let (_d, rt) = enforcing();
    let e = queue_one(&rt, Tier::Normal);
    let (err, v) = call(&rt, "reprioritize", json!({"entry": e, "tier": "low", "why": "can wait for the build"}));
    assert!(!err, "{v}");
    assert_eq!(v["from"], "normal");
    assert_eq!(rt.with_governor(|g| g.queue()[0].tier), Tier::Low);
    // Never above the allocation's tier (Normal), never Critical, nothing unqueued.
    let (_, v) = call(&rt, "reprioritize", json!({"entry": e, "tier": "high", "why": "x"}));
    assert_eq!(v["code"], "not_allowed", "{v}");
    let (_, v) = call(&rt, "reprioritize", json!({"entry": e, "tier": "critical", "why": "x"}));
    assert_eq!(v["code"], "bad_request");
    let (_, v) = call(&rt, "reprioritize", json!({"entry": 9999, "tier": "low", "why": "x"}));
    assert_eq!(v["code"], "stale");
    assert_eq!(rt.db.audit_log(Some("a1"), 5).unwrap().iter().filter(|a| a.actor == "overwatch").count(), 1);
    // The action is a line of its own, with "Restore tier".
    let entry = settle(&rt).into_iter().find(|x| x.kind == "action").expect("an action entry");
    let line = entry.line.clone().unwrap();
    assert_eq!(
        (line["verb"].as_str(), line["by"].as_str(), line["undo"]["label"].as_str()),
        (Some("reprioritized"), Some("overwatch"), Some("Restore tier"))
    );
    assert_eq!(line["by_detail"], "can wait for the build");
    let r = rt.chat_undo(&entry.id(), None).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(rt.with_governor(|g| g.queue()[0].tier), Tier::Normal);
    assert!(r["entry"]["line"]["undone_at_ms"].is_u64() && r["entry"]["line"].get("undo").is_none());
}

#[test]
fn nudge_resume_and_restart_fresh_are_audited_and_posted_as_actions() {
    let (_d, rt) = manual();
    // nudge: the next turn of the agent, from the Overwatch.
    let (err, v) = call(&rt, "nudge", json!({"agent": "a1", "text": "Run the tests before you commit."}));
    assert!(!err, "{v}");
    assert_eq!(texts(&rt), ["ORCH: from the Overwatch: Run the tests before you commit."]);
    // resume_agent: through the shared handler (a running agent is refused).
    let (err, v) = call(&rt, "resume_agent", json!({"agent": "a1"}));
    assert!(!err, "{v}");
    assert_eq!(v["how"], "resumed");
    assert!(texts(&rt).last().unwrap().contains("you were resumed by the Overwatch"), "{:?}", texts(&rt));
    // restart_fresh: a new session, the unstarted turns kept.
    let before = rt.store.read(|s| s.agents["a1"].session_id.clone());
    let (err, v) = call(&rt, "restart_fresh", json!({"agent": "a1", "why": "never started"}));
    // (a1 has no worktree process: the restart may refuse to spawn in this sandbox; the session id moved first)
    let _ = (err, v);
    assert_ne!(rt.store.read(|s| s.agents["a1"].session_id.clone()), before, "a fresh session id");
    // Unknown agents, and the Overwatch itself.
    for (tool, agent) in [("nudge", "nobody"), ("nudge", ID), ("resume_agent", ID), ("restart_fresh", "nobody")] {
        let (err, v) = call(&rt, tool, json!({"agent": agent, "text": "x", "why": "x"}));
        assert!(err, "{tool} {agent}: {v}");
    }
    // Every acting call is audited and a line.
    let audit: Vec<String> = rt
        .db
        .audit_log(Some("a1"), 20)
        .unwrap()
        .into_iter()
        .filter(|a| a.actor == "overwatch")
        .map(|a| a.action)
        .collect();
    assert!(audit.contains(&"nudge".to_string()) && audit.contains(&"resume_agent".to_string()), "{audit:?}");
    let verbs: Vec<String> = settle(&rt)
        .into_iter()
        .filter(|e| e.kind == "action")
        .map(|e| e.line.unwrap()["verb"].as_str().unwrap().to_string())
        .collect();
    assert!(verbs.contains(&"nudged".to_string()) && verbs.contains(&"resumed".to_string()), "{verbs:?}");
    // No undo: it already ran.
    assert!(settle(&rt).iter().filter(|e| e.kind == "action").all(|e| e.line.as_ref().unwrap().get("undo").is_none()));
}

#[test]
fn the_leads_nudge_and_resume_use_the_same_handlers() {
    let (_d, rt) = manual();
    let (text, data) = rt.nudge_agent("a1", "your lead", "go on").unwrap();
    assert!(text.starts_with("Told a1") && data["turn"].is_string());
    assert_eq!(texts(&rt), ["ORCH: from your lead: go on"]);
    let (_, _, how) = rt.resume_member("a1", "lead", "your lead", "idle").unwrap();
    assert_eq!(how, "resumed");
    assert!(texts(&rt).last().unwrap().starts_with("ORCH: you were resumed by your lead."));
    rt.pause_all();
    assert_eq!(rt.resume_member("a1", "overwatch", "the Overwatch", "idle").unwrap().2, "queued", "Pause all holds it");
}

#[test]
fn post_writes_a_text_entry_answering_the_message_of_its_turn() {
    let (_d, rt) = manual();
    let (err, v) = call(&rt, "post", json!({"text": "x".repeat(2001)}));
    assert!(err && v["code"] == "bad_request");
    // A user message wakes it; its post answers that entry.
    let sent = rt.chat_send("why is checkout slow today?", None).unwrap();
    rt.overwatch_pump_at(t0() + 1);
    let (err, v) =
        call(&rt, "post", json!({"text": "**Checkout** waits on the build queue: see [agent:back-1](agent:back-1)."}));
    assert!(!err, "{v}");
    let e = rt.db.chat_get(v["entry"].as_str().unwrap().trim_start_matches("c-").parse().unwrap()).unwrap().unwrap();
    assert_eq!((e.author.as_str(), e.kind.as_str()), ("overwatch", "text"));
    assert_eq!(json!(e)["reply_to"], sent.id());
    // The entry is pushed with the next `now`.
    let snap = rt.now_snapshot();
    assert!(
        snap["chat"]["entries"].as_array().unwrap().iter().any(|x| x["id"] == e.id() && x["author"] == "overwatch")
    );
}

// ------------------------------------------------------------ chat_send

#[test]
fn a_message_sent_while_paused_is_pending_until_the_pause_lifts() {
    let (_d, rt) = manual();
    rt.pause_all();
    let a = rt.chat_send("anything new?", None).unwrap();
    let b = rt.chat_send("never mind this one", None).unwrap();
    assert_eq!(json!(a)["delivery"], "pending");
    rt.chat_discard(&b.id()).unwrap();
    rt.overwatch_pump_at(t0());
    assert!(super::tests::ow_turns(&rt).is_empty(), "paused: nothing is delivered");
    rt.resume_all();
    rt.overwatch_pump_at(t0() + 1);
    let turns = super::tests::ow_turns(&rt);
    assert!(turns.len() == 1 && turns[0].contains("anything new?") && !turns[0].contains("never mind"), "{turns:?}");
    assert_eq!(json!(rt.db.chat_get(a.n).unwrap().unwrap())["delivery"], "delivered");
}

#[test]
fn chat_send_tracks_delivery_and_discard_only_takes_pending() {
    let (_d, rt) = manual();
    assert!(rt.chat_send("  ", None).is_err());
    assert!(rt.chat_send(&"x".repeat(2001), None).unwrap_err().to_string().contains("bad_request"));
    // Waking until a turn takes it.
    let a = rt.chat_send("status of the checkout run?", None).unwrap();
    assert_eq!(json!(a)["delivery"], "waking");
    let queued: Vec<String> = rt.ow().queue.iter().map(|e| format!("{:?} {}", e.kind, e.summary)).collect();
    assert_eq!(rt.overwatch_status().events, 1, "{queued:?}");
    rt.overwatch_pump_at(t0() + 1);
    assert_eq!(json!(rt.db.chat_get(a.n).unwrap().unwrap())["delivery"], "delivered");
    assert!(rt.chat_discard(&a.id()).is_err(), "delivered: nothing to discard");
    // While down: pending, kept, and discard removes it for good.
    rt.overwatch_down(why::STOPPED);
    let b = rt.chat_send("anyone there?", None).unwrap();
    assert_eq!(json!(b)["delivery"], "pending");
    // The first message was in the turn that never ended: it waits again.
    assert_eq!(json!(rt.db.chat_get(a.n).unwrap().unwrap())["delivery"], "pending");
    assert_eq!(rt.overwatch_pending().len(), 2);
    let r = rt.chat_discard(&b.id()).unwrap();
    assert_eq!(r["removed"], b.id());
    assert!(rt.db.chat_get(b.n).unwrap().is_none());
    assert_eq!(rt.overwatch_pending().len(), 1);
    // A kept message goes with the restart and reads delivered afterwards.
    let c = rt.chat_send("back now?", None).unwrap();
    assert_eq!(json!(c)["delivery"], "pending");
    rt.overwatch_restart().unwrap();
    let last = super::tests::ow_turns(&rt).pop().unwrap();
    assert!(last.contains("back now?") && last.contains("status of the checkout run?"), "{last}");
    assert!(!last.contains("anyone there?"), "discarded: {last}");
    assert_eq!(json!(rt.db.chat_get(c.n).unwrap().unwrap())["delivery"], "delivered");
    assert_eq!(json!(rt.db.chat_get(a.n).unwrap().unwrap())["delivery"], "delivered");
}

#[test]
fn a_pending_message_from_an_earlier_day_comes_with_the_next_briefing() {
    let (_d, rt) = manual();
    rt.set_overwatch_settings(Settings { enabled: false, ..rt.overwatch_settings() }).ok();
    let mut s = rt.overwatch_settings();
    s.enabled = false;
    rt.db.set_setting("permissions.hold_secs", &json!(300)).unwrap();
    rt.set_overwatch_settings(s).unwrap();
    let old = rt.chat_send("remember the release note", None).unwrap();
    assert_eq!(json!(old)["delivery"], "pending");
    // The app restarts the next day with the Overwatch on.
    at(&rt, t0() + 26 * 3600 * 1000);
    let mut s = rt.overwatch_settings();
    s.enabled = true;
    rt.set_overwatch_settings(s).unwrap();
    *rt.overwatch.sched.lock().unwrap() = Sched::default();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0() + 26 * 3600 * 1000);
    let brief = std::fs::read_to_string(rt.overwatch_day_dir("2026-11-06").join("briefing.md")).unwrap();
    assert!(brief.contains("sent earlier") && brief.contains("remember the release note"), "{brief}");
    let turn = super::tests::ow_turns(&rt).pop().unwrap();
    assert!(turn.contains("remember the release note"), "{turn}");
    let _ = (Arc::new(()), Ordering::SeqCst, Duration::ZERO, HookPayload::parse(&json!({})));
}

// ------------------------------------------------------------ world, inspect

#[test]
fn world_and_inspect_have_the_contracts_shapes() {
    let (_d, rt) = enforcing();
    queue_one(&rt, Tier::Normal);
    let n = deciding_item(&rt, "pnpm add zod");
    let (err, w) = call(&rt, "world", json!({}));
    assert!(!err, "{w}");
    for k in ["agents", "runs", "counts", "decisions", "queue", "governor", "overwatch"] {
        assert!(w.get(k).is_some(), "{k} in {w}");
    }
    assert!(
        w["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == format!("inbox-{n}") && d.get("payload").is_none())
    );
    assert_eq!(w["queue"][0]["agent"], "a1");
    for k in ["week_pct", "plan_pct", "reserve_left_pct", "last_actions"] {
        assert!(w["governor"].get(k).is_some(), "{k}");
    }
    assert!(w["governor"]["last_actions"].as_array().unwrap().len() <= 20);
    assert!(w["overwatch"]["status"].is_string());
    let (_, w) = call(&rt, "world", json!({"project": "nope"}));
    assert!(w["agents"].as_array().unwrap().is_empty() && w["decisions"].as_array().unwrap().is_empty());
    // inspect
    let tr = rt.cfg.projects_root.join("-fake");
    std::fs::create_dir_all(&tr).unwrap();
    let sid = rt.store.read(|s| s.agents["a1"].session_id.clone());
    let lines: Vec<String> = (0..12)
        .map(|i| json!({"type": "assistant", "message": {"content": [{"type": "text", "text": format!("step {i} {}", "z".repeat(500))}]}}).to_string())
        .collect();
    std::fs::write(tr.join(format!("{sid}.jsonl")), lines.join("\n")).unwrap();
    let (err, v) = call(&rt, "inspect", json!({"agent": "a1"}));
    assert!(!err, "{v}");
    for k in ["attention", "task", "run", "messages", "transcript_tail", "decisions", "worktree", "feature_docs"] {
        assert!(v.get(k).is_some(), "{k} in {v}");
    }
    let tail = v["transcript_tail"].as_array().unwrap();
    assert_eq!(tail.len(), 10);
    assert!(tail[9].as_str().unwrap().starts_with("step 11") && tail[0].as_str().unwrap().starts_with("step 2"));
    assert!(tail.iter().all(|t| t.as_str().unwrap().chars().count() <= 401));
    assert_eq!(v["decisions"][0]["id"], format!("inbox-{n}"));
    let (err, v) = call(&rt, "inspect", json!({"agent": "nobody"}));
    assert!(err && v["code"] == "bad_request");
}

// ------------------------------------------------------------ the tools are the Overwatch's alone

#[test]
fn other_agents_cannot_call_the_overwatch_tools() {
    let (_d, rt) = manual();
    for t in orch_team::compile::OVERWATCH_TOOLS.iter().filter(|t| !["nudge", "resume_agent"].contains(t)) {
        let (is_err, text, _) = rt.on_mcp(
            &orch_protocol::McpMsg {
                agent_id: "a1".into(),
                tool: (*t).into(),
                arguments: json!({}),
                deadline_ms: 1000,
            },
            None,
        );
        assert!(is_err && text.contains("not one of your tools"), "{t}: {text}");
    }
}

// ------------------------------------------------------------ chat_send mentions

use orch_store::{Feature, Project, TeamRun};
use orch_team::{RunState, Scope};

/// A project with the feature `f_1` (slug `checkout`), its run `r_1` in `status`, and the docs folder.
fn mentionable(rt: &Runtime, d: &std::path::Path, status: RunState) -> std::path::PathBuf {
    let proj = Project {
        id: "p_1".into(),
        name: "shop".into(),
        path: d.join("shop"),
        default_branch: "main".into(),
        remotes: vec![],
        worktree_root: d.join("shop.worktrees"),
        task_backend: json!({"type": "local"}),
        track_orchestrator_dir: true,
        settings: json!({}),
        created_at_ms: 1,
        last_active_at_ms: 1,
        last_fetch_at_ms: None,
        last_fetch_error: None,
        archived: false,
    };
    rt.db.upsert_project(&proj).unwrap();
    let wt = d.join("integration");
    std::fs::create_dir_all(wt.join(".orchestrator/features/checkout")).unwrap();
    let f = Feature {
        id: "f_1".into(),
        project_id: "p_1".into(),
        slug: "checkout".into(),
        title: "Checkout v2".into(),
        brief: "b".into(),
        base: "main".into(),
        integration_worktree: Some(wt.clone()),
        allocation: None,
        created_at_ms: 1,
        archived: false,
    };
    rt.db.upsert_feature(&f).unwrap();
    set_run(rt, status);
    wt
}

fn set_run(rt: &Runtime, status: RunState) {
    rt.db
        .upsert_run(&TeamRun {
            id: "r_1".into(),
            team_id: "full-stack".into(),
            team_scope: Scope::Project("p_1".into()),
            team_version: 1,
            feature_id: "f_1".into(),
            status,
            resume_status: None,
            pr_url: None,
            started_at_ms: 1,
            ended_at_ms: None,
            pr_number: None,
            pr: None,
        })
        .unwrap();
}

fn mention(id: &str) -> Value {
    json!({"kind": "feature", "id": id})
}

#[test]
fn a_mention_resolves_into_the_entry_meta_and_the_event_line() {
    let (d, rt) = manual();
    let wt = mentionable(&rt, d.path(), RunState::Executing);
    let ctx = json!({"mentions": [mention("f_1"), mention("f_1")]});
    let e = rt.chat_send_with("what is @checkout waiting on?", None, Some(&ctx)).unwrap();
    let m = &e.meta.as_ref().unwrap()["mentions"];
    assert_eq!(m.as_array().unwrap().len(), 1, "duplicates collapse: {m}");
    assert_eq!(
        m[0],
        json!({"kind": "feature", "id": "f_1", "slug": "checkout", "title": "Checkout v2",
               "project_id": "p_1", "project": "shop"})
    );
    let audit = rt.db.audit_log(None, 5).unwrap();
    assert!(audit.iter().any(|a| a.action == "chat_send" && a.detail["mentions"] == 1), "{audit:?}");
    rt.overwatch_pump_at(t0() + 1);
    let turn = &super::tests::ow_turns(&rt)[0];
    let line = turn.lines().find(|l| l.contains("user_message")).unwrap();
    let json_part = line.split(" · mentions ").nth(1).expect("mentions on the line");
    let v: Value = serde_json::from_str(json_part).unwrap();
    assert_eq!(v[0]["run"], "r_1");
    assert_eq!(v[0]["status"], "executing");
    assert_eq!(v[0]["project"], "shop");
    assert_eq!(v[0]["branch"], "orch/checkout/integration");
    assert_eq!(v[0]["worktree"], json!(wt));
    assert_eq!(v[0]["docs_dir"], json!(wt.join(".orchestrator/features/checkout")));
    assert!(v[0]["agents"].is_array() && v[0]["pr_url"].is_null());
}

#[test]
fn unknown_ids_are_dropped_and_no_meta_is_written_when_none_resolve() {
    let (d, rt) = manual();
    mentionable(&rt, d.path(), RunState::Executing);
    let e = rt.chat_send_with("hi @gone", None, Some(&json!({"mentions": [mention("f_nope")]}))).unwrap();
    assert!(e.meta.as_ref().unwrap().get("mentions").is_none());
    let e = rt.chat_send_with("hi", None, Some(&json!({"mentions": [mention("f_nope"), mention("f_1")]}))).unwrap();
    assert_eq!(e.meta.as_ref().unwrap()["mentions"].as_array().unwrap().len(), 1);
    // An archived feature is kept.
    let mut f = rt.db.feature("f_1").unwrap().unwrap();
    f.archived = true;
    rt.db.upsert_feature(&f).unwrap();
    let e = rt.chat_send_with("old", None, Some(&json!({"mentions": [mention("f_1")]}))).unwrap();
    assert_eq!(e.meta.as_ref().unwrap()["mentions"][0]["id"], "f_1");
}

#[test]
fn a_bad_mentions_shape_is_refused() {
    let (d, rt) = manual();
    mentionable(&rt, d.path(), RunState::Executing);
    let bad = |ctx: Value| {
        let e = rt.chat_send_with("x", None, Some(&ctx)).unwrap_err().to_string();
        assert!(e.starts_with("bad_request"), "{ctx}: {e}");
    };
    bad(json!({"mentions": "f_1"}));
    bad(json!({"mentions": [{"kind": "feature"}]}));
    bad(json!({"mentions": [{"kind": "feature", "id": 3}]}));
    bad(json!({"mentions": ["f_1"]}));
    bad(json!({"mentions": [{"kind": "agent", "id": "a1"}]}));
    bad(json!({"mentions": (0..11).map(|i| mention(&format!("f_{i}"))).collect::<Vec<_>>()}));
    assert!(rt.db.chat_pending_user().unwrap().is_empty(), "nothing was sent");
    let ten: Vec<Value> = (0..10).map(|_| mention("f_1")).collect();
    assert!(rt.chat_send_with("x", None, Some(&json!({"mentions": ten}))).is_ok());
    assert!(rt.chat_send_with("x", None, Some(&json!({"mentions": null}))).is_ok());
}

#[test]
fn mentions_and_plan_ride_together() {
    let (d, rt) = manual();
    mentionable(&rt, d.path(), RunState::Spec);
    let ctx = json!({"plan": "p_1", "mentions": [mention("f_1")]});
    let e = rt.chat_send_with("plan next to @checkout", None, Some(&ctx)).unwrap();
    let m = e.meta.as_ref().unwrap();
    assert!(m["plan"].is_object() && m["mentions"][0]["slug"] == "checkout", "{m}");
    rt.overwatch_pump_at(t0() + 1);
    let turn = &super::tests::ow_turns(&rt)[0];
    let line = turn.lines().find(|l| l.contains("user_message")).unwrap();
    assert!(line.find(" · plan ").unwrap() < line.find(" · mentions ").unwrap(), "{line}");
}

#[test]
fn a_pending_message_re_resolves_its_mentions_on_rebuild() {
    let (d, rt) = manual();
    mentionable(&rt, d.path(), RunState::Executing);
    rt.chat_send_with("status of @checkout?", None, Some(&json!({"mentions": [mention("f_1")]}))).unwrap();
    // The in-memory event is lost (app restart) and the run moved on meanwhile.
    rt.ow().queue.clear();
    set_run(&rt, RunState::PrGate);
    rt.overwatch_load_db_pending();
    let ev = rt.ow().pending_user.last().cloned().expect("rebuilt");
    let m = ev.mentions.expect("mentions re-resolved");
    assert_eq!((m[0]["id"].as_str(), m[0]["status"].as_str()), (Some("f_1"), Some("pr_gate")));
    // A feature deleted meanwhile leaves no mentions on the event.
    assert!(rt.mentions_of_meta(Some(&json!({"mentions": [{"id": "f_gone"}]}))).is_none());
    assert!(rt.mentions_of_meta(None).is_none());
}
