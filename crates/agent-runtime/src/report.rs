//! FR-818 weekly report: plan vs actual per allocation, usage per project,
//! feature, role and model, time in each pressure state, governor actions,
//! alerts and limit hits. Built at each weekly reset (stored, never purged)
//! and on demand for the current week (not stored).
//!
//! Usage is in pp of the week first; `cost_usd` is the statusline's API-price
//! estimate of the same tokens, shown muted as "≈ API $" (decision D7).
//! Attribution: agent → project, team run → feature, node → role, the
//! sample's model → model family. What the weekly reading counts beyond the
//! app's own samples is the "outside the app" row.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use governor::jiff::Timestamp;
use governor::{ActionKind, Calibration, ForecastSnapshot, GovernorAction, PressureState, WeeklyPlan};
use orch_store::UsageRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// One line of a usage breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub key: String,
    pub label: String,
    /// pp of the week.
    pub pp: f64,
    /// The same tokens at API prices (D7: secondary, never a bill).
    pub cost_usd: f64,
    /// Share of the week's total usage, 0–1.
    pub share: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub weekly_pct: f64,
    pub agents_pp: f64,
    pub interactive_pp: f64,
    pub outside_pp: f64,
    pub reserve_pct: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AllocationLine {
    pub id: String,
    pub target: String,
    pub weight: f64,
    pub tier: String,
    pub planned_pp: f64,
    pub used_pp: f64,
    pub delta_pp: f64,
    pub queued_p50_min: Option<f64>,
    pub queued_max_min: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateTime {
    pub state: PressureState,
    pub minutes: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionCount {
    pub kind: String,
    pub count: u32,
    pub undone: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlertLine {
    pub ts: Timestamp,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LimitHit {
    pub window: String,
    pub ts: Timestamp,
    pub resets_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelWindowPeak {
    pub name: String,
    pub family: String,
    pub peak_pct: f64,
}

/// `WeeklyReport` (api-contract §5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeeklyReport {
    pub id: String,
    pub week_start: Timestamp,
    pub reset_at: Timestamp,
    pub in_progress: bool,
    pub provenance: governor::Provenance,
    pub totals: Totals,
    pub allocations: Vec<AllocationLine>,
    pub by_project: Vec<Row>,
    pub by_feature: Vec<Row>,
    pub by_role: Vec<Row>,
    pub by_model: Vec<Row>,
    pub states: Vec<StateTime>,
    pub actions: Vec<ActionCount>,
    pub alerts: Vec<AlertLine>,
    pub limit_hits: Vec<LimitHit>,
    pub model_windows: Vec<ModelWindowPeak>,
    /// "What to change for next week": plain sentences from the numbers above.
    #[serde(default)]
    pub suggestions: Vec<String>,
}

/// Where an agent's usage goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attribution {
    /// `(id, name)`.
    pub project: Option<(String, String)>,
    pub feature: Option<(String, String)>,
    /// The team node's role id; `None` for a solo session.
    pub role: Option<String>,
    /// The role's model, when a sample has none.
    pub model: Option<String>,
}

/// What a report is built from.
pub struct Inputs<'a> {
    pub plan: &'a WeeklyPlan,
    /// The week's statusline samples (`[week_start, reset_at)`).
    pub samples: &'a [UsageRecord],
    pub agents: &'a BTreeMap<String, Attribution>,
    /// Allocation id → project name, for agents gone since.
    pub projects: &'a BTreeMap<String, String>,
    pub calibration: &'a Calibration,
    pub actions: &'a [GovernorAction],
    pub forecasts: &'a [ForecastSnapshot],
    /// Share of the agents' plan that is due by now (1 for a finished week).
    pub plan_fraction: f64,
    /// The weekly % to report when known (the live snapshot for the current
    /// week); else the week's last reading.
    pub weekly_pct: Option<f64>,
    pub in_progress: bool,
    pub now: Timestamp,
}

const OUTSIDE: &str = "outside";
const OUTSIDE_LABEL: &str = "outside the app";

/// `opus`, `sonnet`, `haiku`, `fable` or `other`.
pub fn family(model: Option<&str>) -> &'static str {
    governor::FAMILIES[governor::family_index(model)]
}

fn ms(t: Timestamp) -> u64 {
    t.as_millisecond().max(0) as u64
}

use orch_protocol::round1;

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[derive(Default)]
struct Acc {
    label: String,
    pp: f64,
    usd: f64,
}

fn rows(m: BTreeMap<String, Acc>, outside: f64, total: f64) -> Vec<Row> {
    let mut out: Vec<Row> =
        m.into_iter().map(|(key, a)| Row { key, label: a.label, pp: a.pp, cost_usd: a.usd, share: 0.0 }).collect();
    if outside > 0.0 {
        out.push(Row { key: OUTSIDE.into(), label: OUTSIDE_LABEL.into(), pp: outside, cost_usd: 0.0, share: 0.0 });
    }
    for r in &mut out {
        r.share = if total > 0.0 { round2(r.pp / total) } else { 0.0 };
        r.pp = round1(r.pp);
        r.cost_usd = round2(r.cost_usd);
    }
    out.sort_by(|a, b| b.pp.total_cmp(&a.pp).then_with(|| a.key.cmp(&b.key)));
    out
}

fn percentile(v: &mut [f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    Some(round1(v[i]))
}

pub fn build(i: &Inputs) -> WeeklyReport {
    let plan = i.plan;
    let k = i.calibration.cost_per_weekly_pct.max(1e-9);
    // FR-814 per-model weights: a family with its own fit converts with it.
    let k_of =
        |fam: &str| -> f64 { i.calibration.per_model.as_ref().and_then(|m| m.get(fam)).map_or(k, |m| m.k).max(1e-9) };
    let in_week = |s: &&UsageRecord| s.ts_ms >= ms(plan.week_start) && s.ts_ms < ms(plan.reset_at);
    let (mut by_project, mut by_feature, mut by_role, mut by_model) =
        (BTreeMap::<String, Acc>::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let (mut agents_pp, mut interactive_pp) = (0.0, 0.0);
    let mut alloc_used: BTreeMap<String, f64> = BTreeMap::new();
    let mut last_reading: Option<(u64, f64)> = None;
    let mut peaks: BTreeMap<String, f64> = BTreeMap::new();
    for s in i.samples.iter().filter(in_week) {
        if let (Some(w), Some(r)) = (s.weekly_pct, s.weekly_resets_at) {
            // A reading of this week (not an agent's stale copy of the last one).
            let this_week = Timestamp::from_second(r).is_ok_and(|r| governor::same_reset(r, plan.reset_at));
            if this_week && last_reading.is_none_or(|(t, _)| s.ts_ms >= t) {
                last_reading = Some((s.ts_ms, w));
            }
        }
        for mw in &s.model_windows {
            let p = peaks.entry(mw.name.clone()).or_insert(0.0);
            *p = p.max(mw.used_pct);
        }
        let Some(agent) = &s.agent_id else { continue };
        let cost = s.cost_delta.unwrap_or(0.0).max(0.0);
        if cost <= 0.0 || !cost.is_finite() {
            continue;
        }
        let at = i.agents.get(agent);
        let fam = family(s.model.as_deref().or(at.and_then(|a| a.model.as_deref())));
        let pp = cost / k_of(fam);
        let class = s.class.as_deref().unwrap_or("interactive");
        if class == "interactive" {
            interactive_pp += pp;
        } else {
            agents_pp += pp;
            if let Some(a) = &s.allocation {
                *alloc_used.entry(a.clone()).or_insert(0.0) += pp;
            }
        }
        let add = |m: &mut BTreeMap<String, Acc>, key: String, label: String| {
            let e = m.entry(key).or_insert_with(|| Acc { label, ..Default::default() });
            e.pp += pp;
            e.usd += cost;
        };
        let project = at.and_then(|a| a.project.clone()).or_else(|| {
            let a = s.allocation.as_ref()?;
            Some((a.clone(), i.projects.get(a).cloned()?))
        });
        let (pk, pl) = project.unwrap_or_else(|| ("unknown".into(), "unknown project".into()));
        add(&mut by_project, pk, pl);
        let (fk, fl) = at.and_then(|a| a.feature.clone()).unwrap_or_else(|| ("solo".into(), "solo sessions".into()));
        add(&mut by_feature, fk, fl);
        let role = at.and_then(|a| a.role.clone());
        let (rk, rl) = role.map_or_else(|| ("solo".into(), "solo session".into()), |r| (r.clone(), r));
        add(&mut by_role, rk, rl);
        add(&mut by_model, fam.into(), fam.into());
    }
    let app_pp = agents_pp + interactive_pp;
    let observed = last_reading.is_some();
    let weekly_pct = i.weekly_pct.or(last_reading.map(|(_, w)| w)).unwrap_or_else(|| {
        // No reading this week: the app's own usage plus the unattributed rate.
        let hours = (ms(i.now.min(plan.reset_at)).saturating_sub(ms(plan.week_start))) as f64 / 3.6e6;
        app_pp + i.calibration.unattributed_rate * hours
    });
    let outside_pp = (weekly_pct - app_pp).max(0.0);
    let total = weekly_pct.max(app_pp);

    // Plan vs actual: each allocation's share of the agents' budget by now.
    let budget = (100.0 - plan.reserve_pct).max(0.0) * i.plan_fraction.clamp(0.0, 1.0);
    let weights: f64 = plan.allocations.iter().map(|a| a.weight).sum::<f64>().max(1e-9);
    let waits = queue_waits(i.actions, i.samples);
    let allocations = plan
        .allocations
        .iter()
        .map(|a| {
            let planned = budget * a.weight / weights;
            let used = alloc_used.get(&a.id.0).copied().unwrap_or(0.0);
            let mut w = waits.get(&a.id.0).cloned().unwrap_or_default();
            AllocationLine {
                id: a.id.0.clone(),
                target: a.target.clone(),
                weight: a.weight,
                tier: orch_store::tag(&a.tier),
                planned_pp: round1(planned),
                used_pp: round1(used),
                delta_pp: round1(used - planned),
                queued_max_min: w.iter().copied().reduce(f64::max).map(round1),
                queued_p50_min: percentile(&mut w, 0.5),
            }
        })
        .collect();

    // Time in each pressure state, from the 5-min forecast snapshots (a gap
    // over 15 min, the app not running, counts nothing).
    let mut minutes: BTreeMap<PressureState, f64> = BTreeMap::new();
    let snaps: Vec<&ForecastSnapshot> =
        i.forecasts.iter().filter(|f| f.ts >= plan.week_start && f.ts < plan.reset_at).collect();
    for w in snaps.windows(2) {
        let dt = (w[1].ts.as_millisecond() - w[0].ts.as_millisecond()) as f64 / 60_000.0;
        if dt > 0.0 && dt <= 15.0 {
            *minutes.entry(w[0].state).or_insert(0.0) += dt;
        }
    }
    let states = [PressureState::Green, PressureState::Amber, PressureState::Red, PressureState::CatchUp]
        .into_iter()
        .map(|s| StateTime { state: s, minutes: minutes.get(&s).copied().unwrap_or(0.0).round() })
        .collect();

    let mut counts: BTreeMap<&'static str, (u32, u32)> = BTreeMap::new();
    let (mut alerts, mut limit_hits) = (vec![], vec![]);
    for a in i.actions.iter().filter(|a| a.ts >= plan.week_start && a.ts < plan.reset_at) {
        let c = counts.entry(a.kind_name()).or_insert((0, 0));
        c.0 += 1;
        c.1 += a.undone_at.is_some() as u32;
        match &a.action {
            ActionKind::Alert { .. } | ActionKind::ReserveExhausted => {
                alerts.push(AlertLine { ts: a.ts, text: a.reason.clone() })
            }
            ActionKind::LimitReached { window, resets_at } => {
                limit_hits.push(LimitHit { window: orch_store::tag(window), ts: a.ts, resets_at: *resets_at })
            }
            _ => {}
        }
    }
    let mut actions: Vec<ActionCount> =
        counts.into_iter().map(|(k, (count, undone))| ActionCount { kind: k.into(), count, undone }).collect();
    actions.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.kind.cmp(&b.kind)));

    let mut r = WeeklyReport {
        id: plan.id.clone(),
        week_start: plan.week_start,
        reset_at: plan.reset_at,
        in_progress: i.in_progress,
        provenance: if observed { governor::Provenance::Observed } else { governor::Provenance::Estimated },
        totals: Totals {
            weekly_pct: round1(weekly_pct),
            agents_pp: round1(agents_pp),
            interactive_pp: round1(interactive_pp),
            outside_pp: round1(outside_pp),
            reserve_pct: plan.reserve_pct,
        },
        allocations,
        by_project: rows(by_project, outside_pp, total),
        by_feature: rows(by_feature, outside_pp, total),
        by_role: rows(by_role, outside_pp, total),
        by_model: rows(by_model, outside_pp, total),
        states,
        actions,
        alerts,
        limit_hits,
        model_windows: peaks
            .into_iter()
            .map(|(name, peak)| ModelWindowPeak { family: family(Some(&name)).into(), name, peak_pct: peak })
            .collect(),
        suggestions: vec![],
    };
    r.suggestions = suggestions(&r);
    r
}

/// Rules of thumb over a report (pp first; never dollars).
pub fn suggestions(r: &WeeklyReport) -> Vec<String> {
    let mut out = vec![];
    let t = &r.totals;
    for a in &r.allocations {
        let waited = a.queued_p50_min.is_some_and(|m| m >= 30.0);
        if a.delta_pp <= -5.0 && waited {
            out.push(format!(
                "Raise {}'s weight: it used {:.1} of {:.1} planned pp and its work waited {:.0} min (median).",
                a.target,
                a.used_pp,
                a.planned_pp,
                a.queued_p50_min.unwrap_or(0.0)
            ));
        } else if a.delta_pp >= 5.0 {
            out.push(format!(
                "{} used {:.1} pp over its plan: lower its weight or give it a hard cap.",
                a.target, a.delta_pp
            ));
        }
    }
    if t.interactive_pp > t.reserve_pct + 2.0 {
        out.push(format!(
            "Your own work used {:.1} pp, more than the {:.0} % reserve: raise the reserve.",
            t.interactive_pp, t.reserve_pct
        ));
    }
    if t.outside_pp >= 15.0 {
        out.push(format!(
            "{:.0} pp of the week was used outside the app (claude.ai, other sessions): plan for it with a larger reserve.",
            t.outside_pp
        ));
    }
    let red: f64 = r.states.iter().filter(|s| s.state == PressureState::Red).map(|s| s.minutes).sum();
    if red >= 600.0 {
        out.push(format!(
            "The week spent {:.0} h in Red: lower some weights, or schedule long work off-hours.",
            red / 60.0
        ));
    }
    for w in r.model_windows.iter().filter(|w| w.peak_pct >= 90.0) {
        out.push(format!(
            "The {} window peaked at {:.0} %: give long-running roles a downgrade to a lighter model.",
            w.family, w.peak_pct
        ));
    }
    if r.limit_hits.iter().any(|h| h.window == "weekly") {
        out.push("The weekly limit was reached: lower the agents' share or the weights.".into());
    }
    if !r.in_progress && t.weekly_pct > 0.0 && t.weekly_pct < 60.0 {
        out.push(format!(
            "Only {:.0} % of the week was used: there is room for more agent work (higher live caps or weights).",
            t.weekly_pct
        ));
    }
    out
}

/// Minutes each queued turn waited, per allocation: a `queue` action paired
/// with the `release` of the same entry (ids restart at each boot, so the
/// latest queue of that entry and agent before the release).
fn queue_waits(actions: &[GovernorAction], samples: &[UsageRecord]) -> BTreeMap<String, Vec<f64>> {
    // The allocation of an agent: from its samples (agents may be gone).
    let mut alloc_of: BTreeMap<&str, &str> = BTreeMap::new();
    for s in samples {
        if let (Some(a), Some(al)) = (&s.agent_id, &s.allocation) {
            alloc_of.insert(a, al);
        }
    }
    let mut queued: BTreeMap<(u64, &str), Timestamp> = BTreeMap::new();
    let mut out: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for a in actions {
        match &a.action {
            ActionKind::Queue { entry: Some(e), agent } => {
                queued.insert((e.0, agent.0.as_str()), a.ts);
            }
            ActionKind::Release { entry, agent } => {
                if let Some(t) = queued.remove(&(entry.0, agent.0.as_str())) {
                    let min = (a.ts.as_millisecond() - t.as_millisecond()) as f64 / 60_000.0;
                    let alloc = alloc_of.get(agent.0.as_str()).copied().unwrap_or(crate::runtime::DEFAULT_ALLOCATION);
                    out.entry(alloc.to_string()).or_default().push(min.max(0.0));
                }
            }
            _ => {}
        }
    }
    out
}

/// The Inbox payload of a "Weekly report ready" digest item (level 4).
pub fn inbox_payload(id: &str) -> Value {
    json!({"kind": "info", "digest": "weekly_report", "report": id})
}

impl Runtime {
    /// Who each agent of the store is, for attribution.
    fn attributions(&self) -> BTreeMap<String, Attribution> {
        let projects: BTreeMap<String, String> =
            self.db.projects(true).unwrap_or_default().into_iter().map(|p| (p.id, p.name)).collect();
        let features: BTreeMap<String, String> =
            self.db.features(None).unwrap_or_default().into_iter().map(|f| (f.id, f.title)).collect();
        let agents: Vec<crate::store::AgentRecord> = self.store.read(|s| s.agents.values().cloned().collect());
        agents
            .into_iter()
            .map(|a| {
                let project = self.agent_project(&a).map(|p| (p.clone(), projects.get(&p).cloned().unwrap_or(p)));
                let feature = a.team.as_ref().map(|t| {
                    (t.feature_id.clone(), features.get(&t.feature_id).cloned().unwrap_or(t.feature_id.clone()))
                });
                let role = a.team.as_ref().map(|_| a.role.name.clone());
                (a.id.clone(), Attribution { project, feature, role, model: a.role.model.clone() })
            })
            .collect()
    }

    /// Builds the report of `plan`'s week from the store.
    pub(crate) fn build_report(
        &self,
        plan: &WeeklyPlan,
        calibration: &Calibration,
        plan_fraction: f64,
        weekly_pct: Option<f64>,
        in_progress: bool,
        now: Timestamp,
    ) -> Result<WeeklyReport> {
        let start = ms(plan.week_start);
        let end = Some(ms(plan.reset_at));
        let samples = self.db.usage_between(start, end, Some(orch_store::UsageSource::Statusline))?;
        let actions = self.db.actions_between(plan.week_start, plan.reset_at)?;
        let forecasts = self.db.forecasts_between(start, end)?;
        let projects: BTreeMap<String, String> =
            self.db.projects(true).unwrap_or_default().into_iter().map(|p| (p.id, p.name)).collect();
        let agents = self.attributions();
        Ok(build(&Inputs {
            plan,
            samples: &samples,
            agents: &agents,
            projects: &projects,
            calibration,
            actions: &actions,
            forecasts: &forecasts,
            plan_fraction,
            weekly_pct,
            in_progress,
            now,
        }))
    }

    /// The report of a week that just ended (FR-818): stored, and a "Weekly
    /// report ready" digest in the Inbox. Once per week (a realigned plan is
    /// the same week).
    pub(crate) fn finish_week(&self, plan: &WeeklyPlan, calibration: &Calibration, now: Timestamp) -> Result<()> {
        if self.db.report_for_reset(plan.reset_at)?.is_some() {
            return Ok(());
        }
        let r = self.build_report(plan, calibration, 1.0, None, false, now)?;
        let has_usage = r.totals.weekly_pct > 0.0 || r.by_model.iter().any(|m| m.pp > 0.0);
        if !has_usage {
            return Ok(()); // a week the app never saw used: nothing to report
        }
        let data = serde_json::to_value(&r)?;
        self.db.save_report(&r.id, r.week_start, r.reset_at, Some(r.totals.weekly_pct), &data)?;
        let body = format!(
            "Weekly report ready\nThe week ended at {:.0} % (agents {:.1} pp, you {:.1} pp, outside the app {:.1} pp).",
            r.totals.weekly_pct, r.totals.agents_pp, r.totals.interactive_pp, r.totals.outside_pp
        );
        self.store.write(|s| s.add_inbox_item(None, "info", body, inbox_payload(&r.id), 0))?;
        self.event("-", "weekly_report", json!({"report": r.id}));
        Ok(())
    }

    /// At start: the report of the last week the app saw, when it was not
    /// running at that week's reset (FR-818). Its `k` is the estimator this
    /// week started with.
    pub(crate) fn backfill_report(&self, now: Timestamp) {
        let plan = self.with_governor(|g| g.plan().clone());
        let start = plan.week_start;
        // The last week the app saw start (its estimator row).
        let Ok(Some((prev_start, _))) = self.db.week_estimator_before(start) else { return };
        let week = governor::jiff::SignedDuration::from_hours(168);
        let prev = WeeklyPlan {
            id: format!("week-{}", prev_start.as_second()),
            week_start: prev_start,
            reset_at: prev_start.checked_add(week).unwrap_or(start).min(start),
            ..plan
        };
        let Ok(Some(e)) = self.db.week_estimator(start) else { return };
        if let Err(err) = self.finish_week(&prev, &e.calibrator.snapshot(start), now) {
            self.event("-", "weekly_report_failed", json!(err.to_string()));
        }
    }

    /// `report_view`: a stored report, or `current` (built now, not stored).
    pub fn report_view(&self, id: &str) -> Result<Value> {
        if id != "current" {
            return self.db.report(id)?.ok_or_else(|| anyhow!("no report {id}"));
        }
        let now = Timestamp::now();
        let (plan, calibration, fraction, used) = self.with_governor(|g| {
            let snap = g.snapshot(now);
            (g.plan().clone(), snap.calibration.clone(), g.curve().fraction(now), snap.used_central)
        });
        Ok(serde_json::to_value(self.build_report(&plan, &calibration, fraction, Some(used), true, now)?)?)
    }

    /// `reports_list`: the current week first, then the stored ones, newest first.
    pub fn reports_list(&self) -> Result<Value> {
        let (plan, used) = self.with_governor(|g| (g.plan().clone(), g.snapshot(Timestamp::now()).used_central));
        let iso = |ms: i64| Timestamp::from_millisecond(ms).map(|t| t.to_string()).unwrap_or_default();
        let mut out = vec![json!({
            "id": "current", "week_start": plan.week_start.to_string(), "reset_at": plan.reset_at.to_string(),
            "final_weekly_pct": (used * 10.0).round() / 10.0, "in_progress": true,
        })];
        for r in self.db.reports()? {
            if r.reset_at_ms == plan.reset_at.as_millisecond() {
                continue;
            }
            out.push(json!({
                "id": r.id, "week_start": iso(r.week_start_ms), "reset_at": iso(r.reset_at_ms),
                "final_weekly_pct": r.final_weekly_pct, "in_progress": false,
            }));
        }
        Ok(json!({"reports": out}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::jiff::SignedDuration;
    use governor::{ActionId, ActionMode, AgentId, AllocationId, BudgetAllocation, Outcome, Tier};

    fn t(h: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000).unwrap().checked_add(SignedDuration::from_hours(h)).unwrap()
    }

    fn plan() -> WeeklyPlan {
        let mut p = crate::runtime::test_plan(t(72), false);
        p.week_start = t(0);
        p.reset_at = t(168);
        p.id = "week-x".into();
        p.reserve_pct = 10.0;
        p.allocations = vec![
            BudgetAllocation {
                id: "shop".into(),
                target: "shop".into(),
                weight: 3.0,
                tier: Tier::Normal,
                hard_cap_pct: None,
            },
            BudgetAllocation {
                id: "api".into(),
                target: "api".into(),
                weight: 1.0,
                tier: Tier::Normal,
                hard_cap_pct: None,
            },
        ];
        p
    }

    fn sample(h: i64, agent: &str, cost: f64, class: &str, alloc: &str, model: &str, w: Option<f64>) -> UsageRecord {
        UsageRecord {
            ts_ms: ms(t(h)),
            agent_id: Some(agent.into()),
            model: Some(model.into()),
            cost_delta: Some(cost),
            weekly_pct: w,
            weekly_resets_at: w.map(|_| t(168).as_second()),
            class: Some(class.into()),
            allocation: Some(alloc.into()),
            provenance: "observed".into(),
            ..Default::default()
        }
    }

    fn action(h: i64, kind: ActionKind) -> GovernorAction {
        GovernorAction {
            id: ActionId(0),
            ts: t(h),
            state: PressureState::Green,
            action: kind,
            reason: "r".into(),
            expected_effect: String::new(),
            mode: ActionMode::Auto,
            outcome: Outcome::Applied,
            undo: None,
            undone_at: None,
            ask: None,
        }
    }

    #[test]
    fn plan_vs_actual_per_model_role_and_outside() {
        let plan = plan();
        let samples = vec![
            sample(1, "A", 20.0, "autonomous", "shop", "claude-opus-4-1", Some(4.0)),
            sample(2, "B", 10.0, "autonomous", "api", "claude-sonnet-4-5", None),
            sample(3, "U", 10.0, "interactive", "shop", "claude-sonnet-4-5", Some(10.0)),
            // Last week's stale copy of the weekly reading: ignored.
            UsageRecord {
                weekly_pct: Some(99.0),
                weekly_resets_at: Some(t(0).as_second()),
                ..sample(4, "B", 0.0, "autonomous", "api", "x", None)
            },
            // After the reset: another week.
            sample(170, "A", 50.0, "autonomous", "shop", "claude-opus-4-1", Some(1.0)),
        ];
        let mut agents = BTreeMap::new();
        agents.insert(
            "A".into(),
            Attribution {
                project: Some(("shop".into(), "Shop".into())),
                feature: Some(("f1".into(), "Checkout".into())),
                role: Some("backend".into()),
                model: None,
            },
        );
        let projects: BTreeMap<String, String> = [("api".to_string(), "API".to_string())].into();
        let cal = Calibration {
            ts: t(100),
            cost_per_weekly_pct: 10.0,
            confidence: 0.8,
            unattributed_rate: 0.0,
            per_model: None,
        };
        let mut q = action(5, ActionKind::Queue { entry: Some(governor::EntryId(1)), agent: AgentId("A".into()) });
        q.state = PressureState::Amber;
        let actions = vec![
            q,
            action(6, ActionKind::Release { entry: governor::EntryId(1), agent: AgentId("A".into()) }),
            action(7, ActionKind::Alert { level_pct: 50.0 }),
            action(8, ActionKind::LimitReached { window: governor::Window::FiveHour, resets_at: t(10) }),
            GovernorAction { undone_at: Some(t(9)), ..action(8, ActionKind::PauseAll) },
        ];
        let r = build(&Inputs {
            plan: &plan,
            samples: &samples,
            agents: &agents,
            projects: &projects,
            calibration: &cal,
            actions: &actions,
            forecasts: &[],
            plan_fraction: 1.0,
            weekly_pct: None,
            in_progress: false,
            now: t(168),
        });
        assert_eq!(r.provenance, governor::Provenance::Observed);
        assert_eq!(
            (r.totals.weekly_pct, r.totals.agents_pp, r.totals.interactive_pp, r.totals.outside_pp),
            (10.0, 3.0, 1.0, 6.0)
        );
        let shop = &r.allocations[0];
        assert_eq!((shop.planned_pp, shop.used_pp, shop.delta_pp), (67.5, 2.0, -65.5));
        assert_eq!((shop.queued_p50_min, shop.queued_max_min), (Some(60.0), Some(60.0)));
        let models: Vec<(&str, f64, f64)> = r.by_model.iter().map(|m| (m.key.as_str(), m.pp, m.cost_usd)).collect();
        assert_eq!(models, [("outside", 6.0, 0.0), ("opus", 2.0, 20.0), ("sonnet", 2.0, 20.0)]);
        let roles: Vec<(&str, f64)> = r.by_role.iter().map(|m| (m.key.as_str(), m.pp)).collect();
        assert_eq!(roles, [("outside", 6.0), ("backend", 2.0), ("solo", 2.0)]);
        assert_eq!(r.by_project.iter().find(|p| p.key == "api").map(|p| p.label.as_str()), Some("API"));
        assert_eq!(r.by_feature.iter().find(|p| p.key == "f1").map(|p| p.share), Some(0.2));
        assert_eq!(r.alerts.len(), 1);
        assert_eq!(r.limit_hits[0].window, "five_hour");
        let pause = r.actions.iter().find(|a| a.kind == "pause_all").unwrap();
        assert_eq!((pause.count, pause.undone), (1, 1));
        // Under 60 % of the week and an allocation 65 pp under plan that waited an hour.
        assert!(r.suggestions.iter().any(|x| x.starts_with("Raise shop's weight")), "{:?}", r.suggestions);
        assert!(r.suggestions.iter().any(|x| x.starts_with("Only 10 %")), "{:?}", r.suggestions);

        // Per-model weights: opus converts with its own k.
        let mut pm = BTreeMap::new();
        pm.insert("opus".to_string(), governor::ModelK { k: 20.0, samples: 30, fitted: true });
        let cal2 = Calibration { per_model: Some(pm), ..cal };
        let r2 = build(&Inputs {
            calibration: &cal2,
            ..Inputs {
                plan: &plan,
                samples: &samples,
                agents: &agents,
                projects: &projects,
                calibration: &cal,
                actions: &[],
                forecasts: &[],
                plan_fraction: 0.5,
                weekly_pct: Some(12.0),
                in_progress: true,
                now: t(84),
            }
        });
        assert_eq!(r2.by_model.iter().find(|m| m.key == "opus").map(|m| m.pp), Some(1.0));
        assert_eq!(r2.allocations[0].planned_pp, 33.8);
        assert_eq!(r2.totals.weekly_pct, 12.0);
        let _ = AllocationId::from("x");
    }
}
