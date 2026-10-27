//! A task's own workflow on top of its status: a plan the lead approves
//! before any edit (plan first), and a review before the lead merges, by the
//! owner itself (`/code-review` with fixes, then `/simplify`) or by the
//! team's reviewer node. The state lives in [`orch_store::TaskFlow`]; it is
//! the core's and never synced to a task backend.

use anyhow::{Result, anyhow, bail};
use orch_protocol::{HookOutput, ToolDecision, now_ms};
use orch_store::Task;
use orch_team::{EdgeKind, ReviewMode, TaskStatus, rules};
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::{RunCtx, Sender, cut};

/// What the owner runs on its branch before it reports (in order).
pub const SELF_REVIEW: &[&str] = &["/code-review high --fix --max-findings all", "/simplify"];

/// The turn after the review commands.
pub const AFTER_SELF_REVIEW: &str = "ORCH: your self-review is done. Commit its fixes with mcp__orch__commit (tests must still pass), then call mcp__orch__report_done again with your summary and `review`: what was fixed, and each finding left with why (`unfixed`). Your lead decides on what is left.";

/// Tools that change files: refused until a plan-first task's plan is approved.
const EDIT_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// The review as the lead reads it.
pub fn render_review(v: &Value) -> String {
    let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut s = String::new();
    if let Some(r) = v.get("reviewer").and_then(Value::as_str) {
        s.push_str(&format!("\nReviewed by {r}: {}", v.get("summary").and_then(Value::as_str).unwrap_or("")));
        for f in list("findings") {
            s.push_str(&format!("\n- {}", f.as_str().unwrap_or("")));
        }
        return s;
    }
    let fixed = list("fixed");
    let unfixed = list("unfixed");
    s.push_str(&format!("\nSelf-review: {} fixed, {} left.", fixed.len(), unfixed.len()));
    for f in fixed.iter().filter_map(Value::as_str) {
        s.push_str(&format!("\n- fixed: {f}"));
    }
    for u in &unfixed {
        let finding = u.get("finding").and_then(Value::as_str).or(u.as_str()).unwrap_or("");
        let why = u.get("reason").and_then(Value::as_str).map(|r| format!(" ({r})")).unwrap_or_default();
        s.push_str(&format!("\n- LEFT: {finding}{why}"));
    }
    s
}

/// The node that reviews `me`'s work: a `reviews` edge from it, else a
/// node whose role is a reviewer.
fn reviewer_of(cx: &RunCtx, me: &str) -> Option<String> {
    cx.team
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Reviews && e.from == me)
        .map(|e| e.to.clone())
        .or_else(|| cx.team.nodes.iter().find(|n| n.role_id.contains("reviewer") && n.id != me).map(|n| n.id.clone()))
        .filter(|n| !cx.team.is_lead(n))
}

impl Runtime {
    fn save_flow(&self, t: &Task) -> Result<()> {
        self.db.upsert_task(t)?;
        Ok(())
    }

    /// PreToolUse of a team agent on a plan-first task whose plan is not
    /// approved yet: edits are refused.
    pub(crate) fn plan_lock(&self, rec: &AgentRecord, tool: &str) -> Option<HookOutput> {
        let task = rec.team.as_ref()?.task?;
        let edits = EDIT_TOOLS.contains(&tool) || tool == "mcp__orch__commit";
        if !edits {
            return None;
        }
        let t = self.db.task(task).ok()??;
        if !t.flow.plan_first || t.flow.plan_approved {
            return None;
        }
        let why = if t.flow.plan.is_some() {
            "your plan is waiting for your lead's approval; end your turn, the decision comes as a new turn"
        } else {
            "send your plan with mcp__orch__submit_plan first"
        };
        Some(HookOutput::pre_tool_use(
            ToolDecision::Deny,
            &format!("ORCH: {} is a plan-first task: {why}.", t.handle()),
        ))
    }

    /// Merging a task needs its review first (unless the team reviews none).
    pub(crate) fn review_missing(&self, cx: &RunCtx, t: &Task) -> Option<String> {
        if cx.team.review == ReviewMode::None || t.flow.review.is_some() {
            return None;
        }
        Some(if t.flow.reviewing {
            format!("{} is still being reviewed: merge it when the report comes", t.handle())
        } else {
            format!("{} has not been reviewed yet: wait for its report", t.handle())
        })
    }

    /// `report_done` before the review: start it. `Some(text)` is the answer
    /// to the owner (the report waits); `None` means report now.
    pub(crate) fn review_first(
        &self,
        cx: &RunCtx,
        rec: &AgentRecord,
        me: &str,
        t: &mut Task,
        args: &Value,
    ) -> Result<Option<String>> {
        if let Some(r) = args.get("review").filter(|v| v.is_object()) {
            t.flow.review = Some(r.clone());
            t.flow.reviewing = false;
            return Ok(None);
        }
        if t.flow.review.is_some() {
            return Ok(None);
        }
        match cx.team.review {
            ReviewMode::None => Ok(None),
            ReviewMode::SelfReview if t.flow.reviewing => {
                bail!("pass `review` (what you fixed, and each finding you left with why) with this report")
            }
            ReviewMode::SelfReview => {
                t.flow.reviewing = true;
                self.save_flow(t)?;
                for c in SELF_REVIEW {
                    self.deliver_command(&rec.id, c)?;
                }
                self.deliver_kind(&rec.id, AFTER_SELF_REVIEW, governor::TurnKind::Task)?;
                self.run_event(&cx.run.id, Some(&rec.id), "task.self_review", json!({"task": t.id}));
                Ok(Some(format!(
                    "Before {} goes to your lead, you review your branch: the orchestrator types {} next. End your turn now.",
                    t.handle(),
                    SELF_REVIEW.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(" then ")
                )))
            }
            ReviewMode::Node if t.flow.reviewing => {
                Ok(Some(format!("{} is with its reviewer; its findings come to you and your lead.", t.handle())))
            }
            ReviewMode::Node => {
                let Some(reviewer) = reviewer_of(cx, me) else { return Ok(None) };
                let branch = t.branch.clone().unwrap_or_default();
                let summary = cut(str_arg(args, "summary").unwrap_or(""), 2_000);
                let mut r =
                    Task::new(&cx.project.id, "", &format!("Review {} ({})", t.handle(), cut(&t.title, 120)), now_ms());
                r.body = format!(
                    "Review {branch} for task id {} ({}). The owner says: {summary}\n\nRead it with `git diff {}...{branch}`, run `/code-review high --max-findings all` on it, then call mcp__orch__submit_review with task {} and your findings. Don't change the branch.",
                    t.id,
                    t.title,
                    cx.feature.integration_branch(),
                    t.id
                );
                r.node = Some(reviewer.clone());
                r.feature_id = Some(cx.feature.id.clone());
                r.team_run_id = Some(cx.run.id.clone());
                let r = self.tasks_for(&cx.project).create(r)?.value;
                t.flow.reviewing = true;
                self.save_flow(t)?;
                self.run_event(
                    &cx.run.id,
                    Some(&rec.id),
                    "task.review_asked",
                    json!({"task": t.id, "reviewer": reviewer, "review_task": r.id}),
                );
                Ok(Some(format!(
                    "{reviewer} reviews {branch} first (task {}); its findings go to you and your lead. End your turn.",
                    r.handle()
                )))
            }
        }
    }

    /// `submit_plan`, `decide_plan`, `submit_review`.
    pub(crate) fn flow_tool(
        &self,
        cx: &RunCtx,
        rec: &AgentRecord,
        me: &str,
        is_lead: bool,
        tool: &str,
        args: &Value,
    ) -> Result<(String, Value)> {
        let task_arg = |k: &str| args.get(k).and_then(Value::as_u64);
        let load = |id: u64| -> Result<Task> {
            let t = self.db.task(id)?.ok_or_else(|| anyhow!("no task {id}"))?;
            if t.team_run_id.as_deref() != Some(&cx.run.id) {
                bail!("task {id} is not part of this feature");
            }
            Ok(t)
        };
        match tool {
            "submit_plan" => {
                if is_lead {
                    bail!("plans are sent to the lead");
                }
                let plan = str_arg(args, "plan").ok_or_else(|| anyhow!("`plan` is required"))?;
                let id = task_arg("task")
                    .or(rec.team.as_ref().and_then(|s| s.task))
                    .ok_or_else(|| anyhow!("you have no current task"))?;
                let mut t = load(id)?;
                if t.node.as_deref() != Some(me) {
                    bail!("task {id} is not yours");
                }
                t.flow.plan = Some(cut(plan, 8_000));
                t.flow.plan_approved = false;
                self.save_flow(&t)?;
                let up = rules::reports_to(&cx.team, me).unwrap_or_else(|| "lead".into());
                self.post_message(
                    cx,
                    Sender(me, Some(&rec.id)),
                    &up,
                    "plan",
                    &format!(
                        "plan for task id {id} ({}):\n{}\n\nApprove it or send it back with mcp__orch__decide_plan.",
                        t.title,
                        cut(plan, 6_000)
                    ),
                    json!({"task": id}),
                )?;
                self.run_event(&cx.run.id, Some(&rec.id), "task.plan", json!({"task": id}));
                Ok((format!("Plan sent to {up}. End your turn; the decision comes as a new turn."), Value::Null))
            }
            "decide_plan" => {
                if !is_lead {
                    bail!("only the lead approves plans");
                }
                let id = task_arg("task").ok_or_else(|| anyhow!("`task` is required"))?;
                let approve =
                    args.get("approve").and_then(Value::as_bool).ok_or_else(|| anyhow!("`approve` is required"))?;
                let note = str_arg(args, "note").map(|n| format!(" {n}")).unwrap_or_default();
                let mut t = load(id)?;
                if t.flow.plan.is_none() {
                    bail!("{} has no plan to decide on", t.handle());
                }
                let owner = t.agent_id.clone().ok_or_else(|| anyhow!("{} has no owner", t.handle()))?;
                let text = if approve {
                    t.flow.plan_approved = true;
                    format!("ORCH: your plan for {} is approved.{note} Implement it now.", t.handle())
                } else {
                    t.flow.plan = None;
                    format!(
                        "ORCH: your plan for {} needs changes:{note} Send a new plan with mcp__orch__submit_plan.",
                        t.handle()
                    )
                };
                self.save_flow(&t)?;
                self.deliver(&owner, &text)?;
                self.run_event(
                    &cx.run.id,
                    Some(&rec.id),
                    "task.plan_decided",
                    json!({"task": id, "approved": approve}),
                );
                Ok((format!("{} told: plan {}.", owner, if approve { "approved" } else { "sent back" }), Value::Null))
            }
            "submit_review" => {
                if is_lead {
                    bail!("the lead decides on reviews; it doesn't submit them");
                }
                let id = task_arg("task").ok_or_else(|| anyhow!("`task` is required"))?;
                let findings: Vec<String> = args
                    .get("findings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|f| cut(f, 1_000))
                    .take(50)
                    .collect();
                let summary = cut(str_arg(args, "summary").unwrap_or(""), 1_000);
                let mut t = load(id)?;
                if t.node.as_deref() == Some(me) {
                    bail!("you can't review your own task");
                }
                let review = json!({"reviewer": me, "findings": findings, "summary": summary});
                t.flow.review = Some(review.clone());
                t.flow.reviewing = false;
                t.flow.review_rounds += 1;
                self.apply_status(&mut t, TaskStatus::Review);
                let svc = self.tasks_for(&cx.project);
                svc.update(t.clone(), now_ms())?;
                // The reviewer's own review task is done.
                if let Some(mine) = rec.team.as_ref().and_then(|s| s.task) {
                    if let Ok(Some(mut r)) = self.db.task(mine) {
                        self.apply_status(&mut r, TaskStatus::Done);
                        svc.update(r, now_ms())?;
                    }
                }
                let owner_note =
                    t.notes.iter().rev().find(|n| Some(n.by.as_str()) == t.node.as_deref()).map(|n| n.text.clone());
                let up = rules::reports_to(&cx.team, t.node.as_deref().unwrap_or("")).unwrap_or_else(|| "lead".into());
                let branch = t.branch.clone().unwrap_or_default();
                self.post_message(
                    cx,
                    Sender(me, Some(&rec.id)),
                    &up,
                    "report",
                    &format!(
                        "task id {id} ({}) is done on {branch}: {}{}",
                        t.title,
                        owner_note.unwrap_or_default(),
                        render_review(&review)
                    ),
                    json!({"task": id, "branch": branch}),
                )?;
                if let Some(owner) = &t.agent_id {
                    if !findings.is_empty() {
                        let _ = self.deliver(
                            owner,
                            &format!(
                                "ORCH: {me} reviewed {}:{} Your lead decides what to fix; wait for it.",
                                t.handle(),
                                render_review(&review)
                            ),
                        );
                    }
                }
                self.run_event(
                    &cx.run.id,
                    Some(&rec.id),
                    "task.reviewed",
                    json!({"task": id, "findings": findings.len()}),
                );
                Ok((format!("Review of {} sent to {up}.", t.handle()), Value::Null))
            }
            other => bail!("unknown tool {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviews_read_well() {
        let s = render_review(
            &json!({"fixed": ["unused import"], "unfixed": [{"finding": "N+1 query", "reason": "needs a schema change"}]}),
        );
        assert!(s.contains("1 fixed, 1 left"));
        assert!(s.contains("LEFT: N+1 query (needs a schema change)"));
        let r = render_review(&json!({"reviewer": "rev", "summary": "ok", "findings": ["a.rs:3 off by one"]}));
        assert!(r.contains("Reviewed by rev: ok") && r.contains("- a.rs:3 off by one"));
    }
}
