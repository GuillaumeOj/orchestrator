//! Unsticking (spec §3.6, D5) and the Overwatch's periodic pass. The FR-411
//! signals are computed for every agent and go to the Overwatch as `stuck`
//! events, once per episode: a lead or a solo session at once, a team member
//! once its lead has had the episode for [`LEAD_FIRST_MS`]. Startup failures
//! and answers given but never acted on are `stuck` too. The pass also lifts
//! a `limit reached` down when the window resets, and ends a turn that passed
//! its ceiling without a Stop.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use orch_protocol::now_ms;
use orch_store::InboxStatus;
use serde_json::json;

use super::{Event, ID, Kind, Stuck, why};
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::supervise::MemberStatus;

/// A team member's lead has it this long before the Overwatch hears of it (D5).
pub const LEAD_FIRST_MS: u64 = 5 * 60_000;
/// An answer given to an agent that did not act on it this long is stuck.
pub const ANSWERED_MS: u64 = 3 * 60_000;
/// After the ceiling's interrupt, a turn with no Stop this long is over.
pub const NO_STOP_GRACE_MS: u64 = 60_000;
/// Without a reset time from the usage, a limit is tried again after this.
const LIMIT_RETRY_MS: u64 = 5 * 60_000;

impl Runtime {
    /// The Overwatch's pass on the 30 s tick (and the wake after it).
    pub(crate) fn overwatch_tick(&self) {
        let now = now_ms();
        self.overwatch_lift_limit(now);
        self.overwatch_watchdog(now);
        if self.overwatch_settings().enabled {
            self.overwatch_stuck_scan(now);
        }
        self.overwatch_wake();
    }

    /// Queues a `stuck` event for every new episode.
    pub(crate) fn overwatch_stuck_scan(&self, now: u64) {
        let agents: Vec<crate::store::AgentRecord> = self.store.read(|s| s.agents.values().cloned().collect());
        let mut current = BTreeSet::new();
        let mut fresh: Vec<Event> = vec![];
        for a in agents.iter().filter(|a| a.id != ID && a.tracker.state != AgentState::Archived) {
            let Some(r) = self.attention_of(&a.id) else { continue };
            let seat = a.team.as_ref();
            let m = MemberStatus {
                agent: a.id.clone(),
                node: seat.map(|t| t.node.clone()).unwrap_or_default(),
                state: a.tracker.state,
                task: seat.and_then(|t| t.task),
                level: r.level,
                status: r.status,
                reason: r.reason.clone(),
                since_ms: r.waiting_on.as_ref().map(|w| w.since_ms).unwrap_or(r.since_ms),
                waiting_on: r.waiting_on.clone(),
            };
            // A prompt waiting on the user is the user's: their cards are events already.
            let Some((what, since, line)) = m.stuck(now).filter(|(w, _, _)| *w != "waiting_for_user") else {
                continue;
            };
            let key = format!("{}:{what}:{since}", a.id);
            current.insert(key.clone());
            let first =
                *self.overwatch.stuck_seen.lock().unwrap_or_else(|e| e.into_inner()).entry(key.clone()).or_insert(now);
            // Its lead has had it first (D5): leads and solo sessions have no one before.
            let member = seat.is_some() && !crate::runtime::is_lead(a);
            if member && now.saturating_sub(first) < LEAD_FIRST_MS {
                continue;
            }
            if self.overwatch.stuck_sent.lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
                fresh.push(stuck_event(&a.id, seat.map(|t| t.run_id.clone()), &line, since, None));
            }
        }
        // Starts that failed (FR-415), once each.
        let startups: Vec<crate::store::InboxItem> =
            self.store.read(|s| s.inbox.iter().filter(|i| i.kind == "startup" && i.open()).cloned().collect());
        for i in startups {
            let key = format!("startup-{}", i.id);
            current.insert(key.clone());
            if self.overwatch.stuck_sent.lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
                let line = i.body.lines().next().unwrap_or("did not start").to_string();
                let mut ev = stuck_event(i.agent_id.as_deref().unwrap_or(""), None, &line, i.created_at_ms, Some(&i));
                ev.decision = Some(crate::decision::decision_id("startup", i.id));
                fresh.push(ev);
            }
        }
        // A parked permission the user answered whose agent never resumed.
        let answered: Vec<crate::store::InboxItem> = self.store.read(|s| {
            s.inbox
                .iter()
                .filter(|i| {
                    i.kind == "permission"
                        && (i.status == InboxStatus::Sent
                            || (i.status == InboxStatus::Effect && i.effect.as_deref() == Some("queued")))
                        && i.sent_at_ms.is_some_and(|t| now.saturating_sub(t) >= ANSWERED_MS)
                        && i.agent_id
                            .as_deref()
                            .is_some_and(|a| s.agents.get(a).is_some_and(|a| a.tracker.state != AgentState::Working))
                })
                .cloned()
                .collect()
        });
        for i in answered {
            let key = format!("answered-{}", i.id);
            current.insert(key.clone());
            if self.overwatch.stuck_sent.lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
                let agent = i.agent_id.clone().unwrap_or_default();
                let line = format!(
                    "{agent} was answered {} ago and has not carried on",
                    crate::now::duration(now - i.sent_at_ms.unwrap_or(now))
                );
                fresh.push(stuck_event(&agent, None, &line, i.sent_at_ms.unwrap_or(now), Some(&i)));
            }
        }
        // Episodes that ended are forgotten (a later one is new).
        self.overwatch.stuck_seen.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| current.contains(k));
        self.overwatch.stuck_sent.lock().unwrap_or_else(|e| e.into_inner()).retain(|k| current.contains(k));
        for ev in fresh {
            self.overwatch_enqueue(ev);
        }
    }

    /// A `limit reached` down ends at the window's reset (once the time is known).
    pub(crate) fn overwatch_lift_limit(&self, now: u64) {
        let until = self.overwatch.limit_until.load(Ordering::SeqCst);
        if until == 0 || now < until {
            return;
        }
        self.overwatch.limit_until.store(0, Ordering::SeqCst);
        let lifted = {
            let mut st = self.ow();
            let limit = st.down.as_deref() == Some(why::LIMIT);
            if limit {
                st.down = None;
            }
            limit
        };
        if lifted {
            self.event(ID, "overwatch.recovered", json!({"after": why::LIMIT}));
            self.overwatch_touch();
        }
    }

    /// When the limit just hit will have reset (the soonest full window).
    pub(crate) fn overwatch_limit_resets(&self, now: u64) -> u64 {
        let view = self.usage_view();
        [&view.five_hour, &view.weekly]
            .into_iter()
            .flatten()
            .filter(|r| r.used_pct >= 100.0)
            .map(|r| r.resets_at.max(0) as u64 * 1000)
            .filter(|t| *t > now)
            .min()
            .unwrap_or(now + LIMIT_RETRY_MS)
    }

    /// The ceiling interrupted the turn (team.rs) and no Stop came: the
    /// session is suspended, so a late Stop can't end the next turn, and the
    /// turn ends as a long one (two in a row put it down).
    pub(crate) fn overwatch_watchdog(&self, now: u64) {
        let ceiling = u64::from(super::TURN_CEILING_MIN) * 60_000;
        let over = self
            .ow()
            .in_flight
            .as_ref()
            .is_some_and(|t| now.saturating_sub(t.started_at_ms) > ceiling + NO_STOP_GRACE_MS);
        if !over {
            return;
        }
        self.event(ID, "overwatch.no_stop", json!({"after_ms": ceiling}));
        if self.live(ID).is_some() {
            let r = self.suspend(ID);
            self.event(
                ID,
                "overwatch.suspended",
                json!({"why": "no Stop", "error": r.err().map(|e| format!("{e:#}"))}),
            );
        }
        self.overwatch_turn_ended(now);
    }
}

/// A `stuck` event (api-contract §2.1).
fn stuck_event(
    agent: &str,
    run: Option<String>,
    line: &str,
    since_ms: u64,
    item: Option<&crate::store::InboxItem>,
) -> Event {
    let mut ev = Event::new(Kind::Stuck, line);
    ev.agent = Some(agent.to_string()).filter(|a| !a.is_empty());
    ev.run = run;
    ev.stuck = Some(Stuck { why: line.to_string(), since_ms });
    let _ = item;
    ev
}
