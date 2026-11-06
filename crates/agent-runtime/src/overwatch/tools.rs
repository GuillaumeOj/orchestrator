//! The Overwatch's tools (api-contract §2.2, spec §3.4): reached only by
//! the agent `overwatch` (its role lists them; `on_mcp` routes here before
//! the policy, so each tool checks the hard lists itself). Errors are
//! `{code, message}`; every acting call is audited with actor `overwatch`.

use governor::{EntryId, Tier, TierError};
use orch_protocol::{InboxDecision, now_ms};
use serde_json::{Value, json};

use super::ID;
use super::deciding::NotDeciding;
use crate::mcp::McpAnswer;
use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::{cut, str_arg};

/// An error answer `{code, message}` (api-contract §2.2 "Errors").
pub fn tool_error(code: &str, message: impl Into<String>) -> McpAnswer {
    let message = message.into();
    let v = json!({"code": code, "message": message});
    (true, v.to_string(), v)
}

/// A refused call: its code (`not_allowed`, `too_late`, `stale`, `ungrounded`,
/// `duplicate`, `bad_request`…) and why.
#[derive(Debug)]
pub struct ToolErr {
    pub code: &'static str,
    pub message: String,
}

pub(crate) type Out = Result<Value, ToolErr>;

pub(crate) fn err(code: &'static str, message: impl Into<String>) -> ToolErr {
    ToolErr { code, message: message.into() }
}

fn bad(message: impl Into<String>) -> ToolErr {
    err("bad_request", message)
}

impl From<anyhow::Error> for ToolErr {
    fn from(e: anyhow::Error) -> ToolErr {
        err("bad_request", format!("{e:#}"))
    }
}

/// `inbox-42` → 42 (a decision id of `prefix`).
pub(crate) fn decision_n(id: &str, prefix: &str) -> Result<u64, ToolErr> {
    id.strip_prefix(prefix)
        .and_then(|n| n.strip_prefix('-'))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| bad(format!("decision {id:?}: expected {prefix}-<n>")))
}

fn require<'a>(args: &'a Value, k: &str) -> Result<&'a str, ToolErr> {
    str_arg(args, k).ok_or_else(|| bad(format!("`{k}` is required")))
}

impl Runtime {
    pub(crate) fn overwatch_tool(&self, _rec: &AgentRecord, tool: &str, args: &Value) -> McpAnswer {
        let r = match tool {
            "world" => self.ow_world(args),
            "inspect" => self.ow_inspect(args),
            "decide_permission" => self.ow_decide_permission(args),
            "answer_question" => self.ow_answer_question(args),
            "annotate" => self.ow_annotate(args),
            "nudge" => self.ow_nudge(args),
            "resume_agent" => self.ow_resume_agent(args),
            "restart_fresh" => self.ow_restart_fresh(args),
            "reprioritize" => self.ow_reprioritize(args),
            "propose_rule" => self.ow_propose_rule(args),
            "post" => self.ow_post(args),
            other => Err(err("not_implemented", format!("{other} is not an Overwatch tool"))),
        };
        match r {
            Ok(v) => (false, v.to_string(), v),
            Err(e) => tool_error(e.code, e.message),
        }
    }

    /// An acting call, audited with actor `overwatch` (FR-709).
    pub(crate) fn ow_audit(&self, agent: Option<&str>, action: &str, target: Option<&str>, detail: Value) {
        let _ = self.db.audit(ID, agent, action, target, "ok", detail);
    }

    /// The agent `args.agent` names; not the Overwatch itself.
    fn ow_agent(&self, args: &Value) -> Result<AgentRecord, ToolErr> {
        let id = require(args, "agent")?;
        if id == ID {
            return Err(err("not_allowed", "it is not for the Overwatch itself"));
        }
        self.store
            .read(|s| s.agents.get(id).cloned())
            .filter(|a| a.tracker.state != crate::state::AgentState::Archived)
            .ok_or_else(|| bad(format!("no agent {id:?} (use world)")))
    }

    // ------------------------------------------------------------ world, inspect

    fn ow_world(&self, args: &Value) -> Out {
        let project = str_arg(args, "project");
        let snap = self.now_snapshot();
        let keep = |v: &Value, key: &str| project.is_none_or(|p| v[key].as_str() == Some(p));
        let list = |k: &str| -> Vec<Value> { snap[k].as_array().cloned().unwrap_or_default() };
        let agents: Vec<Value> = list("agents").into_iter().filter(|a| keep(a, "project")).collect();
        let runs: Vec<Value> = list("runs").into_iter().filter(|r| keep(r, "project")).collect();
        let decisions: Vec<Value> = list("decisions")
            .into_iter()
            .filter(|d| matches!(d["lifecycle"]["state"].as_str(), Some("open" | "parked")))
            .filter(|d| project.is_none_or(|p| d["subject"]["project"].as_str() == Some(p)))
            .map(|mut d| {
                if let Some(m) = d.as_object_mut() {
                    m.remove("payload");
                }
                d
            })
            .collect();
        let now = self.now.clock().0;
        let (queue, governor) = self.with_governor(|g| {
            let by_id: std::collections::BTreeMap<_, _> = g.queue().iter().map(|e| (e.id, e)).collect();
            let queue: Vec<Value> = g.release_order().iter().filter_map(|id| by_id.get(id)).map(|e| json!(e)).collect();
            let snap = g.snapshot(governor::jiff::Timestamp::from_millisecond(now as i64).unwrap_or_default());
            let actions = g.actions();
            let last: Vec<Value> = actions[actions.len().saturating_sub(20)..]
                .iter()
                .map(|a| {
                    json!({"at_ms": a.ts.as_millisecond().max(0), "action": a.kind_name(),
                           "agent": a.action.agent().map(|a| a.0.clone()), "why": a.reason})
                })
                .collect();
            let gov = json!({"week_pct": snap.used, "plan_pct": snap.plan, "reserve_left_pct": snap.reserve_left,
                             "last_actions": last});
            (queue, gov)
        });
        Ok(json!({
            "agents": agents, "runs": runs, "counts": snap["counts"], "decisions": decisions,
            "queue": queue, "governor": governor,
            "overwatch": {"status": self.overwatch_status().status},
        }))
    }

    fn ow_inspect(&self, args: &Value) -> Out {
        let a = self.ow_agent(args)?;
        let attention = self.attention_of(&a.id).map(|r| json!(r));
        let seat = a.team.clone();
        let task = seat
            .as_ref()
            .and_then(|t| t.task)
            .and_then(|id| self.db.task(id).ok().flatten())
            .map(|t| json!({"id": t.id, "title": t.title, "status": t.status, "body": cut(&t.body, 1_500)}));
        let (run, messages) = match &seat {
            Some(t) => {
                let msgs: Vec<Value> = self
                    .db
                    .recent_messages(&t.run_id, 40)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.to_node == t.node || m.from_node == t.node)
                    .rev()
                    .take(10)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .map(|m| {
                        json!({"from": m.from_agent.unwrap_or(m.from_node), "to": m.to_agent.unwrap_or(m.to_node),
                               "kind": m.kind, "body": cut(&m.body, 400), "at_ms": m.created_at_ms})
                    })
                    .collect();
                (Some(t.run_id.clone()), msgs)
            }
            None => (None, vec![]),
        };
        let docs = seat
            .as_ref()
            .and_then(|t| self.db.feature(&t.feature_id).ok().flatten())
            .and_then(|f| {
                let w = f.integration_worktree?;
                let dir = w.join(".orchestrator/features").join(&f.slug);
                Some(
                    crate::team::feature_docs(&w, &f.slug)
                        .into_iter()
                        .map(|n| dir.join(n).display().to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
        let open: Vec<Value> = self
            .open_decisions()
            .into_iter()
            .filter(|d| d.agent.as_deref() == Some(a.id.as_str()))
            .map(|d| json!({"id": d.id, "kind": d.kind, "title": d.title, "waits_on": d.waits_on}))
            .collect();
        Ok(json!({
            "attention": attention, "task": task, "run": run, "messages": messages,
            "transcript_tail": self.assistant_tail(&a.session_id, 10),
            "decisions": open, "worktree": a.worktree.display().to_string(), "feature_docs": docs,
        }))
    }

    /// The last `n` assistant texts of a session's transcript, ≤ 400 chars each.
    fn assistant_tail(&self, session: &str, n: usize) -> Vec<String> {
        let Some(path) = crate::transcript::find(&self.cfg.projects_root, session) else { return vec![] };
        // The end of the file is enough for ten texts (a transcript can be megabytes).
        let Some(text) = read_tail(&path, 512 * 1024) else { return vec![] };
        let mut out: Vec<String> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e["type"] == "assistant")
            .filter_map(|e| {
                let blocks = e["message"]["content"].as_array()?;
                let t: Vec<&str> =
                    blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
                let t = t.join("\n");
                (!t.trim().is_empty()).then(|| cut(t.trim(), 400))
            })
            .collect();
        out.drain(..out.len().saturating_sub(n));
        out
    }

    // ------------------------------------------------------------ decisions

    fn ow_decide_permission(&self, args: &Value) -> Out {
        let n = decision_n(require(args, "decision")?, "inbox")?;
        let effect = require(args, "effect")?;
        let allow = match effect {
            "allow" => true,
            "deny" => false,
            _ => return Err(bad("`effect` is allow or deny")),
        };
        let reason = cut(require(args, "reason")?, 60);
        // Still the Overwatch's to settle: effect overwatch, within its window.
        match self.overwatch_deciding(n) {
            Ok(_) => {}
            Err(NotDeciding::Stale) => return Err(err("stale", "the decision closed meanwhile")),
            Err(NotDeciding::NotAllowed) => {
                return Err(err("not_allowed", "that permission is not one whose effect is `overwatch`"));
            }
            Err(NotDeciding::TooLate) => {
                return Err(err("too_late", "it is waiting for the user now (the deciding window passed)"));
            }
        }
        // The core's hard and user-only lists (B1), whatever the policy says.
        let item = self.store.read(|s| s.inbox.iter().find(|i| i.id == n).cloned());
        let Some(item) = item else { return Err(err("stale", "the decision closed meanwhile")) };
        let agent = item.agent_id.clone().and_then(|a| self.store.read(|s| s.agents.get(&a).cloned()));
        // Without the agent's record the lists can't be checked: the user's.
        let Some(agent) = &agent else {
            self.overwatch_hand_over(n);
            return Err(err("not_allowed", "the core keeps this call for the user: the agent is gone"));
        };
        let tool = item.payload.get("tool").and_then(Value::as_str).unwrap_or("");
        let cwd = item.payload.get("cwd").and_then(Value::as_str);
        if let Some(h) = self.hard_check(agent, tool, &crate::inbox::tool_input(&item), cwd) {
            // Deciding ends at once (F1): the user's.
            self.overwatch_hand_over(n);
            return Err(err("not_allowed", format!("the core keeps this call for the user: {}", h.why)));
        }
        self.set_payload(n, &[("overwatch_reason", json!(reason))]);
        let decision = if allow { InboxDecision::Approve } else { InboxDecision::Reject };
        // The reason is read while deciding; a decision that did not happen keeps none.
        if let Err(e) = self.decide_as(n, decision, None, None, ID) {
            self.unset_payload(n, &["overwatch_reason"]);
            return Err(err("stale", format!("{e:#}")));
        }
        let verb = if allow { "allowed" } else { "denied" };
        let what = item.payload.get("tool").and_then(Value::as_str).unwrap_or("");
        Ok(json!({"ok": true, "line": {"verb": verb, "agent": item.agent_id, "what": what, "by": "overwatch",
                                       "by_detail": reason}}))
    }

    /// Sets payload keys of a stored item (a no-op once it is gone).
    pub(crate) fn set_payload(&self, item: u64, kv: &[(&str, Value)]) {
        let _ = self.store.write(|s| {
            if let Some(i) = s.inbox.iter_mut().find(|i| i.id == item)
                && let Some(p) = i.payload.as_object_mut()
            {
                for (k, v) in kv {
                    p.insert((*k).into(), v.clone());
                }
            }
        });
    }

    /// Takes payload keys off a stored item (a no-op once it is gone).
    pub(crate) fn unset_payload(&self, item: u64, keys: &[&str]) {
        let _ = self.store.write(|s| {
            if let Some(i) = s.inbox.iter_mut().find(|i| i.id == item)
                && let Some(p) = i.payload.as_object_mut()
            {
                for k in keys {
                    p.remove(*k);
                }
            }
        });
    }

    fn ow_annotate(&self, args: &Value) -> Out {
        let id = require(args, "decision")?;
        let note = require(args, "note")?;
        if note.chars().count() > 280 {
            return Err(bad("`note` is at most 280 characters"));
        }
        // The decision must be open, and `recommend` one of its actions.
        let Some(d) = self.open_decisions().into_iter().find(|d| d.id == id) else {
            return Err(err("stale", format!("{id} is not open (any more)")));
        };
        let recommend = str_arg(args, "recommend");
        if let Some(r) = recommend
            && !d.actions.iter().any(|a| a.id == r)
        {
            let ids: Vec<&str> = d.actions.iter().map(|a| a.id).collect();
            return Err(bad(format!("`recommend` is one of the card's actions: {}", ids.join(", "))));
        }
        let sources: Vec<String> = args
            .get("sources")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let note = json!({"note": note, "recommend": recommend, "sources": sources, "at_ms": now_ms()});
        match self.item_of_decision(id) {
            Some(item) => self.set_payload(item, &[("annotation", note)]),
            // A synthesized card (Governor ask, pane prompt, error): kept in memory.
            None => {
                self.overwatch.annotations.lock().unwrap_or_else(|e| e.into_inner()).insert(id.into(), note);
            }
        }
        // An annotated permission is the user's now (F1).
        if let Ok(n) = decision_n(id, "inbox") {
            self.overwatch_hand_over(n);
        }
        self.ow_audit(d.agent.as_deref(), "annotate", Some(id), json!({"decision": id}));
        self.touch("inbox");
        Ok(json!({"ok": true}))
    }

    /// The store item behind a listed decision id (`inbox-3`, `gate-5`, `q-9`…).
    pub(crate) fn item_of_decision(&self, id: &str) -> Option<u64> {
        self.store.read(|s| s.inbox.iter().find(|i| crate::decision::decision_id(&i.kind, i.id) == id).map(|i| i.id))
    }

    // ------------------------------------------------------------ unsticking

    fn ow_nudge(&self, args: &Value) -> Out {
        let a = self.ow_agent(args)?;
        let text = require(args, "text")?;
        self.nudge_agent(&a.id, "the Overwatch", text)?;
        self.ow_audit(Some(&a.id), "nudge", None, json!({"text": cut(text, 300)}));
        self.post_action("nudged", &a.id, &cut(text, 48), Some(cut(text, 60)), None);
        Ok(json!({"ok": true}))
    }

    fn ow_resume_agent(&self, args: &Value) -> Out {
        let a = self.ow_agent(args)?;
        let state = a.tracker.state.label();
        let (_, _, how) = self.resume_member(&a.id, "overwatch", "the Overwatch", state)?;
        self.ow_audit(Some(&a.id), "resume_agent", None, json!({"how": how}));
        self.post_action("resumed", &a.id, &a.id, None, None);
        Ok(json!({"ok": true, "how": how}))
    }

    fn ow_restart_fresh(&self, args: &Value) -> Out {
        let a = self.ow_agent(args)?;
        let why = cut(require(args, "why")?, 60);
        self.restart_fresh(&a.id, &why, ID)?;
        self.ow_audit(Some(&a.id), "restart_fresh", None, json!({"why": why}));
        self.post_action("restarted", &a.id, &a.id, Some(why), None);
        Ok(json!({"ok": true}))
    }

    fn ow_reprioritize(&self, args: &Value) -> Out {
        let entry = args.get("entry").and_then(Value::as_u64).ok_or_else(|| bad("`entry` is a queue entry id"))?;
        let tier = match require(args, "tier")? {
            "low" => Tier::Low,
            "normal" => Tier::Normal,
            "high" => Tier::High,
            _ => return Err(bad("`tier` is low, normal or high")),
        };
        let why = cut(require(args, "why")?, 60);
        let agent = self.with_governor(|g| g.queue().iter().find(|e| e.id.0 == entry).map(|e| e.agent.0.clone()));
        let from =
            self.governor_act(|g, now| (g.set_tier(EntryId(entry), tier, now), Vec::new())).map_err(|e| match e {
                TierError::NotQueued => err("stale", e.to_string()),
                _ => err("not_allowed", e.to_string()),
            })?;
        let agent = agent.unwrap_or_default();
        self.ow_audit(
            Some(&agent),
            "reprioritize",
            None,
            json!({"entry": entry, "from": from, "to": tier, "why": why}),
        );
        let undo = json!({"kind": "tier", "entry": entry, "from": from});
        self.post_action(
            "reprioritized",
            &agent,
            &format!("{from:?} → {tier:?}").to_lowercase(),
            Some(why),
            Some(undo),
        );
        Ok(json!({"ok": true, "from": format!("{from:?}").to_lowercase()}))
    }

    // ------------------------------------------------------------ chat

    fn ow_post(&self, args: &Value) -> Out {
        let text = require(args, "text")?;
        if text.chars().count() > 2_000 {
            return Err(bad("`text` is at most 2000 characters"));
        }
        let entry = self.post_overwatch_text(text, str_arg(args, "thread"))?;
        Ok(json!({"ok": true, "entry": entry.id()}))
    }

    // ------------------------------------------------------------ questions

    fn ow_answer_question(&self, args: &Value) -> Out {
        let id = require(args, "decision")?;
        let n = decision_n(id, "q")?;
        let item = self.store.read(|s| s.inbox.iter().find(|i| i.id == n).cloned());
        let item = item
            .filter(|i| i.kind == "question" && i.open())
            .ok_or_else(|| err("stale", format!("{id} is not open")))?;
        // Grounded: at least one source, each checked.
        let sources: Vec<String> = args
            .get("sources")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        if sources.is_empty() {
            return Err(err("ungrounded", "cite at least one source (a doc path#section, or chat:<id>)"));
        }
        let agent = item.agent_id.clone().and_then(|a| self.store.read(|s| s.agents.get(&a).cloned()));
        let mut shown = vec![];
        for s in &sources {
            shown.push(self.ground_source(agent.as_ref(), s).map_err(|why| err("ungrounded", why))?);
        }
        // One answer per question.
        let questions = item.payload.get("questions").and_then(Value::as_array).map_or(1, Vec::len).max(1);
        let answers = args.get("answers").and_then(Value::as_array).ok_or_else(|| bad("`answers` is a list"))?;
        let mut by_index: Vec<Option<String>> = vec![None; questions];
        for (pos, a) in answers.iter().enumerate() {
            let text = a.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
            let text = text.ok_or_else(|| bad("every answer has a `text`"))?;
            let i = a.get("question_index").and_then(Value::as_u64).map_or(pos, |i| i as usize);
            let slot = by_index.get_mut(i).ok_or_else(|| bad(format!("no question {i} ({questions} asked)")))?;
            if slot.replace(text.to_string()).is_some() {
                return Err(bad(format!("question {i} is answered twice")));
            }
        }
        if let Some(missing) = by_index.iter().position(Option::is_none) {
            return Err(bad(format!(
                "answer every question: {} of {questions} given (missing {missing})",
                answers.len()
            )));
        }
        let message = if questions == 1 {
            by_index[0].clone().unwrap_or_default()
        } else {
            by_index
                .iter()
                .enumerate()
                .map(|(i, a)| format!("{}. {}", i + 1, a.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.set_payload(n, &[("overwatch_sources", json!(shown)), ("overwatch_reason", json!(cut(&message, 60)))]);
        if let Err(e) = self.decide_as(n, InboxDecision::Reply, None, Some(&message), ID) {
            self.unset_payload(n, &["overwatch_sources", "overwatch_reason"]);
            return Err(err("stale", format!("{e:#}")));
        }
        Ok(json!({"ok": true, "line": {"verb": "answered", "agent": item.agent_id, "by": "overwatch",
                                       "sources": shown}}))
    }
}

/// The last `max` bytes of a file as text, starting at a line (a cut first line is dropped).
fn read_tail(path: &std::path::Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let cut = len.saturating_sub(max);
    f.seek(SeekFrom::Start(cut)).ok()?;
    let mut bytes = vec![];
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(if cut > 0 { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default() } else { text })
}
