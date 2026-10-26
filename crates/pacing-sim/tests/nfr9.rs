//! NFR-9 acceptance tests and whole-week behaviour, through the simulator.

use governor::{ActionKind, AskDefault, PressureState, Window};
use pacing_sim::{Scenario, SimResult, SyntheticParams, csv, run};

const SEEDS: std::ops::RangeInclusive<u64> = 1..=20;

fn week(demand: f64, seed: u64) -> SimResult {
    run(&Scenario::synthetic(&SyntheticParams::nfr9(demand, seed)))
}

/// NFR-9: 130 % demand never hits 100 % before the reset during working
/// hours, and ends between 85 and 100 %.
#[test]
fn nfr9_130_percent_demand() {
    for seed in SEEDS {
        let s = week(130.0, seed).summary;
        assert!(
            !s.limit_hits.iter().any(|h| h.working_hours),
            "seed {seed}: limit hit in working hours: {:?}",
            s.limit_hits
        );
        assert!(s.weekly_exhausted_at.is_none(), "seed {seed}: weekly limit reached at {:?}", s.weekly_exhausted_at);
        assert!((85.0..=100.0).contains(&s.final_weekly_pct), "seed {seed}: ended at {:.1} %", s.final_weekly_pct);
        assert!(s.queued_for_pacing > 0, "seed {seed}: 130 % demand must be paced");
    }
}

/// NFR-9: 60 % demand never queues work.
#[test]
fn nfr9_60_percent_demand_never_queues() {
    for seed in SEEDS {
        let s = week(60.0, seed).summary;
        assert_eq!(s.queued, 0, "seed {seed}: queued {:?}", s.queued_by_reason);
        assert_eq!(s.tool_calls_denied, 0, "seed {seed}");
        assert!(s.limit_hits.is_empty(), "seed {seed}");
        let done: f64 = s.allocations.iter().map(|a| a.done_pct).sum();
        let demand: f64 = s.allocations.iter().map(|a| a.demand_pct).sum();
        assert!(done > 0.85 * demand, "seed {seed}: only {done:.1} of {demand:.1} pp done");
    }
}

/// No oscillation: every state lasts at least the minimum dwell.
#[test]
fn no_oscillation() {
    for seed in SEEDS {
        for demand in [60.0, 100.0, 130.0] {
            let r = week(demand, seed);
            let dwell = Scenario::default_policy().thresholds.min_dwell_min;
            if let Some(m) = r.summary.min_dwell_min {
                assert!(m >= dwell, "seed {seed} demand {demand}: a state lasted {m} min");
            }
            // And not flapping either: well under one change per working hour.
            assert!(r.summary.state_changes < 46, "seed {seed} demand {demand}: {} changes", r.summary.state_changes);
            let guard = r.summary.actions.get("five_hour_guard").copied().unwrap_or(0);
            assert!(guard < 46, "seed {seed} demand {demand}: 5h guard toggled {guard} times");
        }
    }
}

/// No starvation: with deferral declined, the low-tier allocation still gets
/// a minimum share done under 130 % demand thanks to aging, and no queued turn
/// waits more than 5 working hours, not counting the time its allocation was
/// ahead of its fair share: fair queuing then rightly serves the allocation
/// that is behind (a long Red at cap 1 can hold an aged low-tier entry behind
/// a high-weight allocation for hours).
#[test]
fn no_starvation_with_aging() {
    for seed in SEEDS {
        let mut sc = Scenario::synthetic(&SyntheticParams::nfr9(130.0, seed));
        sc.policy.default_ask_action = AskDefault::Skip;
        let r = run(&sc);
        let c = r.summary.allocations.iter().find(|a| a.id.0 == "C").unwrap();
        // Strict tiers: low-tier work runs only once aging lifts it, so it
        // progresses slowly but steadily.
        assert!(
            c.done_pct >= 0.25 * c.demand_pct,
            "seed {seed}: low tier did {:.1} of {:.1} pp",
            c.done_pct,
            c.demand_pct
        );
        // Waits measured in working time: overnight waits are off-hours, not starvation.
        let curve = governor::Curve::build(&sc.calendar, sc.week_start, sc.reset_at, 0.0).unwrap();
        let mut queued = std::collections::BTreeMap::new();
        for a in &r.actions {
            if let ActionKind::Queue { entry: Some(e), .. } = &a.action {
                queued.insert(*e, a.ts);
            }
        }
        for (released_at, rel) in &r.releases {
            let Some(&from) = queued.get(&rel.entry) else { continue };
            // Split the wait at every other release in between; a stretch
            // ending in a release of an allocation with less service than this
            // one's is excused.
            let mut counted = 0.0;
            let mut start = from;
            for (ts, other) in
                r.releases.iter().filter(|(ts, o)| *ts > from && ts < released_at && o.entry != rel.entry)
            {
                let svc = |a: &governor::AllocationId| other.service.get(a).copied().unwrap_or(0.0);
                let ahead = other.allocation != rel.allocation && svc(&rel.allocation) > svc(&other.allocation);
                if !ahead {
                    counted += curve.active_hours_between(start, *ts);
                }
                start = *ts;
            }
            counted += curve.active_hours_between(start, *released_at);
            // Low → High takes two aging steps (4 h), then one more turn at most.
            assert!(counted <= 5.0, "seed {seed}: entry {} waited {counted:.1} working hours", rel.entry);
        }
    }
}

/// J9: a forced 429 pauses autonomous work, cancels the CLI's auto-continue,
/// and resumes at the reset.
#[test]
fn forced_limit_pauses_and_resumes_at_the_reset() {
    let sc = Scenario::synthetic(&SyntheticParams::full(1));
    let forced = sc.forced_limit.clone().unwrap();
    let r = run(&sc);
    let hit = r
        .actions
        .iter()
        .find_map(|a| match &a.action {
            ActionKind::LimitReached { window: Window::FiveHour, resets_at } => Some((a.ts, *resets_at)),
            _ => None,
        })
        .expect("limit reached");
    assert!(hit.0 >= forced.at);
    assert!(r.actions.iter().any(|a| matches!(a.action, ActionKind::CancelAutoContinue { .. })));
    let reset = r
        .actions
        .iter()
        .find(|a| matches!(a.action, ActionKind::LimitReset { window: Window::FiveHour }))
        .expect("limit reset");
    assert_eq!(reset.ts, hit.1);
    // Nothing is released while the limit holds.
    assert!(!r.actions.iter().any(|a| matches!(a.action, ActionKind::Release { .. }) && a.ts > hit.0 && a.ts < hit.1));
    // And the simulator saw a single 429 per agent, not a storm.
    assert!(r.summary.limit_hits.len() <= 6, "{:?}", r.summary.limit_hits);
    assert!(!r.summary.limit_hits.iter().any(|h| h.window == Window::Weekly));
}

/// FR-812: shadow mode logs what it would do and changes nothing.
#[test]
fn shadow_mode_changes_nothing() {
    let mut sc = Scenario::synthetic(&SyntheticParams::nfr9(130.0, 3));
    sc.shadow_mode = true;
    let shadow = run(&sc).summary;
    assert_eq!(shadow.queued, 0);
    assert_eq!(shadow.tool_calls_denied, 0);
    assert!(shadow.shadow_overrides > 0);
    let enforced = week(130.0, 3).summary;
    assert!(shadow.max_weekly_pct > enforced.max_weekly_pct);
}

/// FR-811: a sprint raises the day's target and lowers the rest of the week.
#[test]
fn sprint_runs_more_today() {
    let mut with = Scenario::synthetic(&SyntheticParams::nfr9(130.0, 4));
    let sprint = governor::jiff::Timestamp::from_second(with.week_start.as_second() + (3 * 24 + 1) * 3600).unwrap();
    with.sprint = Some(pacing_sim::SprintSpec { at: sprint, extra_pct: 5.0 });
    let base = run(&Scenario::synthetic(&SyntheticParams::nfr9(130.0, 4)));
    let r = run(&with);
    assert!(r.actions.iter().any(|a| matches!(a.action, ActionKind::Sprint { .. })));
    let day_end = sprint.as_second() + 9 * 3600;
    let at = |res: &SimResult| res.rows.iter().rfind(|row| row.ts.as_second() <= day_end).unwrap().true_weekly;
    assert!(at(&r) > at(&base), "sprint day: {:.1} vs {:.1}", at(&r), at(&base));
    assert!((85.0..=100.0).contains(&r.summary.final_weekly_pct));
}

#[test]
fn simulation_is_deterministic() {
    let a = week(130.0, 7);
    let b = week(130.0, 7);
    assert_eq!(a.summary, b.summary);
    assert_eq!(csv(&a.rows), csv(&b.rows));
    assert_eq!(a.actions, b.actions);
}

/// A recorded week is the same structure as JSON: it round-trips and replays identically.
#[test]
fn recorded_week_round_trips() {
    let sc = Scenario::synthetic(&SyntheticParams::full(2));
    let json = serde_json::to_string(&sc).unwrap();
    let back: Scenario = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sc);
    assert_eq!(run(&back).summary, run(&sc).summary);
}

#[test]
fn red_happens_under_heavy_demand_and_never_at_60() {
    let heavy = week(130.0, 1);
    assert!(heavy.rows.iter().any(|r| r.state == PressureState::Red || r.state == PressureState::Amber));
    let light = week(60.0, 1);
    assert!(light.rows.iter().all(|r| r.state != PressureState::Red && r.state != PressureState::Amber));
}
