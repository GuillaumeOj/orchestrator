//! The Inbox side of the runtime (FR-705, FR-707–709): PermissionRequest
//! hooks answered from the store's rules or held for a decision, parked when
//! nobody answers in time, the user's decisions, scoped "always allow" rules,
//! one-shot allows, and the PR gate.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use orch_protocol::{HookOutput, HookPayload, InboxDecision, PermissionBehavior, now_ms};
use orch_store::{InboxItem, InboxStatus, PermissionRule, RuleEffect, RuleScope};
use serde_json::{Value, json};

use crate::permissions::{self, Subject};
use crate::runtime::{Runtime, conn_closed, reserved_slot};
use crate::state::AgentState;
use crate::store::{AgentRecord, HoldKind, HoldRow, HoldStatus, StateFile, TurnStatus};

/// Attention ladder (§9.6): an agent blocked on the user comes first.
pub const PRIORITY_BLOCKING: i64 = 30;
pub const PRIORITY_GATE: i64 = 20;
pub const PRIORITY_ALERT: i64 = 10;

/// The hook's answer when a hold expires (FR-705): the turn ends, the item parks.
pub const PARKED_MESSAGE: &str =
    "ORCH: awaiting the user's approval. Stop here; you will be resumed when it is answered.";
/// Setting overriding [`crate::runtime::RuntimeConfig::permission_hold`], in seconds.
pub const HOLD_SECS_KEY: &str = "permissions.hold_secs";
/// How long the one-shot allow of an approved parked call lives.
pub const ONE_SHOT_TTL_MS: u64 = 3_600_000;
/// After the Stop that ends a parked turn: its hook reply goes out first.
const PARK_SUSPEND_DELAY: Duration = Duration::from_millis(500);

/// Called with every Inbox outcome (`{item, kind, tool, decided_by, effect,
/// created_at_ms, at_ms}`), for an embedding UI's toast.
pub type OutcomeSink = Arc<dyn Fn(&Value) + Send + Sync>;

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

/// A permission item's `tool_input` (`input` in rows from before P4.0).
pub(crate) fn tool_input(item: &InboxItem) -> Value {
    item.payload.get("tool_input").or_else(|| item.payload.get("input")).cloned().unwrap_or(Value::Null)
}

/// Pushes a branch and opens its PR (worktree, base, branch, title, body,
/// draft): the PR's URL. The real one uses git and `gh`.
#[derive(Clone)]
pub struct PrOpener(pub Arc<OpenPr>);

/// (worktree, base, branch, title, body, draft) → the PR's URL.
pub type OpenPr = dyn Fn(&std::path::Path, &str, &str, &str, &str, bool) -> Result<String> + Send + Sync;

impl std::fmt::Debug for PrOpener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrOpener")
    }
}

/// A step of a decision's lifecycle after `decided` (spec §3.3).
pub enum Stage {
    /// The core delivered the answer (hook replied, turn pasted or queued, push started).
    Sent,
    /// What it did, with its detail (ETA, URL).
    Effect(&'static str, Option<String>),
    Failed(String),
    Stale(&'static str),
}

/// How a wait for a decision ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Waited {
    Decided(Box<InboxItem>),
    /// Answered in Claude Code's own dialog (the item was closed by the call's next hook).
    InPane,
    /// The hook's connection closed (Esc in the pane).
    Interrupted,
    /// Nobody answered in time: the item waits, parked (FR-705).
    Parked,
    /// The item could not be created or is gone.
    Expired,
}

/// One tool call waiting on a permission.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub tool: String,
    pub input: Value,
    pub tool_use_id: Option<String>,
    pub cwd: Option<String>,
}

impl Call {
    pub(crate) fn from_hook(p: &HookPayload) -> Call {
        Call {
            tool: p.tool_name.clone().unwrap_or_else(|| "?".into()),
            input: p.tool_input.clone().unwrap_or(Value::Null),
            tool_use_id: p.tool_use_id.clone(),
            cwd: p.cwd.clone(),
        }
    }

    /// The audit detail (FR-709): the call, its item, and the rule that answered.
    pub(crate) fn detail(&self, item: Option<u64>, rule: Option<&PermissionRule>) -> Value {
        let mut d = json!({"item": item, "tool": self.tool, "tool_input": self.input, "tool_use_id": self.tool_use_id});
        if let Some(r) = rule {
            d["rule"] = json!({"id": r.id, "pattern": r.pattern, "effect": r.effect.as_str()});
        }
        d
    }
}

/// The call's main argument (the command, the path, the notebook), if any.
pub(crate) fn main_arg(tool: &str, input: &Value) -> Option<String> {
    let notebook = input.get("notebook_path").and_then(Value::as_str).map(str::to_string);
    permissions::primary_input(tool, input).or(notebook)
}

/// The call's main argument as the dialog shows it, and its description.
fn brief(tool: &str, input: &Value) -> (String, Option<String>) {
    let main = crate::team::cut(&main_arg(tool, input).unwrap_or_else(|| tool.to_string()), 199);
    (main, input.get("description").and_then(Value::as_str).map(str::to_string))
}

/// `{files: [{path, added, removed}], added, removed}` of `base...branch`,
/// computed once when the PR gate is raised (binary files count 0 lines).
pub(crate) fn diff_summary(wt: &std::path::Path, base: &str, branch: &str) -> Value {
    let Ok(files) = orch_git::diff::numstat(wt, &format!("{base}...{branch}")) else { return Value::Null };
    let (added, removed) = files.values().fold((0, 0), |(a, r), (x, y)| (a + x.unwrap_or(0), r + y.unwrap_or(0)));
    let files: Vec<Value> = files
        .iter()
        .map(|(path, (a, r))| json!({"path": path, "added": a.unwrap_or(0), "removed": r.unwrap_or(0)}))
        .collect();
    json!({"files": files, "added": added, "removed": removed})
}

/// The PR gate's patch is cut past this (the gate preview's limit).
const PATCH_MAX: usize = 2 << 20;

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// An open permission item of `agent` waits parked.
fn has_parked(s: &StateFile, agent: &str) -> bool {
    s.inbox.iter().any(|i| i.status == InboxStatus::Parked && i.agent_id.as_deref() == Some(agent))
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

    /// Open (pending or parked) items in attention-ladder order.
    pub fn inbox(&self) -> Vec<InboxItem> {
        let mut v: Vec<InboxItem> = self.store.read(|s| s.inbox.iter().filter(|i| i.open()).cloned().collect());
        v.sort_by_key(|i| (std::cmp::Reverse(i.priority), i.created_at_ms, i.id));
        v
    }

    /// Calls `f` with every Inbox outcome from now on (the app's toast).
    pub fn set_outcome_sink(&self, f: OutcomeSink) {
        *self.outcome_sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// An item was decided or parked: the `inbox_outcome` event (the run
    /// report's deciders and waiting time) and the UI's toast.
    pub(crate) fn outcome(&self, item: &InboxItem, decided_by: &str, effect: &str) {
        self.outcome_with(item, decided_by, effect, None);
    }

    fn outcome_with(&self, item: &InboxItem, decided_by: &str, effect: &str, error: Option<&str>) {
        let mut v = json!({
            "item": item.id, "kind": item.kind, "tool": item.payload.get("tool"), "decided_by": decided_by,
            "effect": effect, "created_at_ms": item.created_at_ms, "at_ms": now_ms(),
        });
        if let Some(e) = error {
            v["error"] = json!(e);
        }
        let sink = self.outcome_sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(f) = sink {
            f(&v);
        }
        self.event(item.agent_id.as_deref().unwrap_or("-"), "inbox_outcome", v);
    }

    /// Audits (FR-709) and reports an item a decider other than the user's
    /// `decide` closed or parked.
    fn closed_by(&self, item: &InboxItem, actor: &str, decision: &str, effect: &str) {
        let s = |k: &str| item.payload.get(k).and_then(Value::as_str).map(str::to_string);
        let call = Call {
            tool: s("tool").unwrap_or_default(),
            input: tool_input(item),
            tool_use_id: s("tool_use_id"),
            cwd: None,
        };
        let detail = call.detail(Some(item.id), None);
        let _ = self.db.audit(actor, item.agent_id.as_deref(), &item.kind, Some(&call.tool), decision, detail);
        self.outcome(item, actor, effect);
    }

    /// Closes every open (pending or parked) item of `agent` as the core's,
    /// with `why` as the decision (the agent was deleted or archived).
    pub(crate) fn close_items_of(&self, agent: &str, why: &str) {
        self.close_items_where(|i| i.agent_id.as_deref() == Some(agent), why);
    }

    /// Closes every open item that `pick` selects as the core's, with `why`
    /// as the decision.
    pub(crate) fn close_items_where(&self, pick: impl Fn(&InboxItem) -> bool, why: &str) {
        let closed = self
            .store
            .write(|s| {
                let ids: Vec<u64> = s.inbox.iter().filter(|i| i.open() && pick(i)).map(|i| i.id).collect();
                ids.into_iter()
                    .filter_map(|id| s.close_inbox(id, InboxStatus::Dismissed, Some(why.into()), "core").ok().cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for i in closed {
            self.advance(i.id, Stage::Effect("dismissed", None));
            self.closed_by(&i, "core", why, "dismissed");
        }
    }

    pub(crate) fn add_item(
        &self,
        agent: Option<&str>,
        kind: &str,
        body: String,
        payload: Value,
        priority: i64,
    ) -> Result<u64> {
        let subject = self.subject_now(agent, payload.get("project").and_then(Value::as_str));
        let thread = crate::decision::thread_of(&subject);
        let id = self.store.write(|s| {
            let id = s.add_inbox_item(agent, kind, body, payload, priority);
            if let Some(i) = s.inbox.iter_mut().find(|i| i.id == id) {
                (i.subject, i.thread) = (subject, thread);
            }
            id
        })?;
        self.event(agent.unwrap_or("-"), "inbox_added", json!({"id": id, "kind": kind}));
        Ok(id)
    }

    /// Who and what a new decision is about, as it stands now (stored with it).
    fn subject_now(&self, agent: Option<&str>, project: Option<&str>) -> Value {
        let rec = agent.and_then(|a| self.store.read(|s| s.agents.get(a).cloned()));
        let feature = rec
            .as_ref()
            .and_then(|r| r.team.as_ref())
            .and_then(|t| self.db.feature(&t.feature_id).ok().flatten())
            .map(|f| f.slug);
        let project = rec.as_ref().and_then(|r| self.agent_project(r)).or_else(|| project.map(str::to_string));
        crate::decision::subject(agent, rec.as_ref(), feature.as_deref(), project.as_deref())
    }

    /// Moves a decided item on in its lifecycle (P4.1, FR-1303); each step
    /// reaches the `now` push.
    pub fn advance(&self, id: u64, stage: Stage) {
        self.advance_where(|i| i.id == id, stage);
    }

    /// [`Runtime::advance`] for every item `which` selects, in one write.
    fn advance_where(&self, which: impl Fn(&InboxItem) -> bool, stage: Stage) {
        let now = now_ms();
        let _ = self.store.write(|s| {
            for i in s.inbox.iter_mut().filter(|i| which(i)) {
                match &stage {
                    Stage::Sent => {
                        i.status = InboxStatus::Sent;
                        i.sent_at_ms = Some(now);
                    }
                    Stage::Effect(effect, detail) => {
                        // A dismissal keeps its status: its effect says so too.
                        if i.status != InboxStatus::Dismissed {
                            i.status = InboxStatus::Effect;
                        }
                        i.sent_at_ms.get_or_insert(now);
                        (i.effect, i.effect_detail, i.effect_at_ms) =
                            (Some((*effect).into()), detail.clone(), Some(now));
                    }
                    Stage::Failed(error) => {
                        i.status = InboxStatus::Failed;
                        (i.error, i.effect_at_ms) = (Some(error.clone()), Some(now));
                    }
                    Stage::Stale(why) => {
                        i.status = InboxStatus::Stale;
                        i.stale_why = Some((*why).into());
                        i.decided_at_ms.get_or_insert(now);
                    }
                }
            }
        });
    }

    /// `agent` started a turn: the answers delivered to it (sent, or queued
    /// by the Governor) took effect. A parked permission still undecided is
    /// not touched.
    pub(crate) fn answers_resumed(&self, agent: &str) {
        let waiting = |i: &InboxItem| {
            i.agent_id.as_deref() == Some(agent)
                && (i.status == InboxStatus::Sent
                    || (i.status == InboxStatus::Effect && i.effect.as_deref() == Some("queued")))
        };
        // Every turn start passes here: the store is written only when one waits.
        if self.store.read(|s| s.inbox.iter().any(waiting)) {
            self.advance_where(waiting, Stage::Effect("resumed", None));
        }
    }

    /// The Governor queued `agent`'s next turn: answers sent to it wait with an ETA.
    pub(crate) fn answers_queued(&self, agent: &str, eta: Option<String>) {
        let sent = |i: &InboxItem| i.agent_id.as_deref() == Some(agent) && i.status == InboxStatus::Sent;
        if self.store.read(|s| s.inbox.iter().any(sent)) {
            self.advance_where(sent, Stage::Effect("queued", eta));
        }
    }

    /// An item's lifecycle now (`inbox_act`'s answer), with a version that
    /// outranks every push the client holds ([`Runtime::answered_lifecycle`]).
    pub fn lifecycle(&self, id: u64) -> Value {
        self.answered_lifecycle(id).map_or(Value::Null, |lc| json!(lc))
    }

    /// Closes an open item; the closed item, or `None` if it was already closed.
    pub(crate) fn close_item(
        &self,
        id: u64,
        status: InboxStatus,
        decision: &str,
        decided_by: &str,
    ) -> Option<InboxItem> {
        self.store.write(|s| s.close_inbox(id, status, Some(decision.into()), decided_by).ok().cloned()).ok().flatten()
    }

    /// Closes the Inbox items of `agent`'s held permission prompts that were
    /// answered in the pane: the one for `tool_use_id`, or all of them.
    pub(crate) fn answered_in_pane(&self, agent: &str, tool_use_id: Option<&str>) {
        let open = |s: &StateFile| -> Vec<u64> {
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
        let closed = self
            .store
            .write(|s| {
                open(s)
                    .into_iter()
                    .filter_map(|id| {
                        s.close_inbox(id, InboxStatus::Dismissed, Some("answered in the pane".into()), "pane")
                            .ok()
                            .cloned()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for i in closed {
            self.closed_by(&i, "pane", "in_pane", "in_pane");
        }
    }

    /// Waits for a decision on `id`, watching the hook connection for EOF.
    /// `None`: the budget ran out with the item still pending.
    pub(crate) fn wait_decision(&self, id: u64, budget: Duration, conn: Option<&UnixStream>) -> Option<Waited> {
        let end = Instant::now() + budget;
        loop {
            let Some(status) = self.store.read(|s| s.inbox.iter().find(|i| i.id == id).map(|i| i.status)) else {
                return Some(Waited::Expired);
            };
            match status {
                InboxStatus::Pending => {}
                InboxStatus::Decided | InboxStatus::Sent | InboxStatus::Effect | InboxStatus::Failed => {
                    let item = self.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned());
                    return Some(item.map_or(Waited::Expired, |i| Waited::Decided(Box::new(i))));
                }
                InboxStatus::Dismissed => return Some(Waited::InPane),
                InboxStatus::Interrupted => return Some(Waited::Interrupted),
                InboxStatus::Parked => return Some(Waited::Parked),
                InboxStatus::Expired | InboxStatus::Stale => return Some(Waited::Expired),
            }
            if Instant::now() >= end {
                return None;
            }
            match conn {
                Some(c) => {
                    if conn_closed(c) {
                        return Some(Waited::Interrupted);
                    }
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// How long a prompt is held for the Inbox: `permission_hold` (or the
    /// `permissions.hold_secs` setting), always 2 s inside orch-hook's deadline.
    pub(crate) fn hold_budget(&self, deadline_ms: u64) -> Duration {
        let hold = self
            .db
            .setting(HOLD_SECS_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_u64())
            .map(Duration::from_secs)
            .unwrap_or(self.cfg.permission_hold);
        hold.min(Duration::from_millis(deadline_ms.saturating_sub(2_000)))
    }

    /// The user approved this exact call after it parked: allowed once
    /// (FR-705), before any rule, and audited as theirs.
    pub(crate) fn one_shot(&self, rec: &AgentRecord, call: &Call) -> bool {
        let Ok(Some(item)) = self.db.take_one_shot(&rec.id, &call.tool, &call.input) else { return false };
        let mut detail = call.detail(item, None);
        detail["one_shot"] = json!(true);
        let _ = self.db.audit("user", Some(&rec.id), "permission", Some(&call.tool), "allow", detail);
        true
    }

    /// PermissionRequest (FR-705): a one-shot allow or a rule decides at
    /// once; otherwise the request becomes an Inbox item and the hook is held
    /// (within orch-hook's deadline) until the user decides. No decision in
    /// time → denied and parked: the agent stops and resumes on the answer.
    pub(crate) fn permission_request(
        &self,
        rec: &AgentRecord,
        p: &HookPayload,
        deadline_ms: u64,
        conn: Option<&UnixStream>,
    ) -> Option<HookOutput> {
        let call = Call::from_hook(p);
        if self.one_shot(rec, &call) {
            return Some(HookOutput::permission_request(PermissionBehavior::Allow, None));
        }
        let subject = self.subject(rec);
        let rules = self.db.rules().unwrap_or_default();
        let mut ask_rule = None;
        if let Some(rule) = permissions::evaluate(&rules, &subject, &call.tool, &call.input) {
            let actor = format!("rule:{}", rule.id);
            let detail = call.detail(None, Some(rule));
            match rule.effect {
                RuleEffect::Allow => {
                    let _ = self.db.audit(&actor, Some(&rec.id), "permission", Some(&call.tool), "allow", detail);
                    return Some(HookOutput::permission_request(PermissionBehavior::Allow, None));
                }
                RuleEffect::Deny => {
                    let _ = self.db.audit(&actor, Some(&rec.id), "permission", Some(&call.tool), "deny", detail);
                    return Some(HookOutput::permission_request(
                        PermissionBehavior::Deny,
                        Some(&format!("ORCH: denied by the user's rule {} ({}).", rule.id, rule.pattern)),
                    ));
                }
                // Its answer is audited by whoever gives it.
                RuleEffect::Ask => ask_rule = Some(rule.id),
            }
        }
        self.permission_request_with(rec, &call, &subject, ask_rule, self.hold_budget(deadline_ms), conn)
    }

    /// The Inbox part of [`Runtime::permission_request`], after the rules.
    pub(crate) fn permission_request_with(
        &self,
        rec: &AgentRecord,
        call: &Call,
        subject: &Subject,
        ask_rule: Option<u64>,
        budget: Duration,
        conn: Option<&UnixStream>,
    ) -> Option<HookOutput> {
        if budget.is_zero() {
            return None;
        }
        let out = match self.hold_for_decision(rec, call, subject, ask_rule, budget, conn) {
            Waited::Decided(i) if allows(&i) => HookOutput::permission_request(PermissionBehavior::Allow, None),
            Waited::Decided(i) => {
                let m = reply(&i).map(|m| format!("ORCH: the user says: {m}"));
                let m = m.unwrap_or_else(|| "ORCH: the user denied this in the Inbox.".into());
                HookOutput::permission_request(PermissionBehavior::Deny, Some(&m))
            }
            Waited::Parked => HookOutput::permission_request(PermissionBehavior::Deny, Some(PARKED_MESSAGE)),
            // The CLI's own dialog has it (or had it) in the pane.
            Waited::InPane | Waited::Interrupted | Waited::Expired => return None,
        };
        // Answered outside the pane: the agent works again until its next hook.
        if let Err(e) = self.permission_answered(&rec.id) {
            self.event(&rec.id, "permission_state", json!({"error": format!("{e:#}")}));
        }
        Some(out)
    }

    /// Holds a tool call for an Inbox decision: a `permission` item, a gate
    /// hold row (for the reconciler), the wait (EOF on `conn` = Esc in the
    /// pane), and the item parked if nobody decided in time. Shared by
    /// PermissionRequest hooks and orch-mcp calls under an `ask` rule.
    /// `ask_rule`: the rule that sent it here (then "always allow" is not
    /// offered: an allow rule would never win over it).
    pub(crate) fn hold_for_decision(
        &self,
        rec: &AgentRecord,
        call: &Call,
        subject: &Subject,
        ask_rule: Option<u64>,
        budget: Duration,
        conn: Option<&UnixStream>,
    ) -> Waited {
        let (tool, input) = (call.tool.as_str(), &call.input);
        let always = if ask_rule.is_some() { None } else { permissions::always_pattern(tool, input) };
        let (summary, description) = brief(tool, input);
        let line = permissions::summary(tool, input);
        let item = match self.add_item(
            Some(&rec.id),
            "permission",
            format!("{}: {line}", rec.id),
            json!({
                "tool": tool,
                "tool_input": input,
                "cwd": call.cwd.clone().unwrap_or_else(|| rec.worktree.display().to_string()),
                "tool_use_id": call.tool_use_id,
                "summary": summary,
                "description": description,
                "ask_rule": ask_rule,
                "always": always,
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
                tool_use_id: call.tool_use_id.clone(),
                kind: HoldKind::Gate,
                status: HoldStatus::Held,
                summary: line,
                created_at_ms: now_ms(),
                inbox_id: Some(item),
            })
        });
        let waited = match self.wait_decision(item, budget, conn) {
            Some(w) => w,
            None => self.park(item),
        };
        if waited == Waited::Interrupted {
            // Esc in the pane: the user answered there (D6).
            if let Some(i) = self.close_item(item, InboxStatus::Interrupted, "interrupted by the user", "pane") {
                self.closed_by(&i, "pane", "interrupted", "in_pane");
            }
        }
        let hold_status = match &waited {
            Waited::Decided(i) if allows(i) => HoldStatus::Allowed,
            Waited::Decided(_) => HoldStatus::Denied,
            Waited::Parked => HoldStatus::Parked,
            Waited::InPane | Waited::Interrupted | Waited::Expired => HoldStatus::Interrupted,
        };
        let _ = self.store.write(|s| {
            if let Some(h) = s.holds.iter_mut().find(|h| h.id == hold_id) {
                h.status = hold_status;
            }
        });
        self.event(&rec.id, "permission", json!({"item": item, "tool": tool, "outcome": hold_status}));
        waited
    }

    /// The hold reached its deadline: the item parks (still open, payload
    /// intact) unless a decision landed at that very moment.
    fn park(&self, id: u64) -> Waited {
        if !self.store.write(|s| s.park_inbox(id)).unwrap_or(false) {
            return self.wait_decision(id, Duration::ZERO, None).unwrap_or(Waited::Expired);
        }
        if let Some(i) = self.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned()) {
            self.closed_by(&i, "expired", "parked", "parked");
        }
        Waited::Parked
    }

    /// Items the reconciler parked at boot: audited and reported as the core's.
    pub(crate) fn parked_at_restart(&self, items: &[u64]) {
        for id in items {
            if let Some(i) = self.store.read(|s| s.inbox.iter().find(|i| i.id == *id).cloned()) {
                self.closed_by(&i, "core", "parked", "parked");
            }
        }
    }

    /// An agent whose turn ended on a parked decision frees its slot (FR-705,
    /// D3): suspended once idle, unless it has a reserved slot (a Lead) or
    /// other work (background tasks, a queued or pasted turn).
    pub(crate) fn suspend_if_parked(&self, id: &str) {
        if !self.store.read(|s| has_parked(s, id)) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        let id = id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(PARK_SUSPEND_DELAY);
            if !me.parked_and_idle(&id) {
                return;
            }
            match me.suspend(&id) {
                Ok(_) => me.event(&id, "parked_suspended", json!({})),
                Err(e) => me.event(&id, "parked_suspended", json!({"error": format!("{e:#}")})),
            }
        });
    }

    fn parked_and_idle(&self, id: &str) -> bool {
        self.live(id).is_some()
            && self.store.read(|s| {
                let Some(a) = s.agents.get(id) else { return false };
                !reserved_slot(a)
                    && a.tracker.state == AgentState::Idle
                    && a.tracker.background_tasks == 0
                    && has_parked(s, id)
                    && !s
                        .turns
                        .iter()
                        .any(|t| t.agent_id == id && matches!(t.status, TurnStatus::Queued | TurnStatus::Delivering))
            })
    }

    /// The agent waits on a parked decision: the idle/queue logic does not
    /// resume it for other work (FR-705).
    pub(crate) fn is_parked(&self, id: &str) -> bool {
        self.store.read(|s| has_parked(s, id))
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
        let rule = self.db.rules().unwrap_or_default().into_iter().find(|r| r.id == id);
        let done = self.db.revoke_rule(id)?;
        self.db.audit(
            "user",
            None,
            "rule_revoked",
            Some(&id.to_string()),
            if done { "ok" } else { "unknown" },
            json!({"rule": id, "pattern": rule.as_ref().map(|r| &r.pattern),
                   "effect": rule.as_ref().map(|r| r.effect.as_str())}),
        )?;
        Ok(done)
    }

    /// How a turn the core queued for `agent` reaches it.
    pub fn delivered_effect(&self, agent: &str) -> &'static str {
        if self.live(agent).is_some() { "delivered" } else { "resumed" }
    }

    /// Tells `agent` the user's decision as a user turn (`result.turn`); the effect.
    fn tell(&self, agent: &str, text: &str, result: &mut Value) -> Result<&'static str> {
        let effect = self.delivered_effect(agent);
        result["turn"] = json!(self.deliver_user(agent, text)?);
        Ok(effect)
    }

    /// Reopens a parked item whose decision could not reach its agent.
    fn unclaim_parked(&self, id: u64) {
        let _ = self.store.write(|s| {
            if let Some(i) = s.inbox.iter_mut().find(|i| i.id == id) {
                i.status = InboxStatus::Parked;
                (i.decided_at_ms, i.decision, i.decided_by) = (None, None, None);
                if let Some(p) = i.payload.as_object_mut() {
                    p.remove("reply");
                }
            }
        });
    }

    /// The user's decision on an Inbox item. The item is claimed atomically
    /// (a second decision is refused), then logged (FR-709) before anything
    /// acts on it. The result's `effect` says what happened.
    pub fn decide(
        &self,
        id: u64,
        decision: InboxDecision,
        scope: Option<&str>,
        message: Option<&str>,
    ) -> Result<Value> {
        let mut item = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == id).cloned())
            .ok_or_else(|| anyhow!("no Inbox item {id}"))?;
        if !item.open() {
            // A failed or stale decision leaves the Inbox only this way.
            if decision == InboxDecision::Dismiss && item.ended() {
                return self.dismiss_ended(&item);
            }
            bail!("Inbox item {id} is already {:?}", item.status);
        }
        // Checked before claiming: an always-allow that can't become a rule is refused whole.
        let rule_spec = if decision == InboxDecision::AlwaysAllow {
            let always = item.payload.get("always").or_else(|| item.payload.get("always_pattern"));
            let Some(pattern) = always.and_then(Value::as_str).map(str::to_string) else {
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
            let was = s.inbox.iter().find(|i| i.id == id).map(|i| i.status);
            let closed = s.close_inbox(id, status, Some(word.clone()), "user").map(|i| {
                if let Some(m) = message {
                    i.payload["reply"] = json!(m);
                }
                was
            });
            // In the same write as the close: between the two, the stale
            // wait would read as a pane prompt of its own.
            if closed.is_ok() && item.kind == "permission" {
                if let Some(a) = item.agent_id.as_deref().and_then(|a| s.agents.get_mut(a)) {
                    if a.tracker.state == AgentState::WaitingForYou {
                        a.tracker.state = AgentState::Working;
                    }
                }
            }
            closed
        })?;
        match claimed {
            Err(now) => bail!("Inbox item {id} is already {now:?}"),
            // The hold may have parked it since it was read: act on the status it was claimed in.
            Ok(Some(was)) => item.status = was,
            Ok(None) => {}
        }
        let target = item.payload.get("tool").and_then(Value::as_str).map(str::to_string);
        self.db.audit(
            "user",
            item.agent_id.as_deref(),
            &item.kind,
            target.as_deref(),
            &word,
            json!({"item": id, "body": item.body, "message": message, "scope": scope, "tool": target,
                   "tool_input": (item.kind == "permission").then(|| tool_input(&item)),
                   "tool_use_id": item.payload.get("tool_use_id")}),
        )?;
        let mut result = json!({"item": id, "decision": word, "decided_by": "user"});
        if let Some((sc, sid, pattern)) = rule_spec {
            result["rule"] = json!(self.add_rule(sc, &sid, &pattern, RuleEffect::Allow, Some(id))?);
        }
        let allowed = matches!(decision, InboxDecision::Approve | InboxDecision::AlwaysAllow);
        let why = message.filter(|m| !m.trim().is_empty()).unwrap_or("The user said no.");
        let mut effect = match decision {
            InboxDecision::Dismiss => "dismissed",
            _ if allowed => "allowed",
            InboxDecision::Reply if item.kind != "permission" => "delivered",
            _ => "denied",
        };
        // Kind-specific follow-ups that don't wait on a held hook. `sent`: the
        // answer went to the agent (a turn) or the push started; its effect follows.
        let mut sent = false;
        let action = item.payload.get("action").and_then(Value::as_str).unwrap_or("");
        let agent = item.agent_id.as_deref();
        match (item.kind.as_str(), action, decision) {
            // A parked permission (FR-705): the hook is gone; the agent is told.
            ("permission", _, _) if item.status == InboxStatus::Parked => {
                if let Some(a) = agent {
                    let tool = target.as_deref().unwrap_or("?");
                    let summary = item.payload.get("summary").and_then(Value::as_str).unwrap_or("");
                    let told = if allowed {
                        let until = now_ms() + ONE_SHOT_TTL_MS;
                        self.db.add_one_shot(a, tool, &tool_input(&item), Some(id), until).and_then(|_| {
                            self.tell(a, &format!("ORCH: approved: retry the call {tool} {summary}"), &mut result)
                        })
                    } else {
                        self.tell(a, &format!("ORCH: denied: {tool} {summary}. {why}"), &mut result)
                    };
                    match told {
                        Ok(e) => {
                            effect = e;
                            sent = true;
                        }
                        // Nobody was told: the agent still waits, so the item does too.
                        Err(e) => {
                            self.unclaim_parked(id);
                            let err = format!("{e:#}");
                            let _ = self.db.audit(
                                "user",
                                Some(a),
                                &item.kind,
                                Some(tool),
                                "failed",
                                json!({"item": id, "tool": tool, "tool_input": tool_input(&item),
                                       "tool_use_id": item.payload.get("tool_use_id"), "error": err}),
                            );
                            self.outcome_with(&item, "user", "failed", Some(&err));
                            return Err(e);
                        }
                    }
                }
            }
            ("gate", "restart_gate", _) => {
                if let Some(a) = agent {
                    let text =
                        if allowed { "ORCH: approved: proceed".to_string() } else { format!("ORCH: rejected: {why}") };
                    effect = self.tell(a, &text, &mut result)?;
                    sent = true;
                }
            }
            ("gate", "push_pr", InboxDecision::Approve) => {
                // The user approves the commits they saw: a branch that moved
                // since is not pushed (the agent asks again).
                if let Some(why) = self.pr_branch_moved(&item.payload) {
                    self.advance(id, Stage::Stale("the branch moved"));
                    if let Some(a) = agent {
                        let _ = self.deliver_user(
                            a,
                            "ORCH: PR gate stale: the branch moved since you asked; call request_pr again.",
                        );
                    }
                    result["effect"] = json!("stale");
                    result["stale_why"] = json!(why);
                    result["lifecycle"] = self.lifecycle(id);
                    self.outcome(&item, "user", "stale");
                    return Ok(result);
                }
                // Sent before the push starts: `open_pr` may end (pr_opened or
                // failed) before this call returns, and must not be overwritten.
                self.advance(id, Stage::Sent);
                if let Some(me) = self.me.upgrade() {
                    let payload = item.payload.clone();
                    std::thread::spawn(move || me.open_pr(id, &payload));
                }
                result["pr"] = json!("pushing and opening the PR");
                result["effect"] = json!(effect);
                result["lifecycle"] = self.lifecycle(id);
                self.outcome(&item, "user", effect);
                return Ok(result);
            }
            ("gate", "push_pr_update", InboxDecision::Approve) => {
                if let Some(me) = self.me.upgrade() {
                    let payload = item.payload.clone();
                    std::thread::spawn(move || me.push_pr_update(id, &payload));
                }
                result["pr"] = json!("pushing to the PR");
            }
            ("gate", "push_pr_update", InboxDecision::Reject | InboxDecision::Reply) => {
                // The PR stays open as it is: the run does not move.
                if let Some(a) = agent {
                    let n = item.payload.get("pr_number").and_then(Value::as_u64).unwrap_or(0);
                    let why = message.map(|m| format!(" They said: {m}")).unwrap_or_default();
                    let _ = self.deliver_user(a, &format!("ORCH: the user declined pushing to PR #{n}.{why}"));
                    effect = self.delivered_effect(a);
                }
            }
            ("gate", "spec_approval", InboxDecision::Approve | InboxDecision::AlwaysAllow) => {
                self.spec_decided(&item.payload, agent, true, None);
                sent = agent.is_some();
            }
            ("gate", "spec_approval", InboxDecision::Reject | InboxDecision::Reply) => {
                self.spec_decided(&item.payload, agent, false, message);
                sent = agent.is_some();
            }
            ("gate", "push_pr", InboxDecision::Reject | InboxDecision::Reply) => {
                if let Some(a) = agent {
                    self.pr_declined(a);
                    let why = message.map(|m| format!(" They said: {m}")).unwrap_or_default();
                    let _ = self.deliver_user(a, &format!("ORCH: the user declined your pull request request.{why}"));
                    effect = self.delivered_effect(a);
                    sent = true;
                }
            }
            // The agent was told the answer comes as a turn: say there is none.
            ("question", "ask_user", InboxDecision::Dismiss) => {
                if let Some(a) = agent {
                    let _ = self.deliver_user(
                        a,
                        "ORCH: the user dismissed your question without answering. Decide yourself, or ask your team.",
                    );
                }
            }
            ("question", _, InboxDecision::Reply) => {
                if let (Some(a), Some(m)) = (agent, message) {
                    let q = item.payload.get("question").and_then(Value::as_str).unwrap_or(&item.body);
                    let text = format!(
                        "ORCH: the user answered your question.\nQuestion: {}\nAnswer: {m}",
                        crate::team::cut(q, 600)
                    );
                    effect = self.tell(a, &text, &mut result)?;
                    sent = true;
                }
            }
            (k, _, InboxDecision::Reply) if k != "permission" => {
                if let (Some(a), Some(m)) = (agent, message) {
                    effect = self.tell(a, m, &mut result)?;
                    sent = true;
                }
            }
            _ => {}
        }
        // The lifecycle (spec §3.3): a budget item applies or skips, a
        // dismissal is its own effect, an answer sent to the agent waits for
        // its turn ([`Runtime::answers_resumed`]), a held hook's answer is done.
        let stage = if item.kind == "budget" {
            effect = if allowed { "applied" } else { "skipped" };
            Stage::Effect(effect, None)
        } else if decision == InboxDecision::Dismiss {
            Stage::Effect("dismissed", None)
        } else if sent {
            Stage::Sent
        } else {
            Stage::Effect(effect, None)
        };
        self.advance(id, stage);
        result["effect"] = json!(effect);
        result["lifecycle"] = self.lifecycle(id);
        self.outcome(&item, "user", effect);
        Ok(result)
    }

    /// Dismisses a failed or stale decision (the user's, audited): the
    /// lifecycle ends `dismissed`.
    fn dismiss_ended(&self, item: &InboxItem) -> Result<Value> {
        let id = item.id;
        let now = now_ms();
        let claimed = self.store.write(|s| {
            let Some(i) = s.inbox.iter_mut().find(|i| i.id == id && i.ended()) else { return false };
            i.status = InboxStatus::Dismissed;
            (i.decision, i.decided_by, i.decided_at_ms) = (Some("dismiss".into()), Some("user".into()), Some(now));
            true
        })?;
        if !claimed {
            bail!("Inbox item {id} is no longer failed or stale");
        }
        self.advance(id, Stage::Effect("dismissed", None));
        self.closed_by(item, "user", "dismiss", "dismissed");
        Ok(
            json!({"item": id, "decision": "dismiss", "decided_by": "user", "effect": "dismissed", "lifecycle": self.lifecycle(id)}),
        )
    }

    /// `request_pr` from orch-mcp: an Inbox gate (the agent can't push).
    pub(crate) fn request_pr(&self, rec: &AgentRecord, args: &Value) -> Result<String> {
        if self.store.read(|s| {
            s.inbox.iter().any(|i| {
                i.pending()
                    && matches!(i.payload.get("action").and_then(Value::as_str), Some("push_pr" | "push_pr_update"))
                    && i.agent_id.as_deref() == Some(&rec.id)
            })
        }) {
            bail!("a pull request request is already waiting for the user");
        }
        // The PR is open: push the new commits to it rather than open another.
        if let Some(run) = self.open_pr_run(rec) {
            return self.request_pr_update(rec, &run);
        }
        let title = args.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        let title = title.ok_or_else(|| anyhow!("title is required"))?.chars().take(256).collect::<String>();
        let body = args.get("body").and_then(Value::as_str).unwrap_or("").chars().take(20_000).collect::<String>();
        let draft = args.get("draft").and_then(Value::as_bool).unwrap_or(false);
        let row = self.store.read(|s| s.worktrees.get(&rec.worktree).cloned());
        let branch = orch_git::git(&rec.worktree, &["symbolic-ref", "--short", "HEAD"])?.trim().to_string();
        let base = row.map(|r| r.base).unwrap_or_else(|| "main".into());
        let head = orch_git::git(&rec.worktree, &["rev-parse", "HEAD"])?.trim().to_string();
        let id = self.add_item(
            Some(&rec.id),
            "gate",
            format!("{}: open a PR \"{title}\" ({branch} → {base})", rec.id),
            json!({"action": "push_pr", "branch": branch, "base": base, "title": title, "body": body, "draft": draft,
                   "worktree": rec.worktree, "head": head, "diff": diff_summary(&rec.worktree, &base, &branch)}),
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

    /// `decision_patch`: the PR gate's patch (`base...branch`), whole or one file.
    pub fn decision_patch(&self, id: u64, path: Option<&str>) -> Result<String> {
        let p = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == id && i.kind == "gate").map(|i| i.payload.clone()))
            .ok_or_else(|| anyhow!("no gate {id}"))?;
        if p.get("action").and_then(Value::as_str) != Some("push_pr") {
            bail!("gate {id} has no patch");
        }
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let range = format!("{}...{}", s("base"), s("branch"));
        let mut args = vec!["diff", range.as_str()];
        if let Some(f) = path {
            args.extend(["--", f]);
        }
        let mut patch = orch_git::git(std::path::Path::new(&s("worktree")), &args)?;
        if patch.len() > PATCH_MAX {
            let cut = (0..=PATCH_MAX).rev().find(|i| patch.is_char_boundary(*i)).unwrap_or(0);
            patch.truncate(cut);
            patch.push_str("\n… (patch truncated)\n");
        }
        Ok(patch)
    }

    /// The PR gate's branch is no longer at the commit the user was shown.
    fn pr_branch_moved(&self, p: &Value) -> Option<String> {
        let head = p.get("head").and_then(Value::as_str)?;
        let wt = std::path::PathBuf::from(p.get("worktree").and_then(Value::as_str)?);
        let branch = p.get("branch").and_then(Value::as_str)?;
        let now = orch_git::git(&wt, &["rev-parse", branch]).ok()?.trim().to_string();
        (now != head).then(|| format!("{branch} is at {} now, not {}", short(&now), short(head)))
    }

    /// After the PR gate is approved: push with the user's credentials, open
    /// the PR, tell the agent (ADR-009: only the app pushes).
    pub(crate) fn open_pr(&self, item: u64, p: &Value) {
        let agent = self.store.read(|s| s.inbox.iter().find(|i| i.id == item).and_then(|i| i.agent_id.clone()));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let wt = std::path::PathBuf::from(s("worktree"));
        let (branch, base) = (s("branch"), s("base"));
        let draft = p["draft"].as_bool().unwrap_or(false);
        let r = match &self.cfg.open_pr {
            Some(f) => (f.0)(&wt, &base, &branch, &s("title"), &s("body"), draft),
            None => orch_git::gh::push(&wt, "origin", &branch)
                .and_then(|_| self.gh_create_pr(&wt, &base, &branch, &s("title"), &s("body"), draft)),
        };
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
        match &r {
            Ok(url) => self.advance(item, Stage::Effect("pr_opened", Some(url.clone()))),
            Err(e) => self.advance(item, Stage::Failed(format!("{e:#}"))),
        }
        if let Some(a) = agent {
            // The digest's pr_opened outcome, on the run's timeline when it has one.
            if let Ok(url) = &r {
                match self.store.read(|s| s.agents.get(&a).and_then(|r| r.team.as_ref().map(|t| t.run_id.clone()))) {
                    Some(run) => self.run_event(&run, Some(&a), "run.pr_opened", json!({"url": url})),
                    None => self.event(&a, "run.pr_opened", json!({"url": url})),
                }
            }
            let _ = self.deliver_user(&a, &msg);
            self.pr_opened(&a, r.as_ref().ok().map(String::as_str), orch_git::gh::rev_sha(&wt, &branch));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::{hook, test_runtime};
    use std::sync::Arc;

    fn perm(cmd: &str) -> orch_protocol::HookMsg {
        hook(
            "PermissionRequest",
            json!({"tool_name": "Bash", "tool_input": {"command": cmd, "description": "Do it"}, "tool_use_id": "tu1",
                   "cwd": "/w/front-1"}),
        )
    }

    fn behavior(o: &Option<HookOutput>) -> Option<String> {
        o.as_ref().map(|o| o.0["hookSpecificOutput"]["decision"]["behavior"].as_str().unwrap().to_string())
    }

    fn message(o: &Option<HookOutput>) -> String {
        o.as_ref().map(|o| o.0["hookSpecificOutput"]["decision"]["message"].as_str().unwrap_or("").to_string()).unwrap()
    }

    fn actors(rt: &Runtime) -> Vec<String> {
        rt.db.audit_log(Some("a1"), 50).unwrap().into_iter().rev().map(|e| e.actor).collect()
    }

    fn item(rt: &Runtime, id: u64) -> InboxItem {
        rt.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned().unwrap())
    }

    fn outcomes(rt: &Runtime) -> Vec<Value> {
        rt.db.write(|_| Ok(())).unwrap(); // the events' detached writes land first
        rt.db.events(0, Some("a1"), Some("inbox_outcome"), 50).unwrap().into_iter().rev().map(|e| e.detail).collect()
    }

    /// Decides the next pending (held) item from another thread, like the UI would.
    fn decide_soon(
        rt: &Arc<Runtime>,
        d: InboxDecision,
        scope: Option<&'static str>,
        msg: Option<&'static str>,
    ) -> std::thread::JoinHandle<u64> {
        let rt = rt.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(i) = rt.inbox().into_iter().find(InboxItem::pending) {
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
        assert!(message(&o).contains("ORCH: denied by the user's rule"));
        assert!(rt.inbox().is_empty());
        let log = rt.db.audit_log(Some("a1"), 10).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.iter().all(|e| e.actor.starts_with("rule:")));
        assert_eq!(log[0].detail["rule"]["pattern"], "Bash(rm:*)");
        assert_eq!(log[0].detail["rule"]["effect"], "deny");
        assert_eq!(log[0].detail["tool_input"]["command"], "rm -rf build");
        assert_eq!(log[0].detail["tool_use_id"], "tu1");
    }

    #[test]
    fn the_hold_is_the_setting_within_the_hook_deadline() {
        let (_d, rt) = test_runtime(true);
        let default = crate::runtime::RuntimeConfig::new(crate::paths::AppPaths::new(std::path::PathBuf::from("/x")));
        assert_eq!(default.permission_hold, Duration::from_secs(300));
        assert_eq!(rt.hold_budget(540_000), Duration::from_millis(300));
        assert_eq!(rt.hold_budget(1_500), Duration::ZERO);
        rt.db.set_setting(HOLD_SECS_KEY, &json!(5)).unwrap();
        assert_eq!(rt.hold_budget(540_000), Duration::from_secs(5));
        assert_eq!(rt.hold_budget(4_000), Duration::from_secs(2), "always 2 s inside the deadline");
    }

    #[test]
    fn inbox_approve_always_reply_while_held() {
        let (_d, rt) = test_runtime(true);
        // Approve from "the UI" while the hook is held.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("make build", Duration::from_secs(5)));
        let id = decide_soon(&rt, InboxDecision::Approve, None, None).join().unwrap();
        assert_eq!(behavior(&h.join().unwrap()).as_deref(), Some("allow"));
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().status), HoldStatus::Allowed);
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().inbox_id), Some(id));
        assert_eq!(item(&rt, id).decided_by.as_deref(), Some("user"));

        // Reply = deny with the user's words.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("npm publish", Duration::from_secs(5)));
        decide_soon(&rt, InboxDecision::Reply, None, Some("not on main")).join().unwrap();
        let o = h.join().unwrap();
        assert_eq!(behavior(&o).as_deref(), Some("deny"));
        assert!(message(&o).contains("not on main"));

        // Always allow → an agent-scoped rule; the next identical call never reaches the Inbox.
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || rt2.permission_request_for_test("cargo fmt", Duration::from_secs(5)));
        decide_soon(&rt, InboxDecision::AlwaysAllow, Some("agent"), None).join().unwrap();
        assert_eq!(behavior(&h.join().unwrap()).as_deref(), Some("allow"));
        let rules = rt.db.rules().unwrap();
        assert_eq!((rules[0].pattern.as_str(), rules[0].scope), ("Bash(cargo fmt)", RuleScope::Agent));
        assert_eq!(behavior(&rt.on_hook(&perm("cargo fmt"), None)).as_deref(), Some("allow"));
        assert!(rt.inbox().is_empty());
        // Every decision is in the audit log (FR-709), and has an outcome.
        let actions: Vec<String> = rt.db.audit_log(None, 50).unwrap().into_iter().map(|e| e.action).collect();
        assert!(
            actions.contains(&"rule_added".to_string()) && actions.iter().filter(|a| *a == "permission").count() >= 3
        );
        let effects: Vec<Value> = outcomes(&rt).into_iter().map(|o| o["effect"].clone()).collect();
        assert_eq!(effects, [json!("allowed"), json!("denied"), json!("allowed")]);
        // A decided item can't be decided again.
        assert!(rt.decide(id, InboxDecision::Reject, None, None).is_err());
    }

    #[test]
    fn an_unanswered_prompt_parks_and_allow_retries_once() {
        let (_d, rt) = test_runtime(true);
        let o = rt.on_hook(&perm("pnpm add zod"), None);
        assert_eq!(behavior(&o).as_deref(), Some("deny"));
        assert_eq!(message(&o), PARKED_MESSAGE);
        let parked = rt.inbox().into_iter().next().expect("still in the Inbox");
        assert_eq!(parked.status, InboxStatus::Parked);
        let p = &parked.payload;
        assert_eq!((p["tool"].as_str(), p["summary"].as_str()), (Some("Bash"), Some("pnpm add zod")));
        assert_eq!(p["tool_input"], json!({"command": "pnpm add zod", "description": "Do it"}));
        assert_eq!((p["cwd"].as_str(), p["tool_use_id"].as_str()), (Some("/w/front-1"), Some("tu1")));
        assert_eq!((p["description"].as_str(), p["always"].as_str()), (Some("Do it"), Some("Bash(pnpm add zod)")));
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().status), HoldStatus::Parked);
        assert!(rt.is_parked("a1"));
        // The turn's end does not close a parked item.
        rt.on_hook(&hook("Stop", json!({})), None);
        assert_eq!(item(&rt, parked.id).status, InboxStatus::Parked);

        let r = rt.decide(parked.id, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(r["effect"], "resumed", "not live: the turn resumes it");
        assert!(!rt.is_parked("a1"));
        let body = rt.store.read(|s| s.turns.last().unwrap().body.clone());
        assert_eq!(body, "ORCH: approved: retry the call Bash pnpm add zod");
        assert_eq!(rt.store.read(|s| s.turns.last().unwrap().kind), governor::TurnKind::User);
        // The retry runs without a second prompt; only that exact call, once.
        assert_eq!(behavior(&rt.on_hook(&perm("pnpm add zod"), None)).as_deref(), Some("allow"));
        assert!(rt.inbox().is_empty());
        assert_eq!(actors(&rt), ["expired", "user", "user"]);
        let last = &rt.db.audit_log(Some("a1"), 1).unwrap()[0];
        assert_eq!((last.detail["one_shot"].as_bool(), last.detail["item"].as_u64()), (Some(true), Some(parked.id)));
        assert_eq!(item(&rt, parked.id).decided_by.as_deref(), Some("user"));
        let o: Vec<(Value, Value)> =
            outcomes(&rt).into_iter().map(|o| (o["decided_by"].clone(), o["effect"].clone())).collect();
        assert_eq!(o, [(json!("expired"), json!("parked")), (json!("user"), json!("resumed"))]);
        assert_eq!(outcomes(&rt)[0]["tool"], "Bash");
        assert_eq!(outcomes(&rt)[0]["created_at_ms"], json!(parked.created_at_ms));
        assert_eq!(
            behavior(&rt.on_hook(&perm("pnpm add zod"), None)).as_deref(),
            Some("deny"),
            "consumed: parks again"
        );
    }

    #[test]
    fn a_parked_deny_or_always_tells_the_agent() {
        let (_d, rt) = test_runtime(true);
        rt.on_hook(&perm("ls"), None);
        let id = rt.inbox()[0].id;
        let r = rt.decide(id, InboxDecision::Reject, None, Some("not now")).unwrap();
        assert_eq!(r["effect"], "resumed");
        assert_eq!(rt.store.read(|s| s.turns.last().unwrap().body.clone()), "ORCH: denied: Bash ls. not now");
        rt.on_hook(&perm("pwd"), None);
        let id = rt.inbox()[0].id;
        rt.decide(id, InboxDecision::Dismiss, None, None).unwrap();
        assert_eq!(
            rt.store.read(|s| s.turns.last().unwrap().body.clone()),
            "ORCH: denied: Bash pwd. The user said no."
        );
        rt.on_hook(&perm("make"), None);
        let id = rt.inbox()[0].id;
        rt.decide(id, InboxDecision::AlwaysAllow, Some("agent"), None).unwrap();
        assert_eq!(rt.db.rules().unwrap()[0].pattern, "Bash(make)");
        assert_eq!(rt.store.read(|s| s.turns.last().unwrap().body.clone()), "ORCH: approved: retry the call Bash make");
    }

    #[test]
    fn a_parked_decision_that_cannot_reach_its_agent_stays_parked() {
        let (_d, rt) = test_runtime(true);
        let id = rt
            .store
            .write(|s| {
                let payload = json!({"tool": "Bash", "tool_input": {"command": "ls"}, "summary": "ls"});
                let id =
                    s.add_inbox_item(Some("gone"), "permission", "gone: Bash: ls".into(), payload, PRIORITY_BLOCKING);
                assert!(s.park_inbox(id));
                id
            })
            .unwrap();
        let e = rt.decide(id, InboxDecision::Approve, None, Some("ok")).unwrap_err();
        assert!(e.to_string().contains("no agent gone"), "{e}");
        let i = item(&rt, id);
        assert_eq!((i.status, i.decided_by.as_deref(), i.decision.as_deref()), (InboxStatus::Parked, None, None));
        assert!(i.payload.get("reply").is_none());
        rt.db.write(|_| Ok(())).unwrap();
        let ev = rt.db.events(0, None, Some("inbox_outcome"), 5).unwrap();
        assert_eq!(
            (ev[0].detail["effect"].as_str(), ev[0].detail["decided_by"].as_str()),
            (Some("failed"), Some("user"))
        );
        // Still decidable once the agent can be told.
        rt.store.write(|s| s.inbox.iter_mut().find(|i| i.id == id).unwrap().agent_id = Some("a1".into())).unwrap();
        assert_eq!(rt.decide(id, InboxDecision::Reject, None, None).unwrap()["effect"], "resumed");
    }

    #[test]
    fn a_parked_agent_is_not_resumed_for_other_work() {
        let (_d, rt) = test_runtime(true);
        rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Suspended).unwrap();
        rt.on_hook(&perm("ls"), None);
        rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Suspended).unwrap();
        rt.deliver("a1", "a team message").unwrap();
        rt.pump();
        let t = rt.store.read(|s| s.turns.last().unwrap().clone());
        assert_eq!((t.status, t.admission), (TurnStatus::Queued, crate::store::Admission::Pending), "not admitted");
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
        assert_eq!((item.payload["ask_rule"].as_u64(), item.payload.get("always")), (Some(1), Some(&Value::Null)));
        let e = rt.decide(item.id, InboxDecision::AlwaysAllow, None, None).unwrap_err();
        assert!(e.to_string().contains("always ask"), "{e}");
        let st = rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).unwrap().status);
        assert_ne!(st, InboxStatus::Decided, "refused whole");
        assert!(rt.db.rules().unwrap().iter().all(|r| r.effect != RuleEffect::Allow));
        let _ = h.join();
    }

    #[test]
    fn rows_from_before_p4_keep_working() {
        let (_d, rt) = test_runtime(true);
        let id = rt
            .store
            .write(|s| {
                s.add_inbox_item(
                    Some("a1"),
                    "permission",
                    "a1: Bash: cargo fmt".into(),
                    json!({"tool": "Bash", "input": {"command": "cargo fmt"}, "always_pattern": "Bash(cargo fmt)"}),
                    PRIORITY_BLOCKING,
                )
            })
            .unwrap();
        rt.decide(id, InboxDecision::AlwaysAllow, Some("agent"), None).unwrap();
        assert_eq!(rt.db.rules().unwrap()[0].pattern, "Bash(cargo fmt)");
        assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].detail["tool_input"]["command"], "cargo fmt");
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
        assert_eq!(i.decided_by.as_deref(), Some("pane"));
        assert_eq!(actors(&rt), ["pane"]);
        assert_eq!(outcomes(&rt)[0]["effect"], "in_pane");
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
        let i = rt.store.read(|s| s.inbox.last().cloned().unwrap());
        assert_eq!((i.status, i.decided_by.as_deref()), (InboxStatus::Interrupted, Some("pane")));
        assert_eq!(actors(&rt), ["pane"], "Esc in the pane is the pane's answer (D6)");
    }

    #[test]
    fn rule_changes_are_audited_with_their_pattern() {
        let (_d, rt) = test_runtime(true);
        let r = rt.add_rule(RuleScope::Agent, "a1", "Bash(ls)", RuleEffect::Allow, None).unwrap();
        assert!(rt.revoke_rule(r.id).unwrap());
        let log = rt.db.audit_log(None, 2).unwrap();
        assert_eq!((log[0].action.as_str(), log[0].detail["pattern"].as_str()), ("rule_revoked", Some("Bash(ls)")));
        assert_eq!(log[0].detail["effect"], "allow");
        assert_eq!((log[1].action.as_str(), log[1].detail["pattern"].as_str()), ("rule_added", Some("Bash(ls)")));
    }

    #[test]
    fn a_restart_gate_proceeds_or_hears_the_reason() {
        let (_d, rt) = test_runtime(true);
        let gate = |rt: &Runtime| {
            rt.add_item(
                Some("a1"),
                "gate",
                "a1: gate still pending after a restart: git push".into(),
                json!({"action": "restart_gate", "agent": "a1", "detail": "git push", "restarted": true}),
                PRIORITY_GATE,
            )
            .unwrap()
        };
        let id = gate(&rt);
        assert_eq!(rt.decide(id, InboxDecision::Approve, None, None).unwrap()["effect"], "resumed");
        assert_eq!(rt.store.read(|s| s.turns.last().unwrap().body.clone()), "ORCH: approved: proceed");
        let id = gate(&rt);
        rt.decide(id, InboxDecision::Reject, None, Some("wrong branch")).unwrap();
        assert_eq!(rt.store.read(|s| s.turns.last().unwrap().body.clone()), "ORCH: rejected: wrong branch");
    }

    /// Acceptance 8: a held permission and a held gate when the core restarts
    /// come back with their payloads, and deciding them works.
    #[test]
    fn held_prompts_come_back_after_a_restart_and_work() {
        let (_d, rt) = test_runtime(true);
        let perm = rt
            .store
            .write(|s| {
                let a = s.agents.get_mut("a1").unwrap();
                a.tracker.state = AgentState::WaitingForYou;
                let payload = json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod"}, "summary": "pnpm add zod",
                                     "cwd": "/w", "tool_use_id": "tu9"});
                let perm = s.add_inbox_item(Some("a1"), "permission", "a1: Bash: pnpm add zod".into(), payload, 30);
                for (id, item, summary) in [("h1", Some(perm), "Bash: pnpm add zod"), ("h2", None, "git push")] {
                    s.holds.push(HoldRow {
                        id: id.into(),
                        agent_id: "a1".into(),
                        tool_use_id: None,
                        kind: HoldKind::Gate,
                        status: HoldStatus::Held,
                        summary: summary.into(),
                        created_at_ms: 0,
                        inbox_id: item,
                    });
                }
                perm
            })
            .unwrap();
        let report = rt.recover(true).unwrap();
        assert!(report.holds.iter().any(|(h, now)| h == "h1" && now.contains("parked")), "{:?}", report.holds);
        assert!(report.holds.iter().any(|(h, now)| h == "h2" && now.contains("restart gate")), "{:?}", report.holds);
        let items = rt.inbox();
        let p = items.iter().find(|i| i.id == perm).unwrap();
        assert_eq!(
            (p.status, p.payload["tool_input"]["command"].as_str()),
            (InboxStatus::Parked, Some("pnpm add zod"))
        );
        assert_eq!(p.payload["restarted"], true);
        let g = items.iter().find(|i| i.kind == "gate").unwrap();
        assert_eq!(
            (g.payload["action"].as_str(), g.payload["detail"].as_str()),
            (Some("restart_gate"), Some("git push"))
        );
        assert_eq!(actors(&rt), ["core"], "the reconciler parked it");
        // Both decisions have their normal effect.
        assert_eq!(rt.decide(perm, InboxDecision::Approve, None, None).unwrap()["effect"], "resumed");
        rt.decide(g.id, InboxDecision::Approve, None, None).unwrap();
        let bodies: Vec<String> = rt.store.read(|s| s.turns.iter().map(|t| t.body.clone()).collect());
        assert!(bodies.contains(&"ORCH: approved: retry the call Bash pnpm add zod".to_string()), "{bodies:?}");
        assert!(bodies.contains(&"ORCH: approved: proceed".to_string()), "{bodies:?}");
        assert_eq!(behavior(&rt.on_hook(&perm_input("pnpm add zod"), None)).as_deref(), Some("allow"));
    }

    fn perm_input(cmd: &str) -> orch_protocol::HookMsg {
        hook("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "tu2"}))
    }

    /// A deleted agent leaves no card nobody can act on (and no waiting time).
    #[test]
    fn deleting_an_agent_closes_its_open_items() {
        let (_d, rt) = test_runtime(true);
        rt.on_hook(&perm("ls"), None); // parked
        let q = rt.add_item(Some("a1"), "question", "q".into(), json!({}), 30).unwrap();
        let other = rt.store.write(|s| s.add_inbox(Some("a2"), "alert", "x".into())).unwrap();
        rt.delete_agent("a1", false, false).unwrap();
        let items = rt.store.read(|s| s.inbox.clone());
        for i in items.iter().filter(|i| i.agent_id.as_deref() == Some("a1")) {
            assert_eq!(
                (i.status, i.decision.as_deref(), i.decided_by.as_deref()),
                (InboxStatus::Dismissed, Some("agent deleted"), Some("core")),
                "{i:?}"
            );
        }
        assert!(items.iter().any(|i| i.id == q));
        assert!(items.iter().find(|i| i.id == other).unwrap().pending(), "another agent's item stays");
        assert!(rt.inbox().iter().all(|i| i.agent_id.as_deref() != Some("a1")));
        let log = rt.db.audit_log(Some("a1"), 20).unwrap();
        assert_eq!(log.iter().filter(|e| e.actor == "core" && e.decision == "agent deleted").count(), 2);
        let effects: Vec<Value> =
            outcomes(&rt).into_iter().filter(|o| o["decided_by"] == "core").map(|o| o["effect"].clone()).collect();
        assert_eq!(effects, [json!("dismissed"), json!("dismissed")]);
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
            self.permission_request_with(&rec, &Call::from_hook(&p), &subject, None, hold, None)
        }
    }

    // ------------------------------------------------------------ P4.1 lifecycle

    fn lc(rt: &Runtime, id: u64) -> (String, Option<String>, Option<String>) {
        let v = rt.lifecycle(id);
        let s = |k: &str| v[k].as_str().map(str::to_string);
        (s("state").unwrap_or_default(), s("effect"), s("effect_detail"))
    }

    #[test]
    fn a_held_permission_ends_allowed_or_denied() {
        let (_d, rt) = test_runtime(true);
        let p = rt.add_item(Some("a1"), "permission", "a1: Bash: ls".into(), json!({"tool": "Bash"}), 30).unwrap();
        let r = rt.decide(p, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(lc(&rt, p), ("effect".into(), Some("allowed".into()), None));
        assert_eq!(r["lifecycle"]["effect"], "allowed");
        let q = rt.add_item(Some("a1"), "permission", "a1: Bash: rm".into(), json!({"tool": "Bash"}), 30).unwrap();
        rt.decide(q, InboxDecision::Reject, None, Some("no")).unwrap();
        assert_eq!(lc(&rt, q).1.as_deref(), Some("denied"));
        // The subject and thread are stored with the item.
        let i = item(&rt, p);
        assert_eq!((i.subject["agent"].as_str(), i.thread.as_deref()), (Some("a1"), Some("a1")));
    }

    #[test]
    fn a_parked_permission_is_sent_then_queued_then_resumed() {
        let (_d, rt) = test_runtime(true);
        let payload = json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod"}, "summary": "pnpm add zod"});
        let p = rt.add_item(Some("a1"), "permission", "a1: Bash".into(), payload.clone(), 30).unwrap();
        let other = rt.add_item(Some("a1"), "permission", "a1: Bash 2".into(), payload, 30).unwrap();
        rt.store.write(|s| assert!(s.park_inbox(p) && s.park_inbox(other))).unwrap();
        rt.decide(p, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(lc(&rt, p).0, "sent", "the agent was told");
        rt.answers_queued("a1", Some("≈14:20".into()));
        assert_eq!(lc(&rt, p), ("effect".into(), Some("queued".into()), Some("≈14:20".into())));
        rt.answers_resumed("a1");
        assert_eq!(lc(&rt, p).1.as_deref(), Some("resumed"));
        assert_eq!(lc(&rt, other).0, "parked", "an undecided parked permission is left alone");
    }

    #[test]
    fn a_question_is_sent_then_resumed_and_a_pane_answer_makes_it_stale() {
        let (_d, rt) = test_runtime(true);
        let q =
            rt.add_item(Some("a1"), "question", "a1: which db?".into(), json!({"question": "which db?"}), 20).unwrap();
        let r = rt.decide(q, InboxDecision::Reply, None, Some("sqlite")).unwrap();
        assert_eq!(r["lifecycle"]["state"], "sent");
        rt.answers_resumed("a1");
        assert_eq!(lc(&rt, q).1.as_deref(), Some("resumed"));
        // A question left in the pane, answered there.
        let rec = rt.store.read(|s| s.agents["a1"].clone());
        let pq = rt.question_item(&rec, "Shall I go on?", vec![], "pane", true, None).unwrap();
        rt.close_pane_questions("a1");
        let i = item(&rt, pq);
        assert_eq!((lc(&rt, pq).0.as_str(), i.stale_why.as_deref()), ("stale", Some("answered in the pane")));
        assert_eq!(i.decided_by.as_deref(), Some("pane"));
        assert!(actors(&rt).contains(&"pane".to_string()), "audited as the pane's");
    }

    #[test]
    fn budget_items_apply_or_skip_and_dismissals_say_so() {
        let (_d, rt) = test_runtime(true);
        let b = rt.add_item(None, "budget", "Reserve used up".into(), json!({"reason": "r"}), 0).unwrap();
        rt.decide(b, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(lc(&rt, b).1.as_deref(), Some("applied"));
        let c = rt.add_item(None, "budget", "Reserve used up".into(), json!({"reason": "r"}), 0).unwrap();
        let r = rt.decide(c, InboxDecision::Reject, None, None).unwrap();
        assert_eq!((lc(&rt, c).1.as_deref(), r["effect"].as_str()), (Some("skipped"), Some("skipped")));
        let a = rt.add_item(Some("a1"), "alert", "a1: x".into(), json!({}), 0).unwrap();
        rt.decide(a, InboxDecision::Dismiss, None, None).unwrap();
        assert_eq!(lc(&rt, a), ("dismissed".into(), Some("dismissed".into()), None));
    }

    /// The lifecycle of question `id` as last pushed (the `now` snapshot).
    fn pushed_lifecycle(rt: &Runtime, id: u64) -> Value {
        let snap = rt.now_snapshot();
        let d = snap["decisions"].as_array().unwrap().iter().find(|d| d["id"] == format!("q-{id}")).cloned();
        d.unwrap()["lifecycle"].clone()
    }

    fn version(rt: &Runtime, id: u64) -> u64 {
        rt.lifecycle(id)["version"].as_u64().unwrap()
    }

    #[test]
    fn the_version_grows_on_every_transition() {
        let (_d, rt) = test_runtime(true);
        let q =
            rt.add_item(Some("a1"), "question", "a1: which db?".into(), json!({"question": "which db?"}), 20).unwrap();
        let v0 = version(&rt, q);
        assert!(v0 >= 1);
        let r = rt.decide(q, InboxDecision::Reply, None, Some("sqlite")).unwrap();
        let v1 = r["lifecycle"]["version"].as_u64().unwrap();
        assert!(v1 > v0, "decided then sent: {v0} → {v1}");
        rt.answers_resumed("a1");
        let v2 = version(&rt, q);
        assert!(v2 > v1, "effect: {v1} → {v2}");
        // A write that changes nothing of the item keeps it.
        rt.store.write(|_| ()).unwrap();
        assert_eq!(version(&rt, q), v2);
        // A push that ran ahead (what it shows changed, not its lifecycle):
        // the next answer still outranks it.
        rt.now_pump(true);
        rt.store
            .write(|s| s.inbox.iter_mut().find(|i| i.id == q).unwrap().subject = json!({"agent": "a1", "x": 1}))
            .unwrap();
        rt.now_pump(false);
        rt.store
            .write(|s| s.inbox.iter_mut().find(|i| i.id == q).unwrap().subject = json!({"agent": "a1", "x": 2}))
            .unwrap();
        rt.now_pump(false);
        let pushed = pushed_lifecycle(&rt, q)["version"].as_u64().unwrap();
        rt.advance(q, Stage::Effect("delivered", None));
        let answered = version(&rt, q);
        assert!(answered > pushed, "the answer is never older than the push held");
        // The lifecycle moves again before the next push: that push outranks the answer.
        rt.advance(q, Stage::Effect("resumed", None));
        rt.now_pump(false);
        let lc = &pushed_lifecycle(&rt, q);
        assert_eq!(lc["effect"], "resumed");
        assert!(lc["version"].as_u64().unwrap() > answered, "a newer lifecycle is never dropped as old");
    }

    #[test]
    fn a_failed_or_stale_decision_can_be_dismissed() {
        let (_d, rt) = test_runtime(true);
        // A PR gate whose push failed.
        let g = rt.add_item(Some("a1"), "gate", "a1: PR".into(), json!({"action": "push_pr"}), 20).unwrap();
        rt.store
            .write(|s| {
                let _ = s.close_inbox(g, InboxStatus::Decided, Some("approve".into()), "user");
            })
            .unwrap();
        rt.advance(g, Stage::Failed("push rejected".into()));
        let before = version(&rt, g);
        let r = rt.decide(g, InboxDecision::Dismiss, None, None).unwrap();
        let lc = &r["lifecycle"];
        assert_eq!(
            (lc["state"].as_str(), lc["effect"].as_str(), lc["decided_by"].as_str()),
            (Some("dismissed"), Some("dismissed"), Some("user"))
        );
        assert_eq!(lc["error"], "push rejected", "kept for the record");
        assert!(lc["version"].as_u64().unwrap() > before);
        assert!(actors(&rt).contains(&"user".to_string()), "audited");
        // A stale permission.
        let p = rt.add_item(Some("a1"), "permission", "a1: Bash".into(), json!({"tool": "Bash"}), 30).unwrap();
        rt.store
            .write(|s| {
                let _ = s.close_inbox(p, InboxStatus::Decided, None, "pane");
            })
            .unwrap();
        rt.advance(p, Stage::Stale("the agent moved on"));
        rt.decide(p, InboxDecision::Dismiss, None, None).unwrap();
        assert_eq!(lc_state(&rt, p), "dismissed");
        // Nothing else applies to an ended decision, and a dismissed one stays dismissed.
        assert!(rt.decide(p, InboxDecision::Dismiss, None, None).is_err());
        // The builder offers dismiss on failed and stale decisions.
        let it = item(&rt, g);
        let mut failed = it.clone();
        failed.status = InboxStatus::Failed;
        let d = crate::decision::build(&crate::decision::Ctx {
            items: &[failed],
            agents: &Default::default(),
            tails: &Default::default(),
            ask: None,
            runs: &[],
            projects: &Default::default(),
            startup_timeout_ms: 30_000,
        });
        assert_eq!(d[0].actions.iter().map(|a| a.id).collect::<Vec<_>>(), ["dismiss", "open"]);
    }

    fn lc_state(rt: &Runtime, id: u64) -> String {
        rt.lifecycle(id)["state"].as_str().unwrap_or("").to_string()
    }
}
