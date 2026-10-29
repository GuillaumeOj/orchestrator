//! The Week planner's read-only forecast (FR-802–804, FR-809, FR-813, J8):
//! ideal vs actual vs forecast for the week and the 5h window, allocations,
//! the next-action preview and the governor log. Built from the Governor's
//! state; the "actual" line is the history the app samples every minute,
//! seeded at start from the forecast snapshots of the store (every 5 min), so
//! it covers the plan week across restarts.

use std::collections::VecDeque;

use governor::jiff::Timestamp;
use governor::{Governor, GovernorAction, PressureState, QueueEntry};
use serde::Serialize;

/// One sample of the week, every minute while the app runs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HistoryPoint {
    pub t_ms: i64,
    /// Total weekly % (W).
    pub used: f64,
    /// The agents' governed share (G).
    pub governed: f64,
    pub five_hour: Option<f64>,
}

#[derive(Default)]
pub struct History {
    points: VecDeque<HistoryPoint>,
}

/// A week of minutes.
const HISTORY_CAP: usize = 7 * 24 * 60;

impl History {
    /// Appends a point; drops points from before `week_start_ms` (a new week).
    pub fn push(&mut self, p: HistoryPoint, week_start_ms: i64) {
        while self.points.front().is_some_and(|f| f.t_ms < week_start_ms) {
            self.points.pop_front();
        }
        if self.points.back().is_some_and(|b| b.t_ms >= p.t_ms) {
            return;
        }
        self.points.push_back(p);
        while self.points.len() > HISTORY_CAP {
            self.points.pop_front();
        }
    }

    /// At most `max` points, evenly thinned (the chart is a few hundred px wide).
    pub fn thinned(&self, max: usize) -> Vec<HistoryPoint> {
        let n = self.points.len();
        if n <= max || max < 2 {
            return self.points.iter().copied().collect();
        }
        let step = (n - 1) as f64 / (max - 1) as f64;
        (0..max).map(|i| self.points[((i as f64 * step).round() as usize).min(n - 1)]).collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CurvePoint {
    pub t_ms: i64,
    /// Where W should be: the agents' ideal curve + your usage + outside-app
    /// usage (so far, then its forecast). `snapshot.plan` at now.
    pub plan_total: f64,
    /// W projected at the current burn, to `snapshot.projected` at the reset (t ≥ now only).
    pub forecast_total: Option<f64>,
    /// W if the Governor closes today's error by the reset (an estimate, t ≥ now only).
    pub with_governor_total: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AllocationRow {
    pub id: String,
    pub target: String,
    pub weight: f64,
    pub tier: governor::Tier,
    pub used_pct: f64,
    /// Its share of the agents' plan right now.
    pub plan_pct: f64,
    pub queued: usize,
    pub live: usize,
    pub hard_cap_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    /// Unique in the log (action ids restart at each core start).
    pub key: String,
    pub action: GovernorAction,
    pub can_undo: bool,
    /// From an earlier start of the app this week.
    pub past: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlannerView {
    pub plan_id: String,
    pub week_start_ms: i64,
    pub reset_at_ms: i64,
    pub now_ms: i64,
    pub shadow: bool,
    pub reserve_pct: f64,
    pub snapshot: governor::ForecastSnapshot,
    pub curve: Vec<CurvePoint>,
    pub actual: Vec<HistoryPoint>,
    /// Working blocks for the chart's background: `(start_ms, end_ms)`.
    pub working: Vec<(i64, i64)>,
    pub allocations: Vec<AllocationRow>,
    pub queue: Vec<QueueEntry>,
    pub log: Vec<LogEntry>,
    pub next: governor::NextActionPreview,
    pub pending_ask: Option<GovernorAction>,
    pub sprint: Option<(u64, i64, f64)>,
    pub five_hour_resets_at_ms: Option<i64>,
    /// First start of the app (the shadow week, FR-812, counts from it).
    pub first_run_ms: Option<i64>,
    /// The user confirmed a plan (otherwise the default one is shown).
    pub plan_set: bool,
    /// FR-816: off-hours windows of the week and the work scheduled in them.
    pub off_hours: OffHoursView,
}

#[derive(Clone, Debug, Serialize)]
pub struct OffHoursWindow {
    pub start: Timestamp,
    pub end: Timestamp,
}

#[derive(Clone, Debug, Serialize)]
pub struct OffHoursView {
    /// The plan's off-hours allowance, % of the week (0 = no off-hours work).
    pub allowance_pct: f64,
    /// The off-hours windows of the week; empty without an allowance.
    pub windows: Vec<OffHoursWindow>,
    pub live_cap: u32,
    pub queued: usize,
    pub running: u32,
}

/// [`off_hours_windows`] of the last plan seen: they change only with the
/// plan's week or calendar, and the planner is polled every few seconds
/// under the Governor's lock (672 time-zone conversions a call).
fn cached_off_hours_windows(gov: &Governor) -> Vec<OffHoursWindow> {
    type Key = (String, i64, i64, String);
    static CACHE: std::sync::Mutex<Option<(Key, Vec<OffHoursWindow>)>> = std::sync::Mutex::new(None);
    let p = gov.plan();
    let key: Key = (
        p.id.clone(),
        p.week_start.as_millisecond(),
        p.reset_at.as_millisecond(),
        serde_json::to_string(&p.calendar).unwrap_or_default(),
    );
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, w)) = c.as_ref().filter(|(k, _)| *k == key) {
        return w.clone();
    }
    let w = off_hours_windows(gov);
    *c = Some((key, w.clone()));
    w
}

/// The week's time outside working blocks, at 15-minute resolution.
fn off_hours_windows(gov: &Governor) -> Vec<OffHoursWindow> {
    let plan = gov.plan();
    let (start, end) = (plan.week_start, plan.reset_at);
    let step = governor::jiff::SignedDuration::from_mins(15);
    let mut out = vec![];
    let mut open: Option<Timestamp> = None;
    let mut t = start;
    while t < end {
        match (gov.in_working_time(t), open) {
            (false, None) => open = Some(t),
            (true, Some(s)) => {
                out.push(OffHoursWindow { start: s, end: t });
                open = None;
            }
            _ => {}
        }
        t = t.checked_add(step).unwrap_or(end);
    }
    if let Some(s) = open {
        out.push(OffHoursWindow { start: s, end });
    }
    out
}

fn ms(t: Timestamp) -> i64 {
    t.as_millisecond()
}

/// Samples the week every `step_h` hours (and at `now`).
pub fn build(
    gov: &Governor,
    now: Timestamp,
    history: &History,
    past: &[GovernorAction],
    five_hour_resets_at: Option<Timestamp>,
    live_by_alloc: &dyn Fn(&str) -> usize,
) -> PlannerView {
    let plan = gov.plan();
    let snap = gov.snapshot(now);
    let curve = gov.curve();
    let budget = (100.0 - plan.reserve_pct - snap.unattributed_forecast).max(0.0);
    let (start, end) = (plan.week_start, plan.reset_at);
    let h_now = curve.hours_at(now);
    let h_left_now = curve.hours_left(now);
    let ideal_now = curve.fraction(now) * budget;
    let mut times: Vec<Timestamp> = vec![];
    let mut t = start;
    while t < end {
        times.push(t);
        t = t.checked_add(governor::jiff::SignedDuration::from_hours(1)).unwrap_or(end);
    }
    times.push(end);
    if now > start && now < end {
        times.push(now);
        times.sort();
        times.dedup();
    }
    // You + outside the app now, exactly as the Governor counts it in `plan`
    // (`snapshot.interactive_used` also holds critical work, which is the
    // agents' own and stays in G).
    let other_now = snap.plan - snap.ideal;
    let other_growth = snap.unattributed_forecast - snap.unattributed_so_far;
    let wall_left = curve.wall_hours_left(now).max(1e-9);
    let raw = &history.points;
    let points = times
        .into_iter()
        .map(|t| {
            let ideal = curve.fraction(t) * budget;
            let wall_done = ((wall_left - curve.wall_hours_left(t)) / wall_left).min(1.0);
            let plan_total = if t < now {
                // You + outside the app as the raw samples of this week saw it at `t`.
                let i = raw.partition_point(|p| p.t_ms <= ms(t));
                let other = i
                    .checked_sub(1)
                    .map(|i| raw[i])
                    .filter(|p| p.t_ms >= ms(start))
                    .map_or(0.0, |p| (p.used - p.governed).max(0.0));
                ideal + other
            } else {
                ideal + other_now + other_growth * wall_done
            };
            let (forecast_total, with_governor_total) = if t >= now {
                // Both close linearly with the working hours left (the wall
                // hours once none are left, e.g. after the week's last block).
                let done = if h_left_now > 1e-9 {
                    ((curve.hours_at(t) - h_now) / h_left_now).clamp(0.0, 1.0)
                } else {
                    wall_done.clamp(0.0, 1.0)
                };
                let fc = (snap.used + (snap.projected - snap.used) * done).min(110.0);
                let wg = plan_total + snap.error * (1.0 - done);
                (Some(fc), Some(wg.max(snap.used)))
            } else {
                (None, None)
            };
            CurvePoint { t_ms: ms(t), plan_total, forecast_total, with_governor_total }
        })
        .collect();

    // Working blocks: scan hourly for weight > 0.
    let mut working = vec![];
    let mut t = start;
    let mut open: Option<i64> = None;
    let step = governor::jiff::SignedDuration::from_mins(30);
    while t < end {
        let w = curve.weight_at(t) > 0.0;
        match (w, open) {
            (true, None) => open = Some(ms(t)),
            (false, Some(s)) => {
                working.push((s, ms(t)));
                open = None;
            }
            _ => {}
        }
        t = t.checked_add(step).unwrap_or(end);
    }
    if let Some(s) = open {
        working.push((s, ms(end)));
    }

    let total_weight: f64 = plan.allocations.iter().map(|a| a.weight).sum::<f64>().max(1e-9);
    let queue = gov.queue().to_vec();
    let allocations = plan
        .allocations
        .iter()
        .map(|a| AllocationRow {
            id: a.id.0.clone(),
            target: a.target.clone(),
            weight: a.weight,
            tier: a.tier,
            used_pct: gov.allocation_usage_pct(&a.id),
            plan_pct: ideal_now * a.weight / total_weight,
            queued: queue.iter().filter(|q| q.allocation == a.id).count(),
            live: live_by_alloc(&a.id.0),
            hard_cap_pct: a.hard_cap_pct,
        })
        .collect();
    let log: Vec<LogEntry> = gov
        .actions()
        .iter()
        .rev()
        .take(50)
        .map(|a| LogEntry {
            key: format!("n{}", a.id.0),
            can_undo: a.undo.is_some() && a.undone_at.is_none(),
            action: a.clone(),
            past: false,
        })
        .chain(past.iter().filter(|a| a.ts >= start).take(50).enumerate().map(|(i, a)| LogEntry {
            key: format!("p{i}"),
            can_undo: false,
            action: a.clone(),
            past: true,
        }))
        .collect();
    let next = gov.next_action_preview(now);
    let pending_ask = next.pending_ask.and_then(|(id, _)| gov.action(id).cloned());
    PlannerView {
        plan_id: plan.id.clone(),
        week_start_ms: ms(start),
        reset_at_ms: ms(end),
        now_ms: ms(now),
        shadow: plan.shadow_mode,
        reserve_pct: plan.reserve_pct,
        actual: actual(history, &snap, start, now),
        snapshot: snap,
        curve: points,
        working,
        allocations,
        queue,
        log,
        next,
        pending_ask,
        sprint: gov.sprint().map(|(id, until, pp)| (id.0, ms(until), pp)),
        five_hour_resets_at_ms: five_hour_resets_at.map(ms),
        first_run_ms: None,
        plan_set: false,
        off_hours: OffHoursView {
            allowance_pct: plan.off_hours_allowance_pct,
            windows: if plan.off_hours_allowance_pct > 0.0 { cached_off_hours_windows(gov) } else { vec![] },
            live_cap: gov.policy().off_hours_live_cap,
            queued: gov.queue().iter().filter(|q| gov.off_hours_agents().contains(&q.agent)).count(),
            running: gov.off_hours_running(),
        },
    }
}

/// The week's usage so far: the thinned history of this week (the in-memory
/// history keeps last week's samples until the next push after the reset), a
/// last point at `now` from the snapshot, then a running max on W and G with
/// `G ≤ W` (W only grows in a week; a dip is an estimate corrected, not usage
/// given back).
fn actual(
    history: &History,
    snap: &governor::ForecastSnapshot,
    week_start: Timestamp,
    now: Timestamp,
) -> Vec<HistoryPoint> {
    let mut out = history.thinned(400);
    out.retain(|p| p.t_ms >= ms(week_start) && p.t_ms < ms(now));
    out.push(HistoryPoint { t_ms: ms(now), used: snap.used, governed: snap.governed, five_hour: snap.five_hour });
    let (mut used, mut governed) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &mut out {
        used = used.max(p.used);
        governed = governed.max(p.governed);
        p.used = used;
        p.governed = governed.min(used);
    }
    out
}

/// The title bar's pressure state in words (§9.5 copy).
pub fn state_label(s: PressureState) -> &'static str {
    match s {
        PressureState::Green => "Green",
        PressureState::Amber => "Amber",
        PressureState::Red => "Red",
        PressureState::CatchUp => "Catch-up",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::{PacingPolicy, Provenance, UsageSample, WindowReading};

    fn gov() -> (Governor, Timestamp) {
        let now: Timestamp = "2026-10-21T09:00:00Z".parse().unwrap();
        let mut g =
            Governor::new(agent_runtime::runtime::default_plan(now, true), PacingPolicy::default(), now).unwrap();
        let reset = g.plan().reset_at;
        g.ingest(&UsageSample {
            ts: now,
            agent: None,
            cost_delta: 0.0,
            five_hour: Some(WindowReading { used_pct: 20.0, resets_at: now }),
            weekly: Some(WindowReading { used_pct: 30.0, resets_at: reset }),
            provenance: Provenance::Observed,
            model: None,
            model_windows: Vec::new(),
        });
        (g, now)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn curve_matches_the_snapshot_at_now() {
        let (g, now) = gov();
        let mut h = History::default();
        h.push(HistoryPoint { t_ms: ms(now), used: 30.0, governed: 30.0, five_hour: Some(20.0) }, 0);
        let v = build(&g, now, &h, &[], None, &|_| 0);
        let s = &v.snapshot;
        let at_now = v.curve.iter().find(|p| p.t_ms == ms(now)).unwrap();
        assert!(close(at_now.plan_total, s.plan), "{} vs {}", at_now.plan_total, s.plan);
        assert!(close(at_now.forecast_total.unwrap(), s.used));
        assert!(close(at_now.with_governor_total.unwrap(), s.used.max(s.plan + s.error)));
        let first = v.curve.first().unwrap();
        assert!(first.plan_total.abs() < 1e-9 && first.forecast_total.is_none() && first.with_governor_total.is_none());
        let last = v.curve.last().unwrap();
        assert!(close(last.forecast_total.unwrap(), s.projected.min(110.0)));
        assert!(
            close(last.with_governor_total.unwrap(), s.used.max(last.plan_total)),
            "the error is closed by the reset"
        );
        assert!(last.plan_total >= at_now.plan_total);
        assert!(!v.working.is_empty() && v.working.len() <= 7);
        assert_eq!(v.allocations.len(), 1);
        assert_eq!(v.actual.len(), 1, "the now point replaces the sample at now");
        assert_eq!(
            v.actual[0],
            HistoryPoint { t_ms: ms(now), used: s.used, governed: s.governed, five_hour: s.five_hour }
        );
    }

    #[test]
    fn dipping_history_draws_a_line_that_only_goes_up() {
        let (g, now) = gov();
        let mut h = History::default();
        let t0 = ms(g.plan().week_start);
        for (i, (used, governed)) in
            [(30.0, 20.0), (34.0, 25.0), (31.0, 18.0), (36.0, 26.0), (35.0, 40.0)].into_iter().enumerate()
        {
            h.push(HistoryPoint { t_ms: t0 + i as i64 * 3_600_000, used, governed, five_hour: None }, 0);
        }
        let v = build(&g, now, &h, &[], None, &|_| 0);
        let a = &v.actual;
        assert_eq!(a.len(), 6);
        assert_eq!(a.last().unwrap().t_ms, ms(now));
        for w in a.windows(2) {
            assert!(w[1].used >= w[0].used && w[1].governed >= w[0].governed, "{w:?}");
        }
        assert!(a.iter().all(|p| p.governed <= p.used), "{a:?}");
        assert_eq!(a[2].used, 34.0);
        assert_eq!(a[2].governed, 25.0);
        assert_eq!(a[4].governed, 36.0, "clamped to W");
        // Past plan points use the raw samples' other share (latest at or before t).
        let at = |t: i64| v.curve.iter().find(|p| p.t_ms == t).unwrap();
        let ideal_at = |t: i64| {
            g.curve().fraction(Timestamp::from_millisecond(t).unwrap())
                * (100.0 - g.plan().reserve_pct - v.snapshot.unattributed_forecast).max(0.0)
        };
        let t2 = t0 + 2 * 3_600_000;
        assert!(close(at(t2).plan_total, ideal_at(t2) + 13.0), "{}", at(t2).plan_total);
        let t4 = t0 + 4 * 3_600_000;
        assert!(close(at(t4).plan_total, ideal_at(t4)), "a negative other share counts as 0");
        // The store's history is not changed.
        assert_eq!(h.points[2].used, 31.0);
    }

    #[test]
    fn last_week_s_samples_are_not_drawn() {
        let (g, now) = gov();
        let mut h = History::default();
        let t0 = ms(g.plan().week_start);
        h.push(HistoryPoint { t_ms: t0 - 60_000, used: 95.0, governed: 10.0, five_hour: None }, 0);
        h.push(HistoryPoint { t_ms: t0 + 3_600_000, used: 5.0, governed: 4.0, five_hour: None }, 0);
        let v = build(&g, now, &h, &[], None, &|_| 0);
        assert!(v.actual.iter().all(|p| p.t_ms >= t0 && p.used < 95.0), "{:?}", v.actual);
        assert!(close(v.curve[0].plan_total, 0.0), "no other share before this week's first sample");
    }

    #[test]
    fn the_lines_close_at_the_reset_after_the_last_working_block() {
        // A Saturday: the default calendar (weekdays 9–19) has no working hours left.
        let now: Timestamp = "2026-10-24T12:00:00Z".parse().unwrap();
        let mut g =
            Governor::new(agent_runtime::runtime::default_plan(now, true), PacingPolicy::default(), now).unwrap();
        assert!(g.curve().hours_left(now) < 1e-9);
        let reset = g.plan().reset_at;
        g.ingest(&UsageSample {
            ts: now,
            agent: None,
            cost_delta: 0.0,
            five_hour: None,
            weekly: Some(WindowReading { used_pct: 30.0, resets_at: reset }),
            provenance: Provenance::Observed,
            model: None,
            model_windows: Vec::new(),
        });
        let v = build(&g, now, &History::default(), &[], None, &|_| 0);
        let s = &v.snapshot;
        let last = v.curve.last().unwrap();
        assert!(close(last.forecast_total.unwrap(), s.projected.min(110.0)));
        assert!(close(last.with_governor_total.unwrap(), s.used.max(last.plan_total)), "{last:?}");
    }

    #[test]
    fn history_keeps_one_week_in_order_and_thins() {
        let mut h = History::default();
        for i in 0..1000 {
            h.push(HistoryPoint { t_ms: i * 60_000, used: i as f64, governed: 0.0, five_hour: None }, 0);
        }
        h.push(HistoryPoint { t_ms: 5, used: 0.0, governed: 0.0, five_hour: None }, 0);
        assert_eq!(h.points.len(), 1000, "out of order point dropped");
        let t = h.thinned(100);
        assert_eq!(t.len(), 100);
        assert_eq!(t.first().unwrap().t_ms, 0);
        assert_eq!(t.last().unwrap().t_ms, 999 * 60_000);
        h.push(HistoryPoint { t_ms: 2_000 * 60_000, used: 1.0, governed: 0.0, five_hour: None }, 1_500 * 60_000);
        assert_eq!(h.points.len(), 1, "a new week drops the old points");
    }
}
