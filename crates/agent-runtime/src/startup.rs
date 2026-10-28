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
use crate::state::AgentState;
use crate::store::{Admission, TurnStatus};

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
    fn startup_item(&self, id: &str) -> Option<u64> {
        self.store.read(|s| {
            s.inbox.iter().find(|i| i.open() && i.kind == "startup" && i.agent_id.as_deref() == Some(id)).map(|i| i.id)
        })
    }

    /// Housekeeping: a live agent still starting past `startup_timeout`, with
    /// no trust dialog (that one is the user's to answer, J0), gets one item.
    pub(crate) fn watch_startups(&self, now: u64) {
        let timeout = self.cfg.startup_timeout.as_millis() as u64;
        for id in self.live_ids() {
            let Some(l) = self.live(&id) else { continue };
            let starting = self.store.read(|s| {
                s.agents.get(&id).is_some_and(|a| a.tracker.state == AgentState::Starting && !a.trust_dialog)
            });
            if !starting
                || l.pty.output.trust_dialog.load(Ordering::Relaxed)
                || now.saturating_sub(l.pty.spawned_at_ms) < timeout
                || self.startup_item(&id).is_some()
            {
                continue;
            }
            let payload = json!({
                "waiting_on": "startup", "mode": if l.resumed { "resume" } else { "new" },
                "since_ms": l.pty.spawned_at_ms, "tail": tail(&l.pty.output.tail_text()),
            });
            let _ = self.add_item(Some(&id), "startup", format!("{id} did not start"), payload, PRIORITY_ALERT);
        }
    }

    /// SessionStart: the agent started, its `startup` item closes.
    pub(crate) fn started(&self, id: &str) {
        let Some(item) = self.startup_item(id) else { return };
        let closed = self
            .store
            .write(|s| s.close_inbox(item, InboxStatus::Dismissed, Some("started".into()), "core").ok().cloned());
        if let Ok(Some(i)) = closed {
            self.outcome(&i, "core", "dismissed");
        }
    }

    /// A pasted turn the process never started goes back to the queue (as
    /// the boot reconciler does), its team messages un-delivered; one the
    /// transcript shows was delivered.
    pub(crate) fn requeue_unstarted(&self, id: &str) {
        if !self.store.read(|s| s.turns.iter().any(|t| t.agent_id == id && t.status == TurnStatus::Delivering)) {
            return;
        }
        let Some(session) = self.store.read(|s| s.agents.get(id).map(|a| a.session_id.clone())) else { return };
        let seen = crate::transcript::summarize_session(&self.cfg.projects_root, &session).markers;
        let msgs = self.store.write(|s| {
            let mut msgs = vec![];
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id && t.status == TurnStatus::Delivering) {
                if seen.contains(&t.id) {
                    t.status = TurnStatus::Delivered;
                } else {
                    t.status = TurnStatus::Queued;
                    t.admission = Admission::Pending;
                    t.delivered_at_ms = None;
                    msgs.extend(&t.messages);
                }
            }
            msgs
        });
        if let Ok(msgs) = msgs
            && !msgs.is_empty()
        {
            let _ = self.db.undeliver_messages(&msgs);
        }
    }

    /// An exit nobody asked for, within [`QUICK_EXIT_MS`] of the spawn: a
    /// resume with no transcript restarts fresh once; otherwise (a fresh or
    /// new start that died too) the user gets a `startup` item and the agent
    /// is in error, with no retry loop.
    pub(crate) fn check_quick_exit(&self, id: &str, l: &Live, status: Option<u32>) {
        if now_ms().saturating_sub(l.pty.spawned_at_ms) >= QUICK_EXIT_MS {
            return;
        }
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else { return };
        let no_transcript = crate::transcript::find(&self.cfg.projects_root, &rec.session_id).is_none();
        let why = if l.resumed && !l.fresh_retry {
            if !no_transcript {
                return; // a session that exists: today's error path, not FR-415
            }
            match self.restart_fresh(id, "resume exited at once with no transcript", "core") {
                Ok(_) => return,
                Err(e) => format!("could not restart fresh: {e:#}"),
            }
        } else {
            "exited at once".to_string()
        };
        let failed = if l.resumed && !l.fresh_retry { "resume" } else { "fresh" };
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker.state = AgentState::Error;
                a.tracker.last_error = Some(format!("{id} could not start: {why}"));
                a.resume_failed = true;
            }
        });
        if self.startup_item(id).is_none() {
            let payload = json!({
                "waiting_on": "startup", "mode": if l.resumed { "resume" } else { "new" },
                "since_ms": l.pty.spawned_at_ms, "failed": failed, "exit_status": status,
                "tail": tail(&l.pty.output.tail_text()),
            });
            let _ = self.add_item(Some(id), "startup", format!("{id} could not resume"), payload, PRIORITY_ALERT);
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
        if self.live(id).is_some() {
            self.suspend(id)?;
        }
        let (old, new, had_brief) = self.fresh_session(id)?;
        if let Some(item) = self.startup_item(id) {
            let closed = self
                .store
                .write(|s| s.close_inbox(item, InboxStatus::Decided, Some("restart_fresh".into()), by).ok().cloned());
            if let Ok(Some(i)) = closed {
                self.outcome(&i, by, "restarted");
            }
        }
        if !had_brief && let Some(brief) = self.brief_turn(&rec) {
            // Queued before the spawn: the paste goes out once the session is ready.
            self.deliver_kind(id, &brief, TurnKind::Task)?;
        }
        self.event(id, "restarted_fresh", json!({"old_session": old, "new_session": new, "why": why}));
        self.spawn_with(id, Some(SessionMode::New(new.clone())), true)?;
        Ok(new)
    }

    /// The store side of [`Runtime::restart_fresh`]: a new session id, the
    /// unstarted turns (pasted, or started in the lost session) queued again.
    /// Returns `(old, new, a brief/task turn is among the queued ones)`.
    pub(crate) fn fresh_session(&self, id: &str) -> Result<(String, String, bool)> {
        let new = uuid::Uuid::new_v4().to_string();
        self.store.write(|s| {
            let mut had_brief = false;
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id) {
                if matches!(t.status, TurnStatus::Delivering | TurnStatus::Delivered) {
                    t.status = TurnStatus::Queued;
                    t.admission = Admission::Pending;
                    t.delivered_at_ms = None;
                }
                had_brief |= t.status == TurnStatus::Queued && t.kind == TurnKind::Task;
            }
            let a = s.agents.get_mut(id).expect("checked by the caller");
            let old = std::mem::replace(&mut a.session_id, new.clone());
            a.resume_failed = false;
            a.flag = None;
            a.tracker.last_error = None;
            if a.tracker.state == AgentState::Error {
                a.tracker.state = AgentState::Suspended;
            }
            (old, new.clone(), had_brief)
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
    use crate::store::TurnRow;

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
        let (old, new, had_brief) = rt.fresh_session("a1").unwrap();
        assert_eq!(old, "s");
        assert_ne!(new, old);
        assert!(had_brief, "the brief is re-queued, not rebuilt");
        let st: Vec<_> = rt.store.read(|s| s.turns.iter().map(|t| (t.status, t.admission)).collect());
        assert_eq!(st[..2], [(TurnStatus::Queued, Admission::Pending), (TurnStatus::Queued, Admission::Pending)]);
        assert_eq!(st[2].0, TurnStatus::Done, "a finished turn stays done");
        let a = rt.store.read(|s| s.agents["a1"].clone());
        assert_eq!((a.session_id, a.resume_failed, a.tracker.state), (new, false, AgentState::Suspended));
        // No task turn left: the caller rebuilds the brief (once).
        rt.store.write(|s| s.turns.retain(|t| t.kind != TurnKind::Task)).unwrap();
        assert!(!rt.fresh_session("a1").unwrap().2);
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
