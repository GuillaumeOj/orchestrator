//! The statusline JSON Claude Code passes to `orch-status` (P0: 2.1.302).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One `rate_limits` window: integer %, reset in epoch seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSample {
    pub used_percentage: f64,
    pub resets_at: i64,
}

/// `rate_limits`. A window missing from the response is unknown, not 0
/// (after a 429 only the limited window is reported).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RateLimits {
    #[serde(default)]
    pub five_hour: Option<WindowSample>,
    #[serde(default)]
    pub seven_day: Option<WindowSample>,
}

impl RateLimits {
    pub fn is_empty(&self) -> bool {
        self.five_hour.is_none() && self.seven_day.is_none()
    }
}

/// The statusline fields the core uses. Missing until the first API response
/// of a session (`rate_limits` absent, `cost.total_cost_usd` 0).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusSample {
    pub session_id: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
    pub rate_limits: Option<RateLimits>,
    pub total_cost_usd: Option<f64>,
    pub context_used_pct: Option<f64>,
    pub context_window_size: Option<u64>,
    /// `prompt_cache.expires_at` (epoch s): resuming before it is nearly free.
    pub prompt_cache_expires_at: Option<i64>,
    pub prompt_cache_ttl: Option<String>,
    pub recache_tokens_if_cold: Option<u64>,
}

impl StatusSample {
    /// Lenient parse: a field with an unexpected shape is dropped, the rest is kept.
    pub fn parse(v: &Value) -> StatusSample {
        let f = |p: &str| v.pointer(p).and_then(Value::as_f64);
        let i = |p: &str| v.pointer(p).and_then(Value::as_i64);
        let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_string);
        let window = |name: &str| -> Option<WindowSample> {
            Some(WindowSample {
                used_percentage: f(&format!("/rate_limits/{name}/used_percentage"))?,
                resets_at: i(&format!("/rate_limits/{name}/resets_at"))?,
            })
        };
        let rl = RateLimits { five_hour: window("five_hour"), seven_day: window("seven_day") };
        let model = match v.get("model") {
            Some(Value::String(m)) => Some(m.clone()),
            Some(m) => m.get("id").or_else(|| m.get("display_name")).and_then(Value::as_str).map(str::to_string),
            None => None,
        };
        StatusSample {
            session_id: s("/session_id"),
            version: s("/version"),
            model,
            rate_limits: if rl.is_empty() { None } else { Some(rl) },
            total_cost_usd: f("/cost/total_cost_usd"),
            context_used_pct: f("/context_window/used_percentage"),
            context_window_size: v.pointer("/context_window/context_window_size").and_then(Value::as_u64),
            prompt_cache_expires_at: i("/prompt_cache/expires_at"),
            prompt_cache_ttl: s("/prompt_cache/ttl"),
            recache_tokens_if_cold: v.pointer("/prompt_cache/recache_tokens_if_cold").and_then(Value::as_u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_p0_sample() {
        let v = json!({"session_id":"s","version":"2.1.302","model":{"id":"claude-haiku-5-5","display_name":"Haiku"},
            "rate_limits":{"five_hour":{"used_percentage":7,"resets_at":1792270200},"seven_day":{"used_percentage":83,"resets_at":1792378800}},
            "context_window":{"context_window_size":1000000,"used_percentage":7},
            "cost":{"total_cost_usd":0.0137},
            "prompt_cache":{"warm":true,"ttl":"1h","expires_at":1792267440,"recache_tokens_if_cold":65798}});
        let s = StatusSample::parse(&v);
        let rl = s.rate_limits.unwrap();
        assert_eq!(rl.five_hour.unwrap().used_percentage, 7.0);
        assert_eq!(rl.seven_day.unwrap().resets_at, 1792378800);
        assert_eq!(s.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(s.total_cost_usd, Some(0.0137));
        assert_eq!(s.prompt_cache_expires_at, Some(1792267440));
        assert_eq!(s.context_used_pct, Some(7.0));
    }

    #[test]
    fn missing_windows_stay_unknown() {
        let first = StatusSample::parse(&json!({"session_id":"s","cost":{"total_cost_usd":0}}));
        assert!(first.rate_limits.is_none());
        let after429 = StatusSample::parse(&json!({"rate_limits":{"five_hour":{"used_percentage":100,"resets_at":1}}}));
        let rl = after429.rate_limits.unwrap();
        assert!(rl.seven_day.is_none());
        assert_eq!(rl.five_hour.unwrap().used_percentage, 100.0);
    }
}
