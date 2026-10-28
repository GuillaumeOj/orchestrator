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
    /// The agents' ideal curve, % of the week.
    pub ideal: f64,
    /// G projected at the current burn (t ≥ now only).
    pub forecast: Option<f64>,
    /// G if the Governor closes today's error by the reset (an estimate).
    pub with_governor: Option<f64>,
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
    let h_left_now = curve.hours_left(now).max(1e-9);
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
    let points = times
        .into_iter()
        .map(|t| {
            let ideal = curve.fraction(t) * budget;
            let (forecast, with_governor) = if t >= now {
                let dh = curve.hours_at(t) - h_now;
                let fc = (snap.governed + snap.burn_pct_per_hour * dh).min(110.0);
                // The error shrinks linearly with the working hours left.
                let left = (1.0 - dh / h_left_now).clamp(0.0, 1.0);
                let wg = ideal + snap.error * left;
                (Some(fc), Some(wg.max(snap.governed)))
            } else {
                (None, None)
            };
            CurvePoint { t_ms: ms(t), ideal, forecast, with_governor }
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
        snapshot: snap,
        curve: points,
        actual: history.thinned(400),
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

    #[test]
    fn curve_matches_the_snapshot_at_now() {
        let (g, now) = gov();
        let mut h = History::default();
        h.push(HistoryPoint { t_ms: ms(now), used: 30.0, governed: 30.0, five_hour: Some(20.0) }, 0);
        let v = build(&g, now, &h, &[], None, &|_| 0);
        let at_now = v.curve.iter().find(|p| p.t_ms == ms(now)).unwrap();
        assert!((at_now.ideal - v.snapshot.ideal).abs() < 1e-6, "{} vs {}", at_now.ideal, v.snapshot.ideal);
        assert!((at_now.forecast.unwrap() - v.snapshot.governed).abs() < 1e-6);
        assert!(v.curve.first().unwrap().ideal.abs() < 1e-9 && v.curve.first().unwrap().forecast.is_none());
        let last = v.curve.last().unwrap();
        assert!((last.with_governor.unwrap() - last.ideal).abs() < 1e-6, "the error is closed by the reset");
        assert!(!v.working.is_empty() && v.working.len() <= 7);
        assert_eq!(v.allocations.len(), 1);
        assert_eq!(v.actual.len(), 1);
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
