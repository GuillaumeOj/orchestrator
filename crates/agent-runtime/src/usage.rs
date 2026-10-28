//! Usage collector (FR-801): per-agent statusline samples merged by recency,
//! fed to the Governor.
//!
//! Statusline `rate_limits` are per-process copies of that agent's last API
//! response, and `refreshInterval` re-renders them while idle. So:
//! - a sample is a **new response** only when its readings or cost changed;
//!   idle re-renders are not re-sent (the Governor wants the response time);
//! - for each window the collector uses the most recent response across all
//!   agents, and a window missing from a response stays unknown.

use std::collections::BTreeMap;

use governor::jiff::Timestamp;
use governor::{AgentId, Provenance, UsageSample, WindowReading};
use orch_protocol::{RateLimits, StatusSample, WindowSample};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reading {
    pub used_pct: f64,
    pub resets_at: i64,
    pub agent_id: String,
    /// When the response carrying it was seen (ms).
    pub ts_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AgentUsage {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub rate_limits: Option<RateLimits>,
    pub total_cost_usd: f64,
    pub context_used_pct: Option<f64>,
    pub prompt_cache_expires_at: Option<i64>,
    pub recache_tokens_if_cold: Option<u64>,
    /// Last statusline render received (any).
    pub last_seen_ms: u64,
    /// Last render that carried a new response.
    pub last_response_ms: Option<u64>,
    pub responses: u64,
    /// The last per-model windows reported (FR-815).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_windows: Vec<(String, WindowSample)>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct UsageView {
    pub five_hour: Option<Reading>,
    pub weekly: Option<Reading>,
    pub agents: BTreeMap<String, AgentUsage>,
}

#[derive(Default)]
pub struct UsageCollector {
    agents: BTreeMap<String, AgentUsage>,
}

fn same(a: Option<WindowSample>, b: Option<WindowSample>) -> bool {
    a == b
}

fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millisecond(ms as i64).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn reading(w: Option<WindowSample>) -> Option<WindowReading> {
    let w = w?;
    Some(WindowReading { used_pct: w.used_percentage, resets_at: Timestamp::from_second(w.resets_at).ok()? })
}

impl UsageCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one statusline render. Returns the Governor sample when it
    /// carries a new API response.
    pub fn ingest(&mut self, agent_id: &str, ts_ms: u64, s: &StatusSample) -> Option<UsageSample> {
        let a = self.agents.entry(agent_id.to_string()).or_default();
        a.last_seen_ms = ts_ms;
        if s.model.is_some() {
            a.model = s.model.clone();
        }
        a.context_used_pct = s.context_used_pct.or(a.context_used_pct);
        a.prompt_cache_expires_at = s.prompt_cache_expires_at.or(a.prompt_cache_expires_at);
        a.recache_tokens_if_cold = s.recache_tokens_if_cold.or(a.recache_tokens_if_cold);
        let new_session = s.session_id.is_some() && s.session_id != a.session_id;
        if new_session {
            a.session_id = s.session_id.clone();
        }
        let cost = s.total_cost_usd.unwrap_or(0.0);
        let rl = s.rate_limits.unwrap_or_default();
        let prev = a.rate_limits.unwrap_or_default();
        let models_changed = !s.model_windows.is_empty() && s.model_windows != a.model_windows;
        let readings_changed =
            !same(rl.five_hour, prev.five_hour) || !same(rl.seven_day, prev.seven_day) || models_changed;
        let cost_changed = (cost - a.total_cost_usd).abs() > 1e-12;
        if rl.is_empty() && s.model_windows.is_empty() && cost == 0.0 {
            // Before the first response of a session: nothing observed yet.
            if new_session {
                a.total_cost_usd = 0.0;
            }
            return None;
        }
        if !readings_changed && !cost_changed && !new_session {
            return None; // idle re-render of the same response
        }
        // A restarted process (resume) starts its cost counter again.
        let delta = if cost >= a.total_cost_usd && !new_session { cost - a.total_cost_usd } else { cost };
        a.total_cost_usd = cost;
        if !rl.is_empty() {
            a.rate_limits = Some(rl);
        }
        if !s.model_windows.is_empty() {
            a.model_windows = s.model_windows.clone();
        }
        a.last_response_ms = Some(ts_ms);
        a.responses += 1;
        Some(UsageSample {
            ts: ts(ts_ms),
            agent: Some(AgentId(agent_id.to_string())),
            cost_delta: delta,
            five_hour: reading(rl.five_hour),
            weekly: reading(rl.seven_day),
            provenance: Provenance::Observed,
            // The session's model, for per-model weights (FR-814).
            model: a.model.clone(),
            model_windows: s
                .model_windows
                .iter()
                .filter_map(|(name, w)| {
                    Some(governor::ModelWindowReading {
                        name: name.clone(),
                        used_pct: w.used_percentage,
                        resets_at: Timestamp::from_second(w.resets_at).ok()?,
                    })
                })
                .collect(),
        })
    }

    pub fn agent(&self, agent_id: &str) -> Option<&AgentUsage> {
        self.agents.get(agent_id)
    }

    /// Most recent response per window, across all agents.
    pub fn view(&self) -> UsageView {
        let mut v = UsageView { agents: self.agents.clone(), ..Default::default() };
        for (id, a) in &self.agents {
            let (Some(rl), Some(t)) = (a.rate_limits, a.last_response_ms) else { continue };
            let pick = |cur: &mut Option<Reading>, w: Option<WindowSample>| {
                if let Some(w) = w {
                    if cur.as_ref().is_none_or(|c| t > c.ts_ms) {
                        *cur = Some(Reading {
                            used_pct: w.used_percentage,
                            resets_at: w.resets_at,
                            agent_id: id.clone(),
                            ts_ms: t,
                        });
                    }
                }
            };
            pick(&mut v.five_hour, rl.five_hour);
            pick(&mut v.weekly, rl.seven_day);
        }
        v
    }

    /// The window an agent's last response says is exhausted (for StopFailure `rate_limit`).
    pub fn exhausted_window(&self, agent_id: &str) -> Option<(governor::Window, i64)> {
        let rl = self.agents.get(agent_id)?.rate_limits?;
        if let Some(w) = rl.five_hour.filter(|w| w.used_percentage >= 100.0) {
            return Some((governor::Window::FiveHour, w.resets_at));
        }
        rl.seven_day.filter(|w| w.used_percentage >= 100.0).map(|w| (governor::Window::Weekly, w.resets_at))
    }

    /// FR-815: the per-model window a rate limit hit, when the agent's last
    /// response shows one used up, or the CLI's message names a model
    /// ("… Opus limit …"): `(window name, resets_at)`.
    pub fn exhausted_model_window(&self, agent_id: &str, message: Option<&str>) -> Option<(String, i64)> {
        let a = self.agents.get(agent_id)?;
        if let Some((n, w)) = a.model_windows.iter().find(|(_, w)| w.used_percentage >= 100.0) {
            return Some((n.clone(), w.resets_at));
        }
        let m = message?.to_ascii_lowercase();
        if !m.contains("limit") {
            return None;
        }
        let fam = governor::FAMILIES[..4].iter().find(|f| m.contains(*f))?;
        let named = a.model_windows.iter().find(|(n, _)| n.contains(fam));
        let resets = named.map(|(_, w)| w.resets_at).or(a.rate_limits?.seven_day.map(|w| w.resets_at))?;
        Some((named.map_or_else(|| format!("seven_day_{fam}"), |(n, _)| n.clone()), resets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(v: serde_json::Value) -> StatusSample {
        StatusSample::parse(&v)
    }

    fn rl(five: f64, week: f64, cost: f64) -> StatusSample {
        s(json!({"session_id":"s","cost":{"total_cost_usd":cost},
            "rate_limits":{"five_hour":{"used_percentage":five,"resets_at":1792489800},"seven_day":{"used_percentage":week,"resets_at":1792983600}}}))
    }

    #[test]
    fn first_render_and_idle_rerenders_are_not_responses() {
        let mut c = UsageCollector::new();
        assert!(c.ingest("a", 1_000, &s(json!({"session_id":"s","cost":{"total_cost_usd":0}}))).is_none());
        let g = c.ingest("a", 2_000, &rl(40.0, 6.0, 0.01)).unwrap();
        assert_eq!(g.cost_delta, 0.01);
        assert_eq!(g.ts, Timestamp::from_millisecond(2_000).unwrap());
        assert_eq!(g.weekly.unwrap().used_pct, 6.0);
        // refreshInterval re-renders the same response every 5 s: not re-sent.
        assert!(c.ingest("a", 7_000, &rl(40.0, 6.0, 0.01)).is_none());
        let g = c.ingest("a", 9_000, &rl(41.0, 6.0, 0.015)).unwrap();
        assert!((g.cost_delta - 0.005).abs() < 1e-9);
        assert_eq!(c.agent("a").unwrap().responses, 2);
    }

    #[test]
    fn merges_by_recency_across_agents_and_keeps_missing_windows_unknown() {
        let mut c = UsageCollector::new();
        c.ingest("x", 1_000, &rl(40.0, 6.0, 0.01));
        c.ingest("y", 2_000, &rl(42.0, 7.0, 0.01));
        // x re-renders its stale copy later: ignored, y stays the most recent.
        c.ingest("x", 3_000, &rl(40.0, 6.0, 0.01));
        let v = c.view();
        assert_eq!(v.five_hour.as_ref().unwrap().used_pct, 42.0);
        assert_eq!(v.weekly.as_ref().unwrap().agent_id, "y");
        // After a 429 only the five-hour window is reported: weekly stays from y.
        let g = c.ingest("z", 4_000, &s(json!({"cost":{"total_cost_usd":0.001},"rate_limits":{"five_hour":{"used_percentage":100,"resets_at":1792492371}}}))).unwrap();
        assert!(g.weekly.is_none());
        let v = c.view();
        assert_eq!(v.five_hour.as_ref().unwrap().used_pct, 100.0);
        assert_eq!(v.weekly.as_ref().unwrap().used_pct, 7.0);
        assert_eq!(c.exhausted_window("z"), Some((governor::Window::FiveHour, 1792492371)));
        assert_eq!(c.exhausted_window("y"), None);
    }

    #[test]
    fn resumed_process_restarts_its_cost() {
        let mut c = UsageCollector::new();
        c.ingest("a", 1_000, &rl(40.0, 6.0, 0.05));
        let g = c
            .ingest(
                "a",
                2_000,
                &s(json!({"session_id":"s2","cost":{"total_cost_usd":0.002},
            "rate_limits":{"five_hour":{"used_percentage":40,"resets_at":1792489800}}})),
            )
            .unwrap();
        assert!((g.cost_delta - 0.002).abs() < 1e-9);
    }

    #[test]
    fn feeds_the_governor() {
        use governor::{BudgetAllocation, Governor, PacingPolicy, Tier, WeeklyPlan, WorkingCalendar};
        let start: Timestamp = "2026-10-19T07:00:00Z".parse().unwrap();
        let plan = WeeklyPlan {
            id: "w".into(),
            week_start: start,
            reset_at: "2026-10-26T07:00:00Z".parse().unwrap(),
            calendar: WorkingCalendar::weekdays("UTC", jiff::civil::time(0, 0, 0, 0), jiff::civil::time(23, 59, 0, 0)),
            reserve_pct: 10.0,
            off_hours_allowance_pct: 0.0,
            shadow_mode: false,
            allocations: vec![BudgetAllocation {
                id: "default".into(),
                target: "all".into(),
                weight: 1.0,
                tier: Tier::Normal,
                hard_cap_pct: None,
            }],
        };
        let mut gov = Governor::new(plan, PacingPolicy::default(), start).unwrap();
        let mut c = UsageCollector::new();
        let ms = (start.as_millisecond() + 3_600_000) as u64;
        let g = c.ingest("a", ms, &rl(10.0, 3.0, 0.2)).unwrap();
        gov.ingest(&g);
        let snap = gov.snapshot(g.ts);
        assert!(snap.used >= 3.0 && snap.used < 4.0, "{}", snap.used);
        assert_eq!(snap.provenance, Provenance::Observed);
    }
}
