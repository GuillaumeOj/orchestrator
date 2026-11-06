//! The Overwatch's writes to the chat (spec §3.8–3.9): its text entries
//! (`post`), the user's messages to it (`chat_send`, `chat_discard`, with
//! their delivery state) and the `action` entries of unsticking.

use anyhow::{Result, bail};
use orch_store::ChatEntry;
use serde_json::{Value, json};

use super::tools::{ToolErr, err};
use super::{Event, ID, Kind, Status};
use crate::runtime::Runtime;

/// A user message over this is refused (`bad_request`).
pub const SEND_MAX: usize = 2_000;

impl Runtime {
    /// Writes an entry and pushes it with the next `now` event.
    pub(crate) fn chat_put(&self, e: ChatEntry) -> Result<ChatEntry> {
        let w = self.db.chat_insert(&e)?;
        self.chat_push(&w);
        Ok(w)
    }

    /// An entry written or changed outside a computation: the next `now` carries it.
    pub(crate) fn chat_push(&self, e: &ChatEntry) {
        let mut posted = self.now.chat_posted();
        match posted.iter_mut().find(|p| p.n == e.n) {
            Some(p) => *p = e.clone(),
            None => posted.push(e.clone()),
        }
        drop(posted);
        self.touch("chat");
    }

    /// `post`: a text entry by the Overwatch (markdown), answering the user
    /// message of the turn it is in when there is one (`reply_to`).
    pub(crate) fn post_overwatch_text(&self, text: &str, thread: Option<&str>) -> Result<ChatEntry, ToolErr> {
        let (now, day) = self.overwatch_today();
        let reply_to = self.overwatch_reply_target();
        let e = ChatEntry {
            day,
            at_ms: now,
            author: ID.into(),
            kind: "text".into(),
            thread: thread.map(str::to_string),
            text: Some(text.to_string()),
            meta: reply_to.map(|r| json!({"reply_to": r})),
            ..Default::default()
        };
        let e = self.chat_put(e).map_err(|e| err("bad_request", format!("{e:#}")))?;
        self.ow_audit(None, "post", Some(&e.id()), json!({"chars": text.chars().count()}));
        Ok(e)
    }

    /// An unsticking or reprioritizing action as a chat entry of kind `action`
    /// (spec §3.8); `undo`: what "Restore tier" needs.
    pub(crate) fn post_action(&self, verb: &str, agent: &str, what: &str, reason: Option<String>, undo: Option<Value>) {
        let (now, day) = self.overwatch_today();
        let mut line = json!({"verb": verb, "agent": agent, "what": what, "by": "overwatch", "state": "effect",
                              "at_ms": now});
        if let Some(r) = reason {
            line["by_detail"] = json!(r);
        }
        let mut meta = None;
        if let Some(ctx) = undo {
            line["undo"] =
                json!({"id": format!("u-{verb}-{now}"), "label": "Restore tier", "until": "it is still queued"});
            meta = Some(json!({"_undo": ctx}));
        }
        let thread = self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.as_ref().map(|t| t.run_id.clone())));
        let e = ChatEntry {
            day,
            at_ms: now,
            author: ID.into(),
            kind: "action".into(),
            thread: thread.or_else(|| Some(agent.to_string())),
            line: Some(line),
            meta,
            ..Default::default()
        };
        if let Err(e) = self.chat_put(e) {
            eprintln!("warn: action entry for {agent} not written: {e:#}");
        }
    }

    // ------------------------------------------------------------ the user's messages

    /// `chat_send`: the user's text entry, and a `user_message` event. It is
    /// `pending` while the Overwatch is off, down or paused (delivered on restart or resume),
    /// else `waking` until a turn takes it.
    pub fn chat_send(&self, text: &str, thread: Option<&str>) -> Result<ChatEntry> {
        let text = text.trim();
        if text.is_empty() {
            bail!("bad_request: an empty message");
        }
        if text.chars().count() > SEND_MAX {
            bail!("bad_request: a message is at most {SEND_MAX} characters");
        }
        let (now, day) = self.overwatch_today();
        let pending = matches!(self.overwatch_status().status, Status::Off | Status::Down | Status::Paused);
        let e = ChatEntry {
            day,
            at_ms: now,
            author: "user".into(),
            kind: "text".into(),
            thread: thread.map(str::to_string),
            text: Some(text.to_string()),
            meta: Some(json!({"delivery": if pending { "pending" } else { "waking" }})),
            ..Default::default()
        };
        let e = self.chat_put(e)?;
        let _ = self.db.audit("user", None, "chat_send", Some(&e.id()), "ok", json!({"chars": text.chars().count()}));
        self.overwatch_user_message(&e.id(), text);
        Ok(e)
    }

    /// `chat_discard`: removes a user entry still `pending`.
    pub fn chat_discard(&self, entry: &str) -> Result<Value> {
        let n: u64 = entry.trim_start_matches("c-").parse().map_err(|_| anyhow::anyhow!("bad_request: {entry}"))?;
        let Some(e) = self.db.chat_get(n)? else { bail!("bad_request: no entry {entry}") };
        let delivery = e.meta.as_ref().and_then(|m| m["delivery"].as_str());
        if e.author != "user" || e.kind != "text" || delivery != Some("pending") {
            bail!("bad_request: only a message that was not delivered can be discarded");
        }
        self.db.chat_delete(n)?;
        self.overwatch_forget_pending(&e.id());
        let _ = self.db.audit("user", None, "chat_discard", Some(&e.id()), "ok", json!({}));
        self.touch("chat");
        Ok(json!({"ok": true, "removed": e.id()}))
    }

    /// The turn took these events: their user entries are `delivered`.
    pub(crate) fn overwatch_mark_delivered(&self, events: &[Event]) {
        for ev in events.iter().filter(|e| e.kind == Kind::UserMessage) {
            let Some(n) = ev.chat.as_deref().and_then(|c| c.trim_start_matches("c-").parse::<u64>().ok()) else {
                continue;
            };
            let Ok(Some(mut e)) = self.db.chat_get(n) else { continue };
            if e.meta.as_ref().and_then(|m| m["delivery"].as_str()) == Some("delivered") {
                continue;
            }
            let mut meta = e.meta.take().unwrap_or_else(|| json!({}));
            meta["delivery"] = json!("delivered");
            e.meta = Some(meta);
            if let Ok(w) = self.db.chat_update(&e) {
                self.chat_push(&w);
            }
        }
    }

    /// A user entry's delivery changed to `pending` (the Overwatch went down
    /// or off with it queued, or an earlier day's).
    pub(crate) fn overwatch_mark_pending(&self, events: &[Event]) {
        for ev in events.iter().filter(|e| e.kind == Kind::UserMessage) {
            let Some(n) = ev.chat.as_deref().and_then(|c| c.trim_start_matches("c-").parse::<u64>().ok()) else {
                continue;
            };
            let Ok(Some(mut e)) = self.db.chat_get(n) else { continue };
            if e.meta.as_ref().and_then(|m| m["delivery"].as_str()) == Some("pending") {
                continue;
            }
            let mut meta = e.meta.take().unwrap_or_else(|| json!({}));
            meta["delivery"] = json!("pending");
            e.meta = Some(meta);
            if let Ok(w) = self.db.chat_update(&e) {
                self.chat_push(&w);
            }
        }
    }
}
