//! The Governor: measurements, pressure, admission, queue and actions (ADR-007).

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::actions::{ActionKind, ActionMode, Ask, GovernorAction, Outcome, Undo};
use crate::calendar::{CalendarError, Curve};
use crate::estimator::{Calibration, Calibrator, FAMILIES, FamilyCalibrator, ModelK, Observation, family_index};
use crate::models::{Downgrade, DowngradeWhen, ModelOverride, ModelWindow, ModelWindowState};
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
    /// The agent is suspended: starting the turn spawns a `claude` process,
    /// which waits for a process slot under memory pressure (FR-203).
    #[serde(default)]
    pub needs_spawn: bool,
    /// The model the turn runs on (FR-815: a tight model window holds it).
    #[serde(default)]
    pub model: Option<String>,
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
            needs_spawn: false,
            model: None,
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
    /// Per-model weekly windows seen (FR-815); empty when never seen.
    #[serde(default)]
    pub model_windows: Vec<ModelWindow>,
}

/// The estimator's state (FR-814): what the host persists to carry the
/// calibration across restarts and weeks ([`Governor::restore_estimator`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EstimatorState {
    pub calibrator: Calibrator,
    pub families: FamilyCalibrator,
}

/// A sprint applied to the curve, kept so a restart can apply it again.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SprintRecord {
    pub at: Timestamp,
    pub until: Timestamp,
    /// The boost passed to [`Curve::with_sprint`].
    pub boost: f64,
    pub extra_pp: f64,
}

/// What a restart would otherwise lose (ADR-007 "State across restarts"): the
/// user's overrides, the Governor's own holds and its queue. The samples are
/// replayed separately. The host stores it and gives it to
/// [`Governor::restore`] on the rebuilt Governor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Carry {
    /// The plan week the week-bound parts (deferrals, ask, sprints) belong to.
    #[serde(default)]
    pub week_start: Option<Timestamp>,
    #[serde(default)]
    pub paused_all: bool,
    #[serde(default)]
    pub paused_agents: Vec<AgentId>,
    #[serde(default)]
    pub deferred: Vec<AllocationId>,
    /// A deferral ask still waiting: its allocation and deadline.
    #[serde(default)]
    pub pending_ask: Option<(AllocationId, Timestamp)>,
    #[serde(default)]
    pub sprints: Vec<SprintRecord>,
    /// Plan limits hit (StopFailure `rate_limit`), until their reset.
    #[serde(default)]
    pub limits: Vec<(Window, Timestamp)>,
    #[serde(default)]
    pub queue: Vec<QueueEntry>,
    /// Turns running now. A replan keeps them, so the rebuilt Governor counts
    /// them against its caps; after a restart their processes are gone, so the
    /// host drops them (the reconciler resumes those agents through the queue).
    #[serde(default)]
    pub live: Vec<LiveRecord>,
    /// Agents whose work waits for off-hours (FR-816).
    #[serde(default)]
    pub off_hours: Vec<AgentId>,
    /// Downgrades in force (FR-819) and agents the user restored.
    #[serde(default)]
    pub downgrades: Vec<(AgentId, ModelOverride)>,
    #[serde(default)]
    pub restored_models: Vec<AgentId>,
    /// Model windows marked exhausted by a rate limit, until their reset.
    #[serde(default)]
    pub model_limits: Vec<(String, Timestamp)>,
}

/// A running turn, as [`Carry`] keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveRecord {
    pub agent: AgentId,
    #[serde(flatten)]
    pub turn: LiveTurn,
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
    /// Off-hours work needs an off-hours allowance in the plan (FR-816).
    NoOffHoursAllowance,
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
            GovernorError::NoOffHoursAllowance => {
                write!(f, "the plan has no off-hours allowance: set one in the Week planner first")
            }
        }
    }
}

impl std::error::Error for GovernorError {}

impl From<CalendarError> for GovernorError {
    fn from(e: CalendarError) -> Self {
        GovernorError::Calendar(e)
    }
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

impl CostClass {
    fn of(interactive: bool, tier: Tier) -> Self {
        if interactive {
            CostClass::Interactive
        } else if tier == Tier::Critical {
            CostClass::Critical
        } else {
            CostClass::Autonomous
        }
    }
}

/// An admitted turn while it runs (opaque outside the Governor; see [`LiveRecord`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveTurn {
    allocation: AllocationId,
    tier: Tier,
    class: CostClass,
    started_at: Timestamp,
    /// Cost so far (the watchdog's usage ceiling).
    cost: f64,
    #[serde(default)]
    ceiling: Option<TurnCeiling>,
    /// Work the user scheduled off-hours (FR-816).
    #[serde(default)]
    scheduled: bool,
}

impl LiveTurn {
    pub fn cost(&self) -> f64 {
        self.cost
    }
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
    /// Total W per weighted hour: the measured burn, completed by the
    /// on-plan pace while the measurement is young.
    burn: f64,
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
    fam: FamilyCalibrator,
    /// The estimator as this week started: a restart restores it, then
    /// replays the week's samples, and gets the live run's fit back.
    week_estimator: EstimatorState,
    /// Raw app cost per model family ([`FAMILIES`]), and at the week's start.
    family_cost: [f64; 5],
    week_family_start: [f64; 5],
    weekly_obs: Option<Observation>,
    five_obs: Option<Observation>,
    /// Where the 5h cost fit starts: advances only on a fit or a new 5h window.
    five_anchor: Option<Observation>,
    /// The reading the next calibration interval starts from.
    calib_anchor: Observation,
    /// The week's accounting started at the reset (W is then known to be 0
    /// even before the first reading).
    reset_seen: bool,
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
    ended: BTreeMap<AgentId, (CostClass, Timestamp)>,
    queue: Vec<QueueEntry>,
    actions: Vec<GovernorAction>,
    next_entry: u64,
    sm: StateMachine,
    guard5: bool,
    /// EWMA of the measured burn, started at 0; `burn_weight` is the weight of
    /// data in it (bias correction), the rest goes to the on-plan pace.
    burn: f64,
    burn_weight: f64,
    burn5: f64,
    last: Option<TickPoint>,
    limits: BTreeMap<Window, Timestamp>,
    paused_all: bool,
    paused_agents: BTreeSet<AgentId>,
    /// Live `claude` processes and the memory cap on them (FR-203), from the host.
    slots: Option<(u32, u32)>,
    /// Fitted cost per 1 % of the 5h window (Run now's 5h cost, FR-809).
    k5: Option<f64>,
    deferred: BTreeMap<AllocationId, ActionId>,
    pending_ask: Option<ActionId>,
    sprints: Vec<SprintRecord>,
    avg_turn_min: f64,
    avg_turn_cost: f64,
    alerts_done: Vec<f64>,
    reserve_alerted: bool,
    last_cap: Option<u32>,
    stats: Stats,
    /// FR-815: the latest reading of each model window (seen at, reading).
    model_obs: BTreeMap<String, (Timestamp, WindowReading)>,
    /// Model windows a rate limit marked exhausted, until their reset.
    model_limits: BTreeMap<String, Timestamp>,
    /// Each agent's current model (turn requests, samples).
    agent_model: BTreeMap<AgentId, String>,
    /// FR-816: agents whose next work the user scheduled off-hours.
    off_hours: BTreeSet<AgentId>,
    /// FR-819: downgrades in force, and agents the user restored (no new
    /// downgrade until the condition clears).
    downgrades: BTreeMap<AgentId, ModelOverride>,
    restored_models: BTreeSet<AgentId>,
}

impl Governor {
    pub fn new(plan: WeeklyPlan, policy: PacingPolicy, now: Timestamp) -> Result<Governor, GovernorError> {
        let (tz, base_curve) = plan_curve(&plan)?;
        let calib = Calibrator::new(policy.initial_cost_per_pct, policy.calibration_decay);
        let fam = FamilyCalibrator::new(policy.calibration_decay);
        let mut g = Governor {
            curve: base_curve.clone(),
            base_curve,
            tz,
            week_estimator: EstimatorState { calibrator: calib.clone(), families: fam.clone() },
            calib,
            fam,
            family_cost: [0.0; 5],
            week_family_start: [0.0; 5],
            weekly_obs: None,
            five_obs: None,
            five_anchor: None,
            // Replaced by `start_week_accounting` below.
            calib_anchor: Observation {
                ts: plan.week_start,
                used_pct: 0.0,
                resets_at: plan.reset_at,
                app_cost: 0.0,
                family_cost: [0.0; 5],
            },
            reset_seen: false,
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
            burn_weight: 0.0,
            burn5: 0.0,
            last: None,
            limits: BTreeMap::new(),
            paused_all: false,
            paused_agents: BTreeSet::new(),
            slots: None,
            k5: None,
            deferred: BTreeMap::new(),
            pending_ask: None,
            sprints: Vec::new(),
            avg_turn_min: 20.0,
            avg_turn_cost: policy.initial_cost_per_pct * 0.5,
            alerts_done: Vec::new(),
            reserve_alerted: false,
            last_cap: None,
            stats: Stats::default(),
            model_obs: BTreeMap::new(),
            model_limits: BTreeMap::new(),
            agent_model: BTreeMap::new(),
            off_hours: BTreeSet::new(),
            downgrades: BTreeMap::new(),
            restored_models: BTreeSet::new(),
            plan,
            policy,
        };
        g.start_week_accounting(now);
        Ok(g)
    }

    /// Switch to the next week's plan (after the weekly reset). Calibration and
    /// the queue carry over; budgets, deferrals and sprints do not (FR-808).
    /// A plan whose week contains the current week's start is the same week
    /// realigned (FR-802) or edited: the week's usage so far is kept, as a
    /// restart replaying the week's samples would keep it.
    pub fn new_week(&mut self, plan: WeeklyPlan, now: Timestamp) -> Result<(), GovernorError> {
        let (tz, curve) = plan_curve(&plan)?;
        let same_week = plan.week_start <= self.plan.week_start && self.plan.week_start < plan.reset_at;
        let start_moved = plan.week_start != self.plan.week_start;
        self.tz = tz;
        self.base_curve = curve.clone();
        self.curve = curve;
        self.plan = plan;
        if let Some(id) = self.pending_ask.take() {
            self.set_outcome(id, Outcome::Skipped);
        }
        self.deferred.clear();
        self.sprints.clear();
        self.limits.retain(|_, until| *until > now);
        if self.weekly_obs.is_some_and(|o| o.ts < self.plan.week_start || o.resets_at <= self.plan.week_start) {
            self.weekly_obs = None;
        }
        if !same_week {
            self.start_week_accounting(now);
        } else {
            if start_moved {
                // Moved to an earlier reset: the Governor did not see that one.
                self.reset_seen = false;
            }
            self.calib_anchor = self.week_anchor();
        }
        self.sm.reset(now);
        self.last_cap = None;
        self.refresh_queue(now);
        Ok(())
    }

    fn start_week_accounting(&mut self, now: Timestamp) {
        self.week_cost_start = self.app_cost;
        self.week_family_start = self.family_cost;
        self.fam.new_week();
        self.week_estimator = EstimatorState { calibrator: self.calib.clone(), families: self.fam.clone() };
        self.reserve_cost = 0.0;
        self.interactive_cost = 0.0;
        self.alloc_cost.clear();
        self.service_offset.clear();
        self.alerts_done.clear();
        self.reserve_alerted = false;
        self.burn = 0.0;
        self.burn_weight = 0.0;
        self.last = None;
        // W is 0 at the reset (with the same offset as every reading), so the
        // week's first reading calibrates against it whenever the app started:
        // at the reset, later in the week, or after a restart that replays the
        // week's samples. The live and replayed paths then agree (FR-813).
        self.reset_seen = now <= add_minutes(self.plan.week_start, self.policy.fresh_minutes);
        self.calib_anchor = self.week_anchor();
    }

    /// Where the week's calibration starts: its latest reading, else the
    /// weekly reset as a reading of W = 0 (with the same offset as every
    /// reading) at the app's cost when the week's accounting started.
    fn week_anchor(&self) -> Observation {
        self.weekly_obs.unwrap_or(Observation {
            ts: self.plan.week_start,
            used_pct: self.policy.reading_offset_pct,
            resets_at: self.plan.reset_at,
            app_cost: self.week_cost_start,
            family_cost: self.week_family_start,
        })
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
        self.live.values().filter(|t| t.class == CostClass::Autonomous).count() as u32
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
        let mut c = self.calib.snapshot(now);
        if self.policy.per_model_weights {
            let k = self.calib.cost_per_pct();
            let own = self.fam.solve(k);
            c.per_model = Some(
                FAMILIES
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.fam.counts[*i] > 0)
                    .map(|(i, f)| {
                        (
                            f.to_string(),
                            ModelK { k: own[i].unwrap_or(k), samples: self.fam.counts[i], fitted: own[i].is_some() },
                        )
                    })
                    .collect(),
            );
        }
        c
    }

    /// The estimator now (persist it on each fit change, FR-814).
    pub fn estimator(&self) -> EstimatorState {
        EstimatorState { calibrator: self.calib.clone(), families: self.fam.clone() }
    }

    /// The estimator as this week started.
    pub fn week_estimator(&self) -> &EstimatorState {
        &self.week_estimator
    }

    /// Restores the estimator a week started with, on a fresh Governor, before
    /// the week's samples are replayed (FR-814: the calibration carries over
    /// a restart and a reset).
    pub fn restore_estimator(&mut self, e: EstimatorState) {
        self.calib = e.calibrator.clone();
        self.fam = e.families.clone();
        self.fam.new_week();
        self.week_estimator = EstimatorState { calibrator: self.calib.clone(), families: self.fam.clone() };
    }

    /// How many units of the shared `k` one unit of a family's cost is
    /// (1 unless per-model weights are on and the family has its own fit).
    fn family_weight(&self, family: usize) -> f64 {
        if !self.policy.per_model_weights {
            return 1.0;
        }
        let k = self.calib.cost_per_pct();
        match self.fam.solve(k)[family] {
            Some(kf) if kf > 0.0 => (k / kf).clamp(0.2, 5.0),
            _ => 1.0,
        }
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

    /// `u` for the week's forecasts, pp per wall hour: the fitted rate, shrunk
    /// toward the policy's prior by the hours of this week already observed.
    /// One heavy day of outside use early in the week is then not extrapolated
    /// over the whole week (FR-813), and the fit takes over as the week goes on.
    fn unattributed_rate(&self) -> f64 {
        let u = self.calib.unattributed_rate();
        let seen = self.weekly_obs.map_or(0.0, |o| hours_between(self.plan.week_start, o.ts).max(0.0));
        let prior_h = self.policy.unattributed_prior_hours.max(0.0);
        if seen + prior_h <= 0.0 {
            return u;
        }
        (u * seen + self.policy.unattributed_prior_pct_per_h.max(0.0) * prior_h) / (seen + prior_h)
    }

    fn metrics(&self, now: Timestamp) -> Metrics {
        let k = self.calib.cost_per_pct();
        let u = self.unattributed_rate();
        let week_start = self.plan.week_start;
        let obs = self.weekly_obs;
        let (base, base_ts, base_cost, fresh) = match obs {
            Some(o) => (o.used_pct, o.ts, o.app_cost, now < add_minutes(o.ts, self.policy.fresh_minutes)),
            None => (0.0, week_start, self.week_cost_start, false),
        };
        let delta = ((self.app_cost - base_cost).max(0.0) / k + u * hours_between(base_ts, now).max(0.0)).max(0.0);
        let w_central = (base + delta).min(100.0);
        // The margin covers what is extrapolated: at most the full margin, and
        // never more than the extrapolated delta itself, except before the
        // week's first reading when the app did not see the reset (FR-814).
        let margin = if fresh {
            0.0
        } else if obs.is_none() && !self.reset_seen {
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
        let wall_left = self.curve.wall_hours_left(now);
        let u_forecast = u_so_far + u * wall_left;
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
        // While the burn is barely measured (after the reset or a restart), the
        // rest of it is the on-plan pace: the agents' budget left plus outside
        // usage in working hours, per weighted hour left.
        let hours_left = self.curve.hours_left(now);
        let active_left = self.curve.active_hours_between(now, self.plan.reset_at);
        let off_left = (wall_left - active_left).max(0.0);
        let on_plan = if hours_left > 1e-9 { ((budget - ideal).max(0.0) + u * active_left) / hours_left } else { 0.0 };
        let burn = (self.burn + (1.0 - self.burn_weight) * on_plan).max(0.0);
        let w_projected = w_eff + burn * hours_left + u * off_left;

        // Projected exhaustion at the current burn rate, along the curve. Shown, not a trigger.
        let eta = if w_eff >= 100.0 {
            Some(now)
        } else if burn > 1e-9 {
            self.curve.time_at_hours(self.curve.hours_at(now) + (100.0 - w_eff) / burn)
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
            burn,
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
            burn_pct_per_hour: m.burn,
            unattributed_so_far: m.u_so_far,
            unattributed_forecast: m.u_forecast,
            reserve_left: m.reserve_left,
            interactive_used: self.reserve_cost / m.k,
            calibration: self.calibration(now),
            model_windows: self.model_windows(now),
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
            self.account(c, class, alloc, s.model.as_deref());
        }
        self.observe_sample(s);
    }

    /// The class [`Governor::ingest`] gives this sample now (store it with the sample).
    pub fn cost_class(&self, s: &UsageSample) -> CostClass {
        match &s.agent {
            Some(agent) => match self.live.get(agent) {
                Some(t) => t.class,
                None => self.class_without_turn(agent, s.ts),
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
            self.account(c, class, allocation, s.model.as_deref());
        }
        self.observe_sample(s);
    }

    fn class_without_turn(&self, agent: &AgentId, ts: Timestamp) -> CostClass {
        match self.ended.get(agent) {
            Some((class, ended_at)) if ts <= *ended_at => *class,
            _ => CostClass::Interactive,
        }
    }

    /// `c` raw cost; counted in shared-`k` units (its family's weight).
    fn account(&mut self, c: f64, class: CostClass, alloc: Option<AllocationId>, model: Option<&str>) {
        let f = family_index(model);
        if c > 0.0 {
            self.family_cost[f] += c;
            self.fam.counts[f] = self.fam.counts[f].saturating_add(1);
        }
        let c = c * self.family_weight(f);
        self.app_cost += c;
        if class != CostClass::Autonomous {
            self.reserve_cost += c;
        }
        if class == CostClass::Interactive {
            self.interactive_cost += c;
        }
        if let Some(a) = alloc {
            *self.alloc_cost.entry(a).or_insert(0.0) += c;
        }
    }

    fn observe_sample(&mut self, s: &UsageSample) {
        if let (Some(agent), Some(model)) = (&s.agent, &s.model) {
            self.agent_model.insert(agent.clone(), model.clone());
        }
        if s.provenance == Provenance::Observed {
            for r in &s.model_windows {
                if !r.used_pct.is_finite() || r.resets_at <= s.ts {
                    continue;
                }
                let newer = self.model_obs.get(&r.name).is_none_or(|(ts, _)| s.ts >= *ts);
                if newer {
                    let reading = WindowReading { used_pct: r.used_pct, resets_at: r.resets_at };
                    self.model_obs.insert(r.name.clone(), (s.ts, reading));
                }
            }
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
        let obs = Observation {
            ts,
            used_pct,
            resets_at: r.resets_at,
            app_cost: self.app_cost,
            family_cost: self.family_cost,
        };
        match window {
            Window::Weekly => {
                self.weekly_obs = Some(obs);
                self.calibrate(obs);
            }
            Window::FiveHour => {
                self.fit_five_hour(obs);
                self.five_obs = Some(obs);
            }
        }
        if r.used_pct >= 100.0 {
            self.mark_limit(window, r.resets_at, ts);
        }
    }

    /// Cost per 5h pp, from two readings of the same 5h window at least
    /// `calibration_min_pp` apart (the anchor stays put between fits, as the
    /// weekly calibration's does). Outside usage in between makes it low, so
    /// the 5h cost of Run now errs high.
    fn fit_five_hour(&mut self, obs: Observation) {
        let anchor = match self.five_anchor {
            Some(a) if a.resets_at == obs.resets_at && obs.used_pct >= a.used_pct => a,
            _ => {
                self.five_anchor = Some(obs);
                return;
            }
        };
        let (dp, dc) = (obs.used_pct - anchor.used_pct, obs.app_cost - anchor.app_cost);
        if dp < self.policy.calibration_min_pp || obs.used_pct >= 100.0 {
            return;
        }
        if dc > 0.0 {
            let k5 = dc / dp;
            self.k5 = Some(self.k5.map_or(k5, |old| old + FIVE_HOUR_FIT_ALPHA * (k5 - old)));
        }
        self.five_anchor = Some(obs);
    }

    /// Cost per 1 % of the 5h window: fitted, else `k` over the policy's ratio.
    fn cost_per_five_hour_pct(&self) -> f64 {
        self.k5.unwrap_or_else(|| self.calib.cost_per_pct() / self.policy.five_hour_per_weekly_pct.max(1e-6))
    }

    fn calibrate(&mut self, obs: Observation) {
        let anchor = self.calib_anchor;
        let dp = obs.used_pct - anchor.used_pct;
        let dh = hours_between(anchor.ts, obs.ts);
        if dp < 0.0 || obs.used_pct >= 100.0 {
            return;
        }
        if dp >= self.policy.calibration_min_pp && dh * 60.0 >= self.policy.calibration_min_minutes as f64 {
            self.calib.update(obs.app_cost - anchor.app_cost, dh, dp);
            let mut dc = [0.0; 5];
            for (i, d) in dc.iter_mut().enumerate() {
                *d = obs.family_cost[i] - anchor.family_cost[i];
            }
            self.fam.update(dc, dh, dp);
            self.calib_anchor = obs;
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
        if turn.class == CostClass::Interactive || self.plan.shadow_mode {
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
        agent: &AgentId,
        alloc: &AllocationId,
        tier: Tier,
        eff_tier: Tier,
        needs_spawn: bool,
        now: Timestamp,
    ) -> Result<(), QueueReason> {
        if let Some(r) = self.hold_reason(agent, tier != Tier::Critical, now) {
            return Err(r);
        }
        if needs_spawn {
            self.memory_check()?;
        }
        if let Some(r) = self.model_window_hold(agent, tier, now) {
            return Err(r);
        }
        if let Some(r) = self.off_hours_hold(agent, now) {
            return Err(r);
        }
        if tier == Tier::Critical {
            return Ok(());
        }
        if self.deferred.contains_key(alloc) {
            return Err(QueueReason::DeferredUntilReset);
        }
        if self.over_hard_cap(alloc) {
            return Err(QueueReason::HardCap);
        }
        if self.sm.state == PressureState::Red && eff_tier < self.policy.red_min_tier {
            return Err(QueueReason::RedTier);
        }
        Ok(())
    }

    fn check_entry(&self, e: &QueueEntry, eff_tier: Tier, now: Timestamp) -> Result<(), QueueReason> {
        self.check(&e.agent, &e.allocation, e.tier, eff_tier, e.needs_spawn, now)
    }

    /// May `release_queue` pick this entry now? In shadow mode only memory
    /// waits are held (the queue holds nothing else then).
    fn releasable(&self, e: &QueueEntry, now: Timestamp) -> bool {
        if self.live.contains_key(&e.agent) {
            false
        } else if self.plan.shadow_mode {
            !e.needs_spawn || self.memory_check().is_ok()
        } else {
            self.check_entry(e, e.effective_tier, now).is_ok()
        }
    }

    /// A new process must wait for a slot under memory pressure (FR-203).
    /// Enforced in shadow mode too: it is a memory limit, not pacing.
    fn memory_check(&self) -> Result<(), QueueReason> {
        match self.slots {
            Some((live, cap)) if live >= cap => Err(QueueReason::MemoryPressure { cap }),
            _ => Ok(()),
        }
    }

    fn over_hard_cap(&self, alloc: &AllocationId) -> bool {
        self.plan
            .allocation(alloc)
            .and_then(|a| a.hard_cap_pct)
            .is_some_and(|cap| self.allocation_usage_pct(alloc) >= cap)
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
        if let Some(m) = &req.model {
            self.agent_model.insert(req.agent.clone(), m.clone());
        }
        if self.live.contains_key(&req.agent) {
            self.end_turn(&req.agent, now);
        }
        let m = self.metrics(now);
        let class = CostClass::of(req.interactive, req.tier);

        // Critical work is admitted now: drop the agent's older queued entry.
        if class == CostClass::Critical {
            self.queue.retain(|e| e.agent != req.agent);
        }
        // Already waiting in the queue: keep one entry per agent.
        if !req.interactive {
            if let Some(e) = self.queue.iter_mut().find(|e| e.agent == req.agent) {
                e.tier = e.tier.max(req.tier);
                // The agent was suspended since: its release now spawns a process.
                e.needs_spawn |= req.needs_spawn;
                let v =
                    Verdict::Queue { entry: Some(e.id), reason: e.reason.clone(), estimated_start: e.estimated_start };
                return Decision::enforced(v);
            }
        }

        let proposed_reason: Option<QueueReason> = match class {
            CostClass::Interactive => None,
            _ => match self.check(&req.agent, &req.allocation, req.tier, req.tier, req.needs_spawn, now) {
                Err(r) => Some(r),
                Ok(()) if class == CostClass::Critical => None,
                Ok(()) if self.live_autonomous() >= self.cap(&m) => Some(self.cap_reason(&m)),
                Ok(()) if self.queued_ahead(&req, now) => Some(QueueReason::BehindQueue),
                Ok(()) => None,
            },
        };

        match proposed_reason {
            None => {
                self.start_live(&req.agent, &req.allocation, req.tier, req.ceiling, class, now);
                self.stats.admitted += 1;
                Decision::admit()
            }
            Some(reason) if self.plan.shadow_mode && !matches!(reason, QueueReason::MemoryPressure { .. }) => {
                // A memory wait is still enforced, behind the policy's reason.
                if req.needs_spawn {
                    if let Err(mem) = self.memory_check() {
                        return self.queue_request(req, mem, now);
                    }
                }
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
            Some(reason) => self.queue_request(req, reason, now),
        }
    }

    fn queue_request(&mut self, req: TurnRequest, reason: QueueReason, now: Timestamp) -> Decision {
        let kind = if req.self_started { TurnKind::Continue } else { req.kind };
        let (spawn, ceiling) = (req.needs_spawn, req.ceiling);
        let id = self.enqueue(req.agent, req.allocation, req.tier, kind, req.self_started, spawn, ceiling, reason, now);
        self.refresh_queue(now);
        let e = self.queue.iter().find(|e| e.id == id).expect("just queued");
        let v = Verdict::Queue { entry: Some(id), reason: e.reason.clone(), estimated_start: e.estimated_start };
        Decision::enforced(v)
    }

    /// Would an eligible queued entry be released before this request?
    fn queued_ahead(&self, req: &TurnRequest, now: Timestamp) -> bool {
        let service = self.service();
        let mine = service.get(&req.allocation).copied().unwrap_or(0.0);
        self.queue.iter().any(|e| {
            let eff = e.aged_tier(now, self.policy.aging_hours);
            !self.live.contains_key(&e.agent)
                && self.check_entry(e, eff, now).is_ok()
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
                self.log(
                    now,
                    ActionKind::HoldToolCall { agent: req.agent.clone() },
                    format!("turn not admitted: {cause}"),
                    "shadow mode: tool allowed anyway".into(),
                    ActionMode::Shadow,
                    Outcome::Logged,
                    None,
                );
                return self.shadow_override(proposed);
            }
            self.stats.tool_calls_denied += 1;
            if let Some(alloc) = self.agent_alloc.get(&req.agent).cloned() {
                let tier = self.plan.allocation(&alloc).map_or(Tier::Normal, |a| a.tier);
                // `enqueue` keeps the agent's entry if it already has one.
                self.enqueue(req.agent.clone(), alloc, tier, TurnKind::Continue, true, false, None, cause, now);
            }
            return Decision::enforced(proposed);
        };
        if turn.class == CostClass::Interactive || req.interactive {
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

        let autonomous = class == CostClass::Autonomous;
        let cause = self.hold_reason(&req.agent, autonomous, now).or_else(|| {
            // Deferral and hard caps also stop a running autonomous turn at
            // its next tool boundary, not only the next turn (FR-811).
            let alloc = &self.live.get(&req.agent)?.allocation;
            if !autonomous {
                None
            } else if self.deferred.contains_key(alloc) {
                Some(QueueReason::DeferredUntilReset)
            } else {
                self.over_hard_cap(alloc).then_some(QueueReason::HardCap)
            }
        });
        let Some(cause) = cause else { return Decision::admit() };
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
            self.enqueue(
                req.agent.clone(),
                t.allocation,
                t.tier,
                TurnKind::Continue,
                false,
                false,
                ceiling,
                cause,
                now,
            );
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
        class: CostClass,
        now: Timestamp,
    ) {
        let scheduled = class != CostClass::Interactive && self.off_hours.contains(agent);
        let turn =
            LiveTurn { allocation: allocation.clone(), tier, class, started_at: now, cost: 0.0, ceiling, scheduled };
        self.live.insert(agent.clone(), turn);
    }

    fn end_turn(&mut self, agent: &AgentId, now: Timestamp) -> Option<LiveTurn> {
        let t = self.live.remove(agent)?;
        self.ended.insert(agent.clone(), (t.class, now));
        if t.scheduled {
            // The scheduled work ran (FR-816); a task marked off-hours is marked again by the host.
            self.off_hours.remove(agent);
        }
        if t.class != CostClass::Interactive {
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

    /// Holds that apply to running turns too: Pause all, a paused agent, a hit
    /// limit, and (for autonomous work) F above the 5h guard.
    fn hold_reason(&self, agent: &AgentId, five_hour_guard_applies: bool, now: Timestamp) -> Option<QueueReason> {
        if self.paused_all {
            Some(QueueReason::PausedAll)
        } else if self.paused_agents.contains(agent) {
            Some(QueueReason::PausedAgent)
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
        needs_spawn: bool,
        ceiling: Option<TurnCeiling>,
        reason: QueueReason,
        now: Timestamp,
    ) -> EntryId {
        if let Some(e) = self.queue.iter_mut().find(|e| e.agent == agent) {
            e.needs_spawn |= needs_spawn;
            return e.id;
        }
        // Start-time fair queuing: an allocation that becomes backlogged starts
        // at the lowest service of the others, so idle time is not banked.
        let busy = self.queue.iter().any(|e| e.allocation == allocation)
            || self.live.values().any(|t| t.allocation == allocation && t.class == CostClass::Autonomous);
        if !busy {
            let service = self.service();
            let floor = self
                .queue
                .iter()
                .map(|e| &e.allocation)
                .chain(self.live.values().filter(|t| t.class == CostClass::Autonomous).map(|t| &t.allocation))
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
            estimated_cost_5h_pct: None,
            binding: Window::Weekly,
            self_started,
            ceiling,
            needs_spawn,
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
                self.live.values().filter(|t| t.allocation == a.id && t.class == CostClass::Autonomous).count() as f64;
            let raw =
                (self.alloc_cost.get(&a.id).copied().unwrap_or(0.0) + live * self.avg_turn_cost) / a.weight.max(1e-6);
            let offset = self.service_offset.get(&a.id).copied().unwrap_or(0.0);
            out.insert(a.id.clone(), raw + offset);
        }
        out
    }

    fn release_queue(&mut self, now: Timestamp) -> Vec<Release> {
        let mut out = Vec::new();
        // In shadow mode the queue only holds memory waits (FR-203).
        let shadow = self.plan.shadow_mode;
        let m = self.metrics(now);
        self.age_queue(now);
        loop {
            let service = self.service();
            let first = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, e)| self.releasable(e, now))
                .min_by(|(_, a), (_, b)| release_cmp(a, b, &service))
                .map(|(i, _)| i);
            let Some(idx) = first else { break };
            let class = CostClass::of(false, self.queue[idx].tier);
            if !shadow && class == CostClass::Autonomous && self.live_autonomous() >= self.cap(&m) {
                break;
            }
            let first = self.queue.remove(idx);
            if first.needs_spawn {
                // The spawn takes a slot until the host reports again.
                if let Some((live, _)) = &mut self.slots {
                    *live += 1;
                }
            }
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
            let update = match self.check_entry(e, e.effective_tier, now) {
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
        let cost_5h = self.five_obs.map(|_| self.avg_turn_cost / self.cost_per_five_hour_pct());
        let guard5 = self.guard5;
        for (idx, (reason, start)) in updates {
            let q = &mut self.queue[idx];
            q.binding = match reason {
                QueueReason::LimitReached { window } => window,
                QueueReason::FiveHourGuard => Window::FiveHour,
                _ if guard5 => Window::FiveHour,
                _ => Window::Weekly,
            };
            q.reason = reason;
            q.estimated_start = start;
            q.estimated_cost_pct = cost_pct;
            q.estimated_cost_5h_pct = cost_5h;
        }
    }

    fn estimate_start(&self, r: &QueueReason, e: &QueueEntry, m: &Metrics, now: Timestamp) -> Option<Timestamp> {
        match r {
            QueueReason::PausedAll | QueueReason::PausedAgent | QueueReason::MemoryPressure { .. } => None,
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
            QueueReason::ScheduledOffHours { starts_at } => Some((*starts_at).max(now)),
            QueueReason::ModelWindow { resets_at, .. } => Some(*resets_at),
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
                self.mode(),
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
                let a = ewma_alpha(dh, self.policy.burn_tau_h);
                self.burn = (self.burn + a * (r - self.burn)).max(0.0);
                self.burn_weight += a * (1.0 - self.burn_weight);
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
        let reason = "red: usage well over plan".to_owned();
        let effect = format!(
            "moves this allocation's work (≈ {queued_pct:.1} pp now in flight or queued, plus new work) after the weekly reset"
        );
        if self.plan.shadow_mode {
            let undo = Some(Undo::LiftDeferral { allocation: allocation.clone() });
            let action = ActionKind::DeferAfterReset { allocation };
            self.log(now, action, reason, effect, ActionMode::Shadow, Outcome::Logged, undo);
        } else {
            self.open_defer_ask(allocation, reason, effect, deadline, now);
        }
    }

    /// Asks to defer `allocation` until the reset; the default applies at `deadline` (FR-810).
    fn open_defer_ask(
        &mut self,
        allocation: AllocationId,
        reason: String,
        effect: String,
        deadline: Timestamp,
        now: Timestamp,
    ) -> ActionId {
        let undo = Some(Undo::LiftDeferral { allocation: allocation.clone() });
        let action = ActionKind::DeferAfterReset { allocation };
        let id = self.log(now, action, reason, effect, ActionMode::Ask, Outcome::Pending, undo);
        let default = self.policy.default_ask_action;
        if let Some(a) = self.action_mut(id) {
            a.ask = Some(Ask { deadline, default });
        }
        self.pending_ask = Some(id);
        id
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
                self.sprints.clear();
                self.mark_undone(now, |a| matches!(a, ActionKind::Sprint { .. }));
            }
            Undo::ResumeAll => return Ok(self.resume_all(now)),
            Undo::ResumeAgent { agent } => return Ok(self.resume_agent(&agent, now)),
            Undo::RestoreModel { agent } => {
                self.downgrades.remove(&agent);
                self.restored_models.insert(agent);
            }
        }
        if let Some(a) = self.action_mut(id) {
            a.undone_at = Some(now);
        }
        Ok(self.release_and_refresh(now))
    }

    /// Pause all is in effect (until Resume all).
    pub fn is_paused_all(&self) -> bool {
        self.paused_all
    }

    /// Pause all autonomous work: running turns stop at their next tool call.
    pub fn pause_all(&mut self, now: Timestamp) -> ActionId {
        self.apply_pause_all(BY_USER, now)
    }

    fn apply_pause_all(&mut self, why: &str, now: Timestamp) -> ActionId {
        self.paused_all = true;
        self.log(
            now,
            ActionKind::PauseAll,
            why.into(),
            "autonomous turns stop at their next tool boundary".into(),
            ActionMode::Auto,
            Outcome::Applied,
            Some(Undo::ResumeAll),
        )
    }

    pub fn resume_all(&mut self, now: Timestamp) -> Vec<Release> {
        self.paused_all = false;
        self.mark_undone(now, |a| matches!(a, ActionKind::PauseAll));
        self.log(
            now,
            ActionKind::ResumeAll,
            BY_USER.into(),
            "queued work resumes".into(),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.release_and_refresh(now)
    }

    /// Marks the open actions that `is_open` matches as undone (a Resume
    /// undoes every Pause still in effect).
    fn mark_undone(&mut self, now: Timestamp, is_open: impl Fn(&ActionKind) -> bool) {
        for a in self.actions.iter_mut().filter(|a| a.undone_at.is_none() && is_open(&a.action)) {
            a.undone_at = Some(now);
        }
    }

    /// Agents paused one by one (until Resume).
    pub fn paused_agents(&self) -> &BTreeSet<AgentId> {
        &self.paused_agents
    }

    /// Pause one agent: its running turn stops at its next tool call and its
    /// queued work waits until Resume. Interactive work is never blocked.
    pub fn pause_agent(&mut self, agent: &AgentId, now: Timestamp) -> ActionId {
        let id = self.apply_pause_agent(agent, BY_USER, now);
        self.refresh_queue(now);
        id
    }

    fn apply_pause_agent(&mut self, agent: &AgentId, why: &str, now: Timestamp) -> ActionId {
        self.paused_agents.insert(agent.clone());
        self.log(
            now,
            ActionKind::PauseAgent { agent: agent.clone() },
            why.into(),
            "its autonomous turn stops at the next tool boundary".into(),
            ActionMode::Auto,
            Outcome::Applied,
            Some(Undo::ResumeAgent { agent: agent.clone() }),
        )
    }

    pub fn resume_agent(&mut self, agent: &AgentId, now: Timestamp) -> Vec<Release> {
        if !self.paused_agents.remove(agent) {
            return Vec::new();
        }
        self.mark_undone(now, |a| matches!(a, ActionKind::PauseAgent { agent: x } if x == agent));
        self.log(
            now,
            ActionKind::ResumeAgent { agent: agent.clone() },
            BY_USER.into(),
            "its queued work resumes".into(),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.release_and_refresh(now)
    }

    /// An admitted turn whose agent must spawn a process found no slot when
    /// the host was about to spawn it (FR-203: the cap is checked at spawn
    /// time too, since processes start and stop between admission and spawn).
    /// `live` and `cap` are the host's process count and memory cap at that
    /// moment. The turn goes back to the queue as a memory wait, with no turn
    /// statistics recorded. Enforced in shadow mode too. Returns its entry.
    pub fn hold_spawn(
        &mut self,
        agent: &AgentId,
        allocation: &AllocationId,
        tier: Tier,
        live: u32,
        cap: u32,
        now: Timestamp,
    ) -> EntryId {
        let (allocation, tier, ceiling) = match self.live.remove(agent) {
            Some(t) => (t.allocation, t.tier, t.ceiling),
            None => (allocation.clone(), tier, None),
        };
        let reason = QueueReason::MemoryPressure { cap };
        let id = self.enqueue(agent.clone(), allocation, tier, TurnKind::Continue, false, true, ceiling, reason, now);
        self.slots = Some((live, cap));
        self.refresh_queue(now);
        id
    }

    /// The agent's process was started outside a release (the user's Resume,
    /// a restart): its queued entry no longer waits for a process slot, or it
    /// would wait for the slot the agent now holds itself. Returns the turns
    /// that may now be released.
    pub fn spawned(&mut self, agent: &AgentId, now: Timestamp) -> Vec<Release> {
        let mut changed = false;
        for e in self.queue.iter_mut().filter(|e| e.agent == *agent && e.needs_spawn) {
            e.needs_spawn = false;
            changed = true;
        }
        if changed { self.release_and_refresh(now) } else { Vec::new() }
    }

    /// The host's live `claude` processes and the memory cap on them (FR-203).
    /// Returns the turns released into freed slots.
    pub fn set_process_slots(&mut self, live: u32, cap: u32, now: Timestamp) -> Vec<Release> {
        let freed = self.slots.is_none_or(|(l, c)| live < l || cap > c);
        self.slots = Some((live, cap));
        if freed && self.queue.iter().any(|e| e.needs_spawn) { self.release_and_refresh(now) } else { Vec::new() }
    }

    /// "Run now" on a queued entry: the user overrides the Governor.
    pub fn run_now(&mut self, entry: EntryId, now: Timestamp) -> Option<Release> {
        let idx = self.queue.iter().position(|e| e.id == entry)?;
        let e = self.queue.remove(idx);
        let service = self.service();
        // Counted like critical work: past the cap, drawing on the reserve.
        self.start_live(&e.agent, &e.allocation, e.tier, e.ceiling, CostClass::Critical, now);
        self.log(
            now,
            ActionKind::RunNow { entry, agent: e.agent.clone() },
            "run now, requested by the user".into(),
            run_now_cost(&e),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.refresh_queue(now);
        Some(release_of(&e, service))
    }

    /// Drop a queued entry (cancelled by the user). Returns its agent.
    pub fn cancel(&mut self, entry: EntryId, now: Timestamp) -> Option<AgentId> {
        let idx = self.queue.iter().position(|e| e.id == entry)?;
        let e = self.queue.remove(idx);
        self.log(
            now,
            ActionKind::Cancel { entry, agent: e.agent.clone() },
            "cancelled by the user".into(),
            format!("≈ {:.1} pp not spent", e.estimated_cost_pct),
            ActionMode::Auto,
            Outcome::Applied,
            None,
        );
        self.refresh_queue(now);
        Some(e.agent)
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
        let applied = self.curve.with_sprint(now, until, boost).1;
        if applied <= 0.0 {
            return Err(GovernorError::NothingToBorrow);
        }
        let applied_pp = applied * h_sprint / total * budget;
        let record = SprintRecord { at: now, until, boost, extra_pp: applied_pp };
        Ok(self.apply_sprint(record, format!("sprint requested (+{extra_pp:.1} pp)"), now))
    }

    fn apply_sprint(&mut self, s: SprintRecord, reason: String, now: Timestamp) -> ActionId {
        self.curve = self.curve.with_sprint(s.at, s.until, s.boost).0;
        self.sprints.push(s);
        let (pp, until) = (s.extra_pp, s.until);
        let hours_after = self.curve.active_hours_between(until, self.plan.reset_at);
        self.log(
            now,
            ActionKind::Sprint { extra_pp: pp, until },
            reason,
            format!("today +{pp:.1} pp until {until}; the remaining {hours_after:.0} working hours lose {pp:.1} pp"),
            ActionMode::Auto,
            Outcome::Applied,
            Some(Undo::EndSprint),
        )
    }

    /// The state to keep across a restart (see [`Carry`]).
    pub fn carry(&self) -> Carry {
        let pending_ask = self.pending_ask.and_then(|id| {
            let a = self.action(id)?;
            match (&a.action, &a.ask) {
                (ActionKind::DeferAfterReset { allocation }, Some(ask)) => Some((allocation.clone(), ask.deadline)),
                _ => None,
            }
        });
        Carry {
            week_start: Some(self.plan.week_start),
            paused_all: self.paused_all,
            paused_agents: self.paused_agents.iter().cloned().collect(),
            deferred: self.deferred.keys().cloned().collect(),
            pending_ask,
            sprints: self.sprints.clone(),
            limits: self.limits.iter().map(|(w, t)| (*w, *t)).collect(),
            queue: self.queue.iter().map(QueueEntry::persistent).collect(),
            live: self.live.iter().map(|(agent, t)| LiveRecord { agent: agent.clone(), turn: t.clone() }).collect(),
            off_hours: self.off_hours.iter().cloned().collect(),
            downgrades: self.downgrades.iter().map(|(a, o)| (a.clone(), o.clone())).collect(),
            restored_models: self.restored_models.iter().cloned().collect(),
            model_limits: self.model_limits.iter().map(|(n, t)| (n.clone(), *t)).collect(),
        }
    }

    /// Applies a [`Carry`] to a Governor just rebuilt from the plan and the
    /// week's samples. The overrides come back as actions in this Governor's
    /// log, so they can still be undone. Deferrals, the ask and sprints are
    /// kept only within their plan week; pauses, running turns and the queue
    /// always are. `keep` filters the queue (the host drops entries whose
    /// agents are gone). Running turns come back first, so the releases below
    /// respect the caps they occupy. Returns the turns to deliver now.
    pub fn restore(&mut self, c: Carry, keep: impl Fn(&QueueEntry) -> bool, now: Timestamp) -> Vec<Release> {
        if c.paused_all && !self.paused_all {
            self.apply_pause_all(RESTORED, now);
        }
        for agent in c.paused_agents {
            if !self.paused_agents.contains(&agent) {
                self.apply_pause_agent(&agent, RESTORED, now);
            }
        }
        for (window, until) in c.limits {
            if until > now {
                self.mark_limit(window, until, now);
            }
        }
        if c.week_start == Some(self.plan.week_start) {
            for allocation in c.deferred {
                if self.plan.allocation(&allocation).is_none() || self.deferred.contains_key(&allocation) {
                    continue;
                }
                let undo = Some(Undo::LiftDeferral { allocation: allocation.clone() });
                let action = ActionKind::DeferAfterReset { allocation: allocation.clone() };
                let effect = "this allocation's work waits for the weekly reset".into();
                let id = self.log(now, action, RESTORED.into(), effect, ActionMode::Auto, Outcome::Applied, undo);
                self.deferred.insert(allocation, id);
            }
            if let Some((allocation, deadline)) = c.pending_ask {
                if self.pending_ask.is_none() && self.plan.allocation(&allocation).is_some() {
                    let effect = "moves this allocation's work after the weekly reset".into();
                    self.open_defer_ask(allocation, RESTORED.into(), effect, deadline, now);
                }
            }
            for s in c.sprints {
                if s.until > now {
                    self.apply_sprint(s, RESTORED.into(), now);
                }
            }
        }
        self.off_hours.extend(c.off_hours);
        self.restored_models.extend(c.restored_models);
        for (name, until) in c.model_limits {
            if until > now {
                self.model_limits.insert(name, until);
            }
        }
        for (agent, o) in c.downgrades {
            if self.downgrades.contains_key(&agent) {
                continue;
            }
            let effect = format!("next tasks on {}", o.to);
            let action = ActionKind::Downgrade { agent: agent.clone(), to: o.to.clone() };
            let undo = Some(Undo::RestoreModel { agent: agent.clone() });
            let id = self.log(now, action, RESTORED.into(), effect, ActionMode::Auto, Outcome::Applied, undo);
            self.downgrades.insert(agent, ModelOverride { action: id, ..o });
        }
        for r in c.live {
            if self.live.contains_key(&r.agent) {
                continue;
            }
            self.agent_alloc.entry(r.agent.clone()).or_insert_with(|| r.turn.allocation.clone());
            self.live.insert(r.agent, r.turn);
        }
        for e in c.queue {
            if self.queue.iter().any(|q| q.id == e.id || q.agent == e.agent)
                || self.live.contains_key(&e.agent)
                || !keep(&e)
            {
                continue;
            }
            self.agent_alloc.entry(e.agent.clone()).or_insert_with(|| e.allocation.clone());
            self.next_entry = self.next_entry.max(e.id.0 + 1);
            self.queue.push(e);
        }
        self.release_and_refresh(now)
    }

    // ------------------------------------------------ P3: model windows, off-hours, downgrade

    /// The per-model weekly windows seen and still running (FR-815). Each has
    /// the week's thresholds on its own usage vs the elapsed week, and is Red
    /// at `model_window_red_pct`.
    pub fn model_windows(&self, now: Timestamp) -> Vec<ModelWindow> {
        let th = &self.policy.thresholds;
        let mut out: Vec<ModelWindow> = self
            .model_obs
            .iter()
            .filter(|(_, (_, r))| r.resets_at > now)
            .map(|(name, (_, r))| {
                let limited = self.model_limits.get(name).is_some_and(|u| *u > now);
                let start = add_hours(r.resets_at, -168.0);
                let frac = (hours_between(start, now) / 168.0).clamp(0.0, 1.0);
                let error = r.used_pct - frac * 100.0;
                let projected = if frac >= 0.05 { r.used_pct / frac } else { r.used_pct };
                let state = if limited || r.used_pct >= 100.0 {
                    ModelWindowState::Exhausted
                } else if r.used_pct >= self.policy.model_window_red_pct
                    || (error >= th.red_enter && projected > 100.0 - th.red_projection_enter_pp)
                {
                    ModelWindowState::Red
                } else if error >= th.amber_enter {
                    ModelWindowState::Amber
                } else {
                    ModelWindowState::Green
                };
                ModelWindow {
                    name: name.clone(),
                    family: FAMILIES[family_index(Some(name))].into(),
                    used_pct: r.used_pct,
                    resets_at: r.resets_at,
                    state,
                }
            })
            .collect();
        // A rate limit named a model whose window was never read.
        for (name, until) in self.model_limits.iter().filter(|(n, u)| **u > now && !self.model_obs.contains_key(*n)) {
            out.push(ModelWindow {
                name: name.clone(),
                family: FAMILIES[family_index(Some(name))].into(),
                used_pct: 100.0,
                resets_at: *until,
                state: ModelWindowState::Exhausted,
            });
        }
        out
    }

    /// A StopFailure `rate_limit` whose message names a model (FR-815): that
    /// model's window is exhausted until `resets_at`.
    pub fn model_limit_hit(&mut self, name: &str, resets_at: Timestamp, now: Timestamp) {
        if resets_at > now {
            self.model_limits.insert(name.to_string(), resets_at);
            self.refresh_queue(now);
        }
    }

    /// The tightest window of a model family, when one is Red or exhausted.
    fn tight_window(&self, model: Option<&str>, now: Timestamp) -> Option<ModelWindow> {
        let fam = family_index(model);
        if fam == FAMILIES.len() - 1 {
            return None; // `other`: no window names it
        }
        self.model_windows(now)
            .into_iter()
            .filter(|w| w.family == FAMILIES[fam] && w.state.tight())
            .max_by_key(|w| w.state)
    }

    /// FR-815: turns of a model whose window is Red wait (critical work
    /// excepted); an exhausted window holds all non-interactive work of it.
    fn model_window_hold(&self, agent: &AgentId, tier: Tier, now: Timestamp) -> Option<QueueReason> {
        let w = self.tight_window(self.agent_model.get(agent).map(String::as_str), now)?;
        let exhausted = w.state == ModelWindowState::Exhausted;
        (exhausted || tier != Tier::Critical).then_some(QueueReason::ModelWindow {
            model: w.family,
            resets_at: w.resets_at,
            exhausted,
        })
    }

    /// FR-816: scheduled work waits for off-hours, then runs under its own cap.
    fn off_hours_hold(&self, agent: &AgentId, now: Timestamp) -> Option<QueueReason> {
        if !self.off_hours.contains(agent) {
            return None;
        }
        if self.in_working_time(now) {
            return Some(QueueReason::ScheduledOffHours { starts_at: self.next_off_hours(now) });
        }
        let running = self.live.iter().filter(|(a, t)| t.scheduled && *a != agent).count() as u32;
        let cap = self.policy.off_hours_live_cap;
        (running >= cap).then_some(QueueReason::LiveCap { state: self.sm.state, cap })
    }

    /// `t` is inside a working block of the plan's calendar (not a holiday).
    pub fn in_working_time(&self, t: Timestamp) -> bool {
        let z = t.to_zoned(self.tz.clone());
        if self.plan.calendar.holidays.contains(&z.date()) {
            return false;
        }
        let time = z.time();
        self.plan
            .calendar
            .blocks
            .iter()
            .any(|b| b.weight > 0.0 && b.day.matches(z.weekday()) && b.start <= time && time < b.end)
    }

    /// The start of the next off-hours window (`now` when outside working time).
    pub fn next_off_hours(&self, now: Timestamp) -> Timestamp {
        let mut t = now;
        for _ in 0..64 {
            if !self.in_working_time(t) {
                return t;
            }
            let z = t.to_zoned(self.tz.clone());
            let end = self
                .plan
                .calendar
                .blocks
                .iter()
                .filter(|b| b.day.matches(z.weekday()) && b.start <= z.time() && z.time() < b.end)
                .map(|b| b.end)
                .max();
            let next = end.and_then(|e| z.date().to_datetime(e).to_zoned(self.tz.clone()).ok()).map(|z| z.timestamp());
            t = match next {
                Some(n) if n > t => n,
                _ => add_minutes(t, 1),
            };
        }
        t
    }

    /// Schedule (or unschedule) an agent's next work for off-hours (FR-816).
    /// Returns when it may start.
    pub fn set_off_hours(&mut self, agent: &AgentId, on: bool, now: Timestamp) -> Result<Timestamp, GovernorError> {
        if on {
            if self.plan.off_hours_allowance_pct <= 0.0 {
                return Err(GovernorError::NoOffHoursAllowance);
            }
            self.off_hours.insert(agent.clone());
        } else {
            self.off_hours.remove(agent);
        }
        self.refresh_queue(now);
        Ok(self.next_off_hours(now))
    }

    /// Agents with work scheduled off-hours.
    pub fn off_hours_agents(&self) -> &BTreeSet<AgentId> {
        &self.off_hours
    }

    /// Scheduled off-hours turns running now.
    pub fn off_hours_running(&self) -> u32 {
        self.live.values().filter(|t| t.scheduled).count() as u32
    }

    /// The downgrade in force for an agent (FR-819).
    pub fn model_override(&self, agent: &AgentId) -> Option<&ModelOverride> {
        self.downgrades.get(agent)
    }

    /// FR-819, at an agent's task boundary (its next turn is a new task):
    /// the model this task runs on, `None` for the role's own. A downgrade
    /// starts when the role's condition holds and ends when it clears; the
    /// user's undo keeps the role's model until the condition clears. Shadow
    /// mode logs the downgrade without applying it.
    pub fn task_boundary_model(
        &mut self,
        agent: &AgentId,
        model: Option<&str>,
        rule: Option<&Downgrade>,
        now: Timestamp,
    ) -> Option<String> {
        let state = self.sm.state;
        let cond = rule.filter(|r| family_index(Some(&r.to)) != family_index(model) || model.is_none()).is_some_and(
            |r| match r.when {
                DowngradeWhen::Amber => matches!(state, PressureState::Amber | PressureState::Red),
                DowngradeWhen::Red => state == PressureState::Red,
                DowngradeWhen::ModelWindow => self.tight_window(model, now).is_some(),
            },
        );
        if !cond {
            self.restored_models.remove(agent);
            if self.downgrades.remove(agent).is_some() {
                self.log(
                    now,
                    ActionKind::RestoreModel { agent: agent.clone() },
                    "the downgrade's condition cleared".into(),
                    format!("next tasks on {}", model.unwrap_or("the role's model")),
                    ActionMode::Auto,
                    Outcome::Applied,
                    None,
                );
            }
            return None;
        }
        if self.restored_models.contains(agent) {
            return None;
        }
        if let Some(o) = self.downgrades.get(agent) {
            return Some(o.to.clone());
        }
        let r = rule?;
        let reason = match r.when {
            DowngradeWhen::ModelWindow => format!("the {} window is tight", FAMILIES[family_index(model)]),
            w => format!("the week is {}", w.label()),
        };
        let effect = format!("this task and the next ones on {} (the prompt cache is lost)", r.to);
        let action = ActionKind::Downgrade { agent: agent.clone(), to: r.to.clone() };
        if self.plan.shadow_mode {
            self.log(now, action, reason, effect, ActionMode::Shadow, Outcome::Logged, None);
            return None;
        }
        let undo = Some(Undo::RestoreModel { agent: agent.clone() });
        let id = self.log(now, action, reason, effect, ActionMode::Auto, Outcome::Applied, undo);
        let o =
            ModelOverride { to: r.to.clone(), from: model.map(str::to_string), when: r.when, since: now, action: id };
        self.downgrades.insert(agent.clone(), o);
        self.agent_model.insert(agent.clone(), r.to.clone());
        Some(r.to.clone())
    }

    /// What the Governor would do next.
    pub fn next_action_preview(&self, now: Timestamp) -> NextActionPreview {
        let m = self.metrics(now);
        let service = self.service();
        // The entry `release_queue` would pick.
        let next_release = self
            .queue
            .iter()
            .filter(|e| self.releasable(e, now))
            .min_by(|a, b| release_cmp(a, b, &service))
            .map(|e| (e.id, e.estimated_start));
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

/// Weight of each new fit of the 5h cost per pp (exponential average).
const FIVE_HOUR_FIT_ALPHA: f64 = 0.3;

/// Reasons of actions the user started, and of the same actions restored.
const BY_USER: &str = "requested by the user";
const RESTORED: &str = "restored after a restart";

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

/// "≈ +2.0 pp" of the week, or the 5h share first when the 5h window holds it (FR-809).
fn run_now_cost(e: &QueueEntry) -> String {
    match (e.binding, e.estimated_cost_5h_pct) {
        (Window::FiveHour, Some(f)) => {
            format!("≈ +{f:.0} % of the 5h window (+{:.1} pp of the week)", e.estimated_cost_pct)
        }
        _ => format!("≈ +{:.1} pp", e.estimated_cost_pct),
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
        QueueReason::PausedAgent => "paused_agent",
        QueueReason::MemoryPressure { .. } => "memory_pressure",
        QueueReason::BehindQueue => "behind_queue",
        QueueReason::ScheduledOffHours { .. } => "scheduled_off_hours",
        QueueReason::ModelWindow { .. } => "model_window",
    }
}
