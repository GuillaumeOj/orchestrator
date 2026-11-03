//! The hand-over of pastes and team messages to the CLI (FR-206, FR-403;
//! fix-team-getting-stucked §2, §3.1, §3.3, §3.4). Each step of a message's
//! lifecycle is set only by evidence from the CLI:
//!
//! - **handed**: pasted in a turn, or injected as tool-call context;
//! - **seen**: the paste's marker came back in UserPromptSubmit, or orch-hook
//!   confirmed it printed the injecting reply ([`Runtime::on_hook_ack`]);
//! - **read**: at the end of the turn, when the model saw all of it
//!   ([`Runtime::settle_messages`]);
//! - back to **queued** when nothing acknowledged it, **failed** after the
//!   paste watchdog's last attempt ([`Runtime::paste_watchdog`]).
//!
//! Spooled hook events are replayed while the core runs, not only at boot
//! ([`Runtime::replay_spool`]).

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use orch_protocol::{HookEvent, HookMsg, HookPayload, Reply, now_ms, spool};
use orch_store::Message;
use serde_json::{Value, json};

use crate::decision::last_lines;
use crate::delivery::{self, marker};
use crate::runtime::{Live, Runtime};
use crate::state::AgentState;
use crate::store::{AgentRecord, TurnRow, TurnStatus};

/// Pastes the CLI may leave unacknowledged before the turn fails.
const MAX_ATTEMPTS: u32 = 3;
/// Setting: seconds a paste may wait for its UserPromptSubmit before the
/// watchdog checks it.
pub const ACK_KEY: &str = "delivery.ack_secs";
const DEFAULT_ACK: Duration = Duration::from_secs(20);
/// `last_error` of a message handed back because the model did not see all of it.
const NOT_FULLY_SEEN: &str = "not_fully_seen";
/// Lines of the PTY tail kept with a delivery failure.
const TAIL_LINES: usize = 40;

/// A hook fired inside a subagent: Claude Code 2.1.x sets `agent_id` on hook
/// payloads only there (the main agent's carry none). Messages injected into
/// a subagent would reach its context, not the agent's, so they wait for the
/// agent's own next tool call or turn. (Fallback, if a CLI version dropped
/// the field: an open SubagentStart, `Tracker::subagents`.)
pub(crate) fn in_subagent(p: &HookPayload) -> bool {
    p.agent_id.as_deref().is_some_and(|a| !a.is_empty())
}

/// Names of files under a `messages/` folder that a tool call's input refers
/// to (`Read` of the path, `cat "…/messages/msg-4.md"`).
pub(crate) fn referenced_files(input: &Value) -> Vec<String> {
    let text = input.to_string();
    let mut out = vec![];
    let mut rest = text.as_str();
    while let Some(i) = rest.find("messages/") {
        rest = &rest[i + "messages/".len()..];
        let name: String =
            rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
        if name.ends_with(".md") {
            out.push(name);
        }
    }
    out
}

/// Whether the model saw all of `m`: every file it was written to (the
/// turn's paste file, its own `msg-<id>.md` when too long) was read, or the
/// agent called `inbox`.
pub(crate) fn fully_seen(m: &Message, paste_file: Option<&str>, referenced: &BTreeSet<String>, inbox: bool) -> bool {
    if inbox {
        return true;
    }
    let own = (m.body.chars().count() > crate::team::inline_cap(m.via.as_deref()))
        .then(|| crate::team::message_file_name(m.id));
    paste_file.map(str::to_string).into_iter().chain(own).all(|f| referenced.contains(&f))
}

/// A hook the CLI did send but the core never got, rebuilt from the transcript.
fn synthetic_hook(rec: &AgentRecord, event: HookEvent, mut payload: Value) -> HookMsg {
    payload["hook_event_name"] = json!(event.as_str());
    if let Some(sid) = &rec.tracker.session_id {
        payload["session_id"] = json!(sid);
    }
    HookMsg { agent_id: rec.id.clone(), event, ts_ms: now_ms(), deadline_ms: 0, payload, acks: vec![] }
}

/// The transcript ends with the agent's finished answer.
pub(crate) fn turn_ended(s: &crate::transcript::Summary) -> bool {
    s.last_type.as_deref() == Some("assistant") && s.last_stop_reason.as_deref() == Some("end_turn")
}

/// What the paste watchdog reads for one unacknowledged paste.
pub(crate) struct PasteFacts<'a> {
    /// Turn markers in the transcript.
    pub markers: BTreeSet<String>,
    /// The transcript ends with the agent's finished answer.
    pub turn_ended: bool,
    /// The PTY's plain-text tail.
    pub tail: &'a str,
    pub idle: bool,
    /// The user has not typed for [`delivery::USER_QUIET`].
    pub quiet: bool,
    /// No hook event came since the paste.
    pub silent: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// 1. The CLI took it: its UserPromptSubmit (and Stop) applied late.
    LateHook { seen: Vec<String>, ended: bool },
    /// 2. Still in the input box: Enter once more.
    Enter,
    /// 3. A menu or modal ate the keys: Esc once, then queued again or failed.
    Esc,
    /// Something else is going on (a turn, the user): look again later.
    Wait,
}

/// The paste watchdog's rule (§3.1), in this order: the transcript first,
/// so Esc is never sent for a turn the CLI started.
pub(crate) fn next_step(turns: &[TurnRow], f: &PasteFacts) -> Step {
    let seen: Vec<String> =
        turns.iter().filter(|t| !t.command && f.markers.contains(&t.id)).map(|t| t.id.clone()).collect();
    if !seen.is_empty() {
        return Step::LateHook { seen, ended: f.turn_ended };
    }
    let flat: String = f.tail.split_whitespace().collect();
    let visible = flat.contains("[Pastedtext")
        || turns.iter().any(|t| {
            if t.command {
                flat.contains(&t.body.split_whitespace().collect::<String>())
            } else {
                flat.contains(&marker(&t.id))
            }
        });
    if visible && !turns.iter().all(|t| t.enter_retried) {
        // Never while the user types: the Enter would send their draft.
        return if f.quiet { Step::Enter } else { Step::Wait };
    }
    if f.idle && f.quiet && f.silent { Step::Esc } else { Step::Wait }
}

/// What the paste watchdog gave up on (the `delivery` decision of T-207).
#[derive(Clone, Debug)]
pub struct DeliveryFailure {
    pub agent: String,
    pub turns: Vec<String>,
    pub messages: Vec<u64>,
    pub attempts: u32,
    pub blocker: &'static str,
    pub since_ms: u64,
    pub tail: String,
}

impl Runtime {
    /// A delivery event, on the run's timeline for a team agent.
    pub(crate) fn delivery_event(&self, agent: &str, kind: &str, mut detail: Value) {
        if let Some(o) = detail.as_object_mut() {
            o.entry("agent").or_insert_with(|| json!(agent));
        }
        match self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.as_ref().map(|t| t.run_id.clone()))) {
            Some(run) => self.run_event(&run, Some(agent), kind, detail),
            None => self.event(agent, kind, detail),
        }
    }

    /// Answers one hook through `write` (the reply line): the state change
    /// is applied first, and a reply carrying team messages that could not
    /// be written gives them back.
    pub fn serve_hook(
        &self,
        m: &HookMsg,
        conn: Option<&std::os::unix::net::UnixStream>,
        write: impl FnOnce(Reply) -> bool,
    ) {
        let (output, ack) = self.on_hook_reply(m, conn);
        if !write(Reply::Hook { output, ack: ack.clone() }) && !ack.is_empty() {
            self.injection_lost(&m.agent_id, &ack);
        }
    }

    /// orch-hook printed the reply that injected `ids`: they are seen.
    pub fn on_hook_ack(&self, agent: &str, ids: &[u64]) {
        let mine: Vec<u64> = self
            .db
            .messages_by_id(ids)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.to_agent.as_deref() == Some(agent) && m.handed_at_ms.is_some() && m.delivered_at_ms.is_none())
            .map(|m| m.id)
            .collect();
        if mine.is_empty() {
            return;
        }
        let _ = self.db.see_messages(&mine, now_ms());
        self.delivery_event(agent, "seen", json!({"messages": mine, "via": "inject"}));
    }

    /// The reply that injected `ids` never left the core: queued again.
    pub(crate) fn injection_lost(&self, agent: &str, ids: &[u64]) {
        let _ = self.db.requeue_messages(ids, Some("write_failed"), true);
        self.delivery_event(agent, "redelivered", json!({"messages": ids, "why": "write_failed", "via": "inject"}));
    }

    /// Injected (before a restart, or into a turn that ended) and never
    /// acknowledged: back to the queue.
    pub(crate) fn requeue_injected_unseen(&self, agent: Option<&str>, why: &str) {
        let unseen: Vec<Message> = self
            .db
            .injected_unread(agent)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.delivered_at_ms.is_none())
            .collect();
        if unseen.is_empty() {
            return;
        }
        let ids: Vec<u64> = unseen.iter().map(|m| m.id).collect();
        let _ = self.db.requeue_messages(&ids, Some(why), true);
        let mut by_agent: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
        for m in unseen {
            by_agent.entry(m.to_agent.unwrap_or_default()).or_default().push(m.id);
        }
        for (a, ids) in by_agent {
            self.delivery_event(&a, "redelivered", json!({"messages": ids, "why": why, "via": "inject"}));
        }
    }

    pub(crate) fn note_inbox_called(&self, agent: &str) {
        self.house().inbox_called.insert(agent.to_string());
    }

    pub(crate) fn note_referenced(&self, agent: &str, p: &HookPayload) {
        // A subagent's Read reaches its own context, not the agent's.
        if in_subagent(p) {
            return;
        }
        let Some(input) = &p.tool_input else { return };
        let files = referenced_files(input);
        if !files.is_empty() {
            self.house().referenced.entry(agent.to_string()).or_default().extend(files);
        }
    }

    /// TurnEnded (§3.3, §3.4): the messages of the turns that ended, and the
    /// ones injected into the agent's turns, are read when the model saw all
    /// of them; one not fully seen is handed back once, then read with
    /// `truncated`. Injected ones never acknowledged go back to the queue.
    pub(crate) fn settle_messages(&self, rec: &AgentRecord, ended: &[TurnRow], now: u64) {
        let id = rec.id.as_str();
        let (referenced, inbox) = {
            let mut h = self.house();
            (h.referenced.remove(id).unwrap_or_default(), h.inbox_called.remove(id))
        };
        if rec.team.is_none() {
            return;
        }
        let mut candidates: Vec<(Message, Option<String>)> = vec![];
        for t in ended.iter().filter(|t| !t.messages.is_empty()) {
            for m in self.db.messages_by_id(&t.messages).unwrap_or_default() {
                candidates.push((m, t.file.clone()));
            }
        }
        self.requeue_injected_unseen(Some(id), "no_ack");
        for m in self.db.injected_unread(Some(id)).unwrap_or_default() {
            if !candidates.iter().any(|(c, _)| c.id == m.id) {
                candidates.push((m, None));
            }
        }
        let (mut read, mut truncated, mut back) = (vec![], vec![], vec![]);
        for (m, file) in candidates {
            if m.read_at_ms.is_some() || m.failed_at_ms.is_some() {
                continue;
            }
            if fully_seen(&m, file.as_deref(), &referenced, inbox) {
                read.push(m.id);
            } else if m.last_error.as_deref() == Some(NOT_FULLY_SEEN) {
                truncated.push(m.id);
            } else {
                back.push(m.id);
            }
        }
        if !read.is_empty() {
            let _ = self.db.ack_messages(&read, now);
            self.delivery_event(id, "read", json!({"messages": read}));
        }
        if !truncated.is_empty() {
            let _ = self.db.ack_messages(&truncated, now);
            self.delivery_event(id, "read", json!({"messages": truncated, "truncated": true}));
        }
        if !back.is_empty() {
            let _ = self.db.requeue_messages(&back, Some(NOT_FULLY_SEEN), false);
            self.delivery_event(id, "redelivered", json!({"messages": back, "why": NOT_FULLY_SEEN}));
        }
    }

    /// Before a turn is pasted again: one the transcript shows the CLI took
    /// is started late instead (true), never typed twice.
    pub(crate) fn already_in_transcript(&self, rec: &AgentRecord, batch: &[TurnRow]) -> bool {
        let sum = crate::transcript::summarize_session(&self.cfg.projects_root, &rec.session_id);
        let taken: Vec<&TurnRow> = batch.iter().filter(|t| sum.markers.contains(&t.id)).collect();
        if taken.is_empty() {
            return false;
        }
        // Their messages went back to the queue with the turn: handed again,
        // so the late prompt marks them seen and they are never sent twice.
        let now = now_ms();
        for t in taken.iter().filter(|t| !t.messages.is_empty()) {
            let _ = self.db.hand_messages(&t.messages, &rec.id, "turn", Some(&t.id), now);
        }
        let seen: Vec<String> = taken.iter().map(|t| t.id.clone()).collect();
        // The tracker learns the turn started (and ended): nothing is pasted into it.
        self.late_turn_start(rec, &seen, turn_ended(&sum));
        true
    }

    // ------------------------------------------------------------ the 5 s pass

    /// The watchdog and spool pass (housekeeping, off the delivery loop).
    pub fn handover_tick(&self, now: u64) {
        self.sweep_spools();
        self.paste_watchdog(now);
    }

    fn ack_ms(&self) -> u64 {
        self.db
            .setting(ACK_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_f64())
            .map(|s| (s.max(1.0) * 1000.0) as u64)
            .unwrap_or(DEFAULT_ACK.as_millis() as u64)
    }

    /// §3.1: a paste with no UserPromptSubmit within `delivery.ack_secs`.
    /// 1. its marker is in the transcript: the hook was lost, applied late;
    /// 2. its text is still in the input box: Enter once more;
    /// 3. otherwise (a menu or modal ate the keys), only when the agent is
    ///    idle, the user quiet and no hook came since the paste: Esc once,
    ///    and the turn goes back to the queue (`attempts` + 1). After
    ///    [`MAX_ATTEMPTS`] it fails. Either way `ready_at` is restored, so
    ///    no paste ever wedges the agent.
    pub fn paste_watchdog(&self, now: u64) {
        let ack = self.ack_ms();
        let due: Vec<(AgentRecord, Vec<TurnRow>)> = self.store.read(|s| {
            let mut by: std::collections::BTreeMap<String, Vec<TurnRow>> = Default::default();
            for t in s.turns.iter().filter(|t| t.status == TurnStatus::Delivering) {
                if t.delivered_at_ms.unwrap_or(0) + ack <= now {
                    by.entry(t.agent_id.clone()).or_default().push(t.clone());
                }
            }
            by.into_iter().filter_map(|(id, turns)| Some((s.agents.get(&id)?.clone(), turns))).collect()
        });
        for (rec, turns) in due {
            // A process gone gives its pastes back on its exit (FR-203).
            let Some(l) = self.live(&rec.id).filter(|l| !l.pty.has_exited()) else { continue };
            self.check_paste(&rec, &l, &turns, now);
        }
    }

    fn check_paste(&self, rec: &AgentRecord, l: &Live, turns: &[TurnRow], now: u64) {
        let id = rec.id.as_str();
        let attempt = turns.iter().map(|t| t.attempts).max().unwrap_or(0) + 1;
        let sum = turns
            .iter()
            .any(|t| !t.command)
            .then(|| crate::transcript::summarize_session(&self.cfg.projects_root, &rec.session_id));
        let tail = l.pty.output.tail_text();
        let pasted = turns.iter().filter_map(|t| t.delivered_at_ms).max().unwrap_or(0);
        let facts = PasteFacts {
            markers: sum.as_ref().map(|s| s.markers.clone()).unwrap_or_default(),
            turn_ended: sum.as_ref().is_some_and(turn_ended),
            tail: &tail,
            idle: rec.tracker.state == AgentState::Idle,
            quiet: now >= l.last_user_input_ms.load(Ordering::SeqCst) + delivery::USER_QUIET.as_millis() as u64,
            silent: rec.last_event_at_ms.unwrap_or(0) <= pasted,
        };
        match next_step(turns, &facts) {
            Step::LateHook { seen, ended } => self.late_turn_start(rec, &seen, ended),
            Step::Enter => {
                if l.pty.write(b"\r").is_ok() {
                    let ids: Vec<&str> = turns.iter().map(|t| t.id.as_str()).collect();
                    let _ = self.store.write(|s| {
                        for t in s.turns.iter_mut().filter(|t| ids.contains(&t.id.as_str())) {
                            t.enter_retried = true;
                            t.delivered_at_ms = Some(now);
                        }
                    });
                    self.delivery_event(
                        id,
                        "redelivered",
                        json!({"turns": ids, "attempt": attempt, "why": "no_ack", "how": "enter"}),
                    );
                }
            }
            Step::Esc => {
                let _ = l.pty.write(b"\x1b");
                self.take_back_paste(rec, Some(l), turns, attempt, &tail, now);
            }
            Step::Wait => {}
        }
    }

    /// The Stop the core never got, applied from the transcript (§3.7).
    pub(crate) fn late_stop(&self, rec: &AgentRecord) {
        self.apply_hook(&synthetic_hook(rec, HookEvent::Stop, json!({})), None, true);
    }

    /// The marker is in the transcript: the UserPromptSubmit (and, when the
    /// turn is over there, the Stop) applied late.
    pub(crate) fn late_turn_start(&self, rec: &AgentRecord, seen: &[String], ended: bool) {
        let mk = |event: HookEvent, payload: Value| synthetic_hook(rec, event, payload);
        let prompt = seen.iter().map(|t| marker(t)).collect::<Vec<_>>().join(" ");
        self.apply_hook(&mk(HookEvent::UserPromptSubmit, json!({"prompt": prompt})), None, true);
        if ended {
            self.apply_hook(&mk(HookEvent::Stop, json!({})), None, true);
        }
        self.delivery_event(
            &rec.id,
            "recovered",
            json!({"turns": seen, "how": "late_hook", "from_state": rec.tracker.state, "ended": ended}),
        );
    }

    /// Step 3: the paste goes back to the queue, or fails at its last attempt.
    pub(crate) fn take_back_paste(
        &self,
        rec: &AgentRecord,
        l: Option<&Live>,
        turns: &[TurnRow],
        attempt: u32,
        tail: &str,
        now: u64,
    ) {
        let ids: Vec<String> = turns.iter().map(|t| t.id.clone()).collect();
        let failed = attempt >= MAX_ATTEMPTS;
        let msgs = self
            .store
            .write(|s| {
                let mut msgs = vec![];
                for t in s.turns.iter_mut().filter(|t| ids.contains(&t.id) && t.status == TurnStatus::Delivering) {
                    t.attempts = attempt;
                    if failed {
                        t.status = TurnStatus::Failed;
                    } else {
                        t.requeue();
                    }
                    msgs.extend(&t.messages);
                }
                msgs
            })
            .unwrap_or_default();
        // The agent takes input again (its other turns, the user's).
        if let Some(l) = l {
            l.ready_at_ms.store(now + delivery::AFTER_STOP.as_millis() as u64, Ordering::SeqCst);
        }
        if failed {
            let _ = self.db.fail_messages(&msgs, "no_ack", now);
            let since = turns.iter().map(|t| t.created_at_ms).min().unwrap_or(now);
            self.on_delivery_failed(&DeliveryFailure {
                agent: rec.id.clone(),
                turns: ids,
                messages: msgs,
                attempts: attempt,
                blocker: "no_ack",
                since_ms: since,
                tail: last_lines(tail, TAIL_LINES),
            });
        } else {
            let _ = self.db.requeue_messages(&msgs, Some("no_ack"), true);
            self.delivery_event(
                &rec.id,
                "redelivered",
                json!({"turns": ids, "messages": msgs, "attempt": attempt, "why": "no_ack", "how": "esc"}),
            );
        }
    }

    /// The paste watchdog gave up: the turns are failed and the agent takes
    /// its other turns. The `delivery` decision (api-contract §2) and the
    /// note to the agent's Lead are raised here.
    pub(crate) fn on_delivery_failed(&self, f: &DeliveryFailure) {
        let rec = self.store.read(|s| s.agents.get(&f.agent).cloned());
        let messages = self.db.messages_by_id(&f.messages).unwrap_or_default();
        let key = format!(
            "{}:delivery:{}",
            f.agent,
            f.messages.first().map(u64::to_string).or(f.turns.first().cloned()).unwrap_or_default()
        );
        let decision = rec.and_then(|rec| {
            self.raise_delivery(
                &rec,
                crate::stuck::Raise {
                    episode: "delivery",
                    blocker: f.blocker.into(),
                    since_ms: f.since_ms,
                    messages,
                    turns: f.turns.clone(),
                    key,
                    stuck_event: false,
                },
            )
        });
        self.delivery_event(
            &f.agent,
            "delivery_failed",
            json!({"turns": f.turns, "messages": f.messages, "attempts": f.attempts, "blocker": f.blocker,
                   "since_ms": f.since_ms, "tail": f.tail,
                   "decision": decision.map(|d| crate::decision::decision_id("delivery", d))}),
        );
    }

    // ------------------------------------------------------------ spool

    fn sweep_spools(&self) {
        let ids: Vec<String> = self.store.read(|s| s.agents.keys().cloned().collect());
        for id in ids {
            self.replay_spool(&id);
        }
    }

    /// §3.3: the events orch-hook spooled while the core was unreachable or
    /// late, applied as soon as it is back, in their order. An event older
    /// than the last one the core applied is stale (a turn that already
    /// ended, or one the core got anyway) and dropped; a stale
    /// UserPromptSubmit still marks its turns delivered.
    pub(crate) fn replay_spool(&self, id: &str) {
        let dir = self.cfg.paths.agent_dir(id);
        // Taken before the check: a live hook waits for a sweep that took the
        // spool, so its own event never applies before the older ones.
        let _replaying = self.replaying.lock().unwrap_or_else(|e| e.into_inner());
        if !spool::spool_path(&dir).exists() {
            return;
        }
        let Ok((records, malformed)) = spool::take(&dir) else { return };
        let Some(last) = self.store.read(|s| s.agents.get(id).map(|a| a.last_event_at_ms.unwrap_or(0))) else {
            return;
        };
        let (mut applied, mut dropped) = (0, 0);
        for r in records {
            if r.ts_ms < last {
                if r.event == HookEvent::UserPromptSubmit {
                    let p = HookPayload::parse(&r.payload);
                    self.mark_delivered(id, &delivery::find_markers(p.prompt.as_deref().unwrap_or("")), None);
                }
                dropped += 1;
                continue;
            }
            let m = HookMsg {
                agent_id: id.to_string(),
                event: r.event,
                ts_ms: r.ts_ms,
                deadline_ms: 0,
                payload: r.payload,
                acks: vec![],
            };
            self.apply_hook(&m, None, true);
            applied += 1;
        }
        self.event(id, "spool_replayed", json!({"applied": applied, "dropped": dropped, "malformed": malformed}));
    }
}

#[cfg(test)]
mod tests;
