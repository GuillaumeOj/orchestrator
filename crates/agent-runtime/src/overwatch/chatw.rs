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

/// Mentions a message may carry, and the agents listed per mention.
const MENTIONS_MAX: usize = 10;
const MENTION_AGENTS_MAX: usize = 12;

/// `context.mentions`: the distinct feature ids, in order. A bad shape is `bad_request`.
fn mention_ids(context: Option<&Value>) -> Result<Vec<String>> {
    let Some(m) = context.and_then(|c| c.get("mentions")).filter(|m| !m.is_null()) else { return Ok(vec![]) };
    let items = m.as_array().ok_or_else(|| anyhow::anyhow!("bad_request: context.mentions is an array"))?;
    if items.len() > MENTIONS_MAX {
        bail!("bad_request: a message mentions at most {MENTIONS_MAX} features");
    }
    let mut ids: Vec<String> = Vec::new();
    for it in items {
        let (Some(kind), Some(id)) = (it["kind"].as_str(), it["id"].as_str()) else {
            bail!("bad_request: a mention is {{kind, id}}");
        };
        if kind != "feature" {
            bail!("bad_request: unknown mention kind {kind:?}");
        }
        if !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

/// Pasted texts a message may carry, their size, and the number range of a placeholder.
const PASTES_MAX: usize = 10;
const PASTE_CHARS_MAX: usize = 200_000;

/// `context.pastes`: `[{n, text}]`, validated (api-contract §1.1). A bad shape is `bad_request`.
fn paste_inputs(context: Option<&Value>) -> Result<Vec<(u64, String)>> {
    let Some(p) = context.and_then(|c| c.get("pastes")).filter(|p| !p.is_null()) else { return Ok(vec![]) };
    let items = p.as_array().ok_or_else(|| anyhow::anyhow!("bad_request: context.pastes is an array"))?;
    if items.len() > PASTES_MAX {
        bail!("bad_request: at most {PASTES_MAX} pastes");
    }
    let mut out: Vec<(u64, String)> = Vec::new();
    for it in items {
        let (Some(n), Some(text)) = (it.get("n"), it.get("text").and_then(Value::as_str)) else {
            bail!("bad_request: a paste is {{n, text}}");
        };
        let n = n
            .as_u64()
            .filter(|n| (1..=99).contains(n))
            .ok_or_else(|| anyhow::anyhow!("bad_request: paste n is 1..=99"))?;
        if out.iter().any(|(m, _)| *m == n) {
            bail!("bad_request: paste n {n} is twice");
        }
        if text.is_empty() {
            bail!("bad_request: paste #{n} is empty");
        }
        if text.chars().count() > PASTE_CHARS_MAX {
            bail!("bad_request: paste #{n} is at most {PASTE_CHARS_MAX} characters");
        }
        out.push((n, text.to_string()));
    }
    Ok(out)
}

/// The pasted texts of a stored entry's meta, as the event carries them.
pub(crate) fn pastes_of_meta(meta: Option<&Value>) -> Option<Vec<Value>> {
    let v = meta?.get("pastes")?.as_array()?;
    (!v.is_empty()).then(|| v.clone())
}

/// Removes the files of a `meta.pastes` list (best effort).
fn remove_paste_files(pastes: &[Value]) {
    for f in pastes.iter().filter_map(|p| p["path"].as_str()) {
        let _ = std::fs::remove_file(f);
    }
}

impl Runtime {
    /// `<root>/overwatch/pastes`: where the pasted texts live (all days).
    pub(crate) fn overwatch_pastes_dir(&self) -> std::path::PathBuf {
        self.cfg.paths.root.join("overwatch").join("pastes")
    }

    /// Writes the kept pastes under `pastes/<day>/<key>-<n>.md`; on a failure removes what it wrote.
    /// Returns the `meta.pastes` list.
    fn write_pastes(&self, day: &str, text: &str, pastes: Vec<(u64, String)>) -> Result<Vec<Value>> {
        let kept: Vec<_> = pastes
            .into_iter()
            .filter(|(n, _)| {
                let ok = text.contains(&format!("[Pasted text #{n} "));
                if !ok {
                    eprintln!("warn: chat_send: paste #{n} has no placeholder in the text, dropped");
                }
                ok
            })
            .collect();
        if kept.is_empty() {
            return Ok(vec![]);
        }
        let dir = self.overwatch_pastes_dir().join(day);
        std::fs::create_dir_all(&dir)?;
        let key: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
        let mut meta: Vec<Value> = vec![];
        for (n, body) in &kept {
            let path = dir.join(format!("{key}-{n}.md"));
            let item = json!({"n": n, "chars": body.chars().count(), "lines": body.lines().count(),
                              "path": path.to_string_lossy()});
            meta.push(item);
            if let Err(e) = std::fs::write(&path, body) {
                remove_paste_files(&meta);
                return Err(anyhow::Error::new(e).context("the pasted text was not written"));
            }
        }
        Ok(meta)
    }

    /// `chat_paste`: the text of paste `n` of a user entry (api-contract §1.2).
    pub fn chat_paste(&self, entry: &str, n: u64) -> Result<Value> {
        let nf = || anyhow::anyhow!("not_found: no pasted text #{n} in {entry}");
        let num: u64 =
            entry.trim_start_matches("c-").parse().map_err(|_| anyhow::anyhow!("not_found: no entry {entry}"))?;
        let e = self.db.chat_get(num)?.ok_or_else(|| anyhow::anyhow!("not_found: no entry {entry}"))?;
        let item = pastes_of_meta(e.meta.as_ref()).and_then(|v| v.into_iter().find(|p| p["n"] == n)).ok_or_else(nf)?;
        let path = std::path::PathBuf::from(item["path"].as_str().ok_or_else(nf)?);
        // Only under the pastes dir, symlinks and `..` resolved.
        let root = self.overwatch_pastes_dir().canonicalize().map_err(|_| nf())?;
        let path = path.canonicalize().map_err(|_| nf())?;
        if !path.starts_with(&root) {
            return Err(nf());
        }
        let text = std::fs::read_to_string(&path).map_err(|_| nf())?;
        Ok(json!({"n": n, "text": text, "chars": text.chars().count(), "lines": text.lines().count()}))
    }

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
        self.chat_send_with(text, thread, None)
    }

    /// `chat_send` with its `context`: `{plan: <project id>}` marks a
    /// planning message (P4.5 api-contract §3.3); the event carries the plan.
    pub fn chat_send_with(&self, text: &str, thread: Option<&str>, context: Option<&Value>) -> Result<ChatEntry> {
        let plan = match context.and_then(|c| c.get("plan")).filter(|p| !p.is_null()) {
            Some(p) => {
                let p = p.as_str().ok_or_else(|| anyhow::anyhow!("bad_request: context.plan is a project id"))?;
                Some(self.plan_context(p)?)
            }
            None => None,
        };
        let mention_ids = mention_ids(context)?;
        let paste_inputs = paste_inputs(context)?;
        let text = text.trim();
        if text.is_empty() {
            bail!("bad_request: an empty message");
        }
        if text.chars().count() > SEND_MAX {
            bail!("bad_request: a message is at most {SEND_MAX} characters");
        }
        let (now, day) = self.overwatch_today();
        let mentions = self.mention_snapshots(&mention_ids);
        let pastes = self.write_pastes(&day, text, paste_inputs)?;
        let pending = matches!(self.overwatch_status().status, Status::Off | Status::Down | Status::Paused);
        let e = ChatEntry {
            day,
            at_ms: now,
            author: "user".into(),
            kind: "text".into(),
            thread: thread.map(str::to_string),
            text: Some(text.to_string()),
            meta: Some({
                let mut m = json!({"delivery": if pending { "pending" } else { "waking" }});
                if let Some(p) = &plan {
                    m["plan"] = p.clone();
                }
                if !mentions.is_empty() {
                    m["mentions"] = json!(mentions);
                }
                if !pastes.is_empty() {
                    m["pastes"] = json!(pastes);
                }
                m
            }),
            ..Default::default()
        };
        let e = match self.chat_put(e) {
            Ok(e) => e,
            Err(err) => {
                remove_paste_files(&pastes);
                return Err(err);
            }
        };
        let paste_chars: u64 = pastes.iter().filter_map(|p| p["chars"].as_u64()).sum();
        let _ = self.db.audit(
            "user",
            None,
            "chat_send",
            Some(&e.id()),
            "ok",
            json!({"chars": text.chars().count(), "mentions": mentions.len(),
                   "pastes": pastes.len(), "paste_chars": paste_chars}),
        );
        let ids: Vec<String> = mentions.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
        let payload = self.mention_payloads(&ids);
        self.overwatch_user_message_with(
            &e.id(),
            text,
            plan,
            (!payload.is_empty()).then_some(payload),
            (!pastes.is_empty()).then_some(pastes),
        );
        Ok(e)
    }

    /// The meta snapshot of each mentioned feature that is in the store
    /// (unknown ids are dropped and logged; archived features are kept).
    fn mention_snapshots(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .filter_map(|id| {
                let Some(f) = self.db.feature(id).ok().flatten() else {
                    eprintln!("warn: chat_send: mention of unknown feature {id:?} dropped");
                    return None;
                };
                let project = self.db.project(&f.project_id).ok().flatten().map(|p| p.name);
                Some(json!({"kind": "feature", "id": f.id, "slug": f.slug, "title": f.title,
                            "project_id": f.project_id, "project": project}))
            })
            .collect()
    }

    /// The rich payload of the `user_message` event for each feature id
    /// (api-contract §2): run and status, branch, docs, worktree, agents, PR.
    pub(crate) fn mention_payloads(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .filter_map(|id| {
                let f = self.db.feature(id).ok().flatten()?;
                let project = self.db.project(&f.project_id).ok().flatten();
                let run = self.db.runs(Some(&f.id), false).ok().and_then(|r| r.into_iter().next());
                let docs = crate::feature_docs::docs_dir_of(&f).filter(|d| d.is_dir());
                let agents: Vec<String> = run
                    .as_ref()
                    .map(|r| self.run_agents(&r.id).into_iter().map(|a| a.id).take(MENTION_AGENTS_MAX).collect())
                    .unwrap_or_default();
                Some(json!({
                    "kind": "feature", "id": f.id, "slug": f.slug, "title": f.title,
                    "project": project.as_ref().map(|p| p.name.clone()),
                    "project_path": project.as_ref().map(|p| p.path.clone()),
                    "run": run.as_ref().map(|r| r.id.clone()),
                    "status": run.as_ref().map(|r| r.status.as_str()),
                    "branch": f.integration_branch(),
                    "docs_dir": docs,
                    "worktree": f.integration_worktree,
                    "agents": agents,
                    "pr_url": run.as_ref().and_then(|r| r.pr_url.clone()),
                }))
            })
            .collect()
    }

    /// The event's mentions of a stored entry's meta (a pending message
    /// rebuilt on restart): `meta.mentions[].id`, resolved again.
    pub(crate) fn mentions_of_meta(&self, meta: Option<&Value>) -> Option<Vec<Value>> {
        let ids: Vec<String> =
            meta?.get("mentions")?.as_array()?.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
        let v = self.mention_payloads(&ids);
        (!v.is_empty()).then_some(v)
    }

    /// The plan payload of a planning message: the project, its docs folder,
    /// its intake source and its last 5 features' titles.
    fn plan_context(&self, project: &str) -> Result<Value> {
        let p = self.db.project(project)?.ok_or_else(|| anyhow::anyhow!("bad_request: no project {project:?}"))?;
        let docs = p.path.join("docs");
        let intake = self.intake_label(&p);
        let mut features = self.db.features(Some(&p.id)).unwrap_or_default();
        features.sort_by_key(|f| std::cmp::Reverse(f.created_at_ms));
        let recent: Vec<String> = features.into_iter().take(5).map(|f| f.title).collect();
        Ok(json!({
            "project": p.id, "name": p.name, "path": p.path,
            "docs_dir": docs.is_dir().then(|| docs.clone()),
            "intake": intake, "recent_briefs": recent,
        }))
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
        if let Some(p) = pastes_of_meta(e.meta.as_ref()) {
            remove_paste_files(&p);
        }
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
