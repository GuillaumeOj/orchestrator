//! An intake item's estimated cost (P4.5 spec §3.4, api-contract §2.2, D5):
//! a range in pp of the week per mode, from the cost of finished team runs
//! and ended solo sessions of the last 8 weeks (converted with the current
//! calibration `k`), per project when it has ≥ 3 samples of the mode, else
//! across projects, else the defaults. Scaled by the item's size; compared
//! with the room left this week. Advice only: it never blocks a start.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use governor::AllocationId;
use governor::jiff::Timestamp;
use orch_protocol::round1;
use orch_store::UsageSource;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::state::AgentState;

/// How far back the history reaches.
pub const HISTORY_MS: u64 = 8 * 7 * 24 * 3600 * 1000;
/// Samples a project needs before its own history is used.
pub const MIN_PROJECT_SAMPLES: usize = 3;
/// The calibration moved this much (relative): the history is recomputed.
pub const K_MOVE: f64 = 0.10;
/// Global settings (F16): the default estimates, pp of the week.
pub const DEFAULT_TEAM_KEY: &str = "intake.default_team_pp";
pub const DEFAULT_SOLO_KEY: &str = "intake.default_solo_pp";
pub const DEFAULT_TEAM_PP: f64 = 6.0;
pub const DEFAULT_SOLO_PP: f64 = 1.5;
/// One sample or the defaults: ± this share.
const SPREAD: f64 = 0.4;

/// Each mode's past costs (pp of the week), by project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples {
    pub team: BTreeMap<String, Vec<f64>>,
    pub solo: BTreeMap<String, Vec<f64>>,
}

impl Samples {
    fn of(&self, mode: &str) -> &BTreeMap<String, Vec<f64>> {
        if mode == "team" { &self.team } else { &self.solo }
    }
}

/// What is left this week for a project.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub pp: f64,
    /// `allocation` | `week`.
    pub scope: &'static str,
}

/// The estimator's inputs, kept between refreshes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EstState {
    /// The calibration the samples were converted with.
    pub k: f64,
    pub samples: Arc<Samples>,
    pub rooms: BTreeMap<String, Room>,
    pub default_team_pp: f64,
    pub default_solo_pp: f64,
}

/// Linear-interpolated percentile of sorted `v`.
fn pct(v: &[f64], p: f64) -> f64 {
    let at = (v.len() - 1) as f64 * p;
    let (lo, hi) = (at.floor() as usize, at.ceil() as usize);
    v[lo] + (v[hi] - v[lo]) * (at - lo as f64)
}

/// p25–p75 of `samples` (± 40 % around a single one).
pub fn range(samples: &[f64]) -> Option<(f64, f64)> {
    let mut v: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite() && *x > 0.0).collect();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => None,
        1 => Some((v[0] * (1.0 - SPREAD), v[0] * (1.0 + SPREAD))),
        _ => Some((pct(&v, 0.25), pct(&v, 0.75))),
    }
}

/// S × 0.5, M × 1, L × 2 (unknown = M).
pub fn size_factor(size: Option<&str>) -> f64 {
    match size {
        Some("S") => 0.5,
        Some("L") => 2.0,
        _ => 1.0,
    }
}

/// One mode's `{low_pp, high_pp, basis, samples}` for `project`.
pub fn mode_estimate(s: &Samples, project: &str, mode: &str, default_pp: f64, factor: f64) -> Value {
    let by = s.of(mode);
    let own = by.get(project).map(Vec::as_slice).unwrap_or(&[]);
    let all: Vec<f64> = by.values().flatten().copied().collect();
    let (basis, n, r) = if own.len() >= MIN_PROJECT_SAMPLES {
        ("project", own.len(), range(own))
    } else if !all.is_empty() {
        ("global", all.len(), range(&all))
    } else {
        ("default", 0, None)
    };
    let (lo, hi) = r.unwrap_or((default_pp * (1.0 - SPREAD), default_pp * (1.0 + SPREAD)));
    json!({"low_pp": round1(lo * factor), "high_pp": round1(hi * factor), "basis": basis, "samples": n})
}

/// The item's `Estimate` (api-contract §2.2, F6, F7).
pub fn estimate(st: &EstState, project: &str, size: Option<&str>, suggested: &str) -> Value {
    let f = size_factor(size);
    let team = mode_estimate(&st.samples, project, "team", st.default_team_pp, f);
    let solo = mode_estimate(&st.samples, project, "solo", st.default_solo_pp, f);
    let room = st.rooms.get(project);
    let high = if suggested == "team" { &team } else { &solo }["high_pp"].as_f64().unwrap_or(0.0);
    json!({
        "team": team,
        "solo": solo,
        "size": size.unwrap_or("M"),
        "room_pp": room.map(|r| r.pp),
        "room_scope": room.map(|r| r.scope),
        "over_room": room.is_some_and(|r| high > r.pp),
    })
}

/// A default estimate setting, clamped to 0.5–50 pp (F16).
fn default_pp(rt: &Runtime, key: &str, fallback: f64) -> f64 {
    rt.db.setting(key).ok().flatten().and_then(|v| v.as_f64()).unwrap_or(fallback).clamp(0.5, 50.0)
}

impl Runtime {
    /// The current calibration `k` (cost per 1 % of the week).
    fn intake_k(&self) -> f64 {
        let now = Timestamp::now();
        self.with_governor(|g| g.calibration(now).cost_per_weekly_pct).max(1e-9)
    }

    /// The history of the last 8 weeks, in pp with `k`.
    pub(crate) fn intake_samples(&self, k: f64, now_ms: u64) -> Samples {
        let since = now_ms.saturating_sub(HISTORY_MS);
        let usage = self.db.usage_between(since, None, Some(UsageSource::Statusline)).unwrap_or_default();
        // Each agent's costs, with their time.
        let mut cost: BTreeMap<&str, Vec<(u64, f64)>> = BTreeMap::new();
        for u in &usage {
            let Some(a) = u.agent_id.as_deref().filter(|a| *a != crate::overwatch::ID) else { continue };
            let c = u.cost_delta.unwrap_or(0.0);
            if c.is_finite() && c > 0.0 {
                cost.entry(a).or_default().push((u.ts_ms, c));
            }
        }
        let sum = |agent: &str, from: u64, to: u64| -> f64 {
            cost.get(agent).map_or(0.0, |v| v.iter().filter(|(t, _)| *t >= from && *t <= to).map(|(_, c)| c).sum())
        };
        let agents: Vec<crate::store::AgentRecord> = self.store.read(|s| s.agents.values().cloned().collect());
        let mut out = Samples::default();
        // Team runs that finished (done) in the window: their members' cost.
        for run in self.db.runs(None, false).unwrap_or_default() {
            let Some(end) = run.ended_at_ms.filter(|e| *e >= since) else { continue };
            if run.status != orch_team::RunState::Done {
                continue;
            }
            let Ok(Some(f)) = self.db.feature(&run.feature_id) else { continue };
            let mut members: BTreeSet<String> = agents
                .iter()
                .filter(|a| a.team.as_ref().is_some_and(|t| t.run_id == run.id))
                .map(|a| a.id.clone())
                .collect();
            members
                .extend(self.db.run_events(&run.id, 5000).unwrap_or_default().into_iter().filter_map(|e| e.agent_id));
            let c: f64 = members.iter().map(|m| sum(m, run.started_at_ms, end)).sum();
            if c > 0.0 {
                out.team.entry(f.project_id).or_default().push(c / k);
            }
        }
        // Solo sessions that ended (deleted or killed) after a turn that cost something.
        for a in agents.iter().filter(|a| a.team.is_none() && !a.is_overwatch()) {
            if !matches!(a.tracker.state, AgentState::Archived | AgentState::Killed) || self.live(&a.id).is_some() {
                continue;
            }
            let c = sum(&a.id, since, u64::MAX);
            if c > 0.0
                && let Some(p) = self.agent_project(a)
            {
                out.solo.entry(p).or_default().push(c / k);
            }
        }
        out
    }

    /// What is left this week for each project: its allocation's plan for
    /// the week minus its use, else the week's agent budget left.
    pub(crate) fn intake_rooms(&self) -> BTreeMap<String, Room> {
        let ids: Vec<String> = self.db.projects(false).unwrap_or_default().into_iter().map(|p| p.id).collect();
        let now = Timestamp::now();
        self.with_governor(|g| {
            let plan = g.plan();
            let budget = (100.0 - plan.reserve_pct).max(0.0);
            let weights: f64 = plan.allocations.iter().map(|a| a.weight).sum::<f64>().max(1e-9);
            let week_left = (budget - g.snapshot(now).used_central).max(0.0);
            ids.into_iter()
                .map(|id| {
                    let wanted = AllocationId(id.clone());
                    let charged = g.resolve_allocation(&wanted).clone();
                    let own = plan.allocations.iter().find(|a| a.id == charged && charged.0 != "default");
                    let room = match own {
                        Some(a) => Room {
                            pp: round1((budget * a.weight / weights - g.allocation_usage_pct(&a.id)).max(0.0)),
                            scope: "allocation",
                        },
                        None => Room { pp: round1(week_left), scope: "week" },
                    };
                    (id, room)
                })
                .collect()
        })
    }

    /// Recomputes the history (on a refresh), or only when the calibration
    /// moved by ≥ 10 % (`force` false); the rooms every time. Touches the
    /// topic when the estimates changed.
    pub(crate) fn intake_estimates_update(&self, force: bool) {
        use std::sync::atomic::Ordering;
        // A refresh of each project at start: the history is read once.
        let now = orch_protocol::now_ms();
        let force = force && now.saturating_sub(self.intake.est_at.load(Ordering::SeqCst)) >= 60_000;
        let k = self.intake_k();
        let old = self.intake.est.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let moved = old.as_ref().is_none_or(|o| ((k - o.k) / o.k.max(1e-9)).abs() >= K_MOVE);
        let (k, samples) = match &old {
            Some(o) if !force && !moved => (o.k, o.samples.clone()),
            _ => {
                self.intake.est_at.store(now, Ordering::SeqCst);
                (k, Arc::new(self.intake_samples(k, now)))
            }
        };
        let st = EstState {
            k,
            samples,
            rooms: self.intake_rooms(),
            default_team_pp: default_pp(self, DEFAULT_TEAM_KEY, DEFAULT_TEAM_PP),
            default_solo_pp: default_pp(self, DEFAULT_SOLO_KEY, DEFAULT_SOLO_PP),
        };
        if old.as_deref() != Some(&st) {
            *self.intake.est.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(st));
            self.db.note([super::TOPIC]);
        }
    }

    /// The estimator's state, computed on first use.
    pub(crate) fn intake_est(&self) -> Arc<EstState> {
        if let Some(s) = self.intake.est.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return s;
        }
        self.intake_estimates_update(true);
        self.intake.est.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(team: &[(&str, &[f64])], solo: &[(&str, &[f64])]) -> EstState {
        let m = |v: &[(&str, &[f64])]| v.iter().map(|(p, s)| (p.to_string(), s.to_vec())).collect();
        EstState {
            k: 1.0,
            samples: Arc::new(Samples { team: m(team), solo: m(solo) }),
            rooms: BTreeMap::new(),
            default_team_pp: 6.0,
            default_solo_pp: 1.5,
        }
    }

    #[test]
    fn project_history_then_global_then_defaults() {
        let s = st(&[("shop", &[4.0, 5.0, 6.0, 8.0]), ("api", &[10.0])], &[]);
        let e = estimate(&s, "shop", None, "team");
        assert_eq!(e["team"], json!({"low_pp": 4.8, "high_pp": 6.5, "basis": "project", "samples": 4}));
        assert_eq!(e["size"], "M");
        // api has one sample of its own: the five across projects.
        let e = estimate(&s, "api", None, "team");
        assert_eq!((e["team"]["basis"].as_str(), e["team"]["samples"].as_u64()), (Some("global"), Some(5)));
        // No solo history anywhere: the default ± 40 %.
        assert_eq!(e["solo"], json!({"low_pp": 0.9, "high_pp": 2.1, "basis": "default", "samples": 0}));
    }

    #[test]
    fn size_scales_and_room_warns() {
        let mut s = st(&[], &[("shop", &[1.0])]);
        s.rooms.insert("shop".into(), Room { pp: 3.0, scope: "allocation" });
        let l = estimate(&s, "shop", Some("L"), "team");
        assert_eq!(l["team"], json!({"low_pp": 7.2, "high_pp": 16.8, "basis": "default", "samples": 0}));
        assert_eq!(
            (l["room_pp"].as_f64(), l["room_scope"].as_str(), l["over_room"].as_bool()),
            (Some(3.0), Some("allocation"), Some(true))
        );
        // One solo sample: ± 40 %, halved for S; under the room.
        let s_ = estimate(&s, "shop", Some("S"), "solo");
        assert_eq!(s_["solo"], json!({"low_pp": 0.3, "high_pp": 0.7, "basis": "global", "samples": 1}));
        assert_eq!(s_["over_room"], false);
        assert_eq!(estimate(&s, "other", None, "team")["over_room"], false, "no room known: no warning");
    }

    #[test]
    fn range_is_the_interquartile_span() {
        assert_eq!(range(&[]), None);
        let (lo, hi) = range(&[2.0, 0.0, f64::NAN]).unwrap();
        assert_eq!((round1(lo), round1(hi)), (1.2, 2.8), "one usable sample: ± 40 %");
        assert_eq!(range(&[1.0, 2.0, 3.0]), Some((1.5, 2.5)));
    }
}
