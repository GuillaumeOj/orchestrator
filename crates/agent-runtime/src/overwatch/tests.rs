//! The scheduler on an injected clock (no process: a turn is a queued
//! `TurnRow`; its end is called by hand), deciding with real hold threads,
//! status, settings and the report line.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use orch_protocol::{HookOutput, HookPayload, InboxDecision};
use serde_json::{Value, json};

use super::*;
use crate::inbox::{Call, HOLD_SECS_KEY, Hold};
use crate::policy::{Effect, Evaluation, Source};
use crate::runtime::Runtime;
use crate::runtime::tests::test_runtime;

const SEC: u64 = 1000;

/// 2026-11-05 10:00 UTC.
pub(super) fn t0() -> u64 {
    "2026-11-05T10:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond() as u64
}

pub(super) fn at(rt: &Runtime, ms: u64) {
    *rt.now.clock.lock().unwrap() = Some((ms, TimeZone::UTC));
}

/// A test runtime driven by hand at `t0`.
pub(super) fn manual() -> (tempfile::TempDir, Arc<Runtime>) {
    let (d, rt) = test_runtime(true);
    rt.overwatch.manual.store(true, Ordering::SeqCst);
    at(&rt, t0());
    (d, rt)
}

pub(super) fn ow_turns(rt: &Runtime) -> Vec<String> {
    rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == ID).map(|t| t.body.clone()).collect())
}

pub(super) fn batched(summary: &str) -> Event {
    let mut e = Event::new(Kind::DecisionOpen, summary);
    e.decision = Some("gate-7".into());
    e
}

pub(super) fn blocking(summary: &str) -> Event {
    let mut e = Event::new(Kind::Stuck, summary);
    e.agent = Some("a1".into());
    e
}

#[test]
fn turn_text_and_event_lines_are_stable() {
    let mut e = Event::new(Kind::Permission, "a1: Bash pnpm add zod · effect overwatch");
    (e.id, e.decision, e.agent, e.deadline_ms) =
        ("ev-3".into(), Some("inbox-42".into()), Some("a1".into()), Some(t0() + 60 * SEC));
    let text = turn_text(&[e], true, t0(), &TimeZone::UTC);
    assert_eq!(
        text,
        "ORCH: Read briefing.md, then handle these events.\n\
         ORCH: Overwatch · 1 event · 10:00:00\n\
         - [ev-3] permission inbox-42 a1 · a1: Bash pnpm add zod · effect overwatch · until 10:01:00"
    );
}

#[test]
fn it_starts_lazily_with_a_briefing_and_batches_what_can_wait() {
    let (_d, rt) = manual();
    rt.overwatch_pump_at(t0() + 600 * SEC);
    assert!(rt.store.read(|s| !s.agents.contains_key(ID)), "a quiet day costs nothing");
    assert_eq!(rt.overwatch_status().status, Status::Asleep);

    let id = rt.overwatch_enqueue(batched("spec gate · checkout")).unwrap();
    assert_eq!(id, "ev-1");
    assert_eq!(rt.overwatch_status().status, Status::Waking);
    rt.overwatch_pump_at(t0() + 29 * SEC);
    assert!(ow_turns(&rt).is_empty(), "batched: waits the window");
    rt.overwatch_pump_at(t0() + 30 * SEC);
    let turns = ow_turns(&rt);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].starts_with("ORCH: Read briefing.md"), "{}", turns[0]);
    assert!(turns[0].contains("- [ev-1] decision_open gate-7 · spec gate · checkout"), "{}", turns[0]);
    let rec = rt.store.read(|s| s.agents[ID].clone());
    let day_dir = rt.overwatch_day_dir("2026-11-05");
    assert_eq!(rec.worktree, day_dir);
    assert_eq!((rec.allocation.as_str(), rec.tier), (ALLOCATION, governor::Tier::Critical));
    assert!(crate::runtime::reserved_slot(&rec), "its own slot, outside the live cap");
    let brief = std::fs::read_to_string(day_dir.join("briefing.md")).unwrap();
    assert!(brief.starts_with("# Overwatch briefing · 2026-11-05"), "{brief}");
    for part in [
        "## The core's hard lists",
        "git push, gh pr create",
        "## Open decisions",
        "## The week",
        "## Policy files today",
    ] {
        assert!(brief.contains(part), "{part} in {brief}");
    }
    assert!(brief.len() <= BRIEFING_MAX);
    assert_eq!(rt.overwatch_status().status, Status::Working);
}

#[test]
fn a_blocking_event_wakes_it_at_once_and_takes_the_batch_along() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(batched("gate"));
    rt.overwatch_pump_at(t0() + SEC);
    assert!(ow_turns(&rt).is_empty());
    rt.overwatch_enqueue(blocking("idle with T-4 for 12 min"));
    rt.overwatch_pump_at(t0() + 2 * SEC);
    let turns = ow_turns(&rt);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].contains("2 events"), "{}", turns[0]);
    assert!(turns[0].contains("[ev-1] decision_open") && turns[0].contains("[ev-2] stuck a1"), "{}", turns[0]);
}

#[test]
fn one_turn_at_a_time_and_what_came_meanwhile_is_the_next_turn() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(blocking("first"));
    rt.overwatch_pump_at(t0());
    rt.overwatch_enqueue(blocking("second"));
    rt.overwatch_pump_at(t0() + SEC);
    assert_eq!(ow_turns(&rt).len(), 1, "a turn is in flight");
    assert_eq!(rt.overwatch_status().events, 2);
    rt.overwatch_turn_ended(t0() + 5 * SEC);
    let turns = ow_turns(&rt);
    assert_eq!(turns.len(), 2);
    assert!(turns[1].starts_with("ORCH: Overwatch · 1 event"), "no briefing on a live session: {}", turns[1]);
    assert!(turns[1].contains("[ev-2] stuck a1 · second"));
    // The turn was recorded with its wake data.
    let r = rt.overwatch_report();
    let t = &r["turns"][0];
    assert_eq!((t["events"].as_u64(), t["how"].as_str()), (Some(1), Some("spawned")));
    assert_eq!(t["queued_at_ms"].as_u64(), Some(t0()));
}

#[test]
fn event_ids_never_repeat_within_the_day_across_a_restart() {
    let (d, rt) = manual();
    for _ in 0..3 {
        rt.overwatch_enqueue(batched("x"));
    }
    drop(rt);
    let rt = Runtime::new(crate::runtime::tests::test_cfg(d.path(), true)).unwrap();
    rt.overwatch.manual.store(true, Ordering::SeqCst);
    at(&rt, t0() + 3600 * SEC);
    assert_eq!(rt.overwatch_enqueue(batched("y")).as_deref(), Some("ev-4"));
    // A new day starts again.
    at(&rt, t0() + 24 * 3600 * SEC);
    assert_eq!(rt.overwatch_enqueue(batched("z")).as_deref(), Some("ev-1"));
}

#[test]
fn the_day_rolls_over_at_the_next_wake_never_mid_turn() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(blocking("day one"));
    rt.overwatch_pump_at(t0());
    let first = rt.store.read(|s| s.agents[ID].session_id.clone());
    // Past midnight with the turn still running: nothing moves.
    let next_day = t0() + 15 * 3600 * SEC;
    at(&rt, next_day);
    rt.overwatch_enqueue(blocking("day two"));
    rt.overwatch_pump_at(next_day);
    assert_eq!(rt.store.read(|s| s.agents[ID].session_id.clone()), first);
    assert_eq!(ow_turns(&rt).len(), 1);
    // The turn ends: the next wake starts the new day's session.
    rt.overwatch_turn_ended(next_day + SEC);
    let rec = rt.store.read(|s| s.agents[ID].clone());
    assert_ne!(rec.session_id, first);
    assert_eq!(rec.worktree, rt.overwatch_day_dir("2026-11-06"));
    assert!(rec.worktree.join("briefing.md").is_file());
    let turns = ow_turns(&rt);
    assert_eq!(turns.len(), 1, "the old session's undelivered turns went with it");
    assert!(turns[0].starts_with("ORCH: Read briefing.md") && turns[0].contains("day two"), "{}", turns[0]);
    assert_eq!(rec.allocation, ALLOCATION, "usage stays on the same id and allocation");
}

#[test]
fn an_idle_session_is_suspended_after_the_grace() {
    let (d, rt) = manual();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    crate::runtime::tests::insert_test_live(&rt, ID, None, 0, d.path());
    // The test terminal is a fake (no real pid): it is alive while live.
    let alive = || rt.live(ID).is_some();
    rt.store.write(|s| s.agents.get_mut(ID).unwrap().tracker.state = crate::state::AgentState::Idle).unwrap();
    rt.store.write(|s| s.turns.retain(|t| t.agent_id != ID)).unwrap();
    rt.overwatch_turn_ended(t0() + 10 * SEC);
    rt.overwatch_pump_at(t0() + 129 * SEC);
    assert!(alive(), "within the grace");
    rt.overwatch_pump_at(t0() + 131 * SEC);
    let end = Instant::now() + Duration::from_secs(10);
    while alive() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(), "suspended after idle_secs");
    assert_eq!(rt.store.read(|s| s.agents[ID].tracker.stop_intent), crate::state::StopIntent::Suspend);
}

#[test]
fn down_drops_events_keeps_user_messages_and_restart_delivers_them() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    rt.overwatch_down(why::STOPPED);
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref()), (Status::Down, Some(why::STOPPED)));
    assert!(!rt.overwatch_may_decide());
    assert_eq!(rt.overwatch_enqueue(batched("dropped")), None);
    assert_eq!(rt.overwatch_user_message("c-9", "why is checkout slow?"), None);
    assert_eq!(rt.overwatch_pending().len(), 1, "kept for the restart");
    rt.overwatch_restart().unwrap();
    assert_eq!(rt.overwatch_status().status, Status::Working);
    let last = ow_turns(&rt).pop().unwrap();
    assert!(last.starts_with("ORCH: Read briefing.md"), "{last}");
    assert!(last.contains("user_message · why is checkout slow?"), "{last}");
    assert!(!last.contains("dropped"));
    let brief = std::fs::read_to_string(rt.overwatch_day_dir("2026-11-05").join("briefing.md")).unwrap();
    assert!(brief.contains("sent earlier (chat:c-9): why is checkout slow?"), "{brief}");
    assert!(rt.overwatch_pending().is_empty());
}

#[test]
fn two_turns_past_the_ceiling_put_it_down() {
    let (_d, rt) = manual();
    let ceiling = u64::from(TURN_CEILING_MIN) * 60 * SEC;
    let mut t = t0();
    for i in 0..2 {
        rt.overwatch_enqueue(blocking(&format!("slow {i}")));
        rt.overwatch_pump_at(t);
        t += ceiling + SEC;
        rt.overwatch_turn_ended(t);
        if i == 0 {
            assert_eq!(rt.overwatch_status().status, Status::Asleep, "one slow turn is not down");
        }
    }
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref()), (Status::Down, Some(why::TOO_SLOW)));
}

#[test]
fn pause_all_holds_it_and_lifting_it_delivers() {
    let (_d, rt) = manual();
    rt.pause_all();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref()), (Status::Paused, Some(why::PAUSE_ALL)));
    assert!(ow_turns(&rt).is_empty());
    rt.resume_all();
    rt.overwatch_pump_at(t0() + SEC);
    assert_eq!(ow_turns(&rt).len(), 1);
}

#[test]
fn off_means_no_session_and_no_events() {
    let (_d, rt) = manual();
    rt.db.set_setting(HOLD_SECS_KEY, &json!(300)).unwrap();
    let mut s = rt.overwatch_settings();
    s.enabled = false;
    rt.set_overwatch_settings(s).unwrap();
    assert_eq!(rt.overwatch_status().status, Status::Off);
    assert_eq!(rt.overwatch_enqueue(blocking("x")), None);
    rt.overwatch_pump_at(t0() + 600 * SEC);
    assert!(rt.store.read(|s| !s.agents.contains_key(ID)));
}

#[test]
fn settings_refuse_a_window_as_long_as_the_hold() {
    let (_d, rt) = manual();
    rt.db.set_setting(HOLD_SECS_KEY, &json!(300)).unwrap();
    let mut s = rt.overwatch_settings();
    assert_eq!(s, Settings::default());
    s.permission_secs = 300;
    assert!(rt.set_overwatch_settings(s.clone()).unwrap_err().to_string().contains("permission_secs"));
    let patched = rt.merge_overwatch_settings(&json!({"batch_secs": 10})).unwrap();
    assert_eq!((patched.batch_secs, patched.permission_secs), (10, 60));
    assert_eq!(rt.set_overwatch_settings(patched).unwrap().batch_secs, 10);
    assert_eq!(rt.overwatch_settings().batch_secs, 10);
}

// ------------------------------------------------------------ deciding (real holds)

/// A held PermissionRequest of `a1` with effect `overwatch` on its own
/// thread (hold 5 s, permission_secs 1 s); returns the item id and the hook's answer.
pub(super) fn held(rt: &Arc<Runtime>, cmd: &str) -> (u64, std::thread::JoinHandle<Option<HookOutput>>) {
    rt.db.set_setting(HOLD_SECS_KEY, &json!(5)).unwrap();
    let mut s = rt.overwatch_settings();
    s.permission_secs = 1;
    rt.set_overwatch_settings(s).unwrap();
    let before = rt.store.read(|s| s.inbox.len());
    let me = rt.clone();
    let cmd = cmd.to_string();
    let h = std::thread::spawn(move || {
        let rec = me.store.read(|s| s.agents["a1"].clone());
        let p = HookPayload::parse(&json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
            "tool_input": {"command": cmd}, "tool_use_id": "tu-1"}));
        let subject = me.subject(&rec);
        let ev = Evaluation { effect: Effect::Overwatch, source: Source::Default, rule: None };
        let hold = Hold { hold: Duration::from_secs(5), ceiling: Duration::from_secs(20) };
        me.permission_request_with(&rec, &Call::from_hook(&p), &subject, &ev, hold, None)
    });
    let end = Instant::now() + Duration::from_secs(5);
    while rt.store.read(|s| s.inbox.len()) == before && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    (rt.store.read(|s| s.inbox.last().unwrap().id), h)
}

pub(super) fn payload(rt: &Runtime, id: u64) -> Value {
    rt.store.read(|s| s.inbox.iter().find(|i| i.id == id).unwrap().payload.clone())
}

pub(super) fn card(rt: &Runtime, id: u64) -> crate::decision::Decision {
    rt.now_pump(true);
    let want = crate::decision::decision_id("permission", id);
    rt.open_decisions().into_iter().find(|d| d.id == want).expect("open card")
}

pub(super) fn allowed(out: Option<HookOutput>) -> bool {
    out.is_some_and(|o| o.0.to_string().contains("\"allow\""))
}

#[test]
fn a_deciding_permission_waits_on_the_overwatch_then_the_user() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (id, h) = held(&rt, "pnpm add zod");
    let p = payload(&rt, id);
    let until = p["deciding_until_ms"].as_u64().expect("deciding");
    // The card is posted at once, waits on the Overwatch: not in the badge.
    let d = card(&rt, id);
    assert_eq!((d.waits_on, d.deciding_until_ms), ("overwatch", Some(until)));
    assert!(!d.counts());
    assert_eq!(rt.overwatch_deciding(id), Ok(until));
    // Its event wakes the Overwatch, with the deadline.
    let q = rt.overwatch.sched.lock().unwrap().queue.clone();
    let ev = q.iter().find(|e| e.kind == Kind::Permission).expect("permission event");
    assert_eq!((ev.decision.clone(), ev.deadline_ms), (Some(format!("inbox-{id}")), Some(until)));
    // The agent is not "needs you" while it decides.
    let att = rt.attention_of("a1").unwrap();
    assert_ne!(att.level, crate::attention::Level::NeedsYou, "{att:?}");
    // 1 s later: the user's.
    std::thread::sleep(Duration::from_millis(1_300));
    let p = payload(&rt, id);
    assert!(p.get("deciding_until_ms").is_none(), "{p}");
    assert_eq!(p["asks_because"]["kind"], "overwatch_unsure");
    let d = card(&rt, id);
    assert_eq!((d.waits_on, d.deciding_until_ms), ("user", None));
    assert!(d.counts());
    assert_eq!(rt.overwatch_deciding(id), Err(deciding::NotDeciding::TooLate));
    assert_eq!(rt.attention_of("a1").unwrap().level, crate::attention::Level::NeedsYou);
    // The hold restarted at hand-over: the user still decides in time.
    rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert!(allowed(h.join().unwrap()));
}

#[test]
fn going_down_hands_every_deciding_card_over_at_once() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (id, h) = held(&rt, "pnpm add zod");
    assert!(payload(&rt, id)["deciding_until_ms"].is_u64());
    rt.overwatch_down(why::STOPPED);
    assert!(payload(&rt, id).get("deciding_until_ms").is_none());
    assert_eq!(card(&rt, id).waits_on, "user");
    rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert!(allowed(h.join().unwrap()));
    // While down, a new one goes straight to the user.
    let (id2, h2) = held(&rt, "pnpm add zx");
    assert!(payload(&rt, id2)["deciding_until_ms"].is_null());
    assert_eq!(card(&rt, id2).waits_on, "user");
    rt.decide(id2, InboxDecision::Reject, None, Some("no")).unwrap();
    assert!(!allowed(h2.join().unwrap()));
}

/// The card as `Now` serves it (the published snapshot, no explicit pump).
fn served(rt: &Runtime, id: u64) -> Value {
    let want = crate::decision::decision_id("permission", id);
    let snap = rt.now_snapshot();
    snap["decisions"].as_array().unwrap().iter().find(|d| d["id"] == want.as_str()).cloned().unwrap()
}

#[test]
fn its_process_exiting_unasked_hands_every_card_over_and_now_shows_it() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (id, h) = held(&rt, "pnpm add zod");
    rt.overwatch_pump();
    assert_eq!(served(&rt, id)["waits_on"], "overwatch");
    // Asked for (suspend, kill): nothing changes.
    rt.overwatch_exited(true, false);
    assert_eq!(served(&rt, id)["waits_on"], "overwatch");
    // Killed from outside mid-turn (on_exit's path): down, and the very next
    // `Now` already has the card with the user.
    rt.overwatch_exited(false, false);
    assert_eq!(rt.overwatch_status().why.as_deref(), Some(why::STOPPED));
    let card = served(&rt, id);
    assert_eq!(card["waits_on"], "user", "{card}");
    assert!(card.get("deciding_until_ms").is_none(), "{card}");
    assert_eq!(card["payload"]["asks_because"]["kind"], "overwatch_unsure");
    rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert!(allowed(h.join().unwrap()));
}

#[test]
fn the_turn_that_carried_a_permission_hands_it_over_when_it_ends_undecided() {
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    let (id, h) = held(&rt, "pnpm add zod");
    rt.overwatch_pump();
    assert!(ow_turns(&rt).last().unwrap().contains(&format!("permission inbox-{id} a1")));
    rt.overwatch_turn_ended(orch_protocol::now_ms());
    assert_eq!(card(&rt, id).waits_on, "user", "ended without a decision");
    rt.decide(id, InboxDecision::Approve, None, None).unwrap();
    assert!(allowed(h.join().unwrap()));
}

#[test]
fn closed_decisions_and_questions_are_events() {
    let (_d, rt) = manual();
    let q = rt.add_item(Some("a1"), "question", "a1: which db?".into(), json!({"question": "which db?"}), 0).unwrap();
    rt.decide(q, InboxDecision::Reply, None, Some("sqlite")).unwrap();
    let alert = rt.add_item(Some("a1"), "alert", "a1: boom".into(), json!({}), 0).unwrap();
    let _ = alert;
    let kinds: Vec<(Kind, Option<String>)> =
        rt.overwatch.sched.lock().unwrap().queue.iter().map(|e| (e.kind, e.decision.clone())).collect();
    let qid = Some(format!("q-{q}"));
    assert_eq!(kinds, [(Kind::Question, qid.clone()), (Kind::DecisionClosed, qid)], "no event for an alert");
}

#[test]
fn the_report_lists_turns_and_wake_percentiles() {
    assert_eq!(percentiles(&[]), None);
    assert_eq!(percentiles(&[5, 1, 3, 2, 4]), Some((3, 5)));
    let r = json!({
        "status": {"status": "asleep", "events": 0},
        "day": "2026-11-05",
        "session": "s-1",
        "turns": [{"events": 3, "queued_at_ms": t0(), "started_at_ms": t0() + 2_000, "first_tool_ms": t0() + 6_200,
                   "ended_at_ms": t0() + 43_000, "how": "resumed", "cache_warm": true}],
        "wake_p50_ms": 6200, "wake_p90_ms": 6200,
    });
    let text = render_report(&r, &TimeZone::UTC);
    assert!(text.contains("10:00:02 resumed warm events=3 wake=6.2s first_tool=4.2s 41.0s"), "{text}");
    assert!(text.contains("wake p50 6.2s · p90 6.2s"), "{text}");
}

#[test]
fn now_carries_the_status_and_hides_the_overwatch_from_the_lanes() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    let snap = rt.now_snapshot();
    assert_eq!(snap["overwatch"]["status"], "working");
    assert_eq!(snap["overwatch"]["agent"], ID);
    assert!(snap["agents"].as_array().unwrap().iter().all(|a| a["id"] != ID), "not in Mission control's lanes");
}

#[test]
fn a_full_live_cap_or_memory_pressure_never_holds_it() {
    use crate::state::TurnOrigin;
    use governor::{TurnKind, Verdict};
    let (_d, rt) = test_runtime(false);
    rt.overwatch.manual.store(true, Ordering::SeqCst);
    at(&rt, t0());
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    let ow = rt.store.read(|s| s.agents[ID].clone());
    let a1 = rt.store.read(|s| s.agents["a1"].clone());
    // Every process slot taken (live cap reached, or lowered by memory pressure).
    rt.governor_act(|g, now| ((), g.set_process_slots(4, 1, now)));
    let normal = rt.decide_turn(&a1, TurnKind::Message, TurnOrigin::App, true);
    assert!(matches!(normal.effective, Verdict::Queue { .. }), "a member waits for a slot: {:?}", normal.effective);
    // The Overwatch has its own slot (as a lead's): it is never asked for one.
    let reserved = crate::runtime::reserved_slot(&ow);
    let d = rt.decide_turn(&ow, TurnKind::Message, TurnOrigin::App, !reserved);
    assert!(matches!(d.effective, Verdict::Admit), "{:?}", d.effective);
}

#[test]
fn a_long_batch_goes_to_messages_which_it_may_read() {
    let (_d, rt) = manual();
    for i in 0..60 {
        rt.overwatch_enqueue(blocking(&format!("agent {i} idle with its task for 12 min, last tool an hour ago")));
    }
    rt.overwatch_pump_at(t0());
    let turn = rt.store.read(|s| s.turns.iter().find(|t| t.agent_id == ID).cloned().unwrap());
    assert!(turn.body.chars().count() > crate::delivery::MAX_PASTE_CHARS, "{}", turn.body.len());
    // The delivery writes it to the Overwatch's own messages/ (FR-206)…
    let agent_dir = rt.cfg.paths.agent_dir(ID);
    let p = crate::delivery::prepare_many(&[(turn.id.as_str(), turn.body.as_str())], &agent_dir);
    let (path, body) = p.file.expect("too long to paste");
    assert!(path.starts_with(agent_dir.join("messages")), "{path:?}");
    assert!(body.contains("[ev-60] stuck a1"));
    assert!(p.text.contains("Read ") && p.text.contains(&path.display().to_string()));
    // …which its read allow-list covers, with its day folder.
    let day = rt.overwatch_day_dir("2026-11-05");
    let reads = rt.overwatch_reads(&day);
    assert!(reads.iter().any(|r| path.starts_with(r)), "{reads:?}");
    assert!(reads.contains(&day));
    assert!(
        !reads.iter().any(|r| r == &rt.cfg.paths.root || r == &rt.cfg.paths.agents_dir()),
        "not the rest of the app"
    );
}

#[test]
fn turning_it_off_and_on_again_clears_down_like_a_restart() {
    let (_d, rt) = manual();
    rt.overwatch_enqueue(blocking("x"));
    rt.overwatch_pump_at(t0());
    rt.overwatch_down(why::STOPPED);
    assert_eq!(rt.overwatch_status().status, Status::Down);
    let mut s = rt.overwatch_settings();
    s.enabled = false;
    rt.set_overwatch_settings(s.clone()).unwrap();
    s.enabled = true;
    rt.set_overwatch_settings(s).unwrap();
    let st = rt.overwatch_status();
    assert_eq!((st.status, st.why), (Status::Asleep, None));
    // It takes events again, in a fresh session.
    assert!(rt.overwatch_enqueue(blocking("again")).is_some());
}
