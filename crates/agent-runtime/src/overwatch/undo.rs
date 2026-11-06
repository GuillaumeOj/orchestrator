//! Undo of the oversight lines (spec §3.8, api-contract §3.2–3.3). A line
//! offers `undo {id, label, until}` while it still means something; the
//! core keeps what it needs in the entry's private `meta._undo`. A used undo
//! reads `undone_at_ms` (answers: `corrected_at_ms`), one that can no longer
//! work `undo_gone` ("back-1 already moved on").

use anyhow::{Result, anyhow, bail};
use governor::{EntryId, Tier};
use orch_store::ChatEntry;
use serde_json::{Value, json};

use crate::decision::Decision;
use crate::runtime::Runtime;

/// One hour: a reversed approval or an override lives that long.
const ONE_SHOT_MS: u64 = 3_600_000;

/// What a line offers.
pub(crate) struct Plan {
    pub label: &'static str,
    pub until: &'static str,
    pub ctx: Value,
}

/// The undo a settled decision's line offers, if any (spec §3.8 table).
pub(crate) fn plan_for(d: &Decision, line: &Value) -> Option<Plan> {
    let by = line["by"].as_str().unwrap_or("");
    let verb = line["verb"].as_str().unwrap_or("");
    let p = &d.payload;
    let agent = d.agent.clone();
    let call = || {
        json!({"agent": agent, "tool": p["tool"], "input": p.get("tool_input").or_else(|| p.get("input")),
               "item": d.id, "task": d.subject.get("task")})
    };
    match (d.kind, by, verb) {
        ("permission", "policy", "allowed") => Some(Plan {
            label: "Revoke rule",
            until: "the rule is in the file",
            ctx: json!({"kind": "rule", "id": line["by_detail"], "file": p.get("policy_file")}),
        }),
        ("permission", "overwatch", "allowed") => {
            let mut ctx = call();
            ctx["kind"] = json!("told");
            Some(Plan { label: "Tell the agent", until: "the agent is still on its task", ctx })
        }
        ("permission", "overwatch", "denied") => {
            let mut ctx = call();
            ctx["kind"] = json!("allow_instead");
            Some(Plan { label: "Allow instead", until: "the agent is still on its task", ctx })
        }
        ("question", "overwatch", "answered") => Some(Plan {
            label: "Correct the answer",
            until: "the agent is still on its task",
            ctx: json!({"kind": "correct", "agent": agent, "item": d.id, "task": d.subject.get("task")}),
        }),
        ("rule_proposal", _, "added") => Some(Plan {
            label: "Remove rule",
            until: "the rule is in the file",
            ctx: json!({"kind": "rule", "id": p.get("added_rule_id"), "file": p.get("added_file")}),
        }),
        _ => None,
    }
}

impl Runtime {
    /// Sets `e.line` from a decision's line: what an earlier line of the
    /// entry says about its undo (used, gone) is kept, else the undo it
    /// offers now is written (with its private context in `meta`).
    pub(crate) fn set_line(&self, e: &mut ChatEntry, d: &Decision, prior: Option<&Value>, now: u64) {
        let mut line = crate::chat::line_of(d, prior, now);
        let old = e.line.clone().unwrap_or(Value::Null);
        let mut used = false;
        for k in ["undone_at_ms", "corrected_at_ms", "undo_gone"] {
            if let Some(v) = old.get(k) {
                line[k] = v.clone();
                used = true;
            }
        }
        if !used && let Some(plan) = plan_for(d, &line) {
            line["undo"] = json!({"id": format!("u-{}", d.id), "label": plan.label, "until": plan.until});
            let mut meta = e.meta.take().unwrap_or_else(|| json!({}));
            meta["_undo"] = plan.ctx;
            e.meta = Some(meta);
        }
        e.line = Some(line);
    }

    /// Why `ctx`'s undo can no longer work (the line's `undo_gone`), or `None`.
    fn undo_gone_reason(&self, ctx: &Value) -> Option<String> {
        let agent_gone = |ctx: &Value| -> Option<String> {
            let id = ctx["agent"].as_str()?;
            let moved = format!("{id} already moved on");
            let rec = self.store.read(|s| s.agents.get(id).cloned());
            let Some(rec) = rec else { return Some(moved) };
            let task = rec.team.as_ref().and_then(|t| t.task);
            (!rec.tracker.state.is_live() || task != ctx["task"].as_u64()).then_some(moved)
        };
        match ctx["kind"].as_str()? {
            "rule" => {
                let id = ctx["id"].as_str()?;
                let file = ctx["file"].as_str();
                let scope = file.and_then(crate::policy::Scope::parse);
                self.policy.find(id, scope.as_ref()).is_none().then(|| "the rule is not in the file any more".into())
            }
            "tier" => {
                let entry = ctx["entry"].as_u64()?;
                (!self.with_governor(|g| g.queue().iter().any(|e| e.id.0 == entry)))
                    .then(|| "it is not queued any more".into())
            }
            "told" | "allow_instead" | "correct" => agent_gone(ctx),
            _ => None,
        }
    }

    /// Entries whose undo went stale get `undo_gone` instead of `undo`
    /// (written back): called when a day is read.
    pub(crate) fn refresh_undo(&self, entries: &mut [ChatEntry]) {
        for e in entries.iter_mut() {
            let Some(ctx) = e.meta.as_ref().and_then(|m| m.get("_undo")).cloned() else { continue };
            let Some(line) = e.line.as_mut().filter(|l| l.get("undo").is_some()) else { continue };
            let Some(why) = self.undo_gone_reason(&ctx) else { continue };
            if let Some(m) = line.as_object_mut() {
                m.remove("undo");
            }
            line["undo_gone"] = json!(why);
            match self.db.chat_update(e) {
                Ok(w) => *e = w,
                Err(err) => eprintln!("warn: chat entry {} undo state not written: {err}", e.id()),
            }
        }
    }

    /// Tells `agent` what the user undid.
    fn undo_deliver(&self, agent: &str, text: &str) -> Result<String> {
        #[cfg(test)]
        if self.overwatch.fail_undo_delivery.swap(false, std::sync::atomic::Ordering::SeqCst) {
            bail!("delivery failed (test)");
        }
        self.deliver_user(agent, text)
    }

    /// `chat_undo {entry, text?}`: undoes what the line offers. The updated
    /// entry; `{ok: false, gone}` with the reason when it can no longer work;
    /// `text` is required to correct an answer.
    pub fn chat_undo(&self, entry: &str, text: Option<&str>) -> Result<Value> {
        let _one = self.overwatch.undoing.lock().unwrap_or_else(|e| e.into_inner());
        let n: u64 = entry.trim_start_matches("c-").parse().map_err(|_| anyhow!("bad_request: {entry}"))?;
        let Some(mut e) = self.db.chat_get(n)? else { bail!("bad_request: no entry {entry}") };
        let ctx = e.meta.as_ref().and_then(|m| m.get("_undo")).cloned();
        let offered = e.line.as_ref().is_some_and(|l| l.get("undo").is_some());
        let (Some(ctx), true) = (ctx, offered) else {
            if let Some(why) = e.line.as_ref().and_then(|l| l["undo_gone"].as_str()) {
                return Ok(json!({"ok": false, "gone": why, "entry": e}));
            }
            bail!("bad_request: {entry} has nothing to undo");
        };
        let kind = ctx["kind"].as_str().unwrap_or("");
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        if kind == "correct" && text.is_none() {
            bail!("bad_request: correcting an answer needs the corrected text");
        }
        if let Some(why) = self.undo_gone_reason(&ctx) {
            if let Some(line) = e.line.as_mut() {
                if let Some(m) = line.as_object_mut() {
                    m.remove("undo");
                }
                line["undo_gone"] = json!(why);
            }
            let w = self.db.chat_update(&e)?;
            self.chat_push(&w);
            return Ok(json!({"ok": false, "gone": why, "entry": w}));
        }
        let agent = ctx["agent"].as_str().unwrap_or("");
        let call = || {
            let tool = ctx["tool"].as_str().unwrap_or("");
            let input = ctx["input"].clone();
            let brief = crate::inbox::main_arg(tool, &input).map(|a| format!("{tool}: {a}")).unwrap_or(tool.into());
            (tool.to_string(), input, brief)
        };
        let now = orch_protocol::now_ms();
        match kind {
            "rule" => {
                let id = ctx["id"].as_str().unwrap_or("");
                if !self.revoke_rule(id, ctx["file"].as_str())? {
                    bail!("bad_request: the rule is not in the file any more");
                }
            }
            "told" => {
                let (tool, input, brief) = call();
                // Rolled back when the agent is not told: a retry adds it again.
                let until = now + ONE_SHOT_MS;
                self.overwatch.deny_once(agent, &tool, input.clone(), until);
                if let Err(e) = self.undo_deliver(
                    agent,
                    &format!("ORCH: the user reversed the Overwatch's approval of `{brief}`: don't repeat it; if you still need it, say why."),
                ) {
                    self.overwatch.forget_deny(agent, &tool, &input, until);
                    return Err(e);
                }
            }
            "allow_instead" => {
                let (tool, input, brief) = call();
                let item = ctx["item"].as_str().and_then(|d| self.item_of_decision(d));
                // Rolled back when the agent is not told: a retry adds it again.
                let id = self.db.add_one_shot(agent, &tool, &input, item, now + ONE_SHOT_MS)?;
                if let Err(e) = self
                    .undo_deliver(agent, &format!("ORCH: the user overrode the denial of `{brief}`: you may retry it."))
                {
                    let _ = self.db.remove_one_shot(id);
                    return Err(e);
                }
            }
            "correct" => {
                let t = text.unwrap_or_default();
                self.deliver_user(
                    agent,
                    &format!("ORCH: the user corrected the Overwatch's answer to your question: {t}"),
                )?;
            }
            "tier" => {
                let from = match ctx["from"].as_str().unwrap_or("normal").to_lowercase().as_str() {
                    "low" => Tier::Low,
                    "high" => Tier::High,
                    _ => Tier::Normal,
                };
                let entry = EntryId(ctx["entry"].as_u64().unwrap_or(0));
                self.governor_act(|g, now| (g.set_tier(entry, from, now), Vec::new()))
                    .map_err(|e| anyhow!("bad_request: {e}"))?;
            }
            other => bail!("bad_request: unknown undo {other:?}"),
        }
        // The line says so, and offers nothing more.
        if let Some(line) = e.line.as_mut() {
            if let Some(m) = line.as_object_mut() {
                m.remove("undo");
            }
            line[if kind == "correct" { "corrected_at_ms" } else { "undone_at_ms" }] = json!(now);
        }
        let w = self.db.chat_update(&e)?;
        self.chat_push(&w);
        let _ = self.db.audit(
            "user",
            ctx["agent"].as_str(),
            "chat_undo",
            Some(&w.id()),
            "ok",
            json!({"entry": w.id(), "undo": kind, "line": w.line.as_ref().map(|l| l["verb"].clone())}),
        );
        Ok(json!({"ok": true, "entry": w}))
    }
}
