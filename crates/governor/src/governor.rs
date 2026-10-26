//! The Governor: measurements, pressure, admission, queue and actions (ADR-007).

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::actions::{ActionKind, ActionMode, Ask, GovernorAction, Outcome, Undo};
use crate::calendar::{CalendarError, Curve};
use crate::estimator::{Calibration, Calibrator, Observation};
use crate::plan::{AskDefault, AutoContinue, PacingPolicy, WeeklyPlan};
use crate::pressure::{PressureInputs, PressureState, StateMachine};
use crate::queue::{QueueEntry, QueueReason, TurnCeiling, TurnKind, release_cmp};
use crate::types::{
    ActionId, AgentId, AllocationId, EntryId, Provenance, Tier, UsageSample, Window, WindowReading, add_hours,
    add_minutes, ewma_alpha, hours_between,
};

/// A turn asking to start (admission point 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRequest {
    pub agent: AgentId,
    pub allocation: AllocationId,
    pub tier: Tier,
    /// The user's own work (typed in a pane, "Run now"): never blocked.
    #[serde(default)]
    pub interactive: bool,
    pub kind: TurnKind,
    /// Started by the agent itself when background work finished (FR-410):
    /// it cannot be held at its start, so if it is not admitted its tool calls
    /// are denied and a `Continue` entry waits in the queue.
    #[serde(default)]
    pub self_started: bool,
    #[serde(default)]
    pub ceiling: Option<TurnCeiling>,
}

impl TurnRequest {
    pub fn task(agent: impl Into<AgentId>, allocation: impl Into<AllocationId>, tier: Tier) -> Self {
        TurnRequest {
            agent: agent.into(),
            allocation: allocation.into(),
            tier,
            interactive: false,
            kind: TurnKind::Task,
            self_started: false,
            ceiling: None,
        }
    }
}

/// A PreToolUse admission request (admission point 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub agent: AgentId,
    /// The tool call belongs to a turn the user typed.
    #[serde(default)]
    pub interactive: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DenyReason {
    /// "paused by governor": the turn ends at this boundary and resumes from the queue.
    PausedByGovernor { cause: QueueReason },
    /// The watchdog interrupted a turn over its ceiling (FR-407).
    TurnCeiling,
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenyReason::PausedByGovernor { cause } => write!(f, "paused by governor ({cause})"),
            DenyReason::TurnCeiling => write!(f, "paused by governor (turn ceiling exceeded)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Admit,
    /// `entry` is `None` when nothing was queued (shadow mode).
    Queue {
        entry: Option<EntryId>,
        reason: QueueReason,
        estimated_start: Option<Timestamp>,
    },
    Deny {
        reason: DenyReason,
    },
}

impl Verdict {
    pub fn is_admit(&self) -> bool {
        matches!(self, Verdict::Admit)
    }
}

/// `effective` is what the host must do; `proposed` is what the policy wanted.
/// They differ only in shadow mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub effective: Verdict,
    pub proposed: Verdict,
}

impl Decision {
    pub fn admit() -> Self {
        Decision { effective: Verdict::Admit, proposed: Verdict::Admit }
    }

    /// Enforced as proposed.
    pub fn enforced(v: Verdict) -> Self {
        Decision { effective: v.clone(), proposed: v }
    }
}

/// A queued turn the Governor released: the host must deliver it now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub entry: EntryId,
    pub agent: AgentId,
    pub allocation: AllocationId,
    pub kind: TurnKind,
    pub self_started: bool,
    /// Fair-queuing service of every allocation when the turn was picked
    /// (attributed usage / weight + start-time offset; lowest goes first
    /// within a tier).
    pub service: BTreeMap<AllocationId, f64>,
}

/// Answer to a StopFailure `rate_limit` (J9).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LimitResponse {
    /// Send Esc to cancel the CLI's own auto-continue at the reset.
    pub cancel_auto_continue: bool,
    /// The queue entry that will resume the agent's work.
    pub entry: Option<EntryId>,
}

/// `ForecastSnapshot` (SPEC §7) with the figures behind it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastSnapshot {
    pub ts: Timestamp,
    /// W used for decisions (central estimate + margin when estimated).
    pub used: f64,
    /// Central estimate of W, without margin.
    pub used_central: f64,
    pub provenance: Provenance,
    pub margin: f64,
    /// `ideal(t)` of ADR-007 (the agents' curve).
    pub ideal: f64,
    /// What W should be now: `ideal + interactive used + unattributed so far`.
    pub plan: f64,
    /// The governed share `G = W − interactive used − unattributed so far`:
    /// the agents' own usage, critical work included (title bar "agents").
    pub governed: f64,
    /// `G − ideal` (= `W − plan`), pp.
    pub error: f64,
    pub eta: Option<Timestamp>,
    /// Total W projected at the reset (the Red rule's input), %.
    pub projected: f64,
    pub state: PressureState,
    pub live_cap: u32,
    pub live_autonomous: u32,
    pub queued: u32,
    pub five_hour: Option<f64>,
    pub five_hour_projected: Option<f64>,
    pub five_hour_guard: bool,
    /// Weekly burn rate, pp per weighted hour of the curve.
    pub burn_pct_per_hour: f64,
    pub unattributed_so_far: f64,
    pub unattributed_forecast: f64,
    /// Headroom left for interactive work: `100 − W − autonomous plan left − unattributed forecast left`.
    pub reserve_left: f64,
    /// Interactive and critical usage this week, pp.
    pub interactive_used: f64,
    pub calibration: Calibration,
}

/// What the Governor will do next (FR-809 next-action preview).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NextActionPreview {
    pub next_release: Option<(EntryId, Option<Timestamp>)>,
    pub pending_ask: Option<(ActionId, Timestamp)>,
    pub target_state: PressureState,
    pub earliest_state_change: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GovernorError {
    Calendar(CalendarError),
    UnknownAction(ActionId),
    NotUndoable(ActionId),
    AlreadyUndone(ActionId),
    NotPending(ActionId),
    NoWorkingTimeToday,
    /// No working time is left after today to borrow from.
    NothingToBorrow,
}

impl std::fmt::Display for GovernorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GovernorError::Calendar(e) => write!(f, "calendar: {e}"),
            GovernorError::UnknownAction(a) => write!(f, "unknown action {a}"),
            GovernorError::NotUndoable(a) => write!(f, "action {a} cannot be undone"),
            GovernorError::AlreadyUndone(a) => write!(f, "action {a} was already undone"),
            GovernorError::NotPending(a) => write!(f, "action {a} is not a pending ask"),
            GovernorError::NoWorkingTimeToday => write!(f, "no working time left today"),
            GovernorError::NothingToBorrow => write!(f, "no working time left this week to borrow from"),
        }
    }
}

impl std::error::Error for GovernorError {}

impl From<CalendarError> for GovernorError {
    fn from(e: CalendarError) -> Self {
        GovernorError::Calendar(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum TurnClass {
    Interactive,
    Critical,
    Autonomous,
}

/// How a sample's cost counts: the user's own work and critical agent work
/// draw from the reserve, autonomous work from the agents' budget. The host
/// stores it with each sample so [`Governor::replay`] can rebuild the week's
/// accounting after a restart (the turns that were live then are gone).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    Interactive,
    Critical,
    Autonomous,
}

impl From<TurnClass> for CostClass {
    fn from(c: TurnClass) -> Self {
        match c {
            TurnClass::Interactive => CostClass::Interactive,
            TurnClass::Critical => CostClass::Critical,
            TurnClass::Autonomous => CostClass::Autonomous,
        }
    }
}

impl From<CostClass> for TurnClass {
    fn from(c: CostClass) -> Self {
        match c {
            CostClass::Interactive => TurnClass::Interactive,
            CostClass::Critical => TurnClass::Critical,
            CostClass::Autonomous => TurnClass::Autonomous,
        }
    }
}

impl TurnClass {
    fn of(interactive: bool, tier: Tier) -> Self {
        if interactive {
            TurnClass::Interactive
        } else if tier == Tier::Critical {
            TurnClass::Critical
        } else {
            TurnClass::Autonomous
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct LiveTurn {
    allocation: AllocationId,
    tier: Tier,
    class: TurnClass,
    started_at: Timestamp,
    cost: f64,
    ceiling: Option<TurnCeiling>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct TickPoint {
    ts: Timestamp,
    w: f64,
    f: Option<f64>,
    f_resets_at: Option<Timestamp>,
}

/// Everything derived from the measurements at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    w_central: f64,
    w_eff: f64,
    margin: f64,
    provenance: Provenance,
    k: f64,
    u_so_far: f64,
    u_forecast: f64,
    /// Reserve used this week by interactive work, pp.
    r_used: f64,
    governed: f64,
    /// `100 − reserve − U_forecast`: what the curve spreads over the week.
    budget: f64,
    reserve_left: f64,
    ideal: f64,
    plan: f64,
    error: f64,
    eta: Option<Timestamp>,
    w_projected: f64,
    hours_left: f64,
    weight_now: f64,
    f: Option<f64>,
    f_projected: Option<f64>,
    f_resets_at: Option<Timestamp>,
}

/// Counters for reports and tests.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub admitted: u64,
    pub queued: u64,
    pub released: u64,
    pub tool_calls_denied: u64,
    /// Shadow mode: decisions that would have queued or denied.
    pub shadow_overrides: u64,
    pub queued_by_reason: BTreeMap<String, u64>,
}

/// The usage-pacing Governor. Pure and deterministic: every method takes the
/// current time, nothing does I/O.
#[derive(Clone, Debug)]
pub struct Governor {
    plan: WeeklyPlan,
    policy: PacingPolicy,
    tz: TimeZone,
    base_curve: Curve,
    curve: Curve,
    calib: Calibrator,
    weekly_obs: Option<Observation>,
    five_obs: Option<Observation>,
    calib_anchor: Option<Observation>,
    app_cost: f64,
    week_cost_start: f64,
    reserve_cost: f64,
    /// The interactive part of `reserve_cost` (critical work is the agents' own).
    interactive_cost: f64,
    alloc_cost: BTreeMap<AllocationId, f64>,
    service_offset: BTreeMap<AllocationId, f64>,
    agent_alloc: BTreeMap<AgentId, AllocationId>,
    live: BTreeMap<AgentId, LiveTurn>,
    /// Each agent's last ended turn: its class and end time, so the turn's
    /// last API response, ingested after Stop, is still attributed to it.
    ended: BTreeMap<AgentId, (TurnClass, Timestamp)>,
    queue: Vec<QueueEntry>,
    actions: Vec<GovernorAction>,
    next_entry: u64,
    sm: StateMachine,
    guard5: bool,
    burn: f64,
    burn5: f64,
    last: Option<TickPoint>,
    limits: BTreeMap<Window, Timestamp>,
    paused_all: bool,
    deferred: BTreeMap<AllocationId, ActionId>,
    pending_ask: Option<ActionId>,
    avg_turn_min: f64,
    avg_turn_cost: f64,
    alerts_done: Vec<f64>,
    reserve_alerted: bool,
    last_cap: Option<u32>,
    stats: Stats,
}

impl Governor {
    pub fn new(plan: WeeklyPlan, policy: PacingPolicy, now: Timestamp) -> Result<Governor, GovernorError> {
        let (tz, base_curve) = plan_curve(&plan)?;
        let calib = Calibrator::new(policy.initial_cost_per_pct, policy.calibration_decay);
        let mut g = Governor {
            curve: base_curve.clone(),
            base_curve,
            tz,
            calib,
            weekly_obs: None,
            five_obs: None,
            calib_anchor: None,
            app_cost: 0.0,
            week_cost_start: 0.0,
            reserve_cost: 0.0,
            interactive_cost: 0.0,
            alloc_cost: BTreeMap::new(),
            service_offset: BTreeMap::new(),
            agent_alloc: BTreeMap::new(),
            live: BTreeMap::new(),
            ended: BTreeMap::new(),
            queue: Vec::new(),
            actions: Vec::new(),
            next_entry: 1,
            sm: StateMachine::new(now),
            guard5: false,
            burn: 0.0,
            burn5: 0.0,
            last: None,
            limits: BTreeMap::new(),
            paused_all: false,
            deferred: BTreeMap::new(),
            pending_ask: None,
            avg_turn_min: 20.0,
            avg_turn_cost: policy.initial_cost_per_pct * 0.5,
            alerts_done: Vec::new(),
            reserve_alerted: false,
            last_cap: None,
            stats: Stats::default(),
            plan,
            policy,
        };
        g.start_week_accounting(now);
        Ok(g)
    }

    /// Switch to the next week's plan (after the weekly reset). Calibration and
    /// the queue carry over; budgets, deferrals and sprints do not (FR-808).
    pub fn new_week(&mut self, plan: WeeklyPlan, now: Timestamp) -> Result<(), GovernorError> {
        let (tz, curve) = plan_curve(&plan)?;
        self.tz = tz;
        self.base_curve = curve.clone();
        self.curve = curve;
        self.plan = plan;
        if let Some(id) = self.pending_ask.take() {
            self.set_outcome(id, Outcome::Skipped);
        }
        self.deferred.clear();
        self.limits.retain(|_, until| *until > now);
        if self.weekly_obs.is_some_and(|o| o.ts < self.plan.week_start || o.resets_at <= self.plan.week_start) {
            self.weekly_obs = None;
        }
        self.start_week_accounting(now);
        self.sm.reset(now);
        self.last_cap = None;
        self.refresh_queue(now);
        Ok(())
    }

    fn start_week_accounting(&mut self, now: Timestamp) {
        self.week_cost_start = self.app_cost;
        self.reserve_cost = 0.0;
        self.interactive_cost = 0.0;
        self.alloc_cost.clear();
        self.service_offset.clear();
        self.alerts_done.clear();
        self.reserve_alerted = false;
        self.burn = self.budget_slope();
        self.last = None;
        // Created at the reset: W is a known reading of 0 (with the same offset as
        // every reading). Without that anchor or any observation, W is unknown.
        let at_reset = now <= add_minutes(self.plan.week_start, self.policy.fresh_minutes);
        self.calib_anchor = at_reset.then_some(Observation {
            ts: self.plan.week_start,
            used_pct: self.policy.reading_offset_pct,
            resets_at: self.plan.reset_at,
            app_cost: self.app_cost,
        });
    }

    /// Prior burn rate: the plan's pace, pp per weighted hour.
    fn budget_slope(&self) -> f64 {
        (100.0 - self.plan.reserve_pct).max(0.0) / self.curve.total_hours().max(1e-9)
    }

    // ---------------------------------------------------------------- accessors

    pub fn plan(&self) -> &WeeklyPlan {
        &self.plan
    }

    pub fn policy(&self) -> &PacingPolicy {
        &self.policy
    }

    pub fn state(&self) -> PressureState {
        self.sm.state
    }

    pub fn queue(&self) -> &[QueueEntry] {
        &self.queue
    }

    pub fn actions(&self) -> &[GovernorAction] {
        &self.actions
    }

    pub fn action(&self, id: ActionId) -> Option<&GovernorAction> {
        // Ids are dense from 1 and actions are never removed.
        self.actions.get((id.0 as usize).checked_sub(1)?).filter(|a| a.id == id)
    }

    fn action_mut(&mut self, id: ActionId) -> Option<&mut GovernorAction> {
        self.actions.get_mut((id.0 as usize).checked_sub(1)?).filter(|a| a.id == id)
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    pub fn live_autonomous(&self) -> u32 {
        self.live.values().filter(|t| t.class == TurnClass::Autonomous).count() as u32
    }

    pub fn is_live(&self, agent: &AgentId) -> bool {
        self.live.contains_key(agent)
    }

    /// Active sprint: its action, end and the pp borrowed for today.
    pub fn sprint(&self) -> Option<(ActionId, Timestamp, f64)> {
        // A new week starts without sprints: only this week's count.
        self.actions.iter().rev().take_while(|a| a.ts >= self.plan.week_start).find_map(|a| match a.action {
            ActionKind::Sprint { extra_pp, until } if a.undone_at.is_none() => Some((a.id, until, extra_pp)),
            _ => None,
        })
    }

    pub fn calibration(&self, now: Timestamp) -> Calibration {
        self.calib.snapshot(now)
    }

    /// Usage attributed to an allocation this week, % of the week.
    pub fn allocation_usage_pct(&self, id: &AllocationId) -> f64 {
        self.alloc_cost.get(id).copied().unwrap_or(0.0) / self.calib.cost_per_pct()
    }

    // ---------------------------------------------------------------- measurements

    fn margin_full(&self) -> f64 {
        let p = &self.policy;
        p.margin_max_pp - (p.margin_max_pp - p.margin_min_pp) * self.calib.confidence()
    }

    fn metrics(&self, now: Timestamp) -> Metrics {
        let k = self.calib.cost_per_pct();
        let u = self.calib.unattributed_rate();
        let week_start = self.plan.week_start;
        let obs = self.weekly_obs;
        let (base, base_ts, base_cost, fresh) = match obs {
            Some(o) => (o.used_pct, o.ts, o.app_cost, now < add_minutes(o.ts, self.policy.fresh_minutes)),
            None => (0.0, week_start, self.week_cost_start, false),
        };
        let delta = ((self.app_cost - base_cost).max(0.0) / k + u * hours_between(base_ts, now).max(0.0)).max(0.0);
        let w_central = (base + delta).min(100.0);
        // The margin covers what is extrapolated: at most the full margin, and
        // never more than the extrapolated delta itself, except when the week
        // has no anchor at all (FR-814: before any observation).
        let margin = if fresh {
            0.0
        } else if obs.is_none() && self.calib_anchor.is_none() {
            self.margin_full()
        } else {
            self.margin_full().min(delta)
        };
        let w_eff = (w_central + margin).min(100.0);
        let provenance = if fresh { Provenance::Observed } else { Provenance::Estimated };

        let u_so_far = match obs {
            Some(o) => {
                (o.used_pct - (o.app_cost - self.week_cost_start).max(0.0) / k).max(0.0)
                    + u * hours_between(o.ts, now).max(0.0)
            }
            None => u * hours_between(week_start, now).max(0.0),
        };
        let u_forecast = u_so_far + u * self.curve.wall_hours_left(now);
        let budget = (100.0 - self.plan.reserve_pct - u_forecast).max(0.0);
        let ideal = self.curve.fraction(now) * budget;
        // The error is on the governed share (ADR-007, decision 2026-10-21):
        // interactive work draws on the reserve and outside-app usage on its
        // forecast, so neither pushes agents into Amber/Red by itself. Critical
        // work also draws on the reserve but is the agents' own, so it stays in
        // G. The projection below stays on the total W, so a week that would
        // run out still turns Red.
        let r_used = self.interactive_cost / k;
        let governed = (w_eff - r_used - u_so_far).max(0.0);
        let error = governed - ideal;
        let plan = w_eff - error;
        let reserve_left = 100.0 - w_eff - (budget - ideal) - (u_forecast - u_so_far);

        // Total W projected at the reset: the burn rate (per weighted hour, so
        // it follows the curve: half-weight hours, a sprint borrowing from the
        // later days) over the weighted hours left, plus the unattributed rate
        // over the other wall hours, where outside usage goes on while the
        // agents are stopped and the burn is not measured.
        let hours_left = self.curve.hours_left(now);
        let off_left =
            (self.curve.wall_hours_left(now) - self.curve.active_hours_between(now, self.plan.reset_at)).max(0.0);
        let w_projected = w_eff + self.burn * hours_left + u * off_left;

        // Projected exhaustion at the current burn rate, along the curve. Shown, not a trigger.
        let eta = if w_eff >= 100.0 {
            Some(now)
        } else if self.burn > 1e-9 {
            self.curve.time_at_hours(self.curve.hours_at(now) + (100.0 - w_eff) / self.burn)
        } else {
            None
        };

        let f = self.five_hour_now(now);
        let f_resets_at = self.five_obs.map(|o| o.resets_at).filter(|r| now < *r);
        let f_projected = f.map(|f| match f_resets_at {
            Some(r) => f + self.burn5 * hours_between(now, r).max(0.0),
            None => f,
        });

        Metrics {
            w_central,
            w_eff,
            margin,
            provenance,
            k,
            u_so_far,
            u_forecast,
            r_used,
            governed,
            budget,
            reserve_left,
            ideal,
            plan,
            error,
            eta,
            w_projected,
            hours_left,
            weight_now: self.curve.weight_at(now),
            f,
            f_projected,
            f_resets_at,
        }
    }

    fn pressure_inputs(&self, m: &Metrics) -> PressureInputs {
        PressureInputs { error_pp: m.error, projected_pp: m.w_projected, hours_left: m.hours_left }
    }

    /// F now: the latest 5h reading, 0 once its window has reset, `None` if never seen.
    fn five_hour_now(&self, now: Timestamp) -> Option<f64> {
        self.five_obs.map(|o| if now < o.resets_at { o.used_pct } else { 0.0 })
    }

    fn cap(&self, m: &Metrics) -> u32 {
        let caps = &self.policy.live_caps;
        let mut cap = if m.weight_now <= 0.0 { caps.off_hours } else { caps.for_state(self.sm.state) };
        // An hour that counts half gets half the agents (rounded up), so the
        // pace follows the curve without waiting for the error to build up.
        if self.policy.scale_caps_by_weight && m.weight_now > 0.0 && m.weight_now < 1.0 {
            cap = ((cap as f64) * m.weight_now).ceil() as u32;
        }
        if self.guard5 {
            cap = cap.min(1);
        }
        cap
    }

    fn active_limit(&self, now: Timestamp) -> Option<(Window, Timestamp)> {
        self.limits.iter().filter(|(_, until)| **until > now).map(|(w, u)| (*w, *u)).max_by_key(|(_, u)| *u)
    }

    /// Current snapshot (title bar, forecast chart).
    pub fn snapshot(&self, now: Timestamp) -> ForecastSnapshot {
        let m = self.metrics(now);
        ForecastSnapshot {
            ts: now,
            used: m.w_eff,
            used_central: m.w_central,
            provenance: m.provenance,
            margin: m.margin,
            ideal: m.ideal,
            plan: m.plan,
            governed: m.governed,
            error: m.error,
            eta: m.eta,
            projected: m.w_projected,
            state: self.sm.state,
            live_cap: self.cap(&m),
            live_autonomous: self.live_autonomous(),
            queued: self.queue.len() as u32,
            five_hour: m.f,
            five_hour_projected: m.f_projected,
            five_hour_guard: self.guard5,
            burn_pct_per_hour: self.burn,
            unattributed_so_far: m.u_so_far,
            unattributed_forecast: m.u_forecast,
            reserve_left: m.reserve_left,
            interactive_used: self.reserve_cost / m.k,
            calibration: self.calib.snapshot(now),
        }
    }

    // ---------------------------------------------------------------- inputs

    /// Record a usage sample (FR-801). Cost is attributed to the agent's current
    /// turn; usage of an agent with no admitted turn is the user's own
    /// (interactive) work. Window readings keep the most recent sample per window.
    pub fn ingest(&mut self, s: &UsageSample) {
        if let Some(agent) = &s.agent {
            let c = if s.cost_delta.is_finite() { s.cost_delta.max(0.0) } else { 0.0 };
            let (class, alloc) = match self.live.get_mut(agent) {
                Some(t) => {
                    t.cost += c;
                    (t.class, Some(t.allocation.clone()))
                }
                None => (self.class_without_turn(agent, s.ts), self.agent_alloc.get(agent).cloned()),
            };
            self.account(c, class, alloc);
        }
        self.observe_sample(s);
    }

    /// The class [`Governor::ingest`] gives this sample now (store it with the sample).
    pub fn cost_class(&self, s: &UsageSample) -> CostClass {
        match &s.agent {
            Some(agent) => match self.live.get(agent) {
                Some(t) => t.class.into(),
                None => self.class_without_turn(agent, s.ts).into(),
            },
            None => CostClass::Interactive,
        }
    }

    /// Re-ingests a stored sample after a restart, with the class and
    /// allocation it had when it was recorded. Same windows and calibration
    /// as [`Governor::ingest`]; no live turn is touched.
    pub fn replay(&mut self, s: &UsageSample, class: CostClass, allocation: Option<AllocationId>) {
        if s.agent.is_some() {
            let c = if s.cost_delta.is_finite() { s.cost_delta.max(0.0) } else { 0.0 };
            self.account(c, class.into(), allocation);
        }
        self.observe_sample(s);
    }

    fn class_without_turn(&self, agent: &AgentId, ts: Timestamp) -> TurnClass {
        match self.ended.get(agent) {
            Some((class, ended_at)) if ts <= *ended_at => *class,
            _ => TurnClass::Interactive,
        }
    }

    fn account(&mut self, c: f64, class: TurnClass, alloc: Option<AllocationId>) {
        self.app_cost += c;
        if class != TurnClass::Autonomous {
            self.reserve_cost += c;
        }
        if class == TurnClass::Interactive {
            self.interactive_cost += c;
        }
        if let Some(a) = alloc {
            *self.alloc_cost.entry(a).or_insert(0.0) += c;
        }
    }

    fn observe_sample(&mut self, s: &UsageSample) {
        if s.provenance == Provenance::Observed {
            if let Some(r) = s.weekly {
                self.observe(Window::Weekly, s.ts, r);
            }
            if let Some(r) = s.five_hour {
                self.observe(Window::FiveHour, s.ts, r);
            }
        }
    }

    fn observe(&mut self, window: Window, ts: Timestamp, r: WindowReading) {
        if !(r.used_pct.is_finite()) || r.resets_at <= ts {
            return;
        }
        if window == Window::Weekly && ts < self.plan.week_start {
            return;
        }
        let current = match window {
            Window::Weekly => self.weekly_obs,
            Window::FiveHour => self.five_obs,
        };
        let newer = current.is_none_or(|o| ts > o.ts || (ts == o.ts && r.used_pct > o.used_pct));
        if !newer {
            return;
        }
        // The statusline reports whole percents; read `n` as the middle of `[n, n + 1)`.
        let used_pct =
            if r.used_pct >= 100.0 { 100.0 } else { (r.used_pct + self.policy.reading_offset_pct).clamp(0.0, 100.0) };
        let obs = Observation { ts, used_pct, resets_at: r.resets_at, app_cost: self.app_cost };
        match window {
            Window::Weekly => {
                self.weekly_obs = Some(obs);
                self.calibrate(obs);
            }
            Window::FiveHour => self.five_obs = Some(obs),
        }
        if r.used_pct >= 100.0 {
            self.mark_limit(window, r.resets_at, ts);
        }
    }

    fn calibrate(&mut self, obs: Observation) {
        let Some(anchor) = self.calib_anchor else {
            self.calib_anchor = Some(obs);
            return;
        };
        let dp = obs.used_pct - anchor.used_pct;
        let dh = hours_between(anchor.ts, obs.ts);
        if dp < 0.0 || obs.used_pct >= 100.0 {
            return;
        }
        if dp >= self.policy.calibration_min_pp && dh * 60.0 >= self.policy.calibration_min_minutes as f64 {
            self.calib.update(obs.app_cost - anchor.app_cost, dh, dp);
            self.calib_anchor = Some(obs);
        }
    }

    fn mark_limit(&mut self, window: Window, resets_at: Timestamp, now: Timestamp) {
        if self.limits.get(&window).is_some_and(|u| *u >= resets_at) {
            return;
        }
        self.limits.insert(window, resets_at);
        self.log(
            now,
            ActionKind::LimitReached { window, resets_at },
            format!("plan limit reached ({window:?})"),
            format!("autonomous work paused until {resets_at}"),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
    }

    /// StopFailure `rate_limit` from an agent (J9). The agent's turn ended; its
    /// work is queued to resume at the reset, and by default the CLI's own
    /// auto-continue is cancelled so the Governor keeps control.
    pub fn rate_limit_hit(
        &mut self,
        agent: Option<&AgentId>,
        window: Window,
        resets_at: Timestamp,
        now: Timestamp,
    ) -> LimitResponse {
        self.mark_limit(window, resets_at, now);
        self.observe(window, now, WindowReading { used_pct: 100.0, resets_at });
        let mut resp = LimitResponse { cancel_auto_continue: false, entry: None };
        let Some(agent) = agent else { return resp };
        let Some(turn) = self.live.get(agent).cloned() else { return resp };
        self.end_turn(agent, now);
        if turn.class == TurnClass::Interactive || self.plan.shadow_mode {
            return resp;
        }
        if self.policy.auto_continue == AutoContinue::CancelAndReschedule {
            let reason = QueueReason::LimitReached { window };
            resp.entry = Some(self.enqueue(
                agent.clone(),
                turn.allocation,
                turn.tier,
                TurnKind::Continue,
                false,
                turn.ceiling,
                reason,
                now,
            ));
            resp.cancel_auto_continue = true;
            self.log(
                now,
                ActionKind::CancelAutoContinue { agent: agent.clone() },
                "the CLI armed its own auto-continue at the reset".into(),
                "the agent resumes through the queue, in priority order".into(),
                ActionMode::Auto,
                Outcome::Applied,
                None,
            );
        }
        resp
    }

    // ---------------------------------------------------------------- admission

    /// The admission checks of ADR-007 for non-interactive work, except the
    /// fair-queuing order. `Ok` means it may run if a slot is free.
    fn check(
        &self,
        alloc: &AllocationId,
        tier: Tier,
        eff_tier: Tier,
        m: &Metrics,
        now: Timestamp,
    ) -> Result<(), QueueReason> {
        if let Some(r) = self.hold_reason(tier != Tier::Critical, now) {
            return Err(r);
        }
        if tier == Tier::Critical {
            return Ok(());
        }
        if self.deferred.contains_key(alloc) {
            return Err(QueueReason::DeferredUntilReset);
        }
        if let Some(cap) = self.plan.allocation(alloc).and_then(|a| a.hard_cap_pct) {
            if self.alloc_cost.get(alloc).copied().unwrap_or(0.0) / m.k >= cap {
                return Err(QueueReason::HardCap);
            }
        }
        if self.sm.state == PressureState::Red && eff_tier < self.policy.red_min_tier {
            return Err(QueueReason::RedTier);
        }
        Ok(())
    }

    fn cap_reason(&self, m: &Metrics) -> QueueReason {
        if m.weight_now <= 0.0 && self.policy.live_caps.off_hours <= self.live_autonomous() {
            QueueReason::OffHours
        } else {
            QueueReason::LiveCap { state: self.sm.state, cap: self.cap(m) }
        }
    }

    /// Admission point 1: a turn wants to start.
    pub fn request_turn(&mut self, req: TurnRequest, now: Timestamp) -> Decision {
        self.agent_alloc.insert(req.agent.clone(), req.allocation.clone());
        if self.live.contains_key(&req.agent) {
            self.end_turn(&req.agent, now);
        }
        let m = self.metrics(now);
        let class = TurnClass::of(req.interactive, req.tier);

        // Critical work is admitted now: drop the agent's older queued entry.
        if class == TurnClass::Critical {
            self.queue.retain(|e| e.agent != req.agent);
        }
        // Already waiting in the queue: keep one entry per agent.
        if !req.interactive {
            if let Some(e) = self.queue.iter_mut().find(|e| e.agent == req.agent) {
                e.tier = e.tier.max(req.tier);
                let v =
                    Verdict::Queue { entry: Some(e.id), reason: e.reason.clone(), estimated_start: e.estimated_start };
                return Decision::enforced(v);
            }
        }

        let proposed_reason: Option<QueueReason> = match class {
            TurnClass::Interactive => None,
            _ => match self.check(&req.allocation, req.tier, req.tier, &m, now) {
                Err(r) => Some(r),
                Ok(()) if class == TurnClass::Critical => None,
                Ok(()) if self.live_autonomous() >= self.cap(&m) => Some(self.cap_reason(&m)),
                Ok(()) if self.queued_ahead(&req, &m, now) => Some(QueueReason::BehindQueue),
                Ok(()) => None,
            },
        };

        match proposed_reason {
            None => {
                self.start_live(&req.agent, &req.allocation, req.tier, req.ceiling, class, now);
                self.stats.admitted += 1;
                Decision::admit()
            }
            Some(reason) if self.plan.shadow_mode => {
                self.start_live(&req.agent, &req.allocation, req.tier, req.ceiling, class, now);
                self.stats.admitted += 1;
                self.log(
                    now,
                    ActionKind::Queue { entry: None, agent: req.agent.clone() },
                    reason.to_string(),
                    "shadow mode: admitted anyway".into(),
                    ActionMode::Shadow,
                    Outcome::Logged,
                    None,
                );
                self.shadow_override(Verdict::Queue { entry: None, reason, estimated_start: None })
            }
            Some(reason) => {
                let kind = if req.self_started { TurnKind::Continue } else { req.kind };
                let id =
                    self.enqueue(req.agent, req.allocation, req.tier, kind, req.self_started, req.ceiling, reason, now);
                self.refresh_queue(now);
                let e = self.queue.iter().find(|e| e.id == id).expect("just queued");
                let v =
                    Verdict::Queue { entry: Some(id), reason: e.reason.clone(), estimated_start: e.estimated_start };
                Decision::enforced(v)
            }
        }
    }

    /// Would an eligible queued entry be released before this request?
    fn queued_ahead(&self, req: &TurnRequest, m: &Metrics, now: Timestamp) -> bool {
        let service = self.service();
        let mine = service.get(&req.allocation).copied().unwrap_or(0.0);
        self.queue.iter().any(|e| {
            let eff = e.aged_tier(now, self.policy.aging_hours);
            !self.live.contains_key(&e.agent)
                && self.check(&e.allocation, e.tier, eff, m, now).is_ok()
                && (eff > req.tier || (eff == req.tier && service.get(&e.allocation).copied().unwrap_or(0.0) <= mine))
        })
    }

    /// Admission point 2: a PreToolUse hook asks whether the tool may run.
    pub fn tool_call(&mut self, req: &ToolCallRequest, now: Timestamp) -> Decision {
        let Some(turn) = self.live.get(&req.agent) else {
            if req.interactive {
                return Decision::admit();
            }
            // A turn the Governor did not admit (self-started while held, or unknown).
            let cause = self
                .queue
                .iter()
                .find(|e| e.agent == req.agent)
                .map(|e| e.reason.clone())
                .unwrap_or(QueueReason::BehindQueue);
            let proposed = Verdict::Deny { reason: DenyReason::PausedByGovernor { cause: cause.clone() } };
            if self.plan.shadow_mode {
                return self.shadow_override(proposed);
            }
            self.stats.tool_calls_denied += 1;
            if let Some(alloc) = self.agent_alloc.get(&req.agent).cloned() {
                let tier = self.plan.allocation(&alloc).map_or(Tier::Normal, |a| a.tier);
                // `enqueue` keeps the agent's entry if it already has one.
                self.enqueue(req.agent.clone(), alloc, tier, TurnKind::Continue, true, None, cause, now);
            }
            return Decision::enforced(proposed);
        };
        if turn.class == TurnClass::Interactive || req.interactive {
            return Decision::admit();
        }
        let (class, cost, started_at, ceiling) = (turn.class, turn.cost, turn.started_at, turn.ceiling);

        // Watchdog (FR-407).
        if let Some(c) = ceiling {
            let used_pct = cost / self.calib.cost_per_pct();
            let minutes = hours_between(started_at, now) * 60.0;
            let over = c.max_pct.is_some_and(|x| used_pct >= x) || c.max_minutes.is_some_and(|x| minutes >= x as f64);
            if over {
                let proposed = Verdict::Deny { reason: DenyReason::TurnCeiling };
                self.log(
                    now,
                    ActionKind::Watchdog { agent: req.agent.clone() },
                    format!("turn over its ceiling ({used_pct:.1} pp, {minutes:.0} min)"),
                    "turn interrupted at this tool boundary; Inbox item for the user".into(),
                    self.mode(),
                    self.auto_outcome(),
                    None,
                );
                if self.plan.shadow_mode {
                    return self.shadow_override(proposed);
                }
                self.stats.tool_calls_denied += 1;
                self.end_turn(&req.agent, now);
                return Decision::enforced(proposed);
            }
        }

        let Some(cause) = self.hold_reason(class == TurnClass::Autonomous, now) else { return Decision::admit() };
        let proposed = Verdict::Deny { reason: DenyReason::PausedByGovernor { cause: cause.clone() } };
        let effect = if self.plan.shadow_mode {
            "shadow mode: tool allowed anyway"
        } else {
            "turn ends at this tool boundary and resumes from the queue"
        };
        let action = ActionKind::HoldToolCall { agent: req.agent.clone() };
        self.log(now, action, cause.to_string(), effect.into(), self.mode(), self.auto_outcome(), None);
        if self.plan.shadow_mode {
            return self.shadow_override(proposed);
        }
        self.stats.tool_calls_denied += 1;
        if let Some(t) = self.end_turn(&req.agent, now) {
            self.enqueue(req.agent.clone(), t.allocation, t.tier, TurnKind::Continue, false, ceiling, cause, now);
        }
        self.refresh_queue(now);
        Decision::enforced(proposed)
    }

    /// The agent's turn ended (Stop, StopFailure, interrupt). Frees its slot and
    /// returns the queued turns released into it.
    pub fn turn_ended(&mut self, agent: &AgentId, now: Timestamp) -> Vec<Release> {
        self.end_turn(agent, now);
        self.release_and_refresh(now)
    }

    fn start_live(
        &mut self,
        agent: &AgentId,
        allocation: &AllocationId,
        tier: Tier,
        ceiling: Option<TurnCeiling>,
        class: TurnClass,
        now: Timestamp,
    ) {
        let turn = LiveTurn { allocation: allocation.clone(), tier, class, started_at: now, cost: 0.0, ceiling };
        self.live.insert(agent.clone(), turn);
    }

    fn end_turn(&mut self, agent: &AgentId, now: Timestamp) -> Option<LiveTurn> {
        let t = self.live.remove(agent)?;
        self.ended.insert(agent.clone(), (t.class, now));
        if t.class != TurnClass::Interactive {
            let minutes = hours_between(t.started_at, now) * 60.0;
            if minutes > 0.0 {
                self.avg_turn_min += 0.2 * (minutes - self.avg_turn_min);
                self.avg_turn_cost += 0.2 * (t.cost - self.avg_turn_cost);
            }
        }
        Some(t)
    }

    /// Shadow mode: admit, and count what the policy would have done.
    fn shadow_override(&mut self, proposed: Verdict) -> Decision {
        self.stats.shadow_overrides += 1;
        Decision { effective: Verdict::Admit, proposed }
    }

    /// Holds that apply to running turns too: Pause all, a hit limit, and (for
    /// autonomous work) F above the 5h guard.
    fn hold_reason(&self, five_hour_guard_applies: bool, now: Timestamp) -> Option<QueueReason> {
        if self.paused_all {
            Some(QueueReason::PausedAll)
        } else if let Some((window, _)) = self.active_limit(now) {
            Some(QueueReason::LimitReached { window })
        } else if five_hour_guard_applies
            && self.five_hour_now(now).is_some_and(|f| f > self.policy.five_hour_guard_pct)
        {
            Some(QueueReason::FiveHourGuard)
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &mut self,
        agent: AgentId,
        allocation: AllocationId,
        tier: Tier,
        kind: TurnKind,
        self_started: bool,
        ceiling: Option<TurnCeiling>,
        reason: QueueReason,
        now: Timestamp,
    ) -> EntryId {
        if let Some(e) = self.queue.iter().find(|e| e.agent == agent) {
            return e.id;
        }
        // Start-time fair queuing: an allocation that becomes backlogged starts
        // at the lowest service of the others, so idle time is not banked.
        let busy = self.queue.iter().any(|e| e.allocation == allocation)
            || self.live.values().any(|t| t.allocation == allocation && t.class == TurnClass::Autonomous);
        if !busy {
            let service = self.service();
            let floor = self
                .queue
                .iter()
                .map(|e| &e.allocation)
                .chain(self.live.values().filter(|t| t.class == TurnClass::Autonomous).map(|t| &t.allocation))
                .filter(|a| **a != allocation)
                .filter_map(|a| service.get(a).copied())
                .fold(f64::INFINITY, f64::min);
            let mine = service.get(&allocation).copied().unwrap_or(0.0);
            if floor.is_finite() && mine < floor {
                *self.service_offset.entry(allocation.clone()).or_insert(0.0) += floor - mine;
            }
        }
        let id = EntryId(self.next_entry);
        self.next_entry += 1;
        let k = self.calib.cost_per_pct();
        self.queue.push(QueueEntry {
            id,
            kind,
            agent: agent.clone(),
            allocation,
            tier,
            enqueued_at: now,
            effective_tier: tier,
            reason: reason.clone(),
            estimated_start: None,
            estimated_cost_pct: self.avg_turn_cost / k,
            self_started,
            ceiling,
        });
        self.stats.queued += 1;
        *self.stats.queued_by_reason.entry(reason_key(&reason).into()).or_insert(0) += 1;
        self.log(
            now,
            ActionKind::Queue { entry: Some(id), agent },
            reason.to_string(),
            format!("defers ≈ {:.1} pp of usage", self.avg_turn_cost / k),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        id
    }

    /// Fair-queuing service per allocation: attributed usage (plus the live
    /// turns' expected cost) divided by weight, plus its start-time offset.
    fn service(&self) -> BTreeMap<AllocationId, f64> {
        let mut out = BTreeMap::new();
        for a in &self.plan.allocations {
            let live =
                self.live.values().filter(|t| t.allocation == a.id && t.class == TurnClass::Autonomous).count() as f64;
            let raw =
                (self.alloc_cost.get(&a.id).copied().unwrap_or(0.0) + live * self.avg_turn_cost) / a.weight.max(1e-6);
            let offset = self.service_offset.get(&a.id).copied().unwrap_or(0.0);
            out.insert(a.id.clone(), raw + offset);
        }
        out
    }

    fn release_queue(&mut self, now: Timestamp) -> Vec<Release> {
        let mut out = Vec::new();
        if self.plan.shadow_mode {
            return out;
        }
        let m = self.metrics(now);
        self.age_queue(now);
        loop {
            let service = self.service();
            let first = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, e)| !self.live.contains_key(&e.agent))
                .filter(|(_, e)| self.check(&e.allocation, e.tier, e.effective_tier, &m, now).is_ok())
                .min_by(|(_, a), (_, b)| release_cmp(a, b, &service))
                .map(|(i, _)| i);
            let Some(idx) = first else { break };
            let class = TurnClass::of(false, self.queue[idx].tier);
            if class == TurnClass::Autonomous && self.live_autonomous() >= self.cap(&m) {
                break;
            }
            let first = self.queue.remove(idx);
            self.start_live(&first.agent, &first.allocation, first.tier, first.ceiling, class, now);
            self.stats.released += 1;
            let waited = hours_between(first.enqueued_at, now);
            self.log(
                now,
                ActionKind::Release { entry: first.id, agent: first.agent.clone() },
                format!("slot free; waited {:.0} min", waited * 60.0),
                "turn delivered".into(),
                ActionMode::Auto,
                Outcome::Applied,
                None,
            );
            out.push(release_of(&first, service));
        }
        out
    }

    fn age_queue(&mut self, now: Timestamp) {
        let aging = self.policy.aging_hours;
        for e in &mut self.queue {
            e.effective_tier = e.aged_tier(now, aging);
        }
    }

    fn release_and_refresh(&mut self, now: Timestamp) -> Vec<Release> {
        let released = self.release_queue(now);
        self.refresh_queue(now);
        released
    }

    /// Recompute effective tiers, reasons and estimated starts of queued entries.
    fn refresh_queue(&mut self, now: Timestamp) {
        if self.queue.is_empty() {
            return;
        }
        let m = self.metrics(now);
        self.age_queue(now);
        let service = self.service();
        let cap = self.cap(&m);
        let full = self.live_autonomous() >= cap;
        let cap_reason = if full { self.cap_reason(&m) } else { QueueReason::BehindQueue };
        // Entries ahead of an eligible one take `cap` slots per average turn.
        let (base, extra) = if m.weight_now <= 0.0 {
            (self.curve.next_active(now).unwrap_or(now), 0.0)
        } else {
            (now, if full { 1.0 } else { 0.0 })
        };
        let slots_per_turn = cap.max(1) as f64;
        let mut order: Vec<usize> = (0..self.queue.len()).collect();
        order.sort_by(|&a, &b| release_cmp(&self.queue[a], &self.queue[b], &service));
        let mut rank = 0.0;
        let mut updates = Vec::with_capacity(order.len());
        for idx in order {
            let e = &self.queue[idx];
            let update = match self.check(&e.allocation, e.tier, e.effective_tier, &m, now) {
                Err(r) => {
                    let start = self.estimate_start(&r, e, &m, now);
                    (r, start)
                }
                Ok(()) => {
                    let slots = (rank / slots_per_turn).floor() + extra;
                    rank += 1.0;
                    (cap_reason.clone(), Some(add_minutes(base, (slots * self.avg_turn_min).round() as i64)))
                }
            };
            updates.push((idx, update));
        }
        let cost_pct = self.avg_turn_cost / m.k;
        for (idx, (reason, start)) in updates {
            let q = &mut self.queue[idx];
            q.reason = reason;
            q.estimated_start = start;
            q.estimated_cost_pct = cost_pct;
        }
    }

    fn estimate_start(&self, r: &QueueReason, e: &QueueEntry, m: &Metrics, now: Timestamp) -> Option<Timestamp> {
        match r {
            QueueReason::PausedAll => None,
            QueueReason::LimitReached { window } => self.limits.get(window).copied(),
            QueueReason::FiveHourGuard => m.f_resets_at,
            QueueReason::DeferredUntilReset | QueueReason::HardCap => Some(self.plan.reset_at),
            QueueReason::RedTier => {
                // When ideal(t) catches up with G − red_exit, or when aging lifts the tier.
                let budget = m.budget.max(1e-9);
                let target = m.governed - self.policy.thresholds.red_exit;
                let by_curve = self
                    .curve
                    .time_at_hours(target / budget * self.curve.total_hours())
                    .map(|t| t.max(self.sm.dwell_end(&self.policy.thresholds)));
                let steps = (self.policy.red_min_tier as i64 - e.tier as i64).max(0) as f64;
                let by_aging = add_hours(e.enqueued_at, steps * self.policy.aging_hours);
                Some(by_curve.map_or(by_aging, |t| t.min(by_aging)).max(now))
            }
            // `check` never returns these; `refresh_queue` estimates slot waits itself.
            QueueReason::OffHours | QueueReason::LiveCap { .. } | QueueReason::BehindQueue => None,
        }
    }

    // ---------------------------------------------------------------- the clock

    /// Advance the control loop: limits, burn rates, 5h guard, pressure state,
    /// asks, alerts, and queue releases. Call it regularly (e.g. every minute)
    /// and after new samples. Returns the turns to deliver now.
    pub fn tick(&mut self, now: Timestamp) -> Vec<Release> {
        // Limits that reset.
        let expired: Vec<Window> = self.limits.iter().filter(|(_, u)| **u <= now).map(|(w, _)| *w).collect();
        for w in expired {
            self.limits.remove(&w);
            self.log(
                now,
                ActionKind::LimitReset { window: w },
                format!("{w:?} window reset"),
                "queued work resumes in priority order".into(),
                ActionMode::Auto,
                Outcome::Applied,
                None,
            );
        }

        self.update_burn(now);
        let m = self.metrics(now);

        // 5h guard (projected F), with hysteresis.
        let guard = match m.f_projected {
            None => false,
            Some(p) if self.guard5 => p >= self.policy.five_hour_guard_exit_pct,
            Some(p) => p > self.policy.five_hour_guard_pct,
        };
        if guard != self.guard5 {
            self.guard5 = guard;
            let (reason, effect) = if guard {
                (
                    format!("5h window projected at {:.0} % by its reset", m.f_projected.unwrap_or(0.0)),
                    "live autonomous agents capped to 1 to keep headroom for interactive work".to_owned(),
                )
            } else {
                ("5h projection back under the guard".to_owned(), "state cap restored".to_owned())
            };
            self.log(
                now,
                ActionKind::FiveHourGuard { active: guard },
                reason,
                effect,
                self.mode(),
                self.auto_outcome(),
                None,
            );
        }

        // Pressure state.
        let inputs = self.pressure_inputs(&m);
        if let Some((from, to)) = self.sm.step(&inputs, &self.policy.thresholds, now) {
            let reason = format!(
                "W {:.1} %, agents {:.1} % vs plan {:.1} % (e {:+.1} pp), {:.1} % projected at the reset",
                m.w_eff, m.governed, m.ideal, m.error, m.w_projected
            );
            let cap = self.cap(&m);
            self.log(
                now,
                ActionKind::StateChange { from, to },
                reason,
                format!("live autonomous agents capped to {cap}"),
                self.mode(),
                self.auto_outcome(),
                None,
            );
            match to {
                PressureState::Red => self.propose_defer(now),
                PressureState::CatchUp if !self.deferred.is_empty() => {
                    let allocations: Vec<AllocationId> = self.deferred.keys().cloned().collect();
                    self.deferred.clear();
                    self.log(
                        now,
                        ActionKind::ReleaseDeferrals { allocations },
                        "catch-up: usage is under plan".into(),
                        "deferred work can run again this week".into(),
                        ActionMode::Auto,
                        Outcome::Applied,
                        None,
                    );
                }
                _ => {}
            }
        }

        // Asks that timed out (FR-810).
        if let Some(id) = self.pending_ask {
            if let Some(deadline) = self.action(id).and_then(|a| a.ask.as_ref()).map(|a| a.deadline) {
                if now >= deadline {
                    let apply = self.policy.default_ask_action == AskDefault::Apply;
                    self.resolve_ask(id, apply);
                }
            }
        }

        // Alerts (FR-817) and the reserve (FR-807).
        let reached: Vec<f64> = self
            .policy
            .alerts_pct
            .iter()
            .copied()
            .filter(|level| m.w_eff >= *level && !self.alerts_done.contains(level))
            .collect();
        for level in reached {
            self.alerts_done.push(level);
            self.log(
                now,
                ActionKind::Alert { level_pct: level },
                format!("weekly usage reached {level:.0} %"),
                "notification".into(),
                ActionMode::Auto,
                Outcome::Logged,
                None,
            );
        }
        if !self.reserve_alerted && m.reserve_left < 0.0 {
            self.reserve_alerted = true;
            self.log(
                now,
                ActionKind::ReserveExhausted,
                format!("no headroom left for interactive work ({:.1} pp)", m.reserve_left),
                "other work is squeezed harder; interactive work is never blocked".into(),
                ActionMode::Auto,
                Outcome::Logged,
                None,
            );
        }

        // Live cap changes.
        let cap = self.cap(&m);
        if self.last_cap != Some(cap) {
            if self.last_cap.is_some() {
                let live = self.live_autonomous();
                let effect = if live > cap {
                    format!("{} agent(s) pause at their next turn boundary", live - cap)
                } else {
                    format!("up to {cap} autonomous agents")
                };
                let reason = if m.weight_now <= 0.0 {
                    "outside working hours".to_owned()
                } else {
                    format!("{} state", self.sm.state)
                };
                self.log(
                    now,
                    ActionKind::LiveCap { cap, live },
                    reason,
                    effect,
                    self.mode(),
                    self.auto_outcome(),
                    None,
                );
            }
            self.last_cap = Some(cap);
        }

        self.release_and_refresh(now)
    }

    fn update_burn(&mut self, now: Timestamp) {
        let m = self.metrics(now);
        if let Some(last) = self.last {
            let dh = self.curve.hours_at(now) - self.curve.hours_at(last.ts);
            if dh > 1e-9 {
                let r = (m.w_central - last.w) / dh;
                self.burn = (self.burn + ewma_alpha(dh, self.policy.burn_tau_h) * (r - self.burn)).max(0.0);
            }
            let dt = hours_between(last.ts, now);
            if dt > 0.0 {
                let r5 = match (m.f, last.f) {
                    (Some(f), Some(lf)) if m.f_resets_at.is_some() && m.f_resets_at == last.f_resets_at => {
                        (f - lf) / dt
                    }
                    _ => 0.0,
                };
                self.burn5 = (self.burn5 + ewma_alpha(dt, self.policy.five_hour_tau_h) * (r5 - self.burn5)).max(0.0);
            }
        }
        self.last = Some(TickPoint { ts: now, w: m.w_central, f: m.f, f_resets_at: m.f_resets_at });
    }

    fn mode(&self) -> ActionMode {
        if self.plan.shadow_mode { ActionMode::Shadow } else { ActionMode::Auto }
    }

    fn auto_outcome(&self) -> Outcome {
        if self.plan.shadow_mode { Outcome::Logged } else { Outcome::Applied }
    }

    /// Entering Red: propose deferring a low-tier allocation until the reset.
    fn propose_defer(&mut self, now: Timestamp) {
        if self.pending_ask.is_some() {
            return;
        }
        let min_tier = self.policy.red_min_tier;
        let candidate = self
            .plan
            .allocations
            .iter()
            .filter(|a| a.tier < min_tier && !self.deferred.contains_key(&a.id))
            .map(|a| {
                let queued = self.queue.iter().filter(|e| e.allocation == a.id).count();
                let live = self.live.values().filter(|t| t.allocation == a.id).count();
                (queued + live, a)
            })
            .filter(|(n, _)| *n > 0)
            .max_by(|(na, a), (nb, b)| {
                na.cmp(nb).then_with(|| b.weight.total_cmp(&a.weight)).then_with(|| b.id.cmp(&a.id))
            })
            .map(|(n, a)| (n, a.id.clone()));
        let Some((turns, allocation)) = candidate else { return };
        let k = self.calib.cost_per_pct();
        let queued_pct = turns as f64 * self.avg_turn_cost / k;
        let deadline = add_minutes(now, self.policy.ask_timeout_min);
        let shadow = self.plan.shadow_mode;
        let id = self.log(
            now,
            ActionKind::DeferAfterReset { allocation: allocation.clone() },
            "red: usage well over plan".into(),
            format!("moves this allocation's work (≈ {queued_pct:.1} pp now in flight or queued, plus new work) after the weekly reset"),
            if shadow { ActionMode::Shadow } else { ActionMode::Ask },
            if shadow { Outcome::Logged } else { Outcome::Pending },
            Some(Undo::LiftDeferral { allocation }),
        );
        if !shadow {
            let default = self.policy.default_ask_action;
            if let Some(a) = self.action_mut(id) {
                a.ask = Some(Ask { deadline, default });
            }
            self.pending_ask = Some(id);
        }
    }

    fn resolve_ask(&mut self, id: ActionId, apply: bool) {
        self.pending_ask = None;
        let Some(a) = self.action(id).cloned() else { return };
        if apply {
            if let ActionKind::DeferAfterReset { allocation } = &a.action {
                self.deferred.insert(allocation.clone(), id);
            }
            self.set_outcome(id, Outcome::Applied);
        } else {
            self.set_outcome(id, Outcome::Skipped);
        }
    }

    fn set_outcome(&mut self, id: ActionId, outcome: Outcome) {
        if let Some(a) = self.action_mut(id) {
            a.outcome = outcome;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn log(
        &mut self,
        now: Timestamp,
        action: ActionKind,
        reason: String,
        expected_effect: String,
        mode: ActionMode,
        outcome: Outcome,
        undo: Option<Undo>,
    ) -> ActionId {
        let id = ActionId(self.actions.len() as u64 + 1);
        self.actions.push(GovernorAction {
            id,
            ts: now,
            state: self.sm.state,
            action,
            reason,
            expected_effect,
            mode,
            outcome,
            ask: None,
            undo,
            undone_at: None,
        });
        id
    }

    // ---------------------------------------------------------------- user controls

    /// Answer an "ask" action (accept = apply it).
    pub fn answer_ask(&mut self, id: ActionId, accept: bool, _now: Timestamp) -> Result<(), GovernorError> {
        if self.pending_ask != Some(id) {
            return Err(GovernorError::NotPending(id));
        }
        self.resolve_ask(id, accept);
        Ok(())
    }

    /// Undo an action from the log (FR-809). Deferrals, sprints and Pause all
    /// can be undone; the other actions are re-decided by the control loop.
    pub fn undo(&mut self, id: ActionId, now: Timestamp) -> Result<Vec<Release>, GovernorError> {
        let a = self.action(id).cloned().ok_or(GovernorError::UnknownAction(id))?;
        if a.undone_at.is_some() {
            return Err(GovernorError::AlreadyUndone(id));
        }
        let undo = a.undo.ok_or(GovernorError::NotUndoable(id))?;
        match undo {
            Undo::LiftDeferral { allocation } => {
                if self.pending_ask == Some(id) {
                    self.resolve_ask(id, false);
                }
                self.deferred.remove(&allocation);
            }
            Undo::EndSprint => {
                // Sprints stack on the curve; ending one restores the plan's curve, so all end.
                self.curve = self.base_curve.clone();
                for a in self.actions.iter_mut().filter(|a| a.id != id) {
                    if matches!(a.action, ActionKind::Sprint { .. }) && a.undone_at.is_none() {
                        a.undone_at = Some(now);
                    }
                }
            }
            Undo::ResumeAll => {
                self.paused_all = false;
            }
        }
        if let Some(a) = self.action_mut(id) {
            a.undone_at = Some(now);
        }
        Ok(self.release_and_refresh(now))
    }

    /// Pause all autonomous work: running turns stop at their next tool call.
    pub fn pause_all(&mut self, now: Timestamp) -> ActionId {
        self.paused_all = true;
        self.log(
            now,
            ActionKind::PauseAll,
            "requested by the user".into(),
            "autonomous turns stop at their next tool boundary".into(),
            ActionMode::Auto,
            Outcome::Applied,
            Some(Undo::ResumeAll),
        )
    }

    pub fn resume_all(&mut self, now: Timestamp) -> Vec<Release> {
        self.paused_all = false;
        self.log(
            now,
            ActionKind::ResumeAll,
            "requested by the user".into(),
            "queued work resumes".into(),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.release_and_refresh(now)
    }

    /// "Run now" on a queued entry: the user overrides the Governor.
    pub fn run_now(&mut self, entry: EntryId, now: Timestamp) -> Option<Release> {
        let idx = self.queue.iter().position(|e| e.id == entry)?;
        let e = self.queue.remove(idx);
        let service = self.service();
        // Counted like critical work: past the cap, drawing on the reserve.
        self.start_live(&e.agent, &e.allocation, e.tier, e.ceiling, TurnClass::Critical, now);
        self.log(
            now,
            ActionKind::RunNow { entry, agent: e.agent.clone() },
            "run now, requested by the user".into(),
            format!("≈ +{:.1} pp", e.estimated_cost_pct),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.refresh_queue(now);
        Some(release_of(&e, service))
    }

    /// Drop a queued entry (task cancelled).
    pub fn cancel(&mut self, entry: EntryId) -> bool {
        let n = self.queue.len();
        self.queue.retain(|e| e.id != entry);
        self.queue.len() != n
    }

    /// Sprint mode (FR-811): raise today's target by `extra_pp` until the end of
    /// today's working time, borrowed from the remaining days.
    pub fn start_sprint(&mut self, extra_pp: f64, now: Timestamp) -> Result<ActionId, GovernorError> {
        let until = self.curve.end_of_working_day(now, &self.tz).ok_or(GovernorError::NoWorkingTimeToday)?;
        let m = self.metrics(now);
        let budget = m.budget.max(1e-9);
        let h_sprint = self.curve.hours_at(until) - self.curve.hours_at(now);
        if h_sprint <= 0.0 {
            return Err(GovernorError::NoWorkingTimeToday);
        }
        let total = self.curve.total_hours();
        let boost = extra_pp.max(0.0) * total / (budget * h_sprint);
        let (curve, applied) = self.curve.with_sprint(now, until, boost);
        if applied <= 0.0 {
            return Err(GovernorError::NothingToBorrow);
        }
        let applied_pp = applied * h_sprint / total * budget;
        let hours_after = curve.active_hours_between(until, self.plan.reset_at);
        self.curve = curve;
        let id = self.log(
            now,
            ActionKind::Sprint { extra_pp: applied_pp, until },
            format!("sprint requested (+{extra_pp:.1} pp)"),
            format!("today +{applied_pp:.1} pp until {until}; the remaining {hours_after:.0} working hours lose {applied_pp:.1} pp"),
            ActionMode::Auto,
            Outcome::Applied,
            Some(Undo::EndSprint),
        );
        Ok(id)
    }

    /// What the Governor would do next.
    pub fn next_action_preview(&self, now: Timestamp) -> NextActionPreview {
        let m = self.metrics(now);
        let service = self.service();
        let next_release =
            self.queue.iter().min_by(|a, b| release_cmp(a, b, &service)).map(|e| (e.id, e.estimated_start));
        let pending_ask =
            self.pending_ask.and_then(|id| self.action(id).and_then(|a| a.ask.as_ref()).map(|a| (id, a.deadline)));
        let inputs = self.pressure_inputs(&m);
        NextActionPreview {
            next_release,
            pending_ask,
            target_state: self.sm.target(&inputs, &self.policy.thresholds),
            earliest_state_change: self.sm.dwell_end(&self.policy.thresholds).max(now),
        }
    }
}

fn plan_curve(plan: &WeeklyPlan) -> Result<(TimeZone, Curve), GovernorError> {
    let tz = TimeZone::get(&plan.calendar.time_zone)
        .map_err(|_| CalendarError::UnknownTimeZone(plan.calendar.time_zone.clone()))?;
    let curve = Curve::build(&plan.calendar, plan.week_start, plan.reset_at, plan.off_hours_allowance_pct)?;
    Ok((tz, curve))
}

fn release_of(e: &QueueEntry, service: BTreeMap<AllocationId, f64>) -> Release {
    Release {
        entry: e.id,
        agent: e.agent.clone(),
        allocation: e.allocation.clone(),
        kind: e.kind,
        self_started: e.self_started,
        service,
    }
}

fn reason_key(r: &QueueReason) -> &'static str {
    match r {
        QueueReason::LiveCap { .. } => "live_cap",
        QueueReason::RedTier => "red_tier",
        QueueReason::OffHours => "off_hours",
        QueueReason::FiveHourGuard => "five_hour_guard",
        QueueReason::LimitReached { .. } => "limit_reached",
        QueueReason::DeferredUntilReset => "deferred",
        QueueReason::HardCap => "hard_cap",
        QueueReason::PausedAll => "paused_all",
        QueueReason::BehindQueue => "behind_queue",
    }
}
