//! The Inbox side of the runtime (FR-705, FR-707–709): PermissionRequest
//! hooks answered from the store's rules or held for a decision, the user's
//! decisions, scoped "always allow" rules, and the PR gate.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use orch_protocol::{HookOutput, HookPayload, InboxDecision, PermissionBehavior, now_ms};
use orch_store::{InboxItem, InboxStatus, RuleEffect, RuleScope};
use serde_json::{Value, json};

use crate::permissions::{self, Subject};
use crate::runtime::{Runtime, conn_closed};
use crate::store::{AgentRecord, HoldKind, HoldRow, HoldStatus};

/// Attention ladder (§9.6): an agent blocked on the user comes first.
pub const PRIORITY_BLOCKING: i64 = 30;
pub const PRIORITY_GATE: i64 = 20;
pub const PRIORITY_ALERT: i64 = 10;

/// The user allowed the held call (approve or always allow).
pub fn allows(item: &InboxItem) -> bool {
    matches!(
        item.decision.as_deref().and_then(orch_store::untag::<InboxDecision>),
        Some(InboxDecision::Approve | InboxDecision::AlwaysAllow)
    )
}

/// The user's words with a decision, if any.
pub fn reply(item: &InboxItem) -> Option<&str> {
    item.payload.get("reply").and_then(Value::as_str)
}

/// How a wait for a decision ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Waited {
    Decided(Box<InboxItem>),
    /// The hook's connection closed (Esc in the pane).
    Interrupted,
    Expired,
}

impl Runtime {
    /// Who an agent is for rule scopes.
    pub fn subject(&self, rec: &AgentRecord) -> Subject {
        Subject {
            agent_id: rec.id.clone(),
            role: rec.role.name.clone(),
            project_id: self.db.project_by_path(&rec.repo).ok().flatten().map(|p| p.id),
        }
    }

    /// Pending items in attention-ladder order.
    pub fn inbox(&self) -> Vec<InboxItem> {
        let mut v: Vec<InboxItem> = self.store.read(|s| s.inbox.iter().filter(|i| i.pending()).cloned().collect());
        v.sort_by_key(|i| (std::cmp::Reverse(i.priority), i.created_at_ms, i.id));
        v
    }

    pub(crate) fn add_item(
        &self,
        agent: Option<&str>,
        kind: &str,
        body: String,
        payload: Value,
        priority: i64,
    ) -> Result<u64> {
        let id = self.store.write(|s| s.add_inbox_item(agent, kind, body, payload, priority))?;
        self.event(agent.unwrap_or("-"), "inbox_added", json!({"id": id, "kind": kind}));
        Ok(id)
    }

    fn close_item(&self, id: u64, status: InboxStatus, decision: Option<String>) {
        let _ = self.store.write(|s| {
            let _ = s.close_inbox(id, status, decision);
        });
    }

    /// Closes the Inbox items of `agent`'s held permission prompts that were
    /// answered in the pane: the one for `tool_use_id`, or all of them.
    pub(crate) fn answered_in_pane(&self, agent: &str, tool_use_id: Option<&str>) {
        let open = |s: &crate::store::StateFile| -> Vec<u64> {
            s.holds
                .iter()
                .filter(|h| h.agent_id == agent && h.kind == HoldKind::Gate && h.status == HoldStatus::Held)
                .filter(|h| tool_use_id.is_none() || h.tool_use_id.as_deref() == tool_use_id)
                .filter_map(|h| h.inbox_id)
                .collect()
        };
        if self.store.read(|s| open(s).is_empty()) {
            return;
        }
        let _ = self.store.write(|s| {
            for item in open(s) {
                let _ = s.close_inbox(item, InboxStatus::Dismissed, Some("answered in the pane".into()));
            }
        });
    }

    /// Waits for a decision on `id`, watching the hook connection for EOF.
    pub(crate) fn wait_decision(&self, id: u64, budget: Duration, conn: Option<&UnixStream>) -> Waited {
        let end = Instant::now() + budget;
        loop {
            if let Some(item) = self.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned()) {
                match item.status {
                    InboxStatus::Pending => {}
                    InboxStatus::Decided => return Waited::Decided(Box::new(item)),
                    InboxStatus::Interrupted => return Waited::Interrupted,
                    _ => return Waited::Expired,
                }
            } else {
                return Waited::Expired;
            }
            if Instant::now() >= end {
                return Waited::Expired;
            }
            match conn {
                Some(c) => {
                    if conn_closed(c) {
                        return Waited::Interrupted;
                    }
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// PermissionRequest (FR-705): a rule decides at once; otherwise the
    /// request becomes an Inbox item and the hook is held (within orch-hook's
    /// deadline) until the user decides. No decision in time → no answer, so
    /// the CLI shows its own dialog in the pane.
    pub(crate) fn permission_request(
        &self,
        rec: &AgentRecord,
        p: &HookPayload,
        deadline_ms: u64,
        conn: Option<&UnixStream>,
    ) -> Option<HookOutput> {
        let tool = p.tool_name.clone().unwrap_or_else(|| "?".into());
        let input = p.tool_input.clone().unwrap_or(Value::Null);
        let subject = self.subject(rec);
        let rules = self.db.rules().unwrap_or_default();
        let mut ask_rule = None;
        if let Some(rule) = permissions::evaluate(&rules, &subject, &tool, &input) {
            let actor = format!("rule:{}", rule.id);
            match rule.effect {
                RuleEffect::Allow => {
                    let _ = self.db.audit(
                        &actor,
                        Some(&rec.id),
                        "permission",
                        Some(&tool),
                        "allow",
                        json!({"input": input}),
                    );
                    return Some(HookOutput::permission_request(PermissionBehavior::Allow, None));
                }
                RuleEffect::Deny => {
                    let _ = self.db.audit(
                        &actor,
                        Some(&rec.id),
                        "permission",
                        Some(&tool),
                        "deny",
                        json!({"input": input}),
                    );
                    return Some(HookOutput::permission_request(
                        PermissionBehavior::Deny,
                        Some(&format!("ORCH: denied by the user's rule {} ({}).", rule.id, rule.pattern)),
                    ));
                }
                RuleEffect::Ask => ask_rule = Some(rule.id),
            }
        }
        let budget = self.cfg.permission_hold.min(Duration::from_millis(deadline_ms.saturating_sub(1_000)));
        self.permission_request_with(rec, p, &subject, ask_rule, budget, conn)
    }

    /// The Inbox part of [`Runtime::permission_request`], after the rules.
    pub(crate) fn permission_request_with(
        &self,
        rec: &AgentRecord,
        p: &HookPayload,
        subject: &Subject,
        ask_rule: Option<u64>,
        budget: Duration,
        conn: Option<&UnixStream>,
    ) -> Option<HookOutput> {
        if budget.is_zero() {
            return None;
        }
        let tool = p.tool_name.clone().unwrap_or_else(|| "?".into());
        let input = p.tool_input.clone().unwrap_or(Value::Null);
        match self.hold_for_decision(rec, &tool, &input, p.tool_use_id.clone(), subject, ask_rule, budget, conn) {
            Waited::Decided(i) if allows(&i) => Some(HookOutput::permission_request(PermissionBehavior::Allow, None)),
            Waited::Decided(i) => {
                let m = reply(&i).map(|m| format!("ORCH: the user says: {m}"));
                let m = m.unwrap_or_else(|| "ORCH: the user denied this in the Inbox.".into());
                Some(HookOutput::permission_request(PermissionBehavior::Deny, Some(&m)))
            }
            // The CLI's own dialog takes over in the pane.
            Waited::Interrupted | Waited::Expired => None,
        }
    }

    /// Holds a tool call for an Inbox decision: a `permission` item, a gate
    /// hold row (for the reconciler), the wait (EOF on `conn` = Esc in the
    /// pane), and the item closed if nobody decided. Shared by
    /// PermissionRequest hooks and orch-mcp calls under an `ask` rule.
    /// `ask_rule`: the rule that sent it here (then "always allow" is not
    /// offered: an allow rule would never win over it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hold_for_decision(
        &self,
        rec: &AgentRecord,
        tool: &str,
        input: &Value,
        tool_use_id: Option<String>,
        subject: &Subject,
        ask_rule: Option<u64>,
        budget: Duration,
        conn: Option<&UnixStream>,
    ) -> Waited {
        let always = if ask_rule.is_some() { None } else { permissions::always_pattern(tool, input) };
        let summary = permissions::summary(tool, input);
        let item = match self.add_item(
            Some(&rec.id),
            "permission",
            format!("{}: {summary}", rec.id),
            json!({
                "tool": tool,
                "input": input,
                "tool_use_id": tool_use_id,
                "always_pattern": always,
                "ask_rule": ask_rule,
                "project_id": subject.project_id,
                "role": subject.role,
            }),
            PRIORITY_BLOCKING,
        ) {
            Ok(i) => i,
            Err(_) => return Waited::Expired,
        };
        let hold_id = crate::delivery::new_turn_id();
        let _ = self.store.write(|s| {
            s.holds.push(HoldRow {
                id: hold_id.clone(),
                agent_id: rec.id.clone(),
                tool_use_id,
                kind: HoldKind::Gate,
                status: HoldStatus::Held,
                summary,
                created_at_ms: now_ms(),
                inbox_id: Some(item),
            })
        });
        let waited = self.wait_decision(item, budget, conn);
        let hold_status = match &waited {
            Waited::Decided(i) if allows(i) => HoldStatus::Allowed,
            Waited::Decided(_) => HoldStatus::Denied,
            Waited::Interrupted => {
                self.close_item(item, InboxStatus::Interrupted, Some("interrupted by the user".into()));
                HoldStatus::Interrupted
            }
            Waited::Expired => {
                self.close_item(item, InboxStatus::Expired, Some("not answered in time".into()));
                HoldStatus::Interrupted
            }
        };
        let _ = self.store.write(|s| {
            if let Some(h) = s.holds.iter_mut().find(|h| h.id == hold_id) {
                h.status = hold_status;
            }
        });
        self.event(&rec.id, "permission", json!({"item": item, "tool": tool, "outcome": hold_status}));
        waited
    }

    /// Adds a permission rule (FR-708) and logs it (FR-709).
    pub fn add_rule(
        &self,
        scope: RuleScope,
        scope_id: &str,
        pattern: &str,
        effect: RuleEffect,
        source_inbox_item: Option<u64>,
    ) -> Result<orch_store::PermissionRule> {
        let rule = self.db.add_rule(scope, scope_id, pattern, effect, source_inbox_item)?;
        self.db.audit("user", None, "rule_added", Some(pattern), effect.as_str(), json!(rule))?;
        Ok(rule)
    }

    /// Revokes a rule (the row is kept) and logs it. False if unknown or already revoked.
    pub fn revoke_rule(&self, id: u64) -> Result<bool> {
        let done = self.db.revoke_rule(id)?;
        self.db.audit(
            "user",
            None,
            "rule_revoked",
            Some(&id.to_string()),
            if done { "ok" } else { "unknown" },
            json!({}),
        )?;
        Ok(done)
    }

    /// The user's decision on an Inbox item. The item is claimed atomically
    /// (a second decision, or one after the hold expired, is refused), then
    /// logged (FR-709) before anything acts on it.
    pub fn decide(
        &self,
        id: u64,
        decision: InboxDecision,
        scope: Option<&str>,
        message: Option<&str>,
    ) -> Result<Value> {
        let item = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == id).cloned())
            .ok_or_else(|| anyhow!("no Inbox item {id}"))?;
        if !item.pending() {
            bail!("Inbox item {id} is already {:?}", item.status);
        }
        // Checked before claiming: an always-allow that can't become a rule is refused whole.
        let rule_spec = if decision == InboxDecision::AlwaysAllow {
            let Some(pattern) = item.payload.get("always_pattern").and_then(Value::as_str).map(str::to_string) else {
                match item.payload.get("ask_rule").and_then(Value::as_u64) {
                    Some(r) => bail!("rule {r} says to always ask for this: revoke it to allow it always"),
                    None => bail!("item {id} cannot become a rule (approve it once instead)"),
                }
            };
            let sc =
                RuleScope::parse(scope.unwrap_or("project")).ok_or_else(|| anyhow!("scope: project | role | agent"))?;
            let sid = match sc {
                RuleScope::Agent => item.agent_id.clone(),
                RuleScope::Role => item.payload.get("role").and_then(Value::as_str).map(str::to_string),
                RuleScope::Project => item.payload.get("project_id").and_then(Value::as_str).map(str::to_string),
            };
            let sid = sid.ok_or_else(|| anyhow!("no {} to scope the rule to", sc.as_str()))?;
            Some((sc, sid, pattern))
        } else {
            None
        };
        let word = orch_store::tag(&decision);
        let status = if decision == InboxDecision::Dismiss { InboxStatus::Dismissed } else { InboxStatus::Decided };
        let claimed = self.store.write(|s| {
            s.close_inbox(id, status, Some(word.clone())).map(|i| {
                if let Some(m) = message {
                    i.payload["reply"] = json!(m);
                }
            })
        })?;
        if let Err(now) = claimed {
            bail!("Inbox item {id} is already {now:?}");
        }
        let target = item.payload.get("tool").and_then(Value::as_str).map(str::to_string);
        self.db.audit(
            "user",
            item.agent_id.as_deref(),
            &item.kind,
            target.as_deref(),
            &word,
            json!({"item": id, "body": item.body, "message": message, "scope": scope}),
        )?;
        let mut result = json!({"item": id, "decision": word});
        if let Some((sc, sid, pattern)) = rule_spec {
            result["rule"] = json!(self.add_rule(sc, &sid, &pattern, RuleEffect::Allow, Some(id))?);
        }
        // Kind-specific follow-ups that don't wait on a held hook.
        let action = item.payload.get("action").and_then(Value::as_str).unwrap_or("");
        match (item.kind.as_str(), action, decision) {
            ("gate", "push_pr", InboxDecision::Approve) => {
                if let Some(me) = self.me.upgrade() {
                    let payload = item.payload.clone();
                    std::thread::spawn(move || me.open_pr(id, &payload));
                }
                result["pr"] = json!("pushing and opening the PR");
            }
            ("gate", "spec_approval", InboxDecision::Approve | InboxDecision::AlwaysAllow) => {
                self.spec_decided(&item.payload, item.agent_id.as_deref(), true, None);
            }
            ("gate", "spec_approval", InboxDecision::Reject | InboxDecision::Reply) => {
                self.spec_decided(&item.payload, item.agent_id.as_deref(), false, message);
            }
            ("gate", "push_pr", InboxDecision::Reject | InboxDecision::Reply) => {
                if let Some(a) = &item.agent_id {
                    self.pr_declined(a);
                    let why = message.map(|m| format!(" They said: {m}")).unwrap_or_default();
                    let _ = self.deliver_user(a, &format!("ORCH: the user declined your pull request request.{why}"));
                }
            }
            // The agent was told the answer comes as a turn: say there is none.
            ("question", "ask_user", InboxDecision::Dismiss) => {
                if let Some(a) = &item.agent_id {
                    let _ = self.deliver_user(
                        a,
                        "ORCH: the user dismissed your question without answering. Decide yourself, or ask your team.",
                    );
                }
            }
            ("question", _, InboxDecision::Reply) => {
                if let (Some(a), Some(m)) = (&item.agent_id, message) {
                    let q = item.payload.get("question").and_then(Value::as_str).unwrap_or(&item.body);
                    let text = format!(
                        "ORCH: the user answered your question.\nQuestion: {}\nAnswer: {m}",
                        crate::team::cut(q, 600)
                    );
                    result["turn"] = json!(self.deliver_user(a, &text)?);
                }
            }
            (k, _, InboxDecision::Reply) if k != "permission" => {
                if let (Some(a), Some(m)) = (&item.agent_id, message) {
                    result["turn"] = json!(self.deliver_user(a, m)?);
                }
            }
            _ => {}
        }
        Ok(result)
    }

    /// `request_pr` from orch-mcp: an Inbox gate (the agent can't push).
    pub(crate) fn request_pr(&self, rec: &AgentRecord, args: &Value) -> Result<String> {
        let title = args.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        let title = title.ok_or_else(|| anyhow!("title is required"))?.chars().take(256).collect::<String>();
        let body = args.get("body").and_then(Value::as_str).unwrap_or("").chars().take(20_000).collect::<String>();
        let draft = args.get("draft").and_then(Value::as_bool).unwrap_or(false);
        let row = self.store.read(|s| s.worktrees.get(&rec.worktree).cloned());
        let branch = orch_git::git(&rec.worktree, &["symbolic-ref", "--short", "HEAD"])?.trim().to_string();
        let base = row.map(|r| r.base).unwrap_or_else(|| "main".into());
        if self.store.read(|s| {
            s.inbox.iter().any(|i| {
                i.pending()
                    && i.payload.get("action") == Some(&json!("push_pr"))
                    && i.agent_id.as_deref() == Some(&rec.id)
            })
        }) {
            bail!("a pull request request is already waiting for the user");
        }
        let id = self.add_item(
            Some(&rec.id),
            "gate",
            format!("{}: open a PR \"{title}\" ({branch} → {base})", rec.id),
            json!({"action": "push_pr", "branch": branch, "base": base, "title": title, "body": body, "draft": draft,
                   "worktree": rec.worktree}),
            PRIORITY_GATE,
        )?;
        self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "pr_requested",
            Some(&branch),
            "pending",
            json!({"item": id}),
        )?;
        Ok(format!(
            "Requested: Inbox item {id} asks the user to push {branch} and open a PR into {base}. You will be told the result; there is nothing else to do for it now."
        ))
    }

    /// After the PR gate is approved: push with the user's credentials, open
    /// the PR, tell the agent (ADR-009: only the app pushes).
    fn open_pr(&self, item: u64, p: &Value) {
        let agent = self.store.read(|s| s.inbox.iter().find(|i| i.id == item).and_then(|i| i.agent_id.clone()));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let wt = std::path::PathBuf::from(s("worktree"));
        let (branch, base) = (s("branch"), s("base"));
        let r = orch_git::gh::push(&wt, "origin", &branch).and_then(|_| {
            orch_git::gh::create_pr(&wt, &base, &branch, &s("title"), &s("body"), p["draft"].as_bool().unwrap_or(false))
        });
        let (decision, msg) = match &r {
            Ok(url) => ("ok", format!("ORCH: the user approved; the orchestrator pushed {branch} and opened {url}")),
            Err(e) => ("failed", format!("ORCH: the user approved, but pushing or opening the PR failed: {e:#}")),
        };
        let _ = self.db.audit(
            "core",
            agent.as_deref(),
            "pr",
            Some(&branch),
            decision,
            json!({"item": item, "result": r.as_ref().map_err(|e| e.to_string())}),
        );
        self.event(agent.as_deref().unwrap_or("-"), "pr", json!({"item": item, "ok": r.is_ok()}));
        if let Some(a) = agent {
            self.pr_opened(&a, r.as_ref().ok().map(String::as_str));
            let _ = self.deliver_user(&a, &msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::{hook, test_runtime};
    use std::sync::Arc;

    fn perm(cmd: &str) -> orch_protocol::HookMsg {
        hook("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "tu1"}))
    }

    fn behavior(o: &Option<HookOutput>) -> Option<String> {
        o.as_ref().map(|o| o.0["hookSpecificOutput"]["decision"]["behavior"].as_str().unwrap().to_string())
    }

    /// Decides the next pending item from another thread, like the UI would.
    fn decide_soon(
        rt: &Arc<Runtime>,
        d: InboxDecision,
        scope: Option<&'static str>,
        msg: Option<&'static str>,
    ) -> std::thread::JoinHandle<u64> {
        let rt = rt.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(i) = rt.inbox().first().cloned() {
                    rt.decide(i.id, d, scope, msg).unwrap();
                    return i.id;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("no Inbox item");
        })
    }

    #[test]
    fn rules_answer_at_once_and_are_audited() {
        let (_d, rt) = test_runtime(true);
        rt.db.add_rule(RuleScope::Role, "solo", "Bash(cargo test:*)", RuleEffect::Allow, None).unwrap();
        rt.db.add_rule(RuleScope::Agent, "a1", "Bash(rm:*)", RuleEffect::Deny, None).unwrap();
        assert_eq!(behavior(&rt.on_hook(&perm("cargo test -p x"), None)).as_deref(), Some("allow"));
        let o = rt.on_hook(&perm("rm -rf build"), None);
        assert_eq!(behavior(&o).as_deref(), Some("deny"));
        assert!(o.unwrap().0.to_string().contains("ORCH: denied by the user's rule"));
        assert!(rt.inbox().is_empty());
        let log = rt.db.audit_log(Some("a1"), 10).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.iter().all(|e| e.actor.starts_with("rule:")));
    }

    #[test]
    fn inbox_approve_always_reply_and_timeouts() {
        let (_d, rt) = test_runtime(true);
        assert!(rt.cfg.permission_hold < Duration::from_secs(1));
        // Nobody answers: the item expires and the pane shows the dialog.
        assert!(rt.on_hook(&perm("ls"), None).is_none());
        let items = rt.store.read(|s| s.inbox.clone());
        assert_eq!(items.last().unwrap().status, InboxStatus::Expired);

        // Approve from "the UI" while the hook is held.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("make build", Duration::from_secs(5)));
        let id = decide_soon(&rt, InboxDecision::Approve, None, None).join().unwrap();
        assert_eq!(behavior(&h.join().unwrap()).as_deref(), Some("allow"));
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().status), HoldStatus::Allowed);
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().inbox_id), Some(id));

        // Reply = deny with the user's words.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("npm publish", Duration::from_secs(5)));
        decide_soon(&rt, InboxDecision::Reply, None, Some("not on main")).join().unwrap();
        let o = h.join().unwrap();
        assert_eq!(behavior(&o).as_deref(), Some("deny"));
        assert!(o.unwrap().0.to_string().contains("not on main"));

        // Always allow → an agent-scoped rule; the next identical call never reaches the Inbox.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("cargo fmt", Duration::from_secs(5)));
        decide_soon(&rt, InboxDecision::AlwaysAllow, Some("agent"), None).join().unwrap();
        assert_eq!(behavior(&h.join().unwrap()).as_deref(), Some("allow"));
        let rules = rt.db.rules().unwrap();
        assert_eq!((rules[0].pattern.as_str(), rules[0].scope), ("Bash(cargo fmt)", RuleScope::Agent));
        assert_eq!(behavior(&rt.on_hook(&perm("cargo fmt"), None)).as_deref(), Some("allow"));
        assert!(rt.inbox().is_empty());
        // Every decision is in the audit log (FR-709).
        let actions: Vec<String> = rt.db.audit_log(None, 50).unwrap().into_iter().map(|e| e.action).collect();
        assert!(
            actions.contains(&"rule_added".to_string()) && actions.iter().filter(|a| *a == "permission").count() >= 3
        );
        // A decided item can't be decided again.
        assert!(rt.decide(id, InboxDecision::Reject, None, None).is_err());
    }

    #[test]
    fn a_decision_is_claimed_once_and_ask_rules_refuse_always_allow() {
        let (_d, rt) = test_runtime(true);
        let id = rt.store.write(|s| s.add_inbox(Some("a1"), "question", "q".into())).unwrap();
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let rt = rt.clone();
                std::thread::spawn(move || rt.decide(id, InboxDecision::Reply, None, Some("yes")).is_ok())
            })
            .collect();
        let wins = hs.into_iter().map(|h| h.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(wins, 1, "exactly one decision acts");
        assert_eq!(rt.store.read(|s| s.turns.iter().filter(|t| t.body.ends_with("Answer: yes")).count()), 1);
        // An item sent to the Inbox by an `ask` rule can be approved, not always-allowed.
        rt.db.add_rule(RuleScope::Role, "solo", "Bash(make:*)", RuleEffect::Ask, None).unwrap();
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.on_hook(&perm("make all"), None));
        let mut item = None;
        for _ in 0..100 {
            item = rt.inbox().into_iter().find(|i| i.kind == "permission");
            if item.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let item = item.unwrap();
        let e = rt.decide(item.id, InboxDecision::AlwaysAllow, None, None).unwrap_err();
        assert!(e.to_string().contains("always ask"), "{e}");
        let st = rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).unwrap().status);
        assert_ne!(st, InboxStatus::Decided, "refused whole");
        assert!(rt.db.rules().unwrap().iter().all(|r| r.effect != RuleEffect::Allow));
        let _ = h.join();
    }

    #[test]
    fn an_answer_in_the_pane_closes_the_item_and_ends_the_hold() {
        let (_d, rt) = test_runtime(true);
        let rt2 = rt.clone();
        let t0 = Instant::now();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("make deploy", Duration::from_secs(10)));
        let mut item = None;
        for _ in 0..200 {
            item = rt.inbox().into_iter().next();
            if item.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let item = item.expect("held");
        assert_eq!(item.payload["tool_use_id"], "tu-make deploy");
        // Another call's hook does not close it.
        rt.on_hook(&hook("PostToolUse", json!({"tool_name": "Bash", "tool_use_id": "other"})), None);
        assert!(rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).unwrap().pending()));
        // The user answered the CLI's dialog: the tool ran.
        rt.on_hook(
            &hook("PostToolUse", json!({"tool_name": "Bash", "tool_use_id": item.payload["tool_use_id"].clone()})),
            None,
        );
        assert!(h.join().unwrap().is_none(), "no decision from the Inbox");
        assert!(t0.elapsed() < Duration::from_secs(5), "the hold ended at once");
        let i = rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).cloned().unwrap());
        assert_eq!((i.status, i.decision.as_deref()), (InboxStatus::Dismissed, Some("answered in the pane")));
    }

    #[test]
    fn esc_in_the_pane_interrupts_the_hold() {
        let (_d, rt) = test_runtime(true);
        let (core_end, hook_end) = UnixStream::pair().unwrap();
        drop(hook_end);
        let t0 = Instant::now();
        let mut m = perm("ls");
        m.deadline_ms = 10_000;
        assert!(rt.on_hook(&m, Some(&core_end)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert_eq!(rt.store.read(|s| s.inbox.last().unwrap().status), InboxStatus::Interrupted);
    }

    #[test]
    fn alerts_are_dismissed_and_replies_become_turns() {
        let (_d, rt) = test_runtime(true);
        let a = rt.store.write(|s| s.add_inbox(Some("a1"), "alert", "x".into())).unwrap();
        let q = rt.store.write(|s| s.add_inbox(Some("a1"), "question", "which db?".into())).unwrap();
        rt.decide(a, InboxDecision::Dismiss, None, None).unwrap();
        let r = rt.decide(q, InboxDecision::Reply, None, Some("use sqlite")).unwrap();
        assert!(r["turn"].is_string());
        let body = rt.store.read(|s| s.turns[0].body.clone());
        assert!(body.contains("Question: which db?") && body.ends_with("Answer: use sqlite"), "{body}");
        assert_eq!(rt.store.read(|s| s.turns[0].kind), governor::TurnKind::User);
        assert!(rt.inbox().is_empty());
    }

    impl Runtime {
        /// A PermissionRequest for `a1` with a longer hold than the test config.
        fn permission_request_for_test(&self, cmd: &str, hold: Duration) -> Option<HookOutput> {
            let rec = self.store.read(|s| s.agents["a1"].clone());
            let p = HookPayload::parse(&json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
                "tool_input": {"command": cmd}, "tool_use_id": format!("tu-{cmd}")}));
            let subject = self.subject(&rec);
            self.permission_request_with(&rec, &p, &subject, None, hold, None)
        }
    }
}
