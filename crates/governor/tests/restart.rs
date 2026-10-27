//! The live path and the restart path give the same Governor for the same
//! samples (ADR-007 "replay", FR-802 realignment, FR-813 outside usage).

mod common;

use common::*;
use governor::*;

/// The plan the app guessed before its first reading: 5 h off the real reset.
fn misaligned() -> WeeklyPlan {
    let reset_at = reset().checked_add(jiff::SignedDuration::from_hours(5)).unwrap();
    WeeklyPlan { id: "guess".into(), week_start: at(hours(5.0)), reset_at, ..plan() }
}

/// A heavy first day: 17 % used outside the app by 18:30 (the app starts
/// then), an agent turn, more outside use in the evening.
fn day_one() -> Vec<(i64, Option<TurnRequest>, UsageSample)> {
    vec![
        (hours(9.5), None, weekly(at(hours(9.5)), "A-1", 0.0, 17.0)),
        (hours(9.6), Some(task("A-1", "A", Tier::High)), weekly(at(hours(9.75)), "A-1", 15.0, 18.0)),
        (hours(12.0), None, weekly(at(hours(12.0)), "user", 0.0, 19.0)),
        (hours(12.5), None, weekly(at(hours(12.5)), "user", 0.0, 21.0)),
    ]
}

#[test]
fn restart_on_a_heavy_first_day_matches_the_live_realignment() {
    let first = at(hours(9.5));
    let now = at(hours(13.0));

    // Live: started at 18:30 on a guessed week, realigned by the first reading,
    // ticked every minute. Each sample is stored with its class, as the core does.
    let mut live = Governor::new(misaligned(), policy(), first).unwrap();
    let mut stored = Vec::new();
    let samples = day_one();
    let mut next = 0;
    for m in hours(9.5)..=hours(13.0) {
        while next < samples.len() && samples[next].0 <= m {
            let (_, turn, s) = &samples[next];
            if let Some(t) = turn {
                assert!(live.request_turn(t.clone(), at(m)).effective.is_admit());
            }
            if live.plan().reset_at != reset() {
                live.new_week(plan(), s.ts).unwrap();
            }
            stored.push((s.clone(), live.cost_class(s), turn.as_ref().map(|t| t.allocation.clone())));
            live.ingest(s);
            if turn.is_some() {
                live.turn_ended(&"A-1".into(), s.ts);
            }
            next += 1;
        }
        live.tick(at(m));
    }

    // Restart: the stored plan, created when the app first saw the week (or,
    // as before, at the week start: the anchor is W = 0 at the reset either
    // way), the samples replayed, one tick.
    let restart = |created: jiff::Timestamp| {
        let mut g = Governor::new(plan(), policy(), created).unwrap();
        for (s, class, alloc) in &stored {
            g.replay(s, *class, alloc.clone());
        }
        g.tick(now);
        g
    };
    for restarted in [restart(first), restart(start())] {
        same_state(&live, &restarted, now);
    }

    // One heavy day is not extrapolated over the week: the agents keep a
    // budget and the week is not projected over the limit.
    let a = live.snapshot(now);
    assert_eq!(live.state(), PressureState::Green, "{a:?}");
    assert!(a.unattributed_forecast < 70.0, "{a:?}");
    assert!(a.projected < 98.0, "{a:?}");
    assert!(live.snapshot(reset()).ideal > 15.0, "agents keep a budget for the week");
}

#[test]
fn realigning_the_same_week_keeps_its_usage() {
    // Cost ingested before the first weekly reading, then a realignment onto an
    // earlier reset: the cost stays the week's, as a replay would count it.
    let mut g = Governor::new(misaligned(), policy(), at(hours(9.5))).unwrap();
    assert!(g.request_turn(task("A-1", "A", Tier::High), at(hours(9.5))).effective.is_admit());
    g.ingest(&UsageSample::cost(at(hours(9.6)), "A-1".into(), 20.0));
    g.new_week(plan(), at(hours(9.7))).unwrap();
    assert!((g.allocation_usage_pct(&"A".into()) - 2.0).abs() < 1e-9);
    let mut replayed = Governor::new(plan(), policy(), at(hours(9.5))).unwrap();
    replayed.replay(&UsageSample::cost(at(hours(9.6)), "A-1".into(), 20.0), CostClass::Autonomous, Some("A".into()));
    let s = weekly(at(hours(9.8)), "user", 0.0, 17.0);
    g.ingest(&s);
    replayed.replay(&s, CostClass::Interactive, None);
    same_state(&g, &replayed, at(hours(9.8)));
}

#[test]
fn overrides_and_the_queue_survive_a_restart() {
    let mut live = governor();
    live.request_turn(task("C-1", "C", Tier::Low), at(0));
    drive_red(&mut live, 40); // asks to defer C
    live.start_sprint(3.0, at(41)).unwrap();
    live.pause_agent(&"B-1".into(), at(42));
    for a in ["A-1", "A-2"] {
        live.request_turn(task(a, "A", Tier::High), at(43));
    }
    live.request_turn(task("A-3", "A", Tier::High), at(44)); // over Red's cap
    live.rate_limit_hit(None, Window::FiveHour, at(200), at(45));
    let carry = live.carry();
    // Through the store and back.
    let carry: Carry = serde_json::from_str(&serde_json::to_string(&carry).unwrap()).unwrap();
    assert_eq!(carry.queue.len(), live.queue().len());
    assert!(carry.queue.len() >= 2, "{:?}", carry.queue);

    let now = at(46);
    let mut restarted = Governor::new(plan(), policy(), start()).unwrap();
    restarted.ingest(&weekly(at(40), "obs", 300.0, 30.0));
    let released = restarted.restore(carry, |e| e.agent != AgentId::from("A-2"), now);
    assert!(released.is_empty(), "the 5h limit still holds");
    let ids = |g: &Governor| g.queue().iter().map(|e| (e.id, e.agent.clone(), e.enqueued_at)).collect::<Vec<_>>();
    assert_eq!(ids(&restarted), ids(&live).into_iter().filter(|e| e.1 != AgentId::from("A-2")).collect::<Vec<_>>());
    assert_eq!(restarted.curve().fraction(at(300)), live.curve().fraction(at(300)), "same sprint curve");
    assert!(restarted.paused_agents().contains(&AgentId::from("B-1")));
    let ask = restarted.next_action_preview(now).pending_ask.expect("the ask is back");
    assert_eq!(ask.1, live.next_action_preview(now).pending_ask.unwrap().1, "same deadline");
    // New entries never reuse a restored id.
    restarted.request_turn(task("B-2", "B", Tier::Normal), now);
    let mut seen: Vec<_> = restarted.queue().iter().map(|e| e.id).collect();
    seen.dedup();
    assert_eq!(seen.len(), restarted.queue().len());
    // Restored actions can be undone.
    let sprint = restarted.sprint().expect("sprint restored").0;
    restarted.undo(sprint, now).unwrap();
    assert_eq!(
        restarted.curve().fraction(at(300)),
        Governor::new(plan(), policy(), start()).unwrap().curve().fraction(at(300))
    );
}

#[test]
fn week_bound_overrides_do_not_cross_into_the_next_week() {
    let mut live = governor();
    live.start_sprint(3.0, at(41)).unwrap();
    live.pause_all(at(42));
    let carry = live.carry();
    let next = WeeklyPlan {
        id: "next".into(),
        week_start: reset(),
        reset_at: reset().checked_add(jiff::SignedDuration::from_hours(168)).unwrap(),
        ..plan()
    };
    let mut g = Governor::new(next, policy(), reset()).unwrap();
    g.restore(carry, |_| true, reset());
    assert!(g.sprint().is_none());
    assert!(g.is_paused_all(), "a pause is the user's, whatever the week");
}

#[test]
fn a_rebuild_keeps_the_running_turns_against_the_caps() {
    let mut live = governor();
    for a in ["A-1", "A-2", "B-1"] {
        live.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    for a in ["C-1", "C-2"] {
        live.request_turn(task(a, "C", Tier::Low), at(1));
    }
    live.ingest(&UsageSample::cost(at(2), "A-1".into(), 7.0));
    let carry = live.carry();
    assert_eq!(carry.live.len(), 3);

    // A replan: the rebuilt Governor knows the 3 running turns, so nothing is released.
    let mut g = governor();
    let released = g.restore(carry.clone(), |_| true, at(3));
    assert!(released.is_empty(), "{released:?}");
    assert_eq!(g.live_autonomous(), 3);
    assert_eq!(g.queue().len(), 2);
    assert_eq!(g.tool_call(&tool("A-1"), at(4)).effective, Verdict::Admit, "still an admitted turn");
    // Its cost so far came along (the watchdog's usage ceiling).
    assert_eq!(g.carry().live.iter().find(|r| r.agent == AgentId::from("A-1")).unwrap().turn.cost(), 7.0);
    // When one ends, a queued turn takes its slot.
    assert_eq!(g.turn_ended(&"A-1".into(), at(5)).len(), 1);

    // After a restart the host drops them (their processes are gone): the queue runs.
    let mut g = governor();
    let released = g.restore(governor::Carry { live: vec![], ..carry }, |_| true, at(3));
    assert_eq!(released.len(), 2);
}

#[test]
fn a_spawn_without_a_slot_goes_back_to_the_queue() {
    let mut g = Governor::new(WeeklyPlan { shadow_mode: true, ..plan() }, policy(), start()).unwrap();
    g.set_process_slots(0, 1, at(0));
    let mut r = task("C-1", "C", Tier::Low);
    r.needs_spawn = true;
    assert_eq!(g.request_turn(r, at(1)).effective, Verdict::Admit);
    // Another process started before the host could spawn C-1.
    let e = g.hold_spawn(&"C-1".into(), &"C".into(), Tier::Low, 1, 1, at(2));
    assert!(!g.is_live(&"C-1".into()));
    let q = g.queue().iter().find(|q| q.id == e).unwrap();
    assert_eq!(q.reason, QueueReason::MemoryPressure { cap: 1 });
    assert!(q.needs_spawn);
    assert!(g.set_process_slots(1, 1, at(3)).is_empty());
    assert_eq!(g.set_process_slots(0, 1, at(4)).len(), 1, "released when a slot frees, in shadow mode too");
}
