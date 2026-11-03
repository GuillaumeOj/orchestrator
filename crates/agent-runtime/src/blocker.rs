//! Why a message waits (fix-team-getting-stucked, api-contract §1): the one
//! vocabulary of cards, chips, the run report and the Lead's notes. Computed
//! here and nowhere else, from facts the caller gathers.

use governor::QueueReason;

use crate::state::AgentState;

/// What holds an agent's next turn in the Governor's queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// `governor:<reason>`.
    Governor(&'static str),
    /// Waiting for a live slot.
    Slot,
}

/// The Governor's reason for holding a turn, in the blocker vocabulary.
pub fn hold_of(r: &QueueReason) -> Hold {
    match r {
        QueueReason::LiveCap { .. } | QueueReason::MemoryPressure { .. } | QueueReason::BehindQueue => Hold::Slot,
        QueueReason::OffHours | QueueReason::ScheduledOffHours { .. } => Hold::Governor("off_hours"),
        QueueReason::FiveHourGuard => Hold::Governor("five_hour"),
        QueueReason::PausedAll | QueueReason::PausedAgent => Hold::Governor("paused"),
        QueueReason::ModelWindow { exhausted: false, .. } => Hold::Governor("model_switch"),
        QueueReason::RedTier
        | QueueReason::LimitReached { .. }
        | QueueReason::DeferredUntilReset
        | QueueReason::HardCap
        | QueueReason::ModelWindow { .. } => Hold::Governor("limit"),
    }
}

/// What the caller knows about an agent that has messages waiting.
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub state: AgentState,
    /// The process is gone and its restart is pending or used up.
    pub crashed: bool,
    /// Parked on a permission decision.
    pub parked: bool,
    /// The Governor holds its next turn.
    pub hold: Option<Hold>,
    /// A paste is in flight and the CLI has not acknowledged it.
    pub unacked: bool,
    /// The user is typing in its pane.
    pub typing: bool,
}

/// The state's key in a blocker (`not_idle:<state>`).
pub fn state_key(s: AgentState) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The blocker string of `f`, first matching rule wins.
pub fn of(f: &Facts) -> String {
    if f.crashed {
        return "crashed".into();
    }
    if f.parked {
        return "parked".into();
    }
    match f.hold {
        Some(Hold::Governor(r)) => return format!("governor:{r}"),
        Some(Hold::Slot) => return "slot".into(),
        None => {}
    }
    if matches!(f.state, AgentState::Starting | AgentState::Created) {
        return "starting_up".into();
    }
    if f.unacked {
        return "no_ack".into();
    }
    if f.typing {
        return "user_typing".into();
    }
    match f.state {
        // Idle with nothing in its way: the core owes it a turn and has not made one.
        AgentState::Idle => "unknown".into(),
        s => format!("not_idle:{}", state_key(s)),
    }
}

/// The cause without its detail: `governor:off_hours` → `governor`.
pub fn kind(blocker: &str) -> &str {
    blocker.split(':').next().unwrap_or(blocker)
}

/// The UI label of a blocker (api-contract §1), for Lead notes and card copy.
pub fn label(blocker: &str) -> String {
    match blocker.split_once(':') {
        Some(("not_idle", "idle")) => "idle".into(),
        Some(("not_idle", s)) => format!("busy ({})", s.replace('_', " ")),
        Some(("governor", r)) => format!("held: {}", r.replace('_', " ")),
        _ => match blocker {
            "starting_up" => "starting".into(),
            "no_ack" => "paste not acknowledged".into(),
            "slot" => "waiting for a slot".into(),
            "parked" => "parked on your answer".into(),
            "crashed" => "crashed".into(),
            "user_typing" => "you are typing".into(),
            _ => "not delivered".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(state: AgentState) -> Facts {
        Facts { state, crashed: false, parked: false, hold: None, unacked: false, typing: false }
    }

    #[test]
    fn the_first_matching_cause_names_the_blocker() {
        use AgentState::*;
        assert_eq!(of(&Facts { crashed: true, hold: Some(Hold::Slot), ..facts(Error) }), "crashed");
        assert_eq!(of(&Facts { parked: true, ..facts(Suspended) }), "parked");
        assert_eq!(of(&Facts { hold: Some(Hold::Governor("off_hours")), ..facts(Suspended) }), "governor:off_hours");
        assert_eq!(of(&Facts { hold: Some(Hold::Slot), ..facts(Suspended) }), "slot");
        assert_eq!(of(&facts(Starting)), "starting_up");
        assert_eq!(of(&Facts { unacked: true, typing: true, ..facts(Idle) }), "no_ack");
        assert_eq!(of(&Facts { typing: true, ..facts(Idle) }), "user_typing");
        assert_eq!(of(&facts(WaitingForYou)), "not_idle:waiting_for_you");
        assert_eq!(of(&facts(Working)), "not_idle:working");
        assert_eq!(of(&facts(Idle)), "unknown");
    }

    #[test]
    fn governor_reasons_map_to_the_vocabulary() {
        let cap = QueueReason::LiveCap { state: governor::PressureState::Green, cap: 2 };
        assert_eq!(hold_of(&cap), Hold::Slot);
        assert_eq!(hold_of(&QueueReason::MemoryPressure { cap: 1 }), Hold::Slot);
        assert_eq!(hold_of(&QueueReason::OffHours), Hold::Governor("off_hours"));
        assert_eq!(hold_of(&QueueReason::FiveHourGuard), Hold::Governor("five_hour"));
        assert_eq!(hold_of(&QueueReason::PausedAll), Hold::Governor("paused"));
        assert_eq!(hold_of(&QueueReason::PausedAgent), Hold::Governor("paused"));
        assert_eq!(hold_of(&QueueReason::DeferredUntilReset), Hold::Governor("limit"));
        assert_eq!(hold_of(&QueueReason::HardCap), Hold::Governor("limit"));
    }

    #[test]
    fn labels_and_kinds() {
        assert_eq!(label("not_idle:waiting_for_you"), "busy (waiting for you)");
        assert_eq!(label("governor:off_hours"), "held: off hours");
        assert_eq!(label("no_ack"), "paste not acknowledged");
        assert_eq!(label("slot"), "waiting for a slot");
        assert_eq!(label("whatever"), "not delivered");
        assert_eq!((kind("governor:limit"), kind("no_ack")), ("governor", "no_ack"));
    }
}
