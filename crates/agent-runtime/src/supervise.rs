//! The lead keeps its team moving (the user talks to the lead only): the core
//! tells it when a member is stuck, and gives it `team_status`, `nudge` and
//! `resume_agent` to act. Permission prompts stay the user's (ADR-009).

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use orch_protocol::now_ms;

use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::attention::{Level, Status};
use crate::state::AgentState;
use crate::store::AgentRecord;
use crate::team::{RunCtx, str_arg};

/// A permission prompt nobody answered this long is worth the lead's note.
pub const WAITING_FOR_USER_MS: u64 = 3 * 60 * 1000;
/// Work queued (or a task idle) this long.
pub const STUCK_MS: u64 = 10 * 60 * 1000;

/// One member's state as the lead sees it: its attention record (P4.1).
#[derive(Clone, Debug, PartialEq)]
pub struct MemberStatus {
    pub agent: String,
    pub node: String,
    pub state: AgentState,
    pub task: Option<u64>,
    pub level: Level,
    pub status: Status,
    /// The record's reason (what it waits on), may be empty.
    pub reason: String,
    /// Since when it is at this level and wait, ms.
    pub since_ms: u64,
}

impl MemberStatus {
    /// What is wrong, with the start of the episode (one note each). A
    /// member waiting on another agent or a task is waiting, not stuck.
    pub fn stuck(&self, now: u64) -> Option<(&'static str, u64, String)> {
        let mins = now.saturating_sub(self.since_ms) / 60_000;
        let waited = |ms: u64| now.saturating_sub(self.since_ms) >= ms;
        let who = &self.agent;
        let why = if self.reason.is_empty() { String::new() } else { format!(" ({})", self.reason) };
        let what = match self.level {
            Level::NeedsYou if waited(WAITING_FOR_USER_MS) => {
                ("waiting_for_user", format!("{who} has waited {mins} min for the user{why}"))
            }
            Level::Error => ("error", format!("{who} is in error{why}; resume it or reassign its task")),
            Level::Queued if waited(STUCK_MS) => ("queued", format!("{who} has had work waiting for {mins} min{why}")),
            Level::Idle if self.task.is_some() && waited(STUCK_MS) => (
                "idle_with_task",
                format!("{who} has been idle for {mins} min with task T-{} not reported done", self.task.unwrap_or_default()),
            ),
            _ => return None,
        };
        Some((what.0, self.since_ms, what.1))
    }

    fn line(&self, now: u64) -> String {
        let task = self.task.map(|t| format!(" · T-{t}")).unwrap_or_default();
        let label = self.status.label();
        let reason = if self.reason.is_empty() {
            String::new()
        } else {
            format!(" · {} · {} min", self.reason, now.saturating_sub(self.since_ms) / 60_000)
        };
        format!("- {} (`{}`): {label}{reason}{task}", self.agent, self.node)
    }
}

impl Runtime {
    /// Every member of the run but the lead, with its queue facts.
    pub(crate) fn members(&self, cx: &RunCtx) -> Vec<MemberStatus> {
        self.members_of(cx, &self.run_agents(&cx.run.id))
    }

    fn members_of(&self, cx: &RunCtx, agents: &[AgentRecord]) -> Vec<MemberStatus> {
        self.now_pump(false);
        agents
            .iter()
            .filter_map(|a| {
                let seat = a.team.as_ref()?;
                if cx.team.is_lead(&seat.node) {
                    return None;
                }
                let r = self.attention_of(&a.id)?;
                Some(MemberStatus {
                    agent: a.id.clone(),
                    node: seat.node.clone(),
                    state: a.tracker.state,
                    task: seat.task,
                    level: r.level,
                    status: r.status,
                    reason: r.reason,
                    since_ms: r.waiting_on.map(|w| w.since_ms).unwrap_or(r.since_ms),
                })
            })
            .collect()
    }

    /// Tells the lead about members stuck for a while (once per episode).
    pub(crate) fn supervise(&self, cx: &RunCtx, now: u64) {
        let agents = self.run_agents(&cx.run.id);
        let lead_node = cx.team.lead().map(|n| n.id.as_str());
        let Some(lead) = agents.iter().find(|a| a.team.as_ref().map(|t| t.node.as_str()) == lead_node) else {
            return;
        };
        let text: BTreeMap<String, String> = self
            .members_of(cx, &agents)
            .iter()
            .filter_map(|m| m.stuck(now).map(|(what, since, line)| (format!("{}:{what}:{since}", m.agent), line)))
            .collect();
        let new = self.supervised(&cx.run.id, &text.keys().cloned().collect());
        if new.is_empty() {
            return;
        }
        let lines: String = new.iter().map(|k| format!("\n- {}", text[k])).collect();
        let body = format!(
            "Team status:{lines}\n\nUnblock them yourself: mcp__orch__nudge to tell a member what to do, mcp__orch__resume_agent to restart a suspended one, mcp__orch__update_task to reassign. Only a permission prompt needs the user."
        );
        let to_node = lead.team.as_ref().map(|t| t.node.clone()).unwrap_or_default();
        let to = Some(lead.id.clone());
        let _ = self.add_run_message(cx, "orchestrator", None, &to_node, to, "status", &body, json!({"stuck": new}));
        self.run_event(&cx.run.id, Some(&lead.id), "team.stuck", json!({"members": new}));
    }

    /// The lead's tools on its team.
    pub(crate) fn lead_tool(&self, cx: &RunCtx, tool: &str, args: &Value) -> Result<(String, Value)> {
        let now = now_ms();
        let member = |args: &Value| -> Result<MemberStatus> {
            let a = str_arg(args, "agent").unwrap_or("");
            self.members(cx)
                .into_iter()
                .find(|m| m.agent == a || m.node == a)
                .ok_or_else(|| anyhow!("no member {a:?} in this team (use team_status)"))
        };
        match tool {
            "team_status" => {
                let ms = self.members(cx);
                if ms.is_empty() {
                    return Ok(("No team members have started yet.".into(), json!([])));
                }
                let lines: Vec<String> = ms.iter().map(|m| m.line(now)).collect();
                let data: Vec<Value> = ms
                    .iter()
                    .map(|m| json!({"agent": m.agent, "node": m.node, "state": m.state, "task": m.task,
                        "queue_reason": (m.level == Level::Queued).then_some(&m.reason), "reason": m.reason}))
                    .collect();
                Ok((lines.join("\n"), json!(data)))
            }
            "nudge" => {
                let m = member(args)?;
                let text = str_arg(args, "text").ok_or_else(|| anyhow!("`text` is required"))?;
                let turn = self.deliver(&m.agent, &format!("ORCH: from your lead: {text}"))?;
                Ok((format!("Told {}; it gets it as its next turn.", m.agent), json!({"turn": turn})))
            }
            "resume_agent" => {
                let m = member(args)?;
                if self.live(&m.agent).is_some() {
                    bail!("{} is running ({}): nudge it instead", m.agent, m.state.label());
                }
                // Its last start died at once even fresh, or its resume has no
                // session to resume (FR-415): a fresh session with its brief.
                // Queued, not spawned: the Governor admits it like any resume (FR-411).
                if self.resume_failed(&m.agent) {
                    let session = self.queue_fresh_restart(&m.agent, "resumed by its lead after a failed start")?;
                    return Ok((
                        format!("{} restarts as a fresh session with its brief when admitted.", m.agent),
                        json!({"session_id": session}),
                    ));
                }
                // A failed resume flags the agent, and the delivery loop never
                // starts a flagged one: the lead's resume is a new attempt.
                self.store.write(|s| {
                    if let Some(a) = s.agents.get_mut(&m.agent) {
                        a.flag = None;
                    }
                })?;
                // Through the Governor like any work: a continue turn.
                let turn = self.deliver_kind(
                    &m.agent,
                    "ORCH: you were resumed by your lead. Carry on with your task, or report where you are.",
                    governor::TurnKind::Continue,
                )?;
                Ok((format!("{} resumes when admitted.", m.agent), json!({"turn": turn})))
            }
            other => bail!("unknown lead tool {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(level: Level) -> MemberStatus {
        MemberStatus {
            agent: "f-back-1".into(),
            node: "back".into(),
            state: AgentState::Idle,
            task: Some(2),
            level,
            status: Status::Idle,
            reason: String::new(),
            since_ms: 0,
        }
    }

    #[test]
    fn what_counts_as_stuck() {
        let now = 60 * 60_000;
        assert!(m(Level::Running).stuck(now).is_none());
        let (what, _, line) = MemberStatus { reason: "permission: Bash ls".into(), ..m(Level::NeedsYou) }.stuck(now).unwrap();
        assert_eq!((what, line.as_str()), ("waiting_for_user", "f-back-1 has waited 60 min for the user (permission: Bash ls)"));
        assert!(m(Level::NeedsYou).stuck(WAITING_FOR_USER_MS - 1).is_none());
        assert_eq!(m(Level::Idle).stuck(now).unwrap().0, "idle_with_task");
        assert_eq!(m(Level::Error).stuck(0).unwrap().0, "error");
        let queued = MemberStatus { reason: "memory pressure · live cap 2/2".into(), ..m(Level::Queued) };
        let (what, since, line) = queued.stuck(now).unwrap();
        assert_eq!((what, since), ("queued", 0));
        assert!(line.contains("memory pressure"));
        assert!(MemberStatus { task: None, ..m(Level::Idle) }.stuck(now).is_none());
        // Waiting on another agent or a task is not stuck.
        assert!(m(Level::Waiting).stuck(now).is_none());
    }
}
