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
