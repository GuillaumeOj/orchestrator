//! Deciding (api-contract §3.1, §5 F1): a permission whose effect is
//! `overwatch` waits on the Overwatch for `permission_secs`, then on the
//! user. The mark is the item's `deciding_until_ms`; `waits_on` follows it.
//! The hold loop ([`Runtime::hold_for_decision`]) waits through it; this
//! module sets, ends and checks it, and turns new items into events.

use orch_protocol::now_ms;
use serde_json::{Value, json};

use super::{Event, ID, Kind};
use crate::runtime::Runtime;

/// The payload key of the deciding mark.
pub const DECIDING_UNTIL: &str = "deciding_until_ms";

/// Why `decide_permission` can't settle a decision (B3 maps it to the
/// tool's error code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotDeciding {
    /// Closed meanwhile.
    Stale,
    /// Not a permission whose effect is `overwatch`.
    NotAllowed,
    /// Handed over to the user (window passed, annotated, turn ended).
    TooLate,
}

impl NotDeciding {
    pub fn code(self) -> &'static str {
        match self {
            NotDeciding::Stale => "stale",
            NotDeciding::NotAllowed => "not_allowed",
            NotDeciding::TooLate => "too_late",
        }
    }
}

/// The open item's deciding deadline, if it is still the Overwatch's.
pub fn deciding_until(payload: &Value) -> Option<u64> {
    payload.get(DECIDING_UNTIL).and_then(Value::as_u64)
}

impl Runtime {
    /// The deciding deadline a new permission with effect `overwatch` gets
    /// (`None`: it goes straight to the user, the Overwatch being off,
    /// down or paused).
    pub(crate) fn overwatch_deciding_deadline(&self, now: u64) -> Option<u64> {
        let secs = self.overwatch_settings().permission_secs;
        self.overwatch_may_decide().then(|| now + secs * 1000)
    }

    /// May the Overwatch still settle `item` now? (B3's `decide_permission`.)
    pub fn overwatch_deciding(&self, item: u64) -> Result<u64, NotDeciding> {
        // Deadlines are wall-clock (the hold's).
        let now = now_ms();
        let found = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == item).map(|i| (i.open(), i.kind.clone(), i.payload.clone())));
        let Some((open, kind, payload)) = found else { return Err(NotDeciding::Stale) };
        if !open {
            return Err(NotDeciding::Stale);
        }
        if kind != "permission" || payload.get("effect").and_then(Value::as_str) != Some("overwatch") {
            return Err(NotDeciding::NotAllowed);
        }
        match deciding_until(&payload) {
            Some(t) if now < t => Ok(t),
            _ => Err(NotDeciding::TooLate),
        }
    }

    /// Hands `item` over to the user (F1): the window passed, it was
    /// annotated, refused as `not_allowed`, its turn ended undecided, or
    /// the Overwatch went off / down / paused. `asks_because` becomes
    /// `overwatch_unsure`; the hold restarts from here. False when it was
    /// not deciding.
    pub fn overwatch_hand_over(&self, item: u64) -> bool {
        let now = now_ms();
        let done = self
            .store
            .write(|s| {
                let Some(i) = s.inbox.iter_mut().find(|i| i.id == item && i.open()) else { return false };
                let Some(p) = i.payload.as_object_mut() else { return false };
                if p.remove(DECIDING_UNTIL).and_then(|v| v.as_u64()).is_none() {
                    return false;
                }
                p.insert("handed_over_ms".into(), json!(now));
                p.insert(
                    "asks_because".into(),
                    json!({"kind": "overwatch_unsure", "detail": "the Overwatch left it to you"}),
                );
                true
            })
            .unwrap_or(false);
        if done {
            self.event(ID, "overwatch.hand_over", json!({"item": item}));
        }
        done
    }

    /// Every deciding permission goes to the user (off, down).
    pub(crate) fn overwatch_hand_over_all(&self) {
        let ids: Vec<u64> = self.store.read(|s| {
            s.inbox.iter().filter(|i| i.open() && deciding_until(&i.payload).is_some()).map(|i| i.id).collect()
        });
        for id in ids {
            self.overwatch_hand_over(id);
        }
    }

    /// A new item becomes an event (api-contract §2.1): a deciding
    /// permission and a question wake it; other decisions waiting on the
    /// user (gates, user-only permissions, budget, startup) are batched for
    /// annotation. Its own items and alerts are not.
    /// `first_line`: the item body's; `deciding`: its deciding mark.
    pub(crate) fn overwatch_on_added(
        &self,
        id: u64,
        agent: Option<&str>,
        kind: &str,
        first_line: &str,
        deciding: Option<u64>,
    ) {
        if agent == Some(ID) {
            return;
        }
        let ev_kind = match kind {
            "permission" if deciding.is_some() => Kind::Permission,
            "question" => Kind::Question,
            "permission" | "gate" | "budget" | "startup" => Kind::DecisionOpen,
            _ => return,
        };
        let mut summary = crate::team::cut(first_line, 200);
        if ev_kind == Kind::Permission {
            summary.push_str(" · effect overwatch");
        }
        let mut ev = Event::new(ev_kind, summary);
        ev.decision = Some(crate::decision::decision_id(kind, id));
        ev.agent = agent.map(str::to_string);
        ev.run = agent.and_then(|a| self.store.read(|s| s.agents.get(a)?.team.as_ref().map(|t| t.run_id.clone())));
        ev.deadline_ms = deciding;
        self.overwatch_enqueue(ev);
    }

    /// A decision closed by the user: batched, so it learns what was
    /// decided. (Closed by itself or the core: no event.)
    pub(crate) fn overwatch_on_closed(&self, item: &orch_store::InboxItem, decided_by: &str, effect: &str) {
        if decided_by != "user" || item.agent_id.as_deref() == Some(ID) {
            return;
        }
        if !matches!(item.kind.as_str(), "permission" | "question" | "gate" | "budget" | "startup") {
            return;
        }
        let first = item.body.lines().next().unwrap_or("");
        let word = item.decision.as_deref().unwrap_or(effect);
        let mut ev = Event::new(Kind::DecisionClosed, crate::team::cut(&format!("{first} · {word} by the user"), 200));
        ev.decision = Some(crate::decision::decision_id(&item.kind, item.id));
        ev.agent = item.agent_id.clone();
        self.overwatch_enqueue(ev);
    }

    /// Today's new outcomes (tasks done, runs finished…): batched.
    pub(crate) fn overwatch_on_outcomes(&self, outcomes: &[crate::digest::Outcome]) {
        if outcomes.is_empty() || !self.overwatch_settings().enabled {
            return;
        }
        for o in outcomes.iter().filter(|o| o.agent.as_deref() != Some(ID)) {
            let mut ev = Event::new(Kind::Outcome, crate::team::cut(&format!("{}: {}", o.kind, o.text), 200));
            ev.agent = o.agent.clone();
            ev.run = o.run.clone();
            self.overwatch_enqueue(ev);
        }
    }
}
