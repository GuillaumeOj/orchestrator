//! Queue entries and weighted fair queuing with strict critical and aging (FR-808).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::pressure::PressureState;
use crate::types::{AgentId, AllocationId, EntryId, Tier, Window, hours_between};

/// Kind of turn (SPEC §7 `Turn.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnKind {
    Task,
    Message,
    Continue,
    User,
}

/// Why a turn waits (shown with every queued agent, FR-809).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueReason {
    /// Live autonomous agents are at the cap for the current state.
    LiveCap {
        state: PressureState,
        cap: u32,
    },
    /// Red admits only `red_min_tier` and above.
    RedTier,
    /// Outside working time and the plan has no off-hours allowance.
    OffHours,
    /// F is above the 5h guard: only interactive and critical work runs.
    FiveHourGuard,
    /// A plan limit was hit; work resumes at the reset (J9).
    LimitReached {
        window: Window,
    },
    DeferredUntilReset,
    HardCap,
    PausedAll,
    /// Waiting behind other queued work picked first by fair queuing.
    BehindQueue,
}

impl std::fmt::Display for QueueReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueReason::LiveCap { state, cap } => write!(f, "{cap} live agents allowed in {state}"),
            QueueReason::RedTier => write!(f, "red: only higher-priority work runs"),
            QueueReason::OffHours => write!(f, "outside working hours"),
            QueueReason::FiveHourGuard => write!(f, "5h guard: window above threshold"),
            QueueReason::LimitReached { window } => write!(f, "plan limit reached ({window:?})"),
            QueueReason::DeferredUntilReset => write!(f, "deferred until the weekly reset"),
            QueueReason::HardCap => write!(f, "allocation hard cap reached"),
            QueueReason::PausedAll => write!(f, "paused by the user"),
            QueueReason::BehindQueue => write!(f, "waiting behind higher-priority work"),
        }
    }
}

/// Turn ceiling of a role, enforced by the watchdog (FR-407).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnCeiling {
    #[serde(default)]
    pub max_pct: Option<f64>,
    #[serde(default)]
    pub max_minutes: Option<i64>,
}

/// `QueueEntry` (SPEC §7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: EntryId,
    pub kind: TurnKind,
    pub agent: AgentId,
    pub allocation: AllocationId,
    pub tier: Tier,
    pub enqueued_at: Timestamp,
    pub effective_tier: Tier,
    pub reason: QueueReason,
    pub estimated_start: Option<Timestamp>,
    /// Estimated cost of running it now, pp of the week ("Run now ≈ +2 pp").
    pub estimated_cost_pct: f64,
    pub self_started: bool,
    #[serde(default)]
    pub ceiling: Option<TurnCeiling>,
}

impl QueueEntry {
    /// Tier after aging: +1 per `aging_hours` waited, up to `High`.
    pub fn aged_tier(&self, now: Timestamp, aging_hours: f64) -> Tier {
        let waited = hours_between(self.enqueued_at, now).max(0.0);
        let steps = if aging_hours > 0.0 { (waited / aging_hours).floor() as u32 } else { 0 };
        self.tier.aged(steps)
    }
}

/// Release order: strict by effective tier, then by allocation service
/// (attributed usage / weight, lowest first), then FIFO. `service` must already
/// include the start-time offset so idle allocations cannot hoard credit.
pub fn release_cmp(a: &QueueEntry, b: &QueueEntry, service: &BTreeMap<AllocationId, f64>) -> Ordering {
    let svc = |e: &QueueEntry| service.get(&e.allocation).copied().unwrap_or(0.0);
    b.effective_tier
        .cmp(&a.effective_tier)
        .then_with(|| svc(a).total_cmp(&svc(b)))
        .then_with(|| a.enqueued_at.cmp(&b.enqueued_at))
        .then_with(|| a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, alloc: &str, tier: Tier, minute: i64) -> QueueEntry {
        QueueEntry {
            id: EntryId(id),
            kind: TurnKind::Task,
            agent: AgentId(format!("ag{id}")),
            allocation: alloc.into(),
            tier,
            enqueued_at: Timestamp::from_second(1_800_000_000 + minute * 60).unwrap(),
            effective_tier: tier,
            reason: QueueReason::BehindQueue,
            estimated_start: None,
            estimated_cost_pct: 0.5,
            self_started: false,
            ceiling: None,
        }
    }

    #[test]
    fn tier_beats_service_and_service_beats_fifo() {
        let es = [entry(1, "a", Tier::Normal, 0), entry(2, "b", Tier::Normal, 5), entry(3, "a", Tier::High, 9)];
        let service: BTreeMap<AllocationId, f64> = [("a".into(), 10.0), ("b".into(), 2.0)].into();
        let mut sorted: Vec<&QueueEntry> = es.iter().collect();
        sorted.sort_by(|a, b| release_cmp(a, b, &service));
        let order: Vec<u64> = sorted.iter().map(|e| e.id.0).collect();
        assert_eq!(order, vec![3, 2, 1]);
    }

    #[test]
    fn aging_adds_a_tier_every_two_hours() {
        let e = entry(1, "a", Tier::Low, 0);
        let at = |m: i64| Timestamp::from_second(1_800_000_000 + m * 60).unwrap();
        assert_eq!(e.aged_tier(at(119), 2.0), Tier::Low);
        assert_eq!(e.aged_tier(at(120), 2.0), Tier::Normal);
        assert_eq!(e.aged_tier(at(240), 2.0), Tier::High);
        assert_eq!(e.aged_tier(at(10_000), 2.0), Tier::High);
    }
}
