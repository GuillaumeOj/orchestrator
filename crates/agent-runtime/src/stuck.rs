//! Stuck detection that sees everything (fix-team-getting-stucked §3.7,
//! F10): the `delivery` decision (api-contract §2), the delivery invariant,
//! waits on the Lead, and the notes the Lead gets. One decision per episode.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow, bail};
use orch_store::{InboxStatus, Message};
use serde_json::{Value, json};

use crate::decision::last_lines;
use crate::delivery;
use crate::inbox::{PRIORITY_ALERT, Stage};
use crate::runtime::{Runtime, reserved_slot};
use crate::state::AgentState;
use crate::store::{AgentRecord, TurnStatus};
use crate::team::{RunCtx, cut};

/// The invariant (spec exit criterion 2): an unread message waits this long.
pub const INVARIANT_MS: u64 = 10 * 60 * 1000;
/// A member waits on an idle Lead this long before the Lead is told.
pub const WAIT_ON_LEAD_MS: u64 = 15 * 60 * 1000;
/// ...and this much longer before the user is.
pub const WAIT_ESCALATE_MS: u64 = 10 * 60 * 1000;
/// Lines of the PTY tail a decision carries.
const TAIL_LINES: usize = 40;

/// One `delivery` decision to raise.
pub(crate) struct Raise {
    /// `delivery`, `stuck_lead`, `stuck_wait` or `working_silent`.
    pub episode: &'static str,
    pub blocker: String,
    pub since_ms: u64,
    pub messages: Vec<Message>,
    pub turns: Vec<String>,
    /// Identifies the episode: raised once, even when dismissed.
    pub key: String,
    /// The run report counts a `stuck` event (the paste watchdog has its own).
    pub stuck_event: bool,
}

impl Runtime {
    /// Agents with a paste in flight, and agents whose pane the user types in.
    pub(crate) fn delivery_flags(&self, now: u64) -> (BTreeSet<String>, BTreeSet<String>) {
        let unacked = self.store.read(|s| {
            s.turns.iter().filter(|t| t.status == TurnStatus::Delivering).map(|t| t.agent_id.clone()).collect()
        });
        let quiet = delivery::USER_QUIET.as_millis() as u64;
        let typing = self
            .live_ids()
            .into_iter()
            .filter(|id| {
                self.live(id).is_some_and(|l| now < l.last_user_input_ms.load(Ordering::SeqCst).saturating_add(quiet))
            })
            .collect();
        (unacked, typing)
    }

    /// The one blocker of `rec`'s waiting messages (api-contract §1).
    pub(crate) fn blocker_for(&self, rec: &AgentRecord, now: u64) -> String {
        let live = self.live(&rec.id).is_some();
        let (unacked, typing) = self.delivery_flags(now);
        let hold = self.with_governor(|g| {
            g.queue().iter().find(|e| e.agent.0 == rec.id).map(|e| crate::blocker::hold_of(&e.reason))
        });
        let parked = self.store.read(|s| {
            s.inbox.iter().any(|i| {
                i.kind == "permission" && i.status == InboxStatus::Parked && i.agent_id.as_deref() == Some(&rec.id)
            })
        });
        crate::blocker::of(&crate::blocker::Facts {
            state: rec.tracker.state,
            crashed: !live && crate::startup::crashed(rec),
            parked,
            hold,
            unacked: unacked.contains(&rec.id),
            typing: typing.contains(&rec.id),
        })
    }

    fn tail_of(&self, id: &str) -> String {
        self.live(id).map(|l| last_lines(&l.pty.output.tail_text(), TAIL_LINES)).unwrap_or_default()
    }

    /// Raises a `delivery` decision for `rec`, unless this episode already
    /// had one (open or closed). The decision id, if one was raised.
    pub(crate) fn raise_delivery(&self, rec: &AgentRecord, r: Raise) -> Option<u64> {
        if !self.house().delivery_raised.insert(r.key.clone()) {
            return None;
        }
        let seen = self.store.read(|s| {
            s.inbox.iter().any(|i| i.kind == "delivery" && i.payload.get("key").and_then(Value::as_str) == Some(&r.key))
        });
        if seen {
            return None;
        }
        let n = r.messages.len();
        let title = if n > 0 && r.episode == "delivery" {
            format!("{} did not receive {n} message(s)", rec.id)
        } else {
            format!("{} looks stuck", rec.id)
        };
        let messages: Vec<Value> = r
            .messages
            .iter()
            .map(|m| {
                json!({"id": m.id, "from": m.from_agent.clone().unwrap_or_else(|| m.from_node.clone()),
                       "kind": m.kind, "preview": cut(&m.body, 120), "attempts": m.attempts})
            })
            .collect();
        let payload = json!({
            "waiting_on": "delivery", "agent": rec.id, "blocker": r.blocker, "since_ms": r.since_ms,
            "messages": messages, "episode": r.episode, "tail": self.tail_of(&rec.id),
            "turns": r.turns, "key": r.key,
        });
        let Ok(id) = self.add_item(Some(&rec.id), "delivery", title, payload, PRIORITY_ALERT) else {
            // Nothing was raised: the episode may be tried again.
            self.house().delivery_raised.remove(&r.key);
            return None;
        };
        if r.stuck_event {
            self.delivery_event(&rec.id, "stuck", json!({"agent": rec.id, "episode": r.episode}));
        }
        // The Lead hears about a member (the Lead's own are the user's only).
        if let Some(seat) = &rec.team
            && !reserved_slot(rec)
        {
            let what = match n {
                0 => format!("{} looks stuck ({})", rec.id, crate::blocker::label(&r.blocker)),
                n => format!("{} has not received {n} message(s) ({})", rec.id, crate::blocker::label(&r.blocker)),
            };
            self.note_lead(
                &seat.run_id,
                &rec.id,
                &format!(
                    "Delivery problem: {what}. The user has a decision for it; you can also mcp__orch__nudge or mcp__orch__resume_agent {}.",
                    rec.id
                ),
            );
        }
        Some(id)
    }

    /// A note to the run's Lead about `about` (a message turn, so it goes
    /// out at the user tier and is not held behind queued work).
    pub(crate) fn note_lead(&self, run: &str, about: &str, text: &str) {
        let Ok(cx) = self.run_ctx(run) else { return };
        let Some(lead) = self.lead_of(&cx) else { return };
        if lead.id == about {
            return;
        }
        let node = lead.team.as_ref().map(|t| t.node.clone()).unwrap_or_default();
        let _ = self.add_run_message(
            &cx,
            "orchestrator",
            None,
            &node,
            Some(lead.id),
            "status",
            text,
            json!({"delivery": about}),
        );
    }

    /// An open decision, or a card built from live state, already names why
    /// this agent is stuck.
    fn names_cause(&self, rec: &AgentRecord) -> bool {
        rec.trust_dialog
            || matches!(rec.tracker.state, AgentState::WaitingForYou | AgentState::Error)
            || self.store.read(|s| {
                s.inbox.iter().any(|i| i.open() && i.kind != "info" && i.agent_id.as_deref() == Some(&rec.id))
            })
    }

    /// §3.7: no message stays unread over [`INVARIANT_MS`] while its target
    /// is not working on a turn, without a decision naming the cause.
    pub(crate) fn delivery_invariant(&self, cx: &RunCtx, now: u64) {
        let limit = self.secs_setting(crate::handover::INVARIANT_KEY, INVARIANT_MS);
        for rec in self.run_agents(&cx.run.id) {
            if rec.tracker.state == AgentState::Working {
                continue;
            }
            let old: Vec<Message> = self
                .unread(&rec)
                .into_iter()
                .filter(|m| {
                    // A retry starts the message's clock again.
                    let since = self.house().retried_at.get(&m.id).copied().unwrap_or(0).max(m.created_at_ms);
                    m.failed_at_ms.is_none() && m.delivered_at_ms.is_none() && now.saturating_sub(since) > limit
                })
                .collect();
            let Some(first) = old.first() else { continue };
            if self.names_cause(&rec) {
                continue;
            }
            let blocker = self.blocker_for(&rec, now);
            // The user's own choice.
            if blocker == "governor:paused" {
                continue;
            }
            let episode = if reserved_slot(&rec) { "stuck_lead" } else { "delivery" };
            let key = format!("{}:{episode}:{}", rec.id, first.id);
            let since_ms = first.created_at_ms;
            self.raise_delivery(
                &rec,
                Raise { episode, blocker, since_ms, turns: vec![], key, messages: old, stuck_event: true },
            );
        }
    }

    /// Closes the `delivery` decisions whose cause is over: every listed
    /// message seen (`delivered`), or, for one without messages, the agent
    /// active again (`recovered`).
    pub(crate) fn close_delivered(&self) {
        #[allow(clippy::type_complexity)]
        let open: Vec<(u64, Option<String>, Vec<u64>, u64, Option<String>)> = self.store.read(|s| {
            s.inbox
                .iter()
                .filter(|i| i.open() && i.kind == "delivery")
                .map(|i| {
                    let ids = i.payload["messages"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| m["id"].as_u64())
                        .collect();
                    (i.id, i.agent_id.clone(), ids, i.created_at_ms, i.payload["episode"].as_str().map(str::to_string))
                })
                .collect()
        });
        for (id, agent, ids, created, episode) in open {
            let done = if ids.is_empty() {
                // Over when the agent moved on: a turn started after the
                // decision, or (a silent turn) it is idle again. A stray
                // hook (a notification, a keystroke) proves nothing.
                let silent = episode.as_deref() == Some("working_silent");
                agent.as_deref().is_some_and(|a| {
                    self.store.read(|s| {
                        s.agents.get(a).is_some_and(|r| {
                            r.tracker.turn_started_ms.is_some_and(|t| t > created)
                                || (silent
                                    && r.tracker.state == AgentState::Idle
                                    && r.last_event_at_ms.is_some_and(|t| t > created))
                        })
                    })
                })
            } else {
                // A failed lookup closes nothing (an empty list would count as all seen).
                self.db
                    .messages_by_id(&ids)
                    .is_ok_and(|msgs| msgs.iter().all(|m| m.delivered_at_ms.is_some() || m.read_at_ms.is_some()))
            };
            if !done {
                continue;
            }
            let why = if ids.is_empty() { "recovered" } else { "delivered" };
            if let Some(i) = self.close_item(id, InboxStatus::Decided, why, "core") {
                self.advance(i.id, Stage::Effect(if ids.is_empty() { "recovered" } else { "delivered" }, None));
                self.outcome(&i, "core", why);
            }
        }
    }

    /// §3.7, the `Working` watchdog on a silent turn: when the transcript
    /// shows the turn ended, the missing Stop is applied (`recovered`);
    /// otherwise a team agent's silence is a decision for the user (and a
    /// note for its Lead). False for an agent with no team: the caller
    /// raises its plain alert.
    pub(crate) fn working_silent(&self, id: &str, turn_start: u64) -> bool {
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else { return true };
        let sum = crate::transcript::summarize_session(&self.cfg.projects_root, &rec.session_id);
        if crate::handover::turn_ended(&sum) {
            self.late_stop(&rec);
            self.delivery_event(
                id,
                "recovered",
                json!({"agent": id, "from_state": "working_silent", "how": "late_hook"}),
            );
            return true;
        }
        if rec.team.is_none() {
            return false;
        }
        self.raise_delivery(
            &rec,
            Raise {
                episode: "working_silent",
                blocker: "not_idle:working".into(),
                since_ms: turn_start,
                messages: vec![],
                turns: vec![],
                key: format!("{id}:working_silent:{turn_start}"),
                stuck_event: true,
            },
        );
        true
    }

    /// `decision_act(id, "retry")`: the messages (and failed turns) of a
    /// `delivery` decision go back to the queue with their attempts reset.
    pub fn retry_delivery(&self, item: u64) -> Result<Value> {
        let it = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == item && i.kind == "delivery").cloned())
            .ok_or_else(|| anyhow!("no delivery decision {item}"))?;
        if !it.open() {
            bail!("delivery decision {item} is already {:?}", it.status);
        }
        let agent = it.agent_id.clone().ok_or_else(|| anyhow!("decision {item} has no agent"))?;
        let ids: Vec<u64> =
            it.payload["messages"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_u64()).collect();
        let turns: Vec<String> = it.payload["turns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect();
        let requeued = self
            .store
            .write(|s| {
                let mut n = 0;
                // Failed turns of a watchdog decision, and a paste still in
                // flight that carries a message of an invariant decision.
                let carries = |t: &crate::store::TurnRow| t.messages.iter().any(|m| ids.contains(m));
                for t in s.turns.iter_mut().filter(|t| {
                    t.agent_id == agent
                        && ((turns.contains(&t.id) && t.status == TurnStatus::Failed)
                            || (t.status == TurnStatus::Delivering && carries(t)))
                }) {
                    t.requeue();
                    t.attempts = 0;
                    n += 1;
                }
                n
            })
            .unwrap_or(0);
        if requeued > 0
            && let Some(l) = self.live(&agent)
        {
            // A paste that was in flight zeroed `ready_at`; the agent takes input again.
            let at = orch_protocol::now_ms() + crate::delivery::AFTER_STOP.as_millis() as u64;
            l.ready_at_ms.store(at, std::sync::atomic::Ordering::SeqCst);
        }
        if !ids.is_empty() {
            self.db.retry_messages(&ids)?;
        }
        if it.payload["episode"] == "stuck_wait" {
            // The member waits on its Lead: it is the Lead that needs the nudge.
            let lead = self.store.read(|s| s.agents.get(&agent).and_then(|a| a.team.clone())).and_then(|seat| {
                let cx = self.run_ctx(&seat.run_id).ok()?;
                self.lead_of(&cx)
            });
            let lead = lead.ok_or_else(|| anyhow!("{agent} has no lead to nudge"))?;
            self.deliver_kind(
                &lead.id,
                &format!("ORCH: {agent} has waited a long time for your answer. Answer it, then carry on."),
                governor::TurnKind::Continue,
            )?;
        } else if ids.is_empty() && requeued == 0 {
            // A stuck agent with nothing to resend: a nudge to carry on.
            self.deliver_kind(
                &agent,
                "ORCH: you look stuck. Carry on with your task, or report where you are.",
                governor::TurnKind::Continue,
            )?;
        }
        self.delivery_event(
            &agent,
            "redelivered",
            json!({"agent": agent, "messages": ids, "attempt": 0, "why": "retry"}),
        );
        // What fails again after this retry is a new episode: the key is
        // released, and the invariant counts the messages' age from now, so
        // it does not raise a second decision at once.
        {
            let mut h = self.house();
            for id in &ids {
                h.retried_at.insert(*id, orch_protocol::now_ms());
            }
            if let Some(key) = it.payload["key"].as_str() {
                h.delivery_raised.remove(key);
            }
        }
        if let Some(key) = it.payload["key"].as_str() {
            let retired = json!(format!("{key}:retried"));
            let _ = self.store.write(|s| {
                if let Some(i) = s.inbox.iter_mut().find(|i| i.id == item) {
                    i.payload["key"] = retired;
                }
            });
        }
        if let Some(i) = self.close_item(item, InboxStatus::Decided, "retry", "user") {
            self.advance(i.id, Stage::Effect("retried", None));
            self.outcome(&i, "user", "retried");
        }
        Ok(
            json!({"ok": true, "effect": "retried", "lifecycle": self.lifecycle(item), "messages": ids, "turns": requeued}),
        )
    }
}
