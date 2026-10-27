//! Weekly plan, allocations and pacing policy (SPEC §7).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::calendar::WorkingCalendar;
use crate::pressure::PressureState;
use crate::types::{AllocationId, Tier};

/// `BudgetAllocation`: a share of the week as a weight, with a priority tier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetAllocation {
    pub id: AllocationId,
    /// Free-form target (project / feature / team), for display.
    #[serde(default)]
    pub target: String,
    pub weight: f64,
    pub tier: Tier,
    /// Hard cap in % of the week (FR-811).
    #[serde(default)]
    pub hard_cap_pct: Option<f64>,
}

/// `WeeklyPlan` (FR-802), aligned to the weekly reset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeeklyPlan {
    pub id: String,
    /// Start of the plan week (the previous weekly reset).
    pub week_start: Timestamp,
    /// Weekly reset (`rate_limits.seven_day.resets_at`).
    pub reset_at: Timestamp,
    pub calendar: WorkingCalendar,
    /// Reserve for interactive and critical work, in % of the week.
    pub reserve_pct: f64,
    /// Share of the budget spread over off-hours; 0 = off-hours count nothing.
    #[serde(default)]
    pub off_hours_allowance_pct: f64,
    /// Log decisions without enforcing them (FR-812).
    #[serde(default)]
    pub shadow_mode: bool,
    pub allocations: Vec<BudgetAllocation>,
}

impl WeeklyPlan {
    pub fn allocation(&self, id: &AllocationId) -> Option<&BudgetAllocation> {
        self.allocations.iter().find(|a| &a.id == id)
    }
}

/// Pressure thresholds (ADR-007 table), in pp of error `e = W − plan(t)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub amber_enter: f64,
    pub amber_exit: f64,
    pub red_enter: f64,
    pub red_exit: f64,
    /// Red is entered when W projected at the reset exceeds `100 − red_projection_enter_pp`.
    pub red_projection_enter_pp: f64,
    /// Red is left only once W projected at the reset is under `100 − red_projection_exit_pp`.
    pub red_projection_exit_pp: f64,
    pub catchup_enter: f64,
    pub catchup_exit: f64,
    /// Catch-up needs fewer than this many weighted hours left.
    pub catchup_hours_left: f64,
    pub min_dwell_min: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            amber_enter: 3.0,
            amber_exit: 1.0,
            red_enter: 8.0,
            red_exit: 5.0,
            red_projection_enter_pp: 2.0,
            red_projection_exit_pp: 4.0,
            catchup_enter: -10.0,
            catchup_exit: -5.0,
            catchup_hours_left: 24.0,
            min_dwell_min: 30,
        }
    }
}

/// Live autonomous agents allowed per state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveCaps {
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    pub catch_up: u32,
    /// Outside working time when the plan has no off-hours allowance.
    pub off_hours: u32,
}

impl LiveCaps {
    /// ADR-007 defaults: `base`, `ceil(base/2)`, 1, `base + 2`; 0 off-hours.
    pub fn from_base(base: u32) -> Self {
        LiveCaps { green: base, amber: base.div_ceil(2), red: 1.min(base), catch_up: base + 2, off_hours: 0 }
    }

    pub fn for_state(&self, s: PressureState) -> u32 {
        match s {
            PressureState::Green => self.green,
            PressureState::Amber => self.amber,
            PressureState::Red => self.red,
            PressureState::CatchUp => self.catch_up,
        }
    }
}

/// What happens to an "ask" action when its timeout expires (FR-810).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskDefault {
    Apply,
    Skip,
}

/// What to do with the CLI's own auto-continue at a limit reset (J9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoContinue {
    /// Send Esc and reschedule through the Governor (default).
    CancelAndReschedule,
    /// Let it run and count it as admitted work.
    Allow,
}

/// `PacingPolicy` (SPEC §7) plus the estimator settings of ADR-007.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PacingPolicy {
    pub thresholds: Thresholds,
    pub live_caps: LiveCaps,
    /// In a work block of weight `w < 1`, live caps are multiplied by `w` (rounded up).
    pub scale_caps_by_weight: bool,
    /// Below `red_min_tier`, work is queued in Red (ADR table: "only tier ≥ normal").
    pub red_min_tier: Tier,
    /// 5h guard: projected F at the 5h reset above this caps live agents to 1;
    /// F itself above this admits only interactive and critical work.
    pub five_hour_guard_pct: f64,
    /// The projected-F cap is lifted once the projection falls below this.
    pub five_hour_guard_exit_pct: f64,
    /// Tau of the 5h burn-rate EWMA, wall hours.
    pub five_hour_tau_h: f64,
    /// 5h-window pp per weekly pp, until the 5h cost is fitted from readings
    /// (Run now's cost in the 5h window, FR-809).
    #[serde(default = "default_five_hour_per_weekly_pct")]
    pub five_hour_per_weekly_pct: f64,
    /// Added to whole-percent readings below 100 (0.5 = read `n` as `n + 0.5`).
    pub reading_offset_pct: f64,
    /// An observed value older than this is stale and W becomes estimated.
    pub fresh_minutes: i64,
    /// Tau of the weekly burn-rate EWMA, in weighted hours of the curve.
    pub burn_tau_h: f64,
    /// Safety margin of the estimator, pp, at zero and full confidence.
    pub margin_max_pp: f64,
    pub margin_min_pp: f64,
    /// Initial guess of `k` (cost units per 1 % of the week).
    pub initial_cost_per_pct: f64,
    /// Calibration interval: at least this many observed pp and minutes.
    pub calibration_min_pp: f64,
    pub calibration_min_minutes: i64,
    /// Exponential forgetting per calibration interval (0 < decay ≤ 1).
    pub calibration_decay: f64,
    /// Prior of the unattributed rate `u`, pp per wall hour, and its weight in
    /// hours of observed week: the forecast blends the two by the hours of
    /// the week seen so far (FR-813).
    #[serde(default)]
    pub unattributed_prior_pct_per_h: f64,
    #[serde(default = "default_unattributed_prior_hours")]
    pub unattributed_prior_hours: f64,
    /// Aging: +1 tier per this many hours waited.
    pub aging_hours: f64,
    pub ask_timeout_min: i64,
    pub default_ask_action: AskDefault,
    pub auto_continue: AutoContinue,
    /// Usage alerts, % of the week (FR-817).
    pub alerts_pct: Vec<f64>,
}

const UNATTRIBUTED_PRIOR_HOURS: f64 = 48.0;

fn default_five_hour_per_weekly_pct() -> f64 {
    5.0
}

fn default_unattributed_prior_hours() -> f64 {
    UNATTRIBUTED_PRIOR_HOURS
}

impl Default for PacingPolicy {
    fn default() -> Self {
        PacingPolicy {
            thresholds: Thresholds::default(),
            live_caps: LiveCaps::from_base(3),
            scale_caps_by_weight: true,
            red_min_tier: Tier::Normal,
            five_hour_guard_pct: 85.0,
            five_hour_guard_exit_pct: 80.0,
            five_hour_tau_h: 2.0,
            five_hour_per_weekly_pct: default_five_hour_per_weekly_pct(),
            reading_offset_pct: 0.5,
            fresh_minutes: 15,
            burn_tau_h: 24.0,
            margin_max_pp: 15.0,
            margin_min_pp: 10.0,
            initial_cost_per_pct: 1.0,
            calibration_min_pp: 3.0,
            calibration_min_minutes: 30,
            calibration_decay: 0.85,
            unattributed_prior_pct_per_h: 0.0,
            unattributed_prior_hours: UNATTRIBUTED_PRIOR_HOURS,
            aging_hours: 2.0,
            ask_timeout_min: 20,
            default_ask_action: AskDefault::Apply,
            auto_continue: AutoContinue::CancelAndReschedule,
            alerts_pct: vec![50.0, 75.0, 90.0],
        }
    }
}
