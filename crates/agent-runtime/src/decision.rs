//! Decisions (FR-1303, P4.1 spec §3.3, api-contract §2): every Inbox item,
//! stored or synthesized, as one shape with its subject, typed context
//! blocks in display order, actions, lifecycle and thread. One builder for
//! every kind; the copy follows p4-design-notes §3.

use std::collections::BTreeMap;
use std::path::PathBuf;

use orch_store::{InboxItem, InboxStatus};
use serde::Serialize;
use serde_json::{Value, json};

use crate::attention::{AskFacts, RunFacts, copy::cut};
use crate::state::AgentState;
use crate::store::AgentRecord;

/// How long a closed decision stays in `now_snapshot` (api-contract §3.2).
pub const RECENT_MS: u64 = 10 * 60 * 1000;

/// The listed id of a store item (the Inbox's since P4.0, kept so
/// `inbox_act` routing and undo keep working): `inbox-3`, `gate-5`…
pub fn decision_id(kind: &str, id: u64) -> String {
    match kind {
        "permission" => format!("inbox-{id}"),
        "startup" => format!("startup-{id}"),
        "gate" => format!("gate-{id}"),
        "question" => format!("q-{id}"),
        _ => format!("alert-{id}"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<&'static str>,
}

fn act(id: &'static str, label: &'static str, key: Option<&'static str>) -> Action {
    Action { id, label, key }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Lifecycle {
    /// open | parked | decided | sent | effect | failed | stale | dismissed | interrupted
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_why: Option<String>,
}

impl Lifecycle {
    pub fn of(i: &InboxItem) -> Lifecycle {
        let state = match i.status {
            InboxStatus::Pending => "open",
            InboxStatus::Parked => "parked",
            InboxStatus::Decided | InboxStatus::Expired => "decided",
            InboxStatus::Sent => "sent",
            InboxStatus::Effect => "effect",
            InboxStatus::Failed => "failed",
            InboxStatus::Stale => "stale",
            InboxStatus::Dismissed => "dismissed",
            InboxStatus::Interrupted => "interrupted",
        };
        Lifecycle {
            state,
            decided_by: i.decided_by.clone(),
            decided_at_ms: i.decided_at_ms,
            sent_at_ms: i.sent_at_ms,
            effect: i.effect.clone(),
            effect_detail: i.effect_detail.clone(),
            effect_at_ms: i.effect_at_ms,
            error: i.error.clone(),
            stale_why: i.stale_why.clone(),
        }
    }

    fn open() -> Lifecycle {
        Lifecycle { state: "open", ..Default::default() }
    }
}

/// One decision (api-contract §2.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Decision {
    pub id: String,
    /// permission | question | gate | startup | alert | budget | ask | trust | wait | error | notion
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<&'static str>,
    /// 1 blocking · 2 errors · 3 budget
    pub level: u8,
    pub subject: Value,
    pub title: String,
    pub summary: String,
    pub context: Vec<Value>,
    pub actions: Vec<Action>,
    pub lifecycle: Lifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_prev: Option<String>,
    /// user | core (being delivered: not counted in the badge)
    pub waits_on: &'static str,
    pub created_ms: u64,
    /// The store's agent, for routing (`subject.agent` too).
    #[serde(skip)]
    pub agent: Option<String>,
    pub payload: Value,
}

impl Decision {
    /// Open and waiting on the user, levels 1–3: the title badge counts it.
    pub fn counts(&self) -> bool {
        matches!(self.lifecycle.state, "open" | "parked") && self.level <= 3
    }
}

/// What the builder reads besides the items.
pub struct Ctx<'a> {
    /// Stored items: open ones and those closed recently.
    pub items: &'a [InboxItem],
    pub agents: &'a BTreeMap<String, AgentRecord>,
    /// The PTY tail of agents a synthesized card shows (trust, wait, error).
    pub tails: &'a BTreeMap<String, String>,
    pub ask: Option<&'a AskFacts>,
    pub runs: &'a [RunFacts],
    pub projects: &'a BTreeMap<PathBuf, String>,
}

/// The last `n` non-blank lines.
fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn s<'v>(p: &'v Value, k: &str) -> &'v str {
    p.get(k).and_then(Value::as_str).unwrap_or("")
}

fn markdown(text: &str, source: Option<&str>) -> Value {
    let mut b = json!({"type": "markdown", "text": text});
    if let Some(src) = source.filter(|s| !s.is_empty()) {
        b["source"] = json!(src);
    }
    b
}

fn pty_tail(text: &str) -> Option<Value> {
    let t = last_lines(text, 12);
    (!t.is_empty()).then(|| json!({"type": "pty_tail", "text": t}))
}

fn error_block(text: &str, detail: Option<&str>) -> Value {
    let mut b = json!({"type": "error", "text": text});
    if let Some(d) = detail.filter(|d| !d.is_empty()) {
        b["detail"] = json!(d);
    }
    b
}

/// The `options` block: every question of a multi-question
/// `AskUserQuestion` (`payload.questions`), else the `ask_user` options.
fn options_block(p: &Value) -> Option<Value> {
    if let Some(qs) = p.get("questions").and_then(Value::as_array).filter(|q| !q.is_empty()) {
        let questions: Vec<Value> = qs
            .iter()
            .map(|q| {
                let options: Vec<Value> = q
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|o| match o {
                        Value::String(l) => json!({"label": l}),
                        o => json!({"label": s(o, "label"), "description": o.get("description")}),
                    })
                    .collect();
                json!({"question": s(q, "question"), "header": q.get("header"),
                       "multi": q.get("multiSelect").or_else(|| q.get("multi")).and_then(Value::as_bool).unwrap_or(false),
                       "options": options})
            })
            .collect();
        return Some(json!({"type": "options", "questions": questions}));
    }
    let opts = p.get("options").and_then(Value::as_array).filter(|o| !o.is_empty())?;
    let options: Vec<Value> = opts.iter().filter_map(Value::as_str).map(|l| json!({"label": l})).collect();
    Some(json!({"type": "options", "questions": [{"question": s(p, "question"), "multi": false, "options": options}]}))
}

fn last_message(p: &Value) -> Option<Value> {
    let lm = p.get("last_message")?;
    let text = lm.get("text").and_then(Value::as_str).or_else(|| lm.as_str())?;
    Some(json!({"type": "last_message", "text": text, "at_ms": lm.get("at_ms").and_then(Value::as_u64).unwrap_or(0)}))
}

/// `{agent, role, node, project, feature, run, task}` (absent parts left
/// out): the agent's seat, its feature's slug, else the project.
pub fn subject(agent: Option<&str>, rec: Option<&AgentRecord>, feature: Option<&str>, project: Option<&str>) -> Value {
    let mut v = serde_json::Map::new();
    if let Some(a) = agent {
        v.insert("agent".into(), json!(a));
    }
    if let Some(r) = rec {
        v.insert("role".into(), json!(r.role.name));
        if let Some(t) = &r.team {
            v.insert("node".into(), json!(t.node));
            v.insert("project".into(), json!(t.project_id));
            v.insert("feature".into(), json!(feature.unwrap_or(&t.feature_id)));
            v.insert("run".into(), json!(t.run_id));
            if let Some(task) = t.task {
                v.insert("task".into(), json!(task));
            }
        }
    }
    if !v.contains_key("project")
        && let Some(p) = project
    {
        v.insert("project".into(), json!(p));
    }
    Value::Object(v)
}

fn subject_of(cx: &Ctx, agent: Option<&str>, project: Option<&str>) -> Value {
    let rec = agent.and_then(|a| cx.agents.get(a));
    let seat = rec.and_then(|r| r.team.as_ref());
    let feature = seat.and_then(|t| cx.runs.iter().find(|f| f.run.id == t.run_id)).map(|f| f.feature.as_str());
    let project =
        rec.filter(|r| r.team.is_none()).and_then(|r| cx.projects.get(&r.repo)).map(String::as_str).or(project);
    subject(agent, rec, feature, project)
}

/// The run id, else the agent id: a decision's thread.
pub fn thread_of(subject: &Value) -> Option<String> {
    subject.get("run").or_else(|| subject.get("agent")).and_then(Value::as_str).map(str::to_string)
}

/// What a stored item is, as a Decision kind and gate.
fn kind_of(i: &InboxItem) -> (&'static str, Option<&'static str>) {
    match i.kind.as_str() {
        "permission" => ("permission", None),
        "question" => ("question", None),
        "gate" => match s(&i.payload, "action") {
            "push_pr" => ("gate", Some("pr")),
            "restart_gate" => ("gate", Some("restart")),
            _ => ("gate", Some("spec")),
        },
        "startup" => ("startup", None),
        "budget" => ("budget", None),
        "notion" => ("notion", None),
        _ if i.payload.get("source").and_then(Value::as_str) == Some("notion") => ("notion", None),
        _ => ("alert", None),
    }
}

/// 1 blocking · 2 errors · 3 budget.
fn level_of(kind: &str) -> u8 {
    match kind {
        "permission" | "question" | "gate" | "trust" | "wait" => 1,
        "budget" | "ask" | "notion" => 3,
        _ => 2,
    }
}

/// A stored item as a Decision.
fn stored(cx: &Ctx, i: &InboxItem) -> Decision {
    let (kind, gate) = kind_of(i);
    let p = &i.payload;
    let agent = i.agent_id.as_deref();
    let who = agent.unwrap_or("");
    let first_line = i.body.lines().next().unwrap_or("").to_string();
    let body_line = first_line.strip_prefix(&format!("{who}: ")).unwrap_or(&first_line).to_string();
    let open = i.open();
    let (title, summary, context, actions): (String, String, Vec<Value>, Vec<Action>) = match (kind, gate) {
        ("permission", _) => {
            let tool = s(p, "tool");
            let input = p.get("tool_input").or_else(|| p.get("input")).cloned().unwrap_or(Value::Null);
            let short = crate::inbox::main_arg(tool, &input).unwrap_or_default();
            let desc = s(p, "description");
            let title = format!("{tool} · {}", if desc.is_empty() { cut(&short, 60) } else { cut(desc, 60) });
            let summary = if i.status == InboxStatus::Parked {
                format!("{who} stopped; it resumes on your answer")
            } else {
                desc.to_string()
            };
            let mut actions = vec![
                act("approve", "Allow once", Some("a")),
                act("reject", "Deny", Some("r")),
                act("reply", "Deny with message", Some("enter")),
            ];
            if p.get("always").is_some_and(Value::is_string) && p.get("ask_rule").is_none_or(Value::is_null) {
                actions.push(act("always", "Always allow", None));
            }
            if i.status == InboxStatus::Pending {
                actions.push(act("in_pane", "Answer in the pane", None));
            }
            actions.push(act("open", "Open pane", Some("o")));
            let block = json!({"type": "tool_call", "tool": tool, "input": input, "cwd": p.get("cwd")});
            (title, summary, vec![block], actions)
        }
        ("question", _) => {
            let q = if s(p, "question").is_empty() { body_line.as_str() } else { s(p, "question") };
            let n = p.get("questions").and_then(Value::as_array).map_or(1, Vec::len);
            let title = if n > 1 { format!("{} · {n} questions", cut(q, 60)) } else { cut(q, 80) };
            let mut ctx = vec![markdown(q, None)];
            ctx.extend(options_block(p));
            ctx.extend(last_message(p));
            let actions = vec![
                act("answer", "Answer", Some("enter")),
                act("reject", "Dismiss", Some("r")),
                act("open", "Open pane", Some("o")),
            ];
            (title, String::new(), ctx, actions)
        }
        ("gate", Some("pr")) => {
            let feature = subject_of(cx, agent, None).get("feature").and_then(Value::as_str).map(str::to_string);
            let title = format!("PR gate · {}", feature.as_deref().unwrap_or(s(p, "branch")));
            let diff = p.get("diff").cloned().unwrap_or(Value::Null);
            let files = diff.get("files").and_then(Value::as_array).map_or(0, Vec::len);
            let summary = format!(
                "{} → {} · {files} files changed · +{} −{} lines",
                s(p, "branch"),
                s(p, "base"),
                diff.get("added").and_then(Value::as_u64).unwrap_or(0),
                diff.get("removed").and_then(Value::as_u64).unwrap_or(0)
            );
            let text = format!("**{}**\n\n{}", s(p, "title"), s(p, "body"));
            let mut ctx = vec![markdown(text.trim_end(), None)];
            if !diff.is_null() {
                ctx.push(json!({"type": "diff_summary", "files": diff.get("files"), "added": diff.get("added"),
                                "removed": diff.get("removed")}));
            }
            let actions = vec![
                act("approve", "Approve: push + open PR", Some("a")),
                act("reject", "Reject", Some("r")),
                act("reply", "Reply", Some("enter")),
                act("open", "Open pane", Some("o")),
            ];
            (title, summary, ctx, actions)
        }
        ("gate", Some("restart")) => (
            format!("Still waiting after a restart · {who}"),
            cut(s(p, "detail"), 120),
            vec![markdown(s(p, "detail"), None)],
            vec![
                act("approve", "Approve", Some("a")),
                act("reject", "Reject", Some("r")),
                act("open", "Open", Some("o")),
            ],
        ),
        ("gate", _) => {
            let feature = if s(p, "feature").is_empty() { "the feature" } else { s(p, "feature") };
            let summary = s(p, "summary");
            let files: Vec<Value> = match p.get("doc_lines").and_then(Value::as_array) {
                Some(d) => d.clone(),
                None => p
                    .get("docs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|d| json!({"path": d}))
                    .collect(),
            };
            let ctx = vec![markdown(summary, Some("spec.md")), json!({"type": "docs", "files": files})];
            let actions = vec![
                act("approve", "Approve spec", Some("a")),
                act("reject", "Send back", Some("r")),
                act("reply", "Send back with notes", Some("enter")),
                act("open", "Open pane", Some("o")),
            ];
            (format!("Spec gate · {feature}"), cut(summary, 120), ctx, actions)
        }
        ("startup", _) => {
            let failed = p.get("failed").and_then(Value::as_str);
            let (title, why) = match failed {
                Some("fresh") => (format!("{who} could not start fresh"), "restarted fresh, failed again".to_string()),
                Some(mode) => (format!("{who} did not start"), crate::attention::copy::start_exited(mode, None)),
                None => (
                    format!("{who} did not start"),
                    "No session after 30 s: a dialog may be waiting in its pane".to_string(),
                ),
            };
            let detail = p.get("exit_status").and_then(Value::as_u64).map(|n| format!("exit status {n}"));
            let mut ctx = vec![error_block(&why, detail.as_deref())];
            ctx.extend(pty_tail(s(p, "tail")));
            let mut actions = vec![act("restart_fresh", "Restart fresh", None), act("dismiss", "Dismiss", None)];
            if failed.is_some() {
                actions.push(act("delete", "Delete", None));
            }
            actions.push(act("open", "Open pane", Some("o")));
            (title, why, ctx, actions)
        }
        ("budget", _) => {
            let reason = if s(p, "reason").is_empty() { first_line.as_str() } else { s(p, "reason") };
            let mut ctx = vec![markdown(&i.body, None)];
            if p.get("cost_week_pp").is_some() || p.get("cost_5h_pp").is_some() {
                ctx.push(
                    json!({"type": "cost", "week_pp": p.get("cost_week_pp"), "five_hour_pp": p.get("cost_5h_pp")}),
                );
            }
            if let Some(d) = p.get("deadline_ms").and_then(Value::as_u64) {
                ctx.push(json!({"type": "countdown", "default": p.get("default"), "deadline_ms": d}));
            }
            let summary = i.body.lines().nth(1).unwrap_or("").to_string();
            (cut(reason, 80), summary, ctx, vec![act("approve", "Apply", Some("a")), act("reject", "Skip", Some("r"))])
        }
        ("notion", _) => {
            let text = p.get("text").and_then(Value::as_str).unwrap_or(&i.body);
            let title = format!(
                "Comment on {} · {}",
                p.get("feature").and_then(Value::as_str).unwrap_or("a page"),
                p.get("doc").and_then(Value::as_str).unwrap_or("Notion")
            );
            let author = p.get("author").and_then(Value::as_str);
            let summary = match author {
                Some(a) => format!("{a}: {}", cut(text, 100)),
                None => cut(text, 100),
            };
            let ctx = vec![markdown(text, p.get("page_url").and_then(Value::as_str))];
            // No reply from the app yet (the page's thread is answered in Notion).
            let actions = vec![act("dismiss", "Dismiss", Some("r")), act("open", "Open in Notion ↗", Some("o"))];
            (title, summary, ctx, actions)
        }
        _ => {
            let source = s(p, "alert");
            let is_error = matches!(source, "api_error" | "spawn_failed") || s(p, "action") == "watchdog";
            let mut ctx = vec![];
            if is_error {
                ctx.push(error_block(&body_line, p.get("error").and_then(Value::as_str)));
                ctx.extend(pty_tail(s(p, "tail")));
            } else if source == "trust" {
                ctx.extend(pty_tail(s(p, "tail")));
            } else {
                ctx.push(markdown(&i.body, None));
            }
            let actions = if source == crate::registry::NEEDS_AUTH_ALERT {
                vec![
                    act("reconnect", "Reconnect", Some("a")),
                    act("dismiss", "Dismiss", Some("r")),
                    act("open_settings", "Open Settings", Some("o")),
                ]
            } else {
                let mut a = vec![act("open", "Open", Some("o")), act("dismiss", "Dismiss", Some("a"))];
                if source == "spawn_failed" {
                    a.push(act("restart_fresh", "Restart fresh", None));
                }
                a
            };
            let title = match (source, agent) {
                ("api_error", Some(a)) => format!("{a} · {}", cut(&body_line, 60)),
                ("spawn_failed", Some(a)) => format!("{a} could not resume"),
                _ => cut(&body_line, 80),
            };
            let summary = i.body.lines().nth(1).map(str::to_string).unwrap_or_else(|| {
                if is_error { p.get("error").and_then(Value::as_str).unwrap_or("").to_string() } else { String::new() }
            });
            (title, cut(&summary, 120), ctx, actions)
        }
    };
    let subject = if i.subject.is_null() { subject_of(cx, agent, i.project_id.as_deref()) } else { i.subject.clone() };
    let thread = i.thread.clone().or_else(|| thread_of(&subject));
    Decision {
        id: decision_id(&i.kind, i.id),
        kind,
        gate,
        level: level_of(kind),
        title,
        summary,
        context,
        actions,
        lifecycle: Lifecycle::of(i),
        thread,
        thread_prev: None,
        waits_on: if open { "user" } else { "core" },
        created_ms: i.created_at_ms,
        agent: i.agent_id.clone(),
        subject,
        payload: p.clone(),
    }
}

/// A card synthesized from live state (D6): no stored row.
#[allow(clippy::too_many_arguments)]
fn synth(
    cx: &Ctx,
    id: String,
    kind: &'static str,
    agent: Option<&str>,
    title: String,
    summary: String,
    context: Vec<Value>,
    actions: Vec<Action>,
    created_ms: u64,
    payload: Value,
) -> Decision {
    let subject = subject_of(cx, agent, None);
    Decision {
        id,
        kind,
        gate: None,
        level: level_of(kind),
        thread: thread_of(&subject),
        subject,
        title,
        summary,
        context,
        actions,
        lifecycle: Lifecycle::open(),
        thread_prev: None,
        waits_on: "user",
        created_ms,
        agent: agent.map(str::to_string),
        payload,
    }
}

/// Every decision: synthesized ones (trust dialog, a prompt only in the
/// pane, an agent in error, the Governor's ask), then the stored items.
/// Each lists the previous decision of its thread.
pub fn build(cx: &Ctx) -> Vec<Decision> {
    let mut out = vec![];
    for a in cx.agents.values() {
        let since = a.last_event_at_ms.unwrap_or(a.created_at_ms);
        let tail = cx.tails.get(&a.id).map(String::as_str).unwrap_or("");
        let held = cx.items.iter().any(|i| i.open() && i.kind == "permission" && i.agent_id.as_deref() == Some(&a.id));
        let open = act("open", "Open pane", Some("o"));
        if a.trust_dialog {
            out.push(synth(
                cx,
                format!("trust-{}", a.id),
                "trust",
                Some(&a.id),
                "Claude Code asks to trust this folder".into(),
                format!("Answer it in {}'s pane: the app never does", a.id),
                pty_tail(tail).into_iter().collect(),
                vec![open],
                since,
                json!({"tail": last_lines(tail, 12)}),
            ));
        } else if a.tracker.state == AgentState::WaitingForYou && !held {
            out.push(synth(
                cx,
                format!("wait-{}", a.id),
                "wait",
                Some(&a.id),
                format!("A prompt in {}'s pane", a.id),
                String::new(),
                pty_tail(tail).into_iter().collect(),
                vec![act("reply", "Reply", Some("enter")), open],
                since,
                json!({"tail": last_lines(tail, 12)}),
            ));
        } else if a.tracker.state == AgentState::Error {
            let error = a.tracker.last_error.clone().unwrap_or_else(|| "Its session is saved; resume it.".into());
            let mut ctx = vec![error_block(&error, None)];
            ctx.extend(pty_tail(tail));
            out.push(synth(
                cx,
                format!("error-{}", a.id),
                "error",
                Some(&a.id),
                format!("{} stopped with an error", a.id),
                cut(&error, 120),
                ctx,
                vec![act("resume", "Resume", Some("r")), open],
                since,
                json!({"error": error, "tail": last_lines(tail, 12)}),
            ));
        }
    }
    if let Some(k) = cx.ask {
        let cost = k.cost_pp.map(|c| format!(" ≈ +{c:.1} pp of the week")).unwrap_or_default();
        let title = match &k.agent {
            Some(a) => format!("Run {a} now?{cost}"),
            None => cut(&k.reason, 80),
        };
        let default = if k.default_apply { "approve" } else { "reject" };
        let summary = format!(
            "{} · default: {} at the deadline",
            k.effect,
            if k.default_apply { "run now" } else { "keep queued" }
        );
        let ctx = vec![
            markdown(&format!("{}\n\n{}", k.reason, k.effect), None),
            json!({"type": "cost", "week_pp": k.cost_pp}),
            json!({"type": "countdown", "default": default, "deadline_ms": k.deadline_ms}),
        ];
        out.push(synth(
            cx,
            format!("ask-{}", k.id),
            "ask",
            k.agent.as_deref(),
            title,
            summary,
            ctx,
            vec![
                act("approve", "Run now", Some("a")),
                act("reject", "Keep queued", Some("r")),
                act("open", "Open", Some("o")),
            ],
            k.created_ms,
            json!({"action": k.id, "cost": k.cost_pp, "default": default, "deadline_ms": k.deadline_ms}),
        ));
    }
    // The trust dialog's stored alert is the synthesized `trust` card while the dialog is up: one card, one count.
    let trusting = |i: &InboxItem| {
        i.payload.get("alert").and_then(Value::as_str) == Some("trust")
            && i.agent_id.as_deref().and_then(|a| cx.agents.get(a)).is_some_and(|a| a.trust_dialog)
    };
    out.extend(cx.items.iter().filter(|i| i.kind != "info" && !trusting(i)).map(|i| stored(cx, i)));
    out.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
    let mut last: BTreeMap<String, String> = BTreeMap::new();
    for d in &mut out {
        if let Some(t) = &d.thread {
            d.thread_prev = last.insert(t.clone(), d.id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests;
