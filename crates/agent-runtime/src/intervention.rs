//! Pane interventions (FR-1311): the user typing into an agent's pane while
//! the agent waits on something, recorded once per wait episode so the run
//! report can count them by cause. Terminal replies (focus, cursor position
//! and device attribute reports, mouse reports) are not typing.

use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow};
use orch_protocol::now_ms;
use serde_json::json;

use crate::runtime::{Live, Runtime};
use crate::state::AgentState;
use crate::store::{StateFile, TurnStatus};

/// What an agent waits on, in priority order (the first that applies wins).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    pub cause: &'static str,
    pub item: Option<u64>,
    pub since_ms: u64,
}

/// `bytes` with the terminal's own replies taken out: what is left was typed.
pub fn typed(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b
            && let Some(n) = reply_len(&bytes[i..])
        {
            i += n;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// The length of a terminal reply starting at `b[0] == ESC`, if it is one:
/// focus `ESC[I` / `ESC[O`, CPR `ESC[r;cR`, DA `ESC[?…c` / `ESC[>…c`, SGR
/// mouse `ESC[<b;x;yM|m`, X10 mouse `ESC[M` + 3 bytes.
fn reply_len(b: &[u8]) -> Option<usize> {
    if b.get(1) != Some(&b'[') {
        return None;
    }
    match b.get(2)? {
        b'I' | b'O' => return Some(3),
        b'M' => return (b.len() >= 6).then_some(6),
        _ => {}
    }
    // CSI parameters, then the final byte.
    let params = b[2..].iter().position(|c| !(c.is_ascii_digit() || matches!(c, b';' | b'?' | b'>' | b'<')))?;
    let end = 2 + params;
    let fin = *b.get(end)?;
    let lead = b[2];
    let is_reply = match fin {
        b'R' => lead.is_ascii_digit() && b[2..end].contains(&b';'),
        b'c' => matches!(lead, b'?' | b'>'),
        b'M' | b'm' => lead == b'<',
        _ => false,
    };
    is_reply.then_some(end + 1)
}

/// What `id` waits on now, if anything (FR-1311 priority order).
pub(crate) fn wait_of(s: &StateFile, id: &str) -> Option<Wait> {
    let rec = s.agents.get(id)?;
    let item = |kind: &str, pending: bool| {
        s.inbox
            .iter()
            .filter(|i| i.agent_id.as_deref() == Some(id) && i.kind == kind)
            .find(|i| if pending { i.pending() } else { i.status == orch_store::InboxStatus::Parked })
    };
    let from = |cause, i: &orch_store::InboxItem| Some(Wait { cause, item: Some(i.id), since_ms: i.created_at_ms });
    if let Some(i) = item("permission", true) {
        return from("permission", i);
    }
    if let Some(i) = item("question", true) {
        return from("question", i);
    }
    if let Some(i) = item("gate", true) {
        return from("gate", i);
    }
    if let Some(i) =
        s.inbox.iter().find(|i| i.agent_id.as_deref() == Some(id) && i.status == orch_store::InboxStatus::Parked)
    {
        return from("parked", i);
    }
    if let Some(i) = s.inbox.iter().find(|i| i.agent_id.as_deref() == Some(id) && i.kind == "startup" && i.open()) {
        return from("startup", i);
    }
    let last = rec.last_event_at_ms.unwrap_or(rec.created_at_ms);
    if rec.trust_dialog {
        return Some(Wait { cause: "trust", item: None, since_ms: last });
    }
    let turn =
        |st: TurnStatus| s.turns.iter().filter(|t| t.agent_id == id && t.status == st).min_by_key(|t| t.created_at_ms);
    if let Some(t) = turn(TurnStatus::Delivering) {
        return Some(Wait { cause: "delivering", item: None, since_ms: t.delivered_at_ms.unwrap_or(t.created_at_ms) });
    }
    if let Some(t) = turn(TurnStatus::Queued) {
        return Some(Wait { cause: "queued", item: None, since_ms: t.created_at_ms });
    }
    let open_task = rec.team.as_ref().is_some_and(|seat| seat.task.is_some());
    (rec.tracker.state == AgentState::Idle && open_task).then_some(Wait {
        cause: "idle_with_task",
        item: None,
        since_ms: last,
    })
}

impl Runtime {
    /// The user's keystrokes for `id`'s pane (the app's terminal or `orchd
    /// attach`): written to the PTY; typing (not terminal replies) counts as
    /// the user's input for delivery and may record a `pane_intervention`.
    pub fn pane_input(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let l = self.live(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        self.pane_write(id, &l, bytes)
    }

    /// [`Runtime::pane_input`] to a given process (an attached client keeps its own).
    pub(crate) fn pane_write(&self, id: &str, l: &Live, bytes: &[u8]) -> Result<()> {
        if !typed(bytes).is_empty() {
            l.last_user_input_ms.store(now_ms(), Ordering::SeqCst);
            self.note_intervention(id);
        }
        l.pty.write(bytes)
    }

    /// One `pane_intervention` per wait episode: the same wait (cause, item,
    /// start) typed into again records nothing more.
    fn note_intervention(&self, id: &str) {
        let Some((wait, run)) = self.store.read(|s| {
            wait_of(s, id).map(|w| (w, s.agents.get(id).and_then(|a| a.team.as_ref().map(|t| t.run_id.clone()))))
        }) else {
            return;
        };
        {
            let mut h = self.house();
            if h.interventions.get(id) == Some(&wait) {
                return;
            }
            h.interventions.insert(id.to_string(), wait.clone());
        }
        let detail = json!({
            "waited_on": wait.cause, "item": wait.item, "since_ms": wait.since_ms,
            "waited_ms": now_ms().saturating_sub(wait.since_ms), "team_run_id": run,
        });
        match &run {
            Some(r) => self.db.add_run_event(now_ms(), r, Some(id), "pane_intervention", detail),
            None => self.event(id, "pane_intervention", detail),
        }
    }

    /// A turn started: the agent's wait episode is over.
    pub(crate) fn end_wait_episode(&self, id: &str) {
        self.house().interventions.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use crate::store::{Admission, TurnRow};

    #[test]
    fn terminal_replies_are_not_typing() {
        for reply in [
            &b"\x1b[I"[..],
            b"\x1b[O",
            b"\x1b[12;40R",
            b"\x1b[?62;22c",
            b"\x1b[>1;10;0c",
            b"\x1b[<0;10;5M",
            b"\x1b[<0;10;5m",
            b"\x1b[M !!",
        ] {
            assert!(typed(reply).is_empty(), "{reply:?}");
        }
        assert_eq!(typed(b"\x1b[Iy\x1b[O"), b"y");
        // Keys are typing: Enter, Esc, arrows, letters.
        for key in [&b"\r"[..], b"\x1b", b"\x1b[A", b"1", b"\x1b[3~"] {
            assert!(!typed(key).is_empty(), "{key:?}");
        }
    }

    #[test]
    fn the_wait_cause_follows_the_priority_order() {
        let (_d, rt) = test_runtime(true);
        assert_eq!(rt.store.read(|s| wait_of(s, "a1")), None, "nothing to wait on");
        rt.store
            .write(|s| s.turns.push(TurnRow::queued("a1", "x", governor::TurnKind::Message, Admission::Pending)))
            .unwrap();
        assert_eq!(rt.store.read(|s| wait_of(s, "a1")).unwrap().cause, "queued");
        let startup = rt.add_item(Some("a1"), "startup", "a1 did not start".into(), json!({}), 10).unwrap();
        assert_eq!(rt.store.read(|s| wait_of(s, "a1")).unwrap().cause, "startup");
        let q = rt.add_item(Some("a1"), "question", "q".into(), json!({}), 30).unwrap();
        let p = rt.add_item(Some("a1"), "permission", "p".into(), json!({}), 30).unwrap();
        let w = rt.store.read(|s| wait_of(s, "a1")).unwrap();
        assert_eq!((w.cause, w.item), ("permission", Some(p)));
        rt.store.write(|s| assert!(s.park_inbox(p))).unwrap();
        assert_eq!(rt.store.read(|s| wait_of(s, "a1")).unwrap().cause, "question", "question before parked");
        rt.decide(q, orch_protocol::InboxDecision::Dismiss, None, None).unwrap();
        assert_eq!(rt.store.read(|s| wait_of(s, "a1")).unwrap().cause, "parked");
        let _ = startup;
    }

    #[test]
    fn one_event_per_wait_episode() {
        let (_d, rt) = test_runtime(true);
        let count = || {
            rt.db.write(|_| Ok(())).unwrap();
            rt.db.events(0, Some("a1"), Some("pane_intervention"), 10).unwrap()
        };
        // Not live in the unit runtime: drive the bookkeeping directly.
        rt.note_intervention("a1");
        assert!(count().is_empty(), "nothing waited on: typing records nothing");
        let p = rt.add_item(Some("a1"), "permission", "p".into(), json!({}), 30).unwrap();
        rt.note_intervention("a1");
        rt.note_intervention("a1");
        let ev = count();
        assert_eq!(ev.len(), 1, "one per episode");
        assert_eq!((ev[0].detail["waited_on"].as_str(), ev[0].detail["item"].as_u64()), (Some("permission"), Some(p)));
        // A turn starts: a later wait on the same thing is a new episode.
        rt.end_wait_episode("a1");
        rt.note_intervention("a1");
        assert_eq!(count().len(), 2);
        assert!(rt.pane_input("a1", b"x").is_err(), "not running");
    }
}
