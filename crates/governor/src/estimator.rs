//! Estimator fallback (FR-814) and unattributed usage (FR-813).
//!
//! Over each calibration interval between two observed weekly values,
//! `ΔW ≈ ΔA / k + u · Δt`, where `ΔA` is the app's attributed cost, `k` the
//! cost per 1 % of the week and `u` the unattributed rate (pp per wall hour).
//! ADR-007 asks for one EWMA scalar `k` and an EWMA of `U`. Fitting both from
//! the same deltas is circular (if `k` is fitted alone it absorbs `U`), so the
//! two are fitted together by exponentially weighted least squares, which is
//! exactly an EWMA of the sufficient statistics. Weak priors (`k = k0`, `u = 0`)
//! keep the fit defined before data arrives.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// `Calibration` record (SPEC §7).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub ts: Timestamp,
    /// `k`: cost units per 1 % of the week.
    pub cost_per_weekly_pct: f64,
    /// 0 (prior only) to 1.
    pub confidence: f64,
    /// `u`: unattributed usage, pp per wall hour.
    pub unattributed_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibrator {
    s_xx: [[f64; 2]; 2],
    s_xy: [f64; 2],
    n_eff: f64,
    decay: f64,
    prior_k: f64,
    a: f64,
    u: f64,
    intervals: u64,
}

const PRIOR_PP: f64 = 5.0;
const PRIOR_HOURS: f64 = 10.0;
/// Weight of each prior pseudo-interval (real intervals weigh 1).
const PRIOR_WEIGHT: f64 = 0.2;

impl Calibrator {
    pub fn new(prior_k: f64, decay: f64) -> Self {
        let prior_k = if prior_k > 0.0 { prior_k } else { 1.0 };
        Calibrator {
            s_xx: [[0.0; 2]; 2],
            s_xy: [0.0; 2],
            n_eff: 0.0,
            decay: decay.clamp(0.01, 1.0),
            prior_k,
            a: 1.0 / prior_k,
            u: 0.0,
            intervals: 0,
        }
    }

    /// `k`.
    pub fn cost_per_pct(&self) -> f64 {
        1.0 / self.a
    }

    /// `u`, pp per wall hour.
    pub fn unattributed_rate(&self) -> f64 {
        self.u
    }

    pub fn confidence(&self) -> f64 {
        self.n_eff / (self.n_eff + 3.0)
    }

    pub fn intervals(&self) -> u64 {
        self.intervals
    }

    pub fn snapshot(&self, ts: Timestamp) -> Calibration {
        Calibration {
            ts,
            cost_per_weekly_pct: self.cost_per_pct(),
            confidence: self.confidence(),
            unattributed_rate: self.u,
        }
    }

    /// Add one interval: attributed cost `d_cost`, `d_hours` wall hours, `d_pct` observed pp.
    pub fn update(&mut self, d_cost: f64, d_hours: f64, d_pct: f64) {
        if !(d_cost.is_finite() && d_hours.is_finite() && d_pct.is_finite())
            || d_cost < 0.0
            || d_hours <= 0.0
            || d_pct < 0.0
        {
            return;
        }
        let x = [d_cost, d_hours];
        for (i, xi) in x.iter().enumerate() {
            for (j, xj) in x.iter().enumerate() {
                self.s_xx[i][j] = self.s_xx[i][j] * self.decay + xi * xj;
            }
            self.s_xy[i] = self.s_xy[i] * self.decay + xi * d_pct;
        }
        self.n_eff = self.n_eff * self.decay + 1.0;
        self.intervals += 1;
        self.solve();
    }

    fn solve(&mut self) {
        // Priors as pseudo-intervals: (k0·5 cost, 0 h) → 5 pp, and (0 cost, 10 h) → 0 pp.
        let pa = self.prior_k * PRIOR_PP;
        let m00 = self.s_xx[0][0] + PRIOR_WEIGHT * pa * pa;
        let m01 = self.s_xx[0][1];
        let m11 = self.s_xx[1][1] + PRIOR_WEIGHT * PRIOR_HOURS * PRIOR_HOURS;
        let b0 = self.s_xy[0] + PRIOR_WEIGHT * pa * PRIOR_PP;
        let b1 = self.s_xy[1];
        let det = m00 * m11 - m01 * m01;
        let (mut a, mut u) = if det.abs() > 1e-12 {
            ((b0 * m11 - b1 * m01) / det, (m00 * b1 - m01 * b0) / det)
        } else {
            (b0 / m00, 0.0)
        };
        if u < 0.0 {
            u = 0.0;
            a = b0 / m00;
        }
        if a <= 0.0 || !a.is_finite() {
            // All usage looks unattributed: keep k, refit u alone.
            a = self.a;
            u = ((b1 - m01 * a) / m11).max(0.0);
        }
        self.a = a;
        self.u = u;
    }
}

/// The most recent observed value of one window, across all agents.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub ts: Timestamp,
    pub used_pct: f64,
    pub resets_at: Timestamp,
    /// The app's cumulative attributed cost when this value was observed.
    pub app_cost: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_holds_without_data() {
        let c = Calibrator::new(10.0, 0.9);
        assert!((c.cost_per_pct() - 10.0).abs() < 1e-9);
        assert_eq!(c.unattributed_rate(), 0.0);
        assert_eq!(c.confidence(), 0.0);
    }

    #[test]
    fn learns_k_and_u_from_mixed_intervals() {
        // Truth: k = 12 cost/pp, u = 0.2 pp/h. Prior k = 8.
        let mut c = Calibrator::new(8.0, 0.95);
        let intervals = [(36.0, 1.0), (60.0, 2.0), (0.0, 15.0), (24.0, 0.5), (48.0, 3.0), (6.0, 12.0)];
        for _ in 0..6 {
            for &(cost, h) in &intervals {
                let pct = cost / 12.0 + 0.2 * h;
                c.update(cost, h, pct);
            }
        }
        assert!((c.cost_per_pct() - 12.0).abs() < 0.3, "k = {}", c.cost_per_pct());
        assert!((c.unattributed_rate() - 0.2).abs() < 0.03, "u = {}", c.unattributed_rate());
        assert!(c.confidence() > 0.8);
    }

    #[test]
    fn negative_unattributed_is_clamped() {
        let mut c = Calibrator::new(10.0, 0.9);
        for _ in 0..10 {
            c.update(50.0, 1.0, 3.0); // implies k ≈ 16.7 and nothing outside
        }
        assert!(c.unattributed_rate() >= 0.0);
        assert!(c.cost_per_pct() > 14.0, "k = {}", c.cost_per_pct());
    }

    #[test]
    fn ignores_invalid_intervals() {
        let mut c = Calibrator::new(10.0, 0.9);
        c.update(-1.0, 1.0, 3.0);
        c.update(1.0, 0.0, 3.0);
        c.update(f64::NAN, 1.0, 3.0);
        assert_eq!(c.intervals(), 0);
    }
}
