#![allow(dead_code)]

use governor::jiff::civil::time;
use governor::jiff::{SignedDuration, Timestamp};
use governor::*;

/// Monday 2026-10-19 09:00 Europe/Paris, the weekly reset.
pub const WEEK_START: &str = "2026-10-19T07:00:00Z";
/// The next Monday 09:00 Paris (UTC+1 after the DST change).
pub const RESET: &str = "2026-10-26T08:00:00Z";

pub fn start() -> Timestamp {
    WEEK_START.parse().unwrap()
}

pub fn reset() -> Timestamp {
    RESET.parse().unwrap()
}

/// `start + minutes`.
pub fn at(minutes: i64) -> Timestamp {
    start().checked_add(SignedDuration::from_mins(minutes)).unwrap()
}

pub fn hours(h: f64) -> i64 {
    (h * 60.0).round() as i64
}

pub fn alloc(id: &str, weight: f64, tier: Tier) -> BudgetAllocation {
    BudgetAllocation { id: id.into(), target: id.into(), weight, tier, hard_cap_pct: None }
}

/// Mon–Fri 09–19 Paris (50 weighted hours), reserve 10 %, A high / B normal / C low.
pub fn plan() -> WeeklyPlan {
    WeeklyPlan {
        id: "test".into(),
        week_start: start(),
        reset_at: reset(),
        calendar: WorkingCalendar::weekdays("Europe/Paris", time(9, 0, 0, 0), time(19, 0, 0, 0)),
        reserve_pct: 10.0,
        off_hours_allowance_pct: 0.0,
        shadow_mode: false,
        allocations: vec![alloc("A", 50.0, Tier::High), alloc("B", 30.0, Tier::Normal), alloc("C", 20.0, Tier::Low)],
    }
}

/// k = 10 cost units per 1 % of the week.
pub fn policy() -> PacingPolicy {
    PacingPolicy { initial_cost_per_pct: 10.0, ..PacingPolicy::default() }
}

pub fn governor() -> Governor {
    Governor::new(plan(), policy(), start()).unwrap()
}

pub fn task(agent: &str, allocation: &str, tier: Tier) -> TurnRequest {
    TurnRequest::task(agent, allocation, tier)
}

pub fn tool(agent: &str) -> ToolCallRequest {
    ToolCallRequest { agent: agent.into(), interactive: false }
}

pub fn weekly(ts: Timestamp, agent: &str, cost: f64, pct: f64) -> UsageSample {
    UsageSample {
        ts,
        agent: Some(agent.into()),
        cost_delta: cost,
        five_hour: None,
        weekly: Some(WindowReading { used_pct: pct, resets_at: reset() }),
        provenance: Provenance::Observed,
    }
}

pub fn five(ts: Timestamp, agent: &str, pct: f64, resets_at: Timestamp) -> UsageSample {
    UsageSample {
        ts,
        agent: Some(agent.into()),
        cost_delta: 0.0,
        five_hour: Some(WindowReading { used_pct: pct, resets_at }),
        weekly: None,
        provenance: Provenance::Observed,
    }
}

pub fn queue_reason(d: &Decision) -> Option<QueueReason> {
    match &d.effective {
        Verdict::Queue { reason, .. } => Some(reason.clone()),
        _ => None,
    }
}

pub fn count(g: &Governor, kind: &str) -> usize {
    g.actions().iter().filter(|a| a.kind_name() == kind).count()
}

/// Push W to 30 % at minute 40 with usage outside any admitted turn (the
/// user's own, k = 10) and tick past the dwell: it is not the agents' share,
/// so the state turns Red through the projection (the week would run out), not
/// the error.
pub fn drive_red(g: &mut Governor, minute: i64) {
    g.ingest(&weekly(at(minute), "obs", 300.0, 30.0));
    g.tick(at(minute));
    assert_eq!(g.state(), PressureState::Red, "{:?}", g.snapshot(at(minute)));
}

/// The restarted Governor matches the live one (ADR-007 "replay"): same
/// measurements, forecast, calibration and state. The burn is measured between
/// ticks, which a replay does not have, so the projection may differ a little.
pub fn same_state(live: &Governor, restarted: &Governor, now: Timestamp) {
    let (a, b) = (live.snapshot(now), restarted.snapshot(now));
    let same = |x: f64, y: f64| (x - y).abs() < 1e-9;
    assert!(same(a.used, b.used) && same(a.margin, b.margin), "{a:?}\n{b:?}");
    assert!(same(a.ideal, b.ideal) && same(a.plan, b.plan) && same(a.governed, b.governed), "{a:?}\n{b:?}");
    assert!(same(a.unattributed_so_far, b.unattributed_so_far), "{a:?}\n{b:?}");
    assert!(same(a.unattributed_forecast, b.unattributed_forecast), "{a:?}\n{b:?}");
    assert!(same(a.reserve_left, b.reserve_left), "{a:?}\n{b:?}");
    assert_eq!(a.calibration, b.calibration);
    assert!((a.projected - b.projected).abs() < 2.0, "{} vs {}", a.projected, b.projected);
    assert_eq!(live.state(), restarted.state());
}
