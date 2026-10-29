//! Starts that never happen (FR-415): a spawn with no SessionStart gets a
//! `startup` Inbox item; a resume that dies at once with no transcript is
//! restarted once as a fresh session; a second quick death is the user's.
//! Also the pasted turns a process took down with it (FR-203).

use std::sync::atomic::Ordering;

use anyhow::{Result, bail};
use governor::TurnKind;
use orch_protocol::now_ms;
use orch_store::InboxStatus;
use serde_json::{Value, json};

use crate::compile::SessionMode;
use crate::inbox::PRIORITY_ALERT;
use crate::runtime::{Live, Runtime};
use crate::state::{AgentState, StopIntent};
use crate::store::{AgentRecord, TurnStatus};

/// A spawn that exits sooner than this did not start (FR-415).
pub const QUICK_EXIT_MS: u64 = 5_000;
/// The PTY tail kept in a `startup` item (its end).
pub const TAIL_MAX: usize = 4_096;

/// The last `TAIL_MAX` bytes of `s`, cut at a char boundary.
fn tail(s: &str) -> String {
    let mut from = s.len().saturating_sub(TAIL_MAX);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    s[from..].to_string()
}

impl Runtime {
    /// The agent's open `startup` item, if any.
    pub(crate) fn startup_item(&self, id: &str) -> Option<u64> {
        self.store.read(|s| {
            s.inbox.iter().find(|i| i.open() && i.kind == "startup" && i.agent_id.as_deref() == Some(id)).map(|i| i.id)
        })
    }

    /// Adds a `startup` item for the spawn `l` (`extra`: how it failed, if it did).
    fn raise_startup(&self, id: &str, l: &Live, title: String, extra: Value) {
        let mut payload = json!({
            "waiting_on": "startup", "mode": if l.resumed { "resume" } else { "new" },
            "since_ms": l.pty.spawned_at_ms, "tail": tail(&l.pty.output.tail_text()),
        });
        if let (Some(p), Value::Object(x)) = (payload.as_object_mut(), extra) {
            p.extend(x);
        }
        let _ = self.add_item(Some(id), "startup", title, payload, PRIORITY_ALERT);
    }

    /// Housekeeping: a live agent still starting past `startup_timeout`, with
    /// no trust dialog (that one is the user's to answer, J0), gets one item
    /// per spawn: one the user dismissed is not raised again.
    pub(crate) fn watch_startups(&self, now: u64) {
        let timeout = self.cfg.startup_timeout.as_millis() as u64;
        for id in self.live_ids() {
            let Some(l) = self.live(&id) else { continue };
            let since = l.pty.spawned_at_ms;
            if now.saturating_sub(since) < timeout || l.pty.output.trust_dialog.load(Ordering::Relaxed) {
                continue;
            }
            let due = self.store.read(|s| {
                let starting = s.agents.get(&id).is_some_and(|a| {
                    a.tracker.state == AgentState::Starting
                        && !a.trust_dialog
                        && a.tracker.stop_intent == StopIntent::None
                });
                // An item open from before, or one raised for this spawn (even dismissed).
                let raised = s.inbox.iter().any(|i| {
                    i.kind == "startup"
                        && i.agent_id.as_deref() == Some(id.as_str())
                        && (i.open() || i.created_at_ms >= since)
                });
                starting && !raised
            });
            if due {
                self.raise_startup(&id, &l, format!("{id} did not start"), Value::Null);
            }
        }
    }

    /// SessionStart: the agent started, its `startup` item closes and a
    /// failed start is behind it (a later resume resumes this session).
    pub(crate) fn started(&self, id: &str) {
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id).filter(|a| a.resume_failed) {
                a.resume_failed = false;
            }
        });
        let Some(item) = self.startup_item(id) else { return };
        if let Some(i) = self.close_item(item, InboxStatus::Dismissed, "started", "core") {
            self.advance(i.id, crate::inbox::Stage::Effect("dismissed", None));
            self.outcome(&i, "core", "dismissed");
        }
    }

    /// A pasted turn the process never started goes back to the queue (as
    /// the boot reconciler does), its team messages un-delivered; one the
    /// transcript shows was delivered.
    pub(crate) fn requeue_unstarted(&self, id: &str) {
        let session = self.store.read(|s| {
            let pasted = s.turns.iter().any(|t| t.agent_id == id && t.status == TurnStatus::Delivering);
            s.agents.get(id).filter(|_| pasted).map(|a| a.session_id.clone())
        });
        let Some(session) = session else { return };
        let seen = crate::transcript::summarize_session(&self.cfg.projects_root, &session).markers;
        let msgs = self.store.write(|s| {
            let mut msgs = vec![];
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id && t.status == TurnStatus::Delivering) {
                if seen.contains(&t.id) {
                    t.status = TurnStatus::Delivered;
                } else {
                    t.requeue();
                    msgs.extend(&t.messages);
                }
            }
            msgs
        });
        self.undeliver(msgs);
    }

    /// An exit nobody asked for, before SessionStart and within
    /// [`QUICK_EXIT_MS`] of the spawn: a resume with no transcript restarts
    /// fresh once; otherwise (a fresh or new start that died too) the user
    /// gets a `startup` item and the agent is in error, with no retry loop.
    pub(crate) fn check_quick_exit(&self, id: &str, l: &Live, status: Option<u32>) {
        if now_ms().saturating_sub(l.pty.spawned_at_ms) >= QUICK_EXIT_MS {
            return;
        }
        let Some(session) = self.store.read(|s| s.agents.get(id).map(|a| a.session_id.clone())) else { return };
        // A resume gets one fresh retry, which starts a new session (not `resumed`).
        let why = if l.resumed {
            if crate::transcript::find(&self.cfg.projects_root, &session).is_some() {
                return; // a session that exists: today's error path, not FR-415
            }
            match self.restart_fresh(id, "resume exited at once with no transcript", "core") {
                Ok(_) => return,
                Err(e) => format!("could not restart fresh: {e:#}"),
            }
        } else {
            "exited at once".to_string()
        };
        let failed = if l.resumed { "resume" } else { "fresh" };
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker.state = AgentState::Error;
                a.tracker.last_error = Some(format!("{id} could not start: {why}"));
                a.resume_failed = true;
            }
        });
        if self.startup_item(id).is_none() {
            let title = if l.resumed { format!("{id} could not resume") } else { format!("{id} could not start") };
            self.raise_startup(id, l, title, json!({"failed": failed, "exit_status": status}));
        }
    }

    /// Starts the agent again as a fresh session in the same worktree
    /// (FR-415): a new session id, its unstarted turns queued again, and its
    /// brief once (re-queued if it was among them, rebuilt otherwise).
    /// `by` closes its `startup` item (`user` from the Inbox, `core`). The
    /// new session id.
    pub fn restart_fresh(&self, id: &str, why: &str, by: &str) -> Result<String> {
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else { bail!("no agent {id}") };
        if rec.tracker.state == AgentState::Archived {
            bail!("agent {id} is deleted");
        }
        // Flagged until the spawn: the delivery loop must not resume the
        // agent meanwhile (a resume of the new, empty session dies at once).
        // A failure leaves the flag saying why.
        let flag = |msg: String| {
            self.store.write(|s| {
                if let Some(a) = s.agents.get_mut(id) {
                    a.flag = Some(msg);
                }
            })
        };
        flag(format!("{id}: restarting as a fresh session"))?;
        let r = self.restart_fresh_flagged(id, &rec, why, by);
        if let Err(e) = &r {
            let _ = flag(format!("{id}: could not restart fresh: {e:#}"));
        }
        r
    }

    /// [`Runtime::restart_fresh`] once the agent is flagged; the spawn clears the flag.
    fn restart_fresh_flagged(&self, id: &str, rec: &AgentRecord, why: &str, by: &str) -> Result<String> {
        if self.live(id).is_some() {
            self.suspend(id)?;
        }
        let new = self.prepare_fresh(id, rec, why, by)?;
        self.spawn(id, Some(SessionMode::New(new.clone())))?;
        Ok(new)
    }

    /// The lead's `resume_agent` after a failed start: the same fresh session,
    /// started by the delivery loop once a turn is admitted, so Pause all,
    /// the plan limits and the memory cap hold it like any resume (FR-411).
    pub fn queue_fresh_restart(&self, id: &str, why: &str) -> Result<String> {
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else { bail!("no agent {id}") };
        if rec.tracker.state == AgentState::Archived {
            bail!("agent {id} is deleted");
        }
        if self.live(id).is_some() {
            bail!("agent {id} is running");
        }
        let new = self.prepare_fresh(id, &rec, why, "core")?;
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.start_fresh = true;
                a.flag = None;
            }
        })?;
        if !self.store.read(|s| s.turns.iter().any(|t| t.agent_id == id && t.status == TurnStatus::Queued)) {
            self.deliver_kind(
                id,
                "ORCH: your lead restarted you as a fresh session. Carry on with your task, or report where you are.",
                TurnKind::Continue,
            )?;
        }
        Ok(new)
    }

    /// The shared part of a fresh restart: a new session, the unstarted turns
    /// queued again (their messages un-delivered), the `startup` item closed
    /// by `by`, and the brief once. The new session id.
    fn prepare_fresh(&self, id: &str, rec: &AgentRecord, why: &str, by: &str) -> Result<String> {
        let brief = self.brief_turn(rec);
        let (old, new, had_brief, msgs) = self.fresh_session(id, brief.as_deref())?;
        self.undeliver(Ok(msgs));
        if let Some(item) = self.startup_item(id)
            && let Some(i) = self.close_item(item, InboxStatus::Decided, "restart_fresh", by)
        {
            self.advance(i.id, crate::inbox::Stage::Effect("restarted", None));
            self.outcome(&i, by, "restarted");
        }
        if !had_brief && let Some(brief) = brief {
            // Queued before the spawn: the paste goes out once the session is ready.
            self.deliver_kind(id, &brief, TurnKind::Task)?;
        }
        self.event(id, "restarted_fresh", json!({"old_session": old, "new_session": new, "why": why}));
        Ok(new)
    }

    /// The store side of [`Runtime::restart_fresh`]: a new session id, the
    /// unstarted turns (pasted, or started in the lost session) queued again.
    /// Returns `(old, new, a queued turn is the brief, the re-queued turns'
    /// messages)`; the flag set by the caller stays until the spawn.
    pub(crate) fn fresh_session(&self, id: &str, brief: Option<&str>) -> Result<(String, String, bool, Vec<u64>)> {
        let new = uuid::Uuid::new_v4().to_string();
        self.store.write(|s| {
            let (mut had_brief, mut msgs) = (false, vec![]);
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id) {
                if matches!(t.status, TurnStatus::Delivering | TurnStatus::Delivered) {
                    t.requeue();
                    msgs.extend(&t.messages);
                }
                had_brief |= t.status == TurnStatus::Queued && !t.command && brief == Some(t.body.as_str());
            }
            let a = s.agents.get_mut(id).expect("checked by the caller");
            let old = std::mem::replace(&mut a.session_id, new.clone());
            a.resume_failed = false;
            a.tracker.last_error = None;
            if a.tracker.state == AgentState::Error {
                a.tracker.state = AgentState::Suspended;
            }
            (old, new.clone(), had_brief, msgs)
        })
    }

    /// Whether an agent's last start failed in a way only a fresh session fixes.
    pub fn resume_failed(&self, id: &str) -> bool {
        self.store.read(|s| s.agents.get(id).is_some_and(|a| a.resume_failed))
    }
}

/// The `restart_fresh` result for an Inbox action or the agent menu.
pub fn restarted(session: &str) -> Value {
    json!({"ok": true, "effect": "restarted", "session_id": session})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use crate::store::{Admission, TurnRow};

    fn turn(kind: TurnKind, status: TurnStatus) -> TurnRow {
        let mut t = TurnRow::queued("a1", "x", kind, Admission::Admitted);
        t.status = status;
        t
    }

    #[test]
    fn a_fresh_session_requeues_unstarted_turns_and_keeps_one_brief() {
        let (_d, rt) = test_runtime(true);
        rt.store
            .write(|s| {
                s.turns.push(turn(TurnKind::Task, TurnStatus::Delivered));
                s.turns.push(turn(TurnKind::Message, TurnStatus::Delivering));
                s.turns.push(turn(TurnKind::Message, TurnStatus::Done));
                let a = s.agents.get_mut("a1").unwrap();
                a.resume_failed = true;
                a.tracker.state = AgentState::Error;
            })
            .unwrap();
        let (old, new, had_brief, _) = rt.fresh_session("a1", Some("x")).unwrap();
        assert_eq!(old, "s");
        assert_ne!(new, old);
        assert!(had_brief, "the brief is re-queued, not rebuilt");
        let st: Vec<_> = rt.store.read(|s| s.turns.iter().map(|t| (t.status, t.admission)).collect());
        assert_eq!(st[..2], [(TurnStatus::Queued, Admission::Pending), (TurnStatus::Queued, Admission::Pending)]);
        assert_eq!(st[2].0, TurnStatus::Done, "a finished turn stays done");
        let a = rt.store.read(|s| s.agents["a1"].clone());
        assert_eq!((a.session_id, a.resume_failed, a.tracker.state), (new, false, AgentState::Suspended));
        // No turn is the brief: the caller rebuilds it (once).
        assert!(!rt.fresh_session("a1", Some("another brief")).unwrap().2);
    }

    /// FR-411: the lead's fresh restart is queued, not spawned: under Pause
    /// all it waits in the Governor's queue like any resume.
    #[test]
    fn a_lead_fresh_restart_waits_for_admission() {
        let (_d, rt) = test_runtime(false);
        rt.store
            .write(|s| {
                let a = s.agents.get_mut("a1").unwrap();
                a.resume_failed = true;
                a.tracker.state = AgentState::Error;
            })
            .unwrap();
        rt.pause_all();
        let new = rt.queue_fresh_restart("a1", "test").unwrap();
        let a = rt.store.read(|s| s.agents["a1"].clone());
        assert_eq!((a.session_id.as_str(), a.start_fresh, a.resume_failed), (new.as_str(), true, false));
        assert_eq!(a.tracker.state, AgentState::Suspended);
        let t = rt.store.read(|s| s.turns.last().cloned().unwrap());
        assert_eq!((t.status, t.kind), (TurnStatus::Queued, TurnKind::Continue));
        rt.pump();
        assert!(rt.live("a1").is_none(), "not spawned past Pause all");
        let t = rt.store.read(|s| s.turns.last().cloned().unwrap());
        assert!(matches!(t.admission, Admission::Waiting(_)), "{:?}", t.admission);
        assert!(rt.store.read(|s| s.agents["a1"].flag.is_none()));
        assert!(rt.queue_fresh_restart("ghost", "x").is_err());
    }

    #[test]
    fn pasted_turns_a_process_took_down_go_back_to_the_queue() {
        let (_d, rt) = test_runtime(true);
        rt.store.write(|s| s.turns.push(turn(TurnKind::Message, TurnStatus::Delivering))).unwrap();
        rt.requeue_unstarted("a1");
        let t = rt.store.read(|s| s.turns[0].clone());
        assert_eq!((t.status, t.admission, t.delivered_at_ms), (TurnStatus::Queued, Admission::Pending, None));
    }

    #[test]
    fn session_start_closes_the_startup_item_and_tails_are_capped() {
        let (_d, rt) = test_runtime(true);
        let id = rt
            .add_item(Some("a1"), "startup", "a1 did not start".into(), json!({"waiting_on": "startup"}), 10)
            .unwrap();
        rt.started("a1");
        let i = rt.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned().unwrap());
        assert_eq!((i.status, i.decided_by.as_deref()), (InboxStatus::Dismissed, Some("core")));
        rt.started("a1"); // nothing open: no-op
        let long = "é".repeat(5_000);
        let t = tail(&long);
        assert!(t.len() <= TAIL_MAX && long.ends_with(&t));
    }
}
