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

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// `Calibration` record (SPEC §7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub ts: Timestamp,
    /// `k`: cost units per 1 % of the week.
    pub cost_per_weekly_pct: f64,
    /// 0 (prior only) to 1.
    pub confidence: f64,
    /// `u`: unattributed usage, pp per wall hour.
    pub unattributed_rate: f64,
    /// FR-814 per-model weights (`PacingPolicy::per_model_weights`): `k` of
    /// each model family seen this week, and its samples. `None` when off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_model: Option<BTreeMap<String, ModelK>>,
}

/// One model family's cost per 1 % of the week.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelK {
    /// The family's own fit once it has [`MIN_FAMILY_SAMPLES`], else the shared `k`.
    pub k: f64,
    /// Samples with cost this week.
    pub samples: u32,
    /// The family's own fit is used.
    pub fitted: bool,
}

/// Model families with their own `k` (anything else is `other`).
pub const FAMILIES: [&str; 5] = ["opus", "sonnet", "haiku", "fable", "other"];
/// A family gets its own `k` from this many samples this week.
pub const MIN_FAMILY_SAMPLES: u32 = 20;

/// `claude-opus-4-1`, `opus`, `Opus 4.1` → `opus` (index in [`FAMILIES`]).
pub fn family_index(model: Option<&str>) -> usize {
    let m = model.unwrap_or("").to_ascii_lowercase();
    FAMILIES[..4].iter().position(|f| m.contains(f)).unwrap_or(4)
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
    /// Recent intervals over the bound, decayed by [`BOUND_MEMORY`] (§3.3).
    #[serde(default)]
    over_bound: f64,
}

const PRIOR_PP: f64 = 5.0;
const PRIOR_HOURS: f64 = 10.0;
/// Weight of each prior pseudo-interval (real intervals weigh 1).
const PRIOR_WEIGHT: f64 = 0.2;

/// fix-week-usage-count §3.3: an interval whose app cost alone explains more
/// than `BOUND_RATIO · ΔW + BOUND_SLACK_PP` at the current `k` (a
/// double-counted cost) is skipped, not clipped: a clipped one still pulls `k`
/// up, and the bound grows with `k`. A sound interval's app share is at most
/// `ΔW`; the slack is a whole-percent reading's rounding.
const BOUND_RATIO: f64 = 1.5;
const BOUND_SLACK_PP: f64 = 1.0;
/// Nothing is skipped before this many real intervals (`k` is still the prior).
const BOUND_MIN_INTERVALS: u64 = 3;
/// Per-interval decay of the recent over-bound count; an over-bound interval
/// is skipped only while that count is under 1. So spikes 3 intervals apart
/// are skipped, and a run's third, or an every-other pattern, is taken: the
/// fit still follows a lasting change of `k`.
const BOUND_MEMORY: f64 = 0.75;

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
            over_bound: 0.0,
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
            per_model: None,
        }
    }

    /// Add one interval: attributed cost `d_cost`, `d_hours` wall hours, `d_pct`
    /// observed pp. Returns whether the fit took it: an invalid interval, or
    /// one over the bound (§3.3), is skipped, and the per-model fit
    /// ([`FamilyCalibrator::update`]) skips it too.
    pub fn update(&mut self, d_cost: f64, d_hours: f64, d_pct: f64) -> bool {
        if !(d_cost.is_finite() && d_hours.is_finite() && d_pct.is_finite())
            || d_cost < 0.0
            || d_hours <= 0.0
            || d_pct < 0.0
        {
            return false;
        }
        let recent = self.over_bound * BOUND_MEMORY;
        let over = self.intervals >= BOUND_MIN_INTERVALS && d_cost * self.a > BOUND_RATIO * d_pct + BOUND_SLACK_PP;
        self.over_bound = recent + f64::from(u8::from(over));
        if over && recent < 1.0 {
            return false;
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
        true
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

const NF: usize = FAMILIES.len();
const NV: usize = NF + 1;

/// FR-814 per-model weights: `ΔW ≈ Σ_f ΔA_f / k_f + u · Δt`, fitted like
/// [`Calibrator`] (exponentially weighted least squares) over the cost of
/// each family. The statistics keep every family; a family with fewer than
/// [`MIN_FAMILY_SAMPLES`] samples this week is pooled with the others at solve
/// time (pooling sums its rows and columns, which is exact), so its weight is
/// the shared one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FamilyCalibrator {
    s_xx: [[f64; NV]; NV],
    s_xy: [f64; NV],
    decay: f64,
    /// Samples with cost this week, per family (reset at each week).
    pub counts: [u32; NF],
}

impl FamilyCalibrator {
    pub fn new(decay: f64) -> Self {
        FamilyCalibrator { s_xx: [[0.0; NV]; NV], s_xy: [0.0; NV], decay: decay.clamp(0.01, 1.0), counts: [0; NF] }
    }

    /// Add one interval the shared fit took ([`Calibrator::update`] returned `true`).
    pub fn update(&mut self, d_cost: [f64; NF], d_hours: f64, d_pct: f64) {
        if d_cost.iter().any(|c| !c.is_finite() || *c < 0.0) || !(d_hours > 0.0 && d_pct.is_finite() && d_pct >= 0.0) {
            return;
        }
        let mut x = [0.0; NV];
        x[..NF].copy_from_slice(&d_cost);
        x[NF] = d_hours;
        for i in 0..NV {
            for j in 0..NV {
                self.s_xx[i][j] = self.s_xx[i][j] * self.decay + x[i] * x[j];
            }
            self.s_xy[i] = self.s_xy[i] * self.decay + x[i] * d_pct;
        }
    }

    pub fn new_week(&mut self) {
        self.counts = [0; NF];
    }

    /// `k` per family, given the shared fit (`k`, `u`): a family with enough
    /// samples gets its own, the rest the shared one.
    pub fn solve(&self, shared_k: f64) -> [Option<f64>; NF] {
        let own: Vec<usize> = (0..NF).filter(|f| self.counts[*f] >= MIN_FAMILY_SAMPLES).collect();
        let mut out = [None; NF];
        if own.is_empty() {
            return out;
        }
        // Reduced columns: each own family, then the pool of the others, then hours.
        let col = |v: usize| -> usize {
            if v == NF { own.len() + 1 } else { own.iter().position(|f| *f == v).unwrap_or(own.len()) }
        };
        let n = own.len() + 2;
        let mut m = vec![vec![0.0; n]; n];
        let mut b = vec![0.0; n];
        for i in 0..NV {
            b[col(i)] += self.s_xy[i];
            for j in 0..NV {
                m[col(i)][col(j)] += self.s_xx[i][j];
            }
        }
        // Weak priors, as for the shared fit: every cost column at the shared
        // `k`, hours at 0 pp.
        let pa = shared_k.max(1e-9) * PRIOR_PP;
        for c in 0..n - 1 {
            m[c][c] += PRIOR_WEIGHT * pa * pa;
            b[c] += PRIOR_WEIGHT * pa * PRIOR_PP;
        }
        m[n - 1][n - 1] += PRIOR_WEIGHT * PRIOR_HOURS * PRIOR_HOURS;
        let Some(a) = solve_linear(m, b) else { return out };
        for (i, f) in own.iter().enumerate() {
            if a[i] > 0.0 && a[i].is_finite() {
                out[*f] = Some(1.0 / a[i]);
            }
        }
        out
    }
}

/// Gaussian elimination with partial pivoting.
fn solve_linear(mut m: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|x, y| m[*x][c].abs().total_cmp(&m[*y][c].abs()))?;
        if m[p][c].abs() < 1e-12 {
            return None;
        }
        m.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = m[r][c] / m[c][c];
            let (top, rest) = m.split_at_mut(r);
            for (x, p) in rest[0][c..n].iter_mut().zip(&top[c][c..n]) {
                *x -= f * p;
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| m[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / m[r][r];
    }
    Some(x)
}

/// The most recent observed value of one window, across all agents.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub ts: Timestamp,
    pub used_pct: f64,
    pub resets_at: Timestamp,
    /// The app's cumulative attributed cost when this value was observed.
    pub app_cost: f64,
    /// The same, raw, per model family ([`FAMILIES`]): the per-model fit's input.
    #[serde(default)]
    pub family_cost: [f64; NF],
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
    fn per_family_k_once_a_family_has_enough_samples() {
        // Truth: opus 30 cost/pp, sonnet 6 cost/pp, u = 0.1 pp/h.
        let mut f = FamilyCalibrator::new(0.995);
        let mixes = [(30.0, 6.0, 1.0), (60.0, 0.0, 2.0), (0.0, 12.0, 1.5), (15.0, 18.0, 3.0), (90.0, 3.0, 0.5)];
        for _ in 0..40 {
            for &(o, s, h) in &mixes {
                let pct = o / 30.0 + s / 6.0 + 0.1 * h;
                f.update([o, s, 0.0, 0.0, 0.0], h, pct);
            }
        }
        assert_eq!(f.solve(12.0), [None; 5], "no family has enough samples yet");
        f.counts = [25, 25, 0, 0, 3];
        let k = f.solve(12.0);
        assert!((k[0].unwrap() - 30.0).abs() < 1.5, "opus {:?}", k[0]);
        assert!((k[1].unwrap() - 6.0).abs() < 0.5, "sonnet {:?}", k[1]);
        assert_eq!((k[2], k[4]), (None, None), "pooled families use the shared k");
        f.new_week();
        assert_eq!(f.counts, [0; 5]);
        assert_eq!(family_index(Some("claude-opus-4-1")), 0);
        assert_eq!(family_index(Some("Sonnet 4.5")), 1);
        assert_eq!(family_index(None), 4);
    }

    /// The §2.2 week: agents only, true `k` 20 $/pp, 6.6 pp of 95 outside the
    /// app, intervals of 3 to 5 pp over 0.5 to 14 h. `spikes`: intervals whose
    /// cost carries a double-counted `spike` $ (a resumed session's past
    /// cost). Returns the fit and its outside estimate `W − app cost / k`
    /// (the true app cost, as the repaired rows give it).
    fn spiked_week(spikes: &[usize], spike: f64) -> (Calibrator, f64) {
        let (pcts, hours) = ([3.0, 3.0, 4.0, 3.0, 5.0], [0.5, 8.0, 2.0, 14.0, 1.0, 5.0, 3.0]);
        let n = 28;
        let raw: f64 = (0..n).map(|i| pcts[i % pcts.len()]).sum();
        let total_h: f64 = (0..n).map(|i| hours[i % hours.len()]).sum();
        let u = 6.6 / total_h;
        let mut c = Calibrator::new(1.0, 0.85);
        let (mut w, mut app) = (0.0, 0.0);
        for i in 0..n {
            let (dp, dh) = (pcts[i % pcts.len()] * 95.0 / raw, hours[i % hours.len()]);
            let cost = 20.0 * (dp - u * dh);
            let extra = if spikes.contains(&i) { spike } else { 0.0 };
            c.update(cost + extra, dh, dp);
            w += dp;
            app += cost;
        }
        let outside = w - app / c.cost_per_pct();
        (c, outside)
    }

    /// A fit at `k` = 20 (60 $ per 3 pp each hour), `n` intervals.
    fn warm(n: usize) -> Calibrator {
        let mut c = Calibrator::new(20.0, 0.85);
        for _ in 0..n {
            assert!(c.update(60.0, 1.0, 3.0));
        }
        c
    }

    /// Acceptance 4: three double-counted intervals keep `k` within 10 % of
    /// 20 and the outside estimate within 3 pp of 6.6, wherever they fall
    /// (3 intervals apart or more: a restart every few hours).
    #[test]
    fn a_double_counted_interval_cannot_capture_k() {
        let (clean, clean_out) = spiked_week(&[], 0.0);
        assert!((clean.cost_per_pct() - 20.0).abs() <= 2.0 && (clean_out - 6.6).abs() <= 3.0);
        for spikes in [[9, 15, 21], [5, 12, 26], [8, 17, 25], [19, 22, 27]] {
            for spike in [80.0, 120.0, 400.0] {
                let (c, out) = spiked_week(&spikes, spike);
                let k = c.cost_per_pct();
                assert!((k - 20.0).abs() <= 2.0, "{spikes:?} +{spike}: k = {k}");
                assert!((out - 6.6).abs() <= 3.0, "{spikes:?} +{spike}: outside {out}");
                assert_eq!(c.intervals(), 25, "the three spikes are skipped");
            }
        }
    }

    /// Over-bound intervals in a row look like a lasting change: the third
    /// is taken (and, being a spike here, moves `k`).
    #[test]
    fn spikes_in_a_row_are_a_lasting_change_and_are_taken() {
        let (c, _) = spiked_week(&[3, 4, 5], 400.0);
        assert_eq!(c.intervals(), 26, "the third spike in a row is taken");
        assert!(c.cost_per_pct() > 22.0, "k = {}", c.cost_per_pct());
    }

    /// The ratio and the slack: an interval just under the bound is taken,
    /// one just over it skipped.
    #[test]
    fn the_bound_is_ratio_times_the_increase_plus_slack() {
        let warm = || warm(5);
        let k = warm().cost_per_pct();
        // ΔW = 4 pp: the bound is (1.5 · 4 + 1) · k = 7 k.
        assert!(warm().update(6.9 * k, 2.0, 4.0), "just under the bound");
        let mut c = warm();
        assert!(!c.update(7.1 * k, 2.0, 4.0), "just over the bound");
        assert_eq!((c.intervals(), c.cost_per_pct()), (5, k), "a skipped interval leaves the fit as it was");
    }

    /// Early in the life of the fit, the prior (`k0 = 1`) is far off: nothing
    /// is skipped before `BOUND_MIN_INTERVALS` real intervals.
    #[test]
    fn nothing_is_skipped_while_the_fit_is_the_prior() {
        let mut c = Calibrator::new(1.0, 0.85);
        for _ in 0..3 {
            assert!(c.update(60.0, 1.0, 3.0), "k0 = 1 says 60 $ is 60 pp");
        }
        assert!((c.cost_per_pct() - 20.0).abs() < 4.0, "k = {}", c.cost_per_pct());
        assert!(!c.update(400.0, 1.0, 3.0), "the bound now applies");
    }

    /// A costlier interval every other one (a model whose cost per pp is
    /// twice the blended `k`) is a pattern, not a spike: it is taken after
    /// a few, so `k` does not stay at the cheap intervals' value.
    #[test]
    fn over_bound_intervals_every_other_one_are_taken() {
        let mut c = warm(5);
        let mut taken = 0;
        for _ in 0..10 {
            taken += c.update(120.0, 1.0, 3.0) as u32;
            c.update(60.0, 1.0, 3.0);
        }
        assert!(taken >= 7, "{taken} of 10 taken");
    }

    /// A lasting change of `k` (3×) is still learned: a run over the bound
    /// is taken from its third interval, and the bound follows the fit.
    #[test]
    fn a_lasting_change_of_k_is_followed() {
        let mut c = warm(10);
        for _ in 0..30 {
            c.update(180.0, 1.0, 3.0);
        }
        assert!((c.cost_per_pct() - 60.0).abs() < 6.0, "k = {}", c.cost_per_pct());
    }

    #[test]
    fn ignores_invalid_intervals() {
        let mut c = Calibrator::new(10.0, 0.9);
        assert!(!c.update(-1.0, 1.0, 3.0));
        assert!(!c.update(1.0, 0.0, 3.0));
        assert!(!c.update(f64::NAN, 1.0, 3.0));
        assert_eq!(c.intervals(), 0);
    }
}
