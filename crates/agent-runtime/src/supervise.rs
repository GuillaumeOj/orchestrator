//! The lead keeps its team moving (the user talks to the lead only): the core
//! tells it when a member is stuck, and gives it `team_status`, `nudge` and
//! `resume_agent` to act. Permission prompts stay the user's (ADR-009).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};
use orch_protocol::now_ms;

use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::{AgentRecord, TurnStatus};
use crate::team::{RunCtx, str_arg};

/// A permission prompt nobody answered this long is worth the lead's note.
pub const WAITING_FOR_USER_MS: u64 = 3 * 60 * 1000;
/// Work queued (or a task idle) this long.
pub const STUCK_MS: u64 = 10 * 60 * 1000;

/// One member's state as the lead sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberStatus {
    pub agent: String,
    pub node: String,
    pub state: AgentState,
    pub task: Option<u64>,
    /// Since when it is in this state (last hook event), ms.
    pub since_ms: u64,
    /// Oldest queued turn, ms.
    pub queued_since_ms: Option<u64>,
    /// Why the Governor holds it, when it does.
    pub queue_reason: Option<String>,
}

impl MemberStatus {
    /// What is wrong, with the start of the episode (one note each).
    pub fn stuck(&self, now: u64) -> Option<(&'static str, u64, String)> {
        let mins = |t: u64| now.saturating_sub(t) / 60_000;
        let who = &self.agent;
        match self.state {
            AgentState::WaitingForYou if now.saturating_sub(self.since_ms) >= WAITING_FOR_USER_MS => {
                return Some((
                    "waiting_for_user",
                    self.since_ms,
                    format!("{who} has waited {} min for the user to answer a permission prompt", mins(self.since_ms)),
                ));
            }
            AgentState::Error => {
                return Some(("error", self.since_ms, format!("{who} is in error; resume it or reassign its task")));
            }
            AgentState::Idle
                if self.task.is_some()
                    && self.queued_since_ms.is_none()
                    && now.saturating_sub(self.since_ms) >= STUCK_MS =>
            {
                return Some((
                    "idle_with_task",
                    self.since_ms,
                    format!(
                        "{who} has been idle for {} min with task T-{} not reported done",
                        mins(self.since_ms),
                        self.task.unwrap_or_default()
                    ),
                ));
            }
            // A long turn with work behind it is busy, not stuck (the
            // watchdog watches long turns).
            AgentState::Working => return None,
            _ => {}
        }
        let q = self.queued_since_ms.filter(|t| now.saturating_sub(*t) >= STUCK_MS)?;
        let why = self.queue_reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
        Some(("queued", q, format!("{who} has had work waiting for {} min{why}", mins(q))))
    }

    fn line(&self, now: u64) -> String {
        let task = self.task.map(|t| format!(" · T-{t}")).unwrap_or_default();
        let queued = match (self.queued_since_ms, &self.queue_reason) {
            (Some(t), Some(r)) => format!(" · queued {} min: {r}", now.saturating_sub(t) / 60_000),
            (Some(t), None) => format!(" · work queued {} min", now.saturating_sub(t) / 60_000),
            _ => String::new(),
        };
        format!("- {} (`{}`): {}{task}{queued}", self.agent, self.node, self.state.label())
    }
}

impl Runtime {
    /// Every member of the run but the lead, with its queue facts.
    pub(crate) fn members(&self, cx: &RunCtx) -> Vec<MemberStatus> {
        self.members_of(cx, &self.run_agents(&cx.run.id))
    }

    fn members_of(&self, cx: &RunCtx, agents: &[AgentRecord]) -> Vec<MemberStatus> {
        let ids: BTreeSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        // Turns are kept in queue order: the first one seen is the oldest.
        let queued: BTreeMap<String, u64> = self.store.read(|s| {
            let mut m: BTreeMap<String, u64> = BTreeMap::new();
            for t in s.turns.iter().filter(|t| t.status == TurnStatus::Queued && ids.contains(t.agent_id.as_str())) {
                m.entry(t.agent_id.clone()).or_insert(t.created_at_ms);
            }
            m
        });
        let reasons: BTreeMap<String, String> =
            self.with_governor(|g| g.queue().iter().map(|e| (e.agent.0.clone(), e.reason.to_string())).collect());
        agents
            .iter()
            .filter_map(|a| {
                let seat = a.team.as_ref()?;
                if cx.team.is_lead(&seat.node) {
                    return None;
                }
                Some(MemberStatus {
                    agent: a.id.clone(),
                    node: seat.node.clone(),
                    state: a.tracker.state,
                    task: seat.task,
                    since_ms: a.last_event_at_ms.unwrap_or(a.created_at_ms),
                    queued_since_ms: queued.get(&a.id).copied(),
                    queue_reason: reasons.get(&a.id).cloned(),
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
                    .map(|m| json!({"agent": m.agent, "node": m.node, "state": m.state, "task": m.task, "queue_reason": m.queue_reason}))
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
                if self.resume_failed(&m.agent) {
                    let session = self.restart_fresh(&m.agent, "resumed by its lead after a failed start", "core")?;
                    return Ok((
                        format!("{} restarted as a fresh session with its brief.", m.agent),
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

    fn m(state: AgentState) -> MemberStatus {
        MemberStatus {
            agent: "f-back-1".into(),
            node: "back".into(),
            state,
            task: Some(2),
            since_ms: 0,
            queued_since_ms: None,
            queue_reason: None,
        }
    }

    #[test]
    fn what_counts_as_stuck() {
        let now = 60 * 60_000;
        assert!(m(AgentState::Working).stuck(now).is_none());
        assert_eq!(m(AgentState::WaitingForYou).stuck(now).unwrap().0, "waiting_for_user");
        assert!(m(AgentState::WaitingForYou).stuck(WAITING_FOR_USER_MS - 1).is_none());
        assert_eq!(m(AgentState::Idle).stuck(now).unwrap().0, "idle_with_task");
        let queued = MemberStatus {
            queued_since_ms: Some(0),
            queue_reason: Some("memory pressure".into()),
            ..m(AgentState::Suspended)
        };
        let (what, since, line) = queued.stuck(now).unwrap();
        assert_eq!((what, since), ("queued", 0));
        assert!(line.contains("memory pressure"));
        assert!(MemberStatus { task: None, ..m(AgentState::Idle) }.stuck(now).is_none());
    }
}
