use std::sync::{Arc, Mutex};

use orch_protocol::InboxDecision;
use orch_store::{InboxStatus, RuleEffect, RuleScope};

use super::*;
use crate::decision::Lifecycle;
use crate::runtime::tests::{hook, test_runtime};
use crate::state::AgentState;

/// A zone far from most machines' own, so nothing leans on the system's.
fn tz() -> TimeZone {
    TimeZone::fixed(jiff::tz::offset(-7))
}

/// Epoch ms of a local wall time in [`tz`].
fn at(date: &str, time: &str) -> u64 {
    let dt: jiff::civil::DateTime = format!("{date}T{time}").parse().unwrap();
    dt.to_zoned(tz()).unwrap().timestamp().as_millisecond() as u64
}

/// The clock the publisher reads: real time (items carry it), in [`tz`].
fn fix_clock(rt: &Runtime, now: u64) {
    *rt.now.clock.lock().unwrap() = Some((now, tz()));
}

fn today(rt: &Runtime) -> String {
    let (now, tz) = rt.now.clock();
    crate::digest::today(now, &tz).1
}

fn collect(rt: &Runtime) -> Arc<Mutex<Vec<Value>>> {
    let got = Arc::new(Mutex::new(vec![]));
    let g = got.clone();
    rt.subscribe_now(Arc::new(move |v| g.lock().unwrap().push(v.clone())));
    got
}

/// Today's entries of `kind` decision, as the snapshot has them.
fn decisions(rt: &Runtime) -> Vec<Value> {
    let snap = rt.now_snapshot();
    snap["chat"]["entries"].as_array().unwrap().iter().filter(|e| e["kind"] == "decision").cloned().collect()
}

fn entry_of(rt: &Runtime, decision: &str) -> Vec<Value> {
    decisions(rt).into_iter().filter(|e| e["decision"] == decision).collect()
}

fn set_state(rt: &Runtime, st: AgentState) {
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = st).unwrap();
}

fn perm_item(rt: &Runtime, cmd: &str) -> u64 {
    rt.add_item(
        Some("a1"),
        "permission",
        format!("a1: Bash: {cmd}"),
        json!({"tool": "Bash", "tool_input": {"command": cmd}}),
        30,
    )
    .unwrap()
}

#[test]
fn a_stored_decision_gets_one_entry_then_its_line_and_card() {
    let (_d, rt) = test_runtime(true);
    fix_clock(&rt, orch_protocol::now_ms());
    let got = collect(&rt);
    rt.now_pump(false);
    let q = rt.add_item(Some("a1"), "question", "a1: which db?".into(), json!({"question": "which db?"}), 20).unwrap();
    rt.now_pump(false);
    let id = format!("q-{q}");
    // The push carries the new entry: open, no line, no card.
    let ev = got.lock().unwrap().last().cloned().unwrap();
    let new = ev["chat"].as_array().unwrap().iter().find(|e| e["decision"] == id.as_str()).cloned().unwrap();
    assert_eq!(
        (new["author"].as_str(), new["thread"].as_str(), new["rev"].as_u64()),
        (Some("core"), Some("a1"), Some(1))
    );
    assert!(new.get("line").is_none() && new.get("card").is_none(), "{new}");
    assert_eq!(new["day"], today(&rt));
    // Nothing new: no chat in the next push, and no second entry.
    rt.touch("x");
    rt.now_pump(false);
    assert!(got.lock().unwrap().last().unwrap().get("chat").is_none());
    // Answered: one write adds the line and the card (payload omitted).
    rt.decide(q, InboxDecision::Reply, None, Some("sqlite")).unwrap();
    rt.now_pump(false);
    let ev = got.lock().unwrap().last().cloned().unwrap();
    let changed = ev["chat"].as_array().unwrap().iter().find(|e| e["decision"] == id.as_str()).cloned().unwrap();
    assert_eq!((changed["id"].clone(), changed["rev"].as_u64()), (new["id"].clone(), Some(2)));
    let line = &changed["line"];
    assert_eq!(
        (line["verb"].as_str(), line["by"].as_str(), line["agent"].as_str(), line["what"].as_str()),
        (Some("answered"), Some("you"), Some("a1"), Some("which db?"))
    );
    assert_eq!(changed["card"]["id"], id.as_str());
    assert!(changed["card"].get("payload").is_none());
    // Its lifecycle goes on while it is listed: the entry follows.
    rt.answers_resumed("a1");
    rt.now_pump(false);
    let all = entry_of(&rt, &id);
    assert_eq!(all.len(), 1, "one entry per decision");
    assert_eq!(all[0]["card"]["lifecycle"]["effect"], "resumed");
    assert_eq!(all[0]["line"]["state"], "effect");
    assert!(all[0]["rev"].as_u64().unwrap() > 2);
}

#[test]
fn a_synthesized_decision_that_clears_is_stale_and_comes_back_as_a_new_entry() {
    let (_d, rt) = test_runtime(true);
    fix_clock(&rt, orch_protocol::now_ms());
    rt.now_pump(false);
    set_state(&rt, AgentState::WaitingForYou);
    rt.now_pump(false);
    let first = entry_of(&rt, "wait-a1");
    assert_eq!(first.len(), 1);
    assert!(first[0].get("line").is_none());
    // Answered in the pane: it clears by itself (ux F8).
    set_state(&rt, AgentState::Working);
    rt.now_pump(false);
    let e = &entry_of(&rt, "wait-a1")[0];
    assert_eq!(
        (e["line"]["verb"].as_str(), e["line"]["by"].as_str(), e["line"]["state"].as_str()),
        (Some("dismissed"), Some("core"), Some("stale"))
    );
    assert_eq!(e["card"]["kind"], "wait");
    // Back later the same day: a new entry.
    set_state(&rt, AgentState::WaitingForYou);
    rt.now_pump(false);
    let both = entry_of(&rt, "wait-a1");
    assert_eq!(both.len(), 2);
    assert_ne!(both[0]["id"], both[1]["id"]);
    assert!(both[1].get("line").is_none());
    // One answered by the user carries the answer instead.
    set_state(&rt, AgentState::Error);
    rt.now_pump(false);
    rt.synth_answered("error-a1", "resumed");
    set_state(&rt, AgentState::Idle);
    rt.now_pump(false);
    let e = &entry_of(&rt, "error-a1")[0];
    assert_eq!((e["line"]["verb"].as_str(), e["line"]["by"].as_str()), (Some("restarted"), Some("you")));
}

#[test]
fn rule_answers_are_posted_settled_and_never_open() {
    let (_d, rt) = test_runtime(true);
    fix_clock(&rt, orch_protocol::now_ms());
    let got = collect(&rt);
    rt.now_pump(false);
    rt.db.add_rule(RuleScope::Agent, "a1", "Bash(cargo add:*)", RuleEffect::Allow, None).unwrap();
    let perm = |cmd: &str| {
        hook("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "t"}))
    };
    assert!(rt.on_hook(&perm("cargo add serde_json"), None).is_some());
    rt.now_pump(false);
    let ev = got.lock().unwrap().last().cloned().unwrap();
    // (The hook also shows a1 waiting in its pane: `wait-a1`, another entry.)
    let audit = |ev: &Value| -> Vec<Value> {
        let chat = ev["chat"].as_array().cloned().unwrap_or_default();
        chat.into_iter().filter(|e| e["decision"].as_str().is_some_and(|d| d.starts_with("audit-"))).collect()
    };
    let posted = audit(&ev);
    assert_eq!(posted.len(), 1, "{ev}");
    let e = &posted[0];
    assert_eq!(e["rev"], 1, "line and card in the first write");
    assert_eq!(
        (e["line"]["verb"].as_str(), e["line"]["by"].as_str(), e["line"]["by_detail"].as_str()),
        (Some("allowed"), Some("rule"), Some("Bash(cargo add:*)"))
    );
    assert_eq!((e["line"]["what"].as_str(), e["line"]["agent"].as_str()), (Some("cargo add serde_json"), Some("a1")));
    assert_eq!(e["card"]["kind"], "permission");
    assert!(e["card"]["lifecycle"]["decided_by"].as_str().unwrap().starts_with("rule:"));
    assert_eq!(e["card"]["actions"], json!([]), "settled: nothing to act on");
    // Never a decision: no card in `now`, no badge.
    let listed = |ev: &Value| ev["decisions"]["upsert"].as_array().unwrap().iter().any(|d| d["kind"] == "permission");
    assert!(!listed(&ev));
    assert!(rt.open_decisions().iter().all(|d| d.kind != "permission"), "never counted in the badge");
    // A deny rule too.
    rt.db.add_rule(RuleScope::Agent, "a1", "Bash(rm:*)", RuleEffect::Deny, None).unwrap();
    rt.on_hook(&perm("rm -rf build"), None);
    rt.now_pump(false);
    let ev = got.lock().unwrap().last().cloned().unwrap();
    assert!(!listed(&ev));
    let e = audit(&ev)[0].clone();
    assert_eq!(e["line"]["verb"], "denied");
}

#[test]
fn pane_and_core_closes_get_their_line_and_unlisted_ones_are_posted_settled() {
    let (_d, rt) = test_runtime(true);
    fix_clock(&rt, orch_protocol::now_ms());
    rt.now_pump(false);
    // Listed open, then answered in the pane.
    let p = perm_item(&rt, "ls");
    rt.now_pump(false);
    rt.store
        .write(|s| s.close_inbox(p, InboxStatus::Dismissed, Some("answered in the pane".into()), "pane").map(|_| ()))
        .unwrap()
        .unwrap();
    rt.now_pump(false);
    let e = &entry_of(&rt, &format!("inbox-{p}"))[0];
    assert_eq!((e["line"]["verb"].as_str(), e["line"]["by"].as_str()), (Some("answered"), Some("pane")));
    // Answered in the pane before the core ever listed it: posted closed.
    let p2 = perm_item(&rt, "pwd");
    rt.store
        .write(|s| s.close_inbox(p2, InboxStatus::Dismissed, Some("answered in the pane".into()), "pane").map(|_| ()))
        .unwrap()
        .unwrap();
    rt.now_pump(false);
    let e = entry_of(&rt, &format!("inbox-{p2}"));
    assert_eq!(e.len(), 1);
    assert_eq!((e[0]["rev"].as_u64(), e[0]["line"]["by"].as_str()), (Some(1), Some("pane")));
    // Closed by the core (the agent went away).
    let a = rt.add_item(Some("a1"), "alert", "a1: x".into(), json!({}), 0).unwrap();
    rt.now_pump(false);
    rt.close_items_of("a1", "agent deleted");
    rt.now_pump(false);
    let e = &entry_of(&rt, &format!("alert-{a}"))[0];
    assert_eq!((e["line"]["verb"].as_str(), e["line"]["by"].as_str()), (Some("dismissed"), Some("core")));
    // A parked permission the core closes: expired (ux F10).
    let pk = perm_item(&rt, "make deploy");
    rt.now_pump(false);
    rt.store.write(|s| s.park_inbox(pk)).unwrap();
    rt.now_pump(false);
    rt.close_items_of("a1", "agent deleted");
    rt.now_pump(false);
    let e = &entry_of(&rt, &format!("inbox-{pk}"))[0];
    assert_eq!((e["line"]["verb"].as_str(), e["line"]["by"].as_str()), (Some("expired"), Some("expired")));
}

#[test]
fn a_decision_closed_while_the_app_was_down_gets_its_line_at_start() {
    let (_d, rt) = test_runtime(true);
    let t0 = orch_protocol::now_ms();
    fix_clock(&rt, t0);
    rt.now_pump(false);
    let q = rt.add_item(Some("a1"), "question", "a1: go on?".into(), json!({"question": "go on?"}), 20).unwrap();
    rt.now_pump(false);
    // Quit; it is answered meanwhile (the reconciler, another client) …
    rt.forget_now();
    rt.decide(q, InboxDecision::Reply, None, Some("yes")).unwrap();
    rt.answers_resumed("a1");
    // … and the app starts 15 min later: past RECENT_MS, it is not listed.
    fix_clock(&rt, t0 + 15 * 60_000);
    rt.now_pump(false);
    assert!(rt.now_snapshot()["decisions"].as_array().unwrap().is_empty());
    let e = entry_of(&rt, &format!("q-{q}"));
    assert_eq!(e.len(), 1);
    assert_eq!((e[0]["line"]["verb"].as_str(), e[0]["line"]["by"].as_str()), (Some("answered"), Some("you")));
    assert_eq!(e[0]["card"]["title"], "go on?", "it expands from the snapshot");
    // A synthesized card that cleared while the app was down: closed from its snapshot.
    set_state(&rt, AgentState::Error);
    rt.now_pump(false);
    rt.forget_now();
    set_state(&rt, AgentState::Idle);
    rt.now_pump(false);
    let e = &entry_of(&rt, "error-a1")[0];
    assert_eq!((e["line"]["state"].as_str(), e["card"]["kind"].as_str()), (Some("stale"), Some("error")));
    // Still open at a restart: the same entry, no second one.
    let q2 = rt.add_item(Some("a1"), "question", "a1: next?".into(), json!({"question": "next?"}), 20).unwrap();
    rt.now_pump(false);
    rt.forget_now();
    rt.now_pump(false);
    assert_eq!(entry_of(&rt, &format!("q-{q2}")).len(), 1);
}

#[test]
fn the_morning_line_once_per_day_at_start_and_at_midnight() {
    let (_d, rt) = test_runtime(true);
    let got = collect(&rt);
    fix_clock(&rt, at("2031-03-14", "08:30:00"));
    rt.now_pump(false);
    let snap = rt.now_snapshot();
    assert_eq!(snap["chat"]["day"], "2031-03-14");
    let e = &snap["chat"]["entries"][0];
    assert_eq!((e["kind"].as_str(), e["author"].as_str()), (Some("morning"), Some("core")));
    // The week part comes from the Governor's data; nothing runs or waits.
    let text = e["text"].as_str().unwrap();
    assert!(text.starts_with("Good morning.") && !text.contains("decision") && !text.contains("run"), "{text}");
    assert!(e.get("thread").is_none());
    assert_eq!(got.lock().unwrap()[0]["chat"][0]["kind"], "morning", "pushed");
    // Recomputed later the same day, or after a restart: no second one.
    rt.now_pump(true);
    fix_clock(&rt, at("2031-03-14", "23:59:59"));
    rt.forget_now();
    let q = rt.add_item(Some("a1"), "question", "a1: q?".into(), json!({"question": "q?"}), 20).unwrap();
    rt.now_pump(true);
    let mornings = |day: &str| {
        rt.db
            .chat_day(day)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "morning")
            .map(|e| e.text.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(mornings("2031-03-14").len(), 1);
    // Local midnight while running: the next day's line, with what waits.
    fix_clock(&rt, at("2031-03-15", "00:00:01"));
    rt.now_pump(true);
    let m = mornings("2031-03-15");
    assert_eq!(m.len(), 1);
    assert!(m[0].starts_with("Good morning. 1 decision waiting"), "{m:?}");
    // The decision open across midnight carries on in today's thread (ux F1).
    let snap = rt.now_snapshot();
    let entries = snap["chat"]["entries"].as_array().unwrap();
    assert_eq!(entries[0]["kind"], "morning", "the morning line comes first");
    let next = entries.iter().find(|e| e["decision"] == format!("q-{q}")).unwrap();
    let old = rt.db.chat_day("2031-03-14").unwrap().into_iter().find(|e| e.kind == "decision").unwrap();
    assert_eq!(next["continued_from"], old.id());
    assert_eq!(json!(old)["continued_to"], next["id"]);
    assert!(old.line.is_none(), "left open");
    // It closes today: the line lands on today's entry.
    rt.decide(q, InboxDecision::Reply, None, Some("x")).unwrap();
    rt.now_pump(false);
    let e = rt.db.chat_day("2031-03-15").unwrap().into_iter().find(|e| e.kind == "decision").unwrap();
    assert!(e.line.is_some());
    assert!(rt.db.chat_day("2031-03-14").unwrap().iter().all(|e| e.line.is_none()));
}

#[test]
fn morning_text_leaves_zero_parts_out() {
    let all_zero = "Good morning. Nothing is running and nothing is waiting for you.";
    assert_eq!(morning_text(0, 0, None), all_zero);
    assert_eq!(morning_text(0, 0, Some((0.2, 35.0))), all_zero);
    assert_eq!(morning_text(0, 0, Some((38.0, 0.0))), all_zero, "no plan: no week part");
    assert_eq!(
        morning_text(1, 1, Some((38.4, 35.2))),
        "Good morning. 1 run in progress, 1 decision waiting, week 38 % against 35 % planned."
    );
    assert_eq!(morning_text(2, 0, None), "Good morning. 2 runs in progress.");
    assert_eq!(morning_text(0, 3, None), "Good morning. 3 decisions waiting.");
    assert_eq!(morning_text(0, 0, Some((12.0, 10.0))), "Good morning. week 12 % against 10 % planned.");
}

fn decision(kind: &'static str, gate: Option<&'static str>, word: Option<&str>, lc: Lifecycle) -> Decision {
    Decision {
        id: "x-1".into(),
        kind,
        gate,
        level: 1,
        subject: json!({"agent": "back-1"}),
        title: "A title that is long enough to be cut by the core at forty-eight".into(),
        summary: String::new(),
        context: vec![],
        actions: vec![],
        lifecycle: lc,
        thread: None,
        thread_prev: None,
        waits_on: "core",
        created_ms: 0,
        agent: Some("back-1".into()),
        synthesized: false,
        decision: word.map(str::to_string),
        payload: json!({"tool": "Bash", "tool_input": {"command": "cargo add serde_json"}}),
    }
}

fn lc(state: &'static str, by: &str) -> Lifecycle {
    Lifecycle { state, decided_by: Some(by.into()), decided_at_ms: Some(7), ..Default::default() }
}

#[test]
fn lines_for_every_decider() {
    let line = |d: &Decision| {
        let l = line_of(d, None, None, 99);
        (l["verb"].as_str().unwrap().to_string(), l["by"].as_str().unwrap().to_string())
    };
    let pair = |v: &str, b: &str| (v.to_string(), b.to_string());
    assert_eq!(line(&decision("permission", None, Some("approve"), lc("effect", "user"))), pair("allowed", "you"));
    assert_eq!(line(&decision("permission", None, Some("always_allow"), lc("sent", "user"))), pair("allowed", "you"));
    assert_eq!(line(&decision("permission", None, Some("reply"), lc("sent", "user"))), pair("denied", "you"));
    assert_eq!(line(&decision("permission", None, Some("approve"), lc("effect", "rule:4"))), pair("allowed", "rule"));
    assert_eq!(line(&decision("permission", None, Some("in_pane"), lc("dismissed", "pane"))), pair("answered", "pane"));
    assert_eq!(line(&decision("gate", Some("spec"), Some("approve"), lc("effect", "user"))), pair("approved", "you"));
    assert_eq!(line(&decision("gate", Some("spec"), Some("reject"), lc("sent", "user"))), pair("sent back", "you"));
    assert_eq!(line(&decision("gate", Some("pr"), Some("reject"), lc("sent", "user"))), pair("rejected", "you"));
    assert_eq!(line(&decision("budget", None, Some("approve"), lc("effect", "user"))), pair("applied", "you"));
    assert_eq!(line(&decision("budget", None, Some("reject"), lc("effect", "expired"))), pair("skipped", "expired"));
    assert_eq!(line(&decision("alert", None, Some("dismiss"), lc("dismissed", "core"))), pair("dismissed", "core"));
    assert_eq!(line(&decision("question", None, Some("reply"), lc("failed", "user"))), pair("failed", "you"));
    assert_eq!(line(&decision("question", None, None, lc("interrupted", "pane"))), pair("interrupted", "pane"));
    assert_eq!(line(&decision("question", None, None, lc("dismissed", "overwatch"))), pair("dismissed", "overwatch"));
    let mut st = decision("startup", None, Some("restart_fresh"), lc("effect", "user"));
    st.lifecycle.effect = Some("restarted".into());
    assert_eq!(line(&st), pair("restarted", "you"));
    // The Governor's ask: answered, or its default taken at the deadline.
    let mut ask = decision("ask", None, None, lc("effect", "user"));
    (ask.synthesized, ask.lifecycle.effect) = (true, Some("applied".into()));
    assert_eq!(line(&ask), pair("run now", "you"));
    ask.lifecycle = lc("dismissed", "core");
    ask.payload = json!({"default": "reject", "deadline_ms": 5});
    assert_eq!(line(&ask), pair("kept queued", "expired"));
    ask.payload = json!({"default": "reject", "deadline_ms": 50});
    assert_eq!(line(&ask), pair("dismissed", "core"), "cleared before its deadline");
    // What: the command for a permission, the title cut at 48 otherwise; the rule's pattern.
    let rule = PermissionRule {
        id: 4,
        scope: RuleScope::Project,
        scope_id: "p".into(),
        pattern: "Bash(cargo add:*)".into(),
        effect: RuleEffect::Allow,
        source_inbox_item_id: None,
        created_at_ms: 0,
        revoked_at_ms: None,
    };
    let l = line_of(&decision("permission", None, Some("approve"), lc("effect", "rule:4")), Some(&rule), None, 99);
    assert_eq!(
        (l["what"].as_str(), l["by_detail"].as_str(), l["at_ms"].as_u64()),
        (Some("cargo add serde_json"), Some("Bash(cargo add:*)"), Some(7))
    );
    assert_eq!(line_of(&decision("permission", None, None, lc("effect", "rule:9")), None, None, 0)["by_detail"], "9");
    let l = line_of(&decision("question", None, Some("reply"), lc("sent", "user")), None, None, 0);
    assert_eq!(l["what"].as_str().unwrap().chars().count(), WHAT_MAX);
    assert_eq!(
        line_text(
            &json!({"verb": "allowed", "agent": "back-1", "what": "cargo add serde_json", "by": "rule", "by_detail": "installs"})
        ),
        "allowed back-1: cargo add serde_json · rule: installs"
    );
}

#[test]
fn chat_days_day_and_search() {
    let (_d, rt) = test_runtime(true);
    fix_clock(&rt, at("2031-03-15", "10:00:00"));
    let put = |day: &str, at_ms: u64, text: &str| {
        let e = ChatEntry {
            day: day.into(),
            at_ms,
            author: "core".into(),
            kind: "morning".into(),
            text: Some(text.into()),
            ..Default::default()
        };
        rt.db.chat_insert(&e).unwrap()
    };
    put("2031-03-13", at("2031-03-13", "09:00:00"), "Good morning. Serde day.");
    put("2031-03-14", at("2031-03-14", "09:00:00"), "Good morning.");
    let mut d = ChatEntry {
        day: "2031-03-14".into(),
        at_ms: at("2031-03-14", "10:00:00"),
        author: "core".into(),
        kind: "decision".into(),
        decision: Some("inbox-1".into()),
        ..Default::default()
    };
    d.line = Some(
        json!({"verb": "allowed", "agent": "back-1", "what": "cargo add serde_json", "by": "rule", "state": "effect"}),
    );
    rt.db.chat_insert(&d).unwrap();
    rt.now_pump(false); // today's morning line
    let days = json!(rt.chat_days(None, None).unwrap());
    assert_eq!(
        days,
        json!([{"day": "2031-03-14", "entries": 2, "decisions": 1}, {"day": "2031-03-13", "entries": 1, "decisions": 0}])
    );
    assert_eq!(json!(rt.chat_days(Some(1), Some("2031-03-14")).unwrap())[0]["day"], "2031-03-13");
    assert_eq!(rt.chat_days(None, Some("2031-04-01")).unwrap().len(), 2, "today stays excluded");
    // A day: its entries and outcomes (rebuilt from events, `seen` omitted).
    rt.db.add_event(at("2031-03-14", "11:00:00"), Some("a1"), "turn.done", json!({"summary": "Fixed the feed"}));
    rt.db.add_event(at("2031-03-15", "00:30:00"), Some("a1"), "turn.done", json!({"summary": "the next day"}));
    rt.db.flush().unwrap();
    let day = rt.chat_day("2031-03-14").unwrap();
    assert_eq!(day["day"], "2031-03-14");
    assert_eq!(day["entries"].as_array().unwrap().len(), 2);
    let outcomes = day["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 1, "{day}");
    assert_eq!(outcomes[0]["kind"], "turn_done");
    assert!(outcomes[0].get("seen").is_none());
    for bad in ["2031-3-14", "yesterday", "2031-02-30", ""] {
        assert!(rt.chat_day(bad).unwrap_err().to_string().starts_with("bad_request"), "{bad}");
    }
    assert!(rt.chat_days(None, Some("x")).is_err());
    // Search: case-insensitive, newest first, today included.
    let hits = rt.chat_search("SERDE", None).unwrap();
    let days: Vec<&str> = hits.as_array().unwrap().iter().map(|h| h["day"].as_str().unwrap()).collect();
    assert_eq!(days, ["2031-03-14", "2031-03-13"]);
    assert_eq!(hits[0]["entry"]["line"]["what"], "cargo add serde_json");
    assert_eq!(rt.chat_search("good morning", Some(1)).unwrap()[0]["day"], "2031-03-15");
    assert!(rt.chat_search("  ", None).unwrap_err().to_string().starts_with("bad_request"));
    // `orchd chat`'s lines.
    let entries: Vec<Value> = rt.chat_day("2031-03-14").unwrap()["entries"].as_array().unwrap().clone();
    assert_eq!(
        render(&entries, &tz()),
        "09:00 core morning Good morning.\n10:00 core decision inbox-1 allowed back-1: cargo add serde_json · rule\n"
    );
}
