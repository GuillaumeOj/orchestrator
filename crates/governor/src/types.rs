//! Identifiers, tiers, windows and usage samples: the Governor's inputs.

use std::fmt;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

/// An agent: one Claude Code process (SPEC §2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

/// A budget allocation of the weekly plan (project, feature or team).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AllocationId(pub String);

/// A queued turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntryId(pub u64);

/// A `GovernorAction` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActionId(pub u64);

macro_rules! display_inner {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    )*};
}
display_inner!(AgentId, AllocationId, EntryId, ActionId);

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        AgentId(s.to_owned())
    }
}

impl From<&str> for AllocationId {
    fn from(s: &str) -> Self {
        AllocationId(s.to_owned())
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        AgentId(s)
    }
}

impl From<String> for AllocationId {
    fn from(s: String) -> Self {
        AllocationId(s)
    }
}

/// Priority tier (FR-802). Ordered: `Low < Normal < High < Critical`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Low,
    Normal,
    High,
    Critical,
}

impl Tier {
    /// Raise by `steps` tiers for aging. Aging stops at `High`: only work that
    /// is critical from the start gets strict priority.
    pub fn aged(self, steps: u32) -> Tier {
        if self == Tier::Critical {
            return self;
        }
        let rank = (self as u32 + steps).min(Tier::High as u32);
        match rank {
            0 => Tier::Low,
            1 => Tier::Normal,
            _ => Tier::High,
        }
    }
}

/// A plan-limit window reported by the statusline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    /// `rate_limits.five_hour` (F).
    FiveHour,
    /// `rate_limits.seven_day` (W).
    Weekly,
}

/// Where a value came from (FR-801).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    Observed,
    Estimated,
}

/// One window of a statusline `rate_limits` reading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowReading {
    /// `used_percentage` (an integer in practice, kept as f64).
    pub used_pct: f64,
    /// `resets_at` (epoch seconds in the statusline).
    pub resets_at: Timestamp,
}

/// A usage sample (SPEC §7 `UsageSample`, reduced to what the Governor needs).
///
/// `ts` must be the time of the API response the readings come from. A
/// statusline re-rendered while idle repeats the agent's last response, so
/// callers must not re-send it with a newer `ts` (P0 recovery spike).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageSample {
    pub ts: Timestamp,
    /// `None` = not attributed to an agent of the app.
    pub agent: Option<AgentId>,
    /// Cost estimate of this agent's usage since its previous sample, in any
    /// consistent unit (USD from the statusline or OTEL, tokens...). The
    /// estimator learns how many of these units make 1 % of the week.
    #[serde(default)]
    pub cost_delta: f64,
    /// `None` = window missing from the response (unknown, not 0).
    #[serde(default)]
    pub five_hour: Option<WindowReading>,
    #[serde(default)]
    pub weekly: Option<WindowReading>,
    pub provenance: Provenance,
    /// The model the cost was spent on (FR-814 per-model weights), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl UsageSample {
    /// A sample carrying only cost, with no window readings.
    pub fn cost(ts: Timestamp, agent: AgentId, cost_delta: f64) -> Self {
        UsageSample {
            ts,
            agent: Some(agent),
            cost_delta,
            five_hour: None,
            weekly: None,
            provenance: Provenance::Observed,
            model: None,
        }
    }
}

/// Seconds between two instants, as f64 hours.
pub(crate) fn hours_between(from: Timestamp, to: Timestamp) -> f64 {
    (to.as_second() - from.as_second()) as f64 / 3600.0
}

pub(crate) fn add_hours(t: Timestamp, hours: f64) -> Timestamp {
    let secs = (hours * 3600.0).round();
    let secs = secs.clamp(-1.0e11, 1.0e11) as i64;
    t.checked_add(SignedDuration::from_secs(secs)).unwrap_or(t)
}

pub(crate) fn add_minutes(t: Timestamp, minutes: i64) -> Timestamp {
    t.checked_add(SignedDuration::from_mins(minutes)).unwrap_or(t)
}

/// Exponential smoothing factor for a step of `dt` with time constant `tau`.
pub(crate) fn ewma_alpha(dt: f64, tau: f64) -> f64 {
    if dt <= 0.0 {
        0.0
    } else if tau <= 0.0 {
        1.0
    } else {
        1.0 - (-dt / tau).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aging_stops_at_high() {
        assert_eq!(Tier::Low.aged(0), Tier::Low);
        assert_eq!(Tier::Low.aged(1), Tier::Normal);
        assert_eq!(Tier::Low.aged(2), Tier::High);
        assert_eq!(Tier::Low.aged(9), Tier::High);
        assert_eq!(Tier::Critical.aged(3), Tier::Critical);
    }

    #[test]
    fn tiers_are_ordered() {
        assert!(Tier::Low < Tier::Normal && Tier::Normal < Tier::High && Tier::High < Tier::Critical);
    }
}
