//! The Inbox (FR-707, §9.6): everything that needs the user, in attention
//! ladder order. Level 1 blocking (permission prompts held for the Inbox,
//! agents waiting for you, trust dialogs), level 2 errors (crashes, API
//! errors, runtime alerts), level 3 budget (the Governor's pending ask), level
//! 4 info (finished turns; a silent digest). Within a level, oldest first.

use serde::Serialize;

use crate::permissions::PendingPermission;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InboxItem {
    /// Stable across refreshes: `perm-3`, `wait-a1`, `alert-7`, `ask-12`, `info-<turn>`.
    pub id: String,
    pub level: u8,
    /// permission / question / trust / error / alert / budget / info
    pub kind: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub detail: String,
    /// Monospace preview (a command, the pane's last lines).
    pub preview: Option<String>,
    pub created_ms: u64,
    /// Keys this item answers to, besides j/k/x (FR-707).
    pub actions: Vec<&'static str>,
}

/// What the Inbox needs to know about an agent.
#[derive(Clone, Debug, Default)]
pub struct AgentFacts {
    pub id: String,
    pub state: String,
    pub since_ms: u64,
    pub trust_dialog: bool,
    pub last_error: Option<String>,
    pub tail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub id: u64,
    /// The store item's kind: `budget` (the Governor's warnings) is level 3, the rest level 2.
    pub kind: String,
    pub agent_id: Option<String>,
    pub body: String,
    pub created_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Ask {
    pub id: u64,
    pub reason: String,
    pub effect: String,
    pub created_ms: u64,
    pub deadline_ms: i64,
    pub default_apply: bool,
}

#[derive(Clone, Debug)]
pub struct Done {
    pub turn_id: String,
    pub agent_id: String,
    pub body: String,
    pub at_ms: u64,
}

#[derive(Default)]
pub struct Inputs<'a> {
    pub permissions: &'a [PendingPermission],
    pub agents: &'a [AgentFacts],
    pub alerts: &'a [Alert],
    pub ask: Option<Ask>,
    pub done: &'a [Done],
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

pub fn build(i: &Inputs) -> Vec<InboxItem> {
    let mut out = vec![];
    for p in i.permissions {
        out.push(InboxItem {
            id: p.id.clone(),
            level: 1,
            kind: "permission".into(),
            agent_id: Some(p.agent_id.clone()),
            title: format!("{}: {}", p.tool, p.description.clone().unwrap_or_else(|| p.summary.clone())),
            detail: format!("{} asks to run {}", p.agent_id, p.tool),
            preview: Some(p.summary.clone()),
            created_ms: p.created_ms,
            actions: vec!["a", "r", "enter", "o", "e"],
        });
    }
    for a in i.agents {
        let held = i.permissions.iter().any(|p| p.agent_id == a.id);
        if a.trust_dialog {
            out.push(InboxItem {
                id: format!("trust-{}", a.id),
                level: 1,
                kind: "trust".into(),
                agent_id: Some(a.id.clone()),
                title: "Claude Code asks to trust this folder".into(),
                detail: format!("{}: answer it in the pane (the app never does)", a.id),
                preview: a.tail.as_deref().map(|t| last_lines(t, 6)),
                created_ms: a.since_ms,
                actions: vec!["o"],
            });
        } else if a.state == "waiting_for_you" && !held {
            out.push(InboxItem {
                id: format!("wait-{}", a.id),
                level: 1,
                kind: "question".into(),
                agent_id: Some(a.id.clone()),
                title: format!("{} is waiting for you", a.id),
                detail: "A prompt or question in its pane".into(),
                preview: a.tail.as_deref().map(|t| last_lines(t, 8)),
                created_ms: a.since_ms,
                actions: vec!["enter", "o"],
            });
        } else if a.state == "error" {
            out.push(InboxItem {
                id: format!("error-{}", a.id),
                level: 2,
                kind: "error".into(),
                agent_id: Some(a.id.clone()),
                title: format!("{} stopped with an error", a.id),
                detail: a.last_error.clone().unwrap_or_else(|| "Its session is saved; resume it from its pane.".into()),
                preview: None,
                created_ms: a.since_ms,
                actions: vec!["o", "r"],
            });
        }
    }
    for al in i.alerts {
        let budget = al.kind == "budget";
        out.push(InboxItem {
            id: format!("alert-{}", al.id),
            level: if budget { 3 } else { 2 },
            kind: if budget { "budget" } else { "alert" }.into(),
            agent_id: al.agent_id.clone(),
            title: al.body.lines().next().unwrap_or("").to_string(),
            detail: al.body.clone(),
            preview: None,
            created_ms: al.created_ms,
            actions: vec!["a", "o"],
        });
    }
    if let Some(a) = &i.ask {
        out.push(InboxItem {
            id: format!("ask-{}", a.id),
            level: 3,
            kind: "budget".into(),
            agent_id: None,
            title: a.reason.clone(),
            detail: format!(
                "{} · default: {} at the deadline",
                a.effect,
                if a.default_apply { "apply" } else { "skip" }
            ),
            preview: None,
            created_ms: a.created_ms,
            actions: vec!["a", "r"],
        });
    }
    for d in i.done {
        out.push(InboxItem {
            id: format!("info-{}", d.turn_id),
            level: 4,
            kind: "info".into(),
            agent_id: Some(d.agent_id.clone()),
            title: format!("{} finished a turn", d.agent_id),
            detail: d.body.chars().take(120).collect(),
            preview: None,
            created_ms: d.at_ms,
            actions: vec!["o"],
        });
    }
    out.sort_by(|a, b| a.level.cmp(&b.level).then(a.created_ms.cmp(&b.created_ms)).then(a.id.cmp(&b.id)));
    out
}

/// The badge counts levels 1–3 (§9.6); level 4 stays in the digest.
pub fn badge(items: &[InboxItem]) -> usize {
    items.iter().filter(|i| i.level <= 3).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, state: &str, since: u64) -> AgentFacts {
        AgentFacts { id: id.into(), state: state.into(), since_ms: since, ..Default::default() }
    }

    #[test]
    fn ladder_order_and_badge() {
        let perm = PendingPermission {
            id: "perm-1".into(),
            agent_id: "front-1".into(),
            tool: "Bash".into(),
            summary: "pnpm add x".into(),
            description: Some("Add the SDK".into()),
            created_ms: 50,
            tool_use_id: None,
        };
        let agents = [
            agent("front-1", "waiting_for_you", 40), // held for the Inbox: not listed twice
            agent("solo", "waiting_for_you", 10),
            agent("ci-1", "error", 5),
            agent("back-1", "working", 1),
        ];
        let alerts = [
            Alert { id: 7, kind: "alert".into(), agent_id: None, body: "x: API error".into(), created_ms: 1 },
            Alert { id: 8, kind: "budget".into(), agent_id: None, body: "reserve used up".into(), created_ms: 0 },
        ];
        let done = [Done { turn_id: "t1".into(), agent_id: "back-1".into(), body: "go".into(), at_ms: 0 }];
        let items = build(&Inputs {
            permissions: std::slice::from_ref(&perm),
            agents: &agents,
            alerts: &alerts,
            ask: Some(Ask {
                id: 12,
                reason: "Defer infra".into(),
                effect: "saves 6 pp".into(),
                created_ms: 0,
                deadline_ms: 0,
                default_apply: true,
            }),
            done: &done,
        });
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["wait-solo", "perm-1", "alert-7", "error-ci-1", "alert-8", "ask-12", "info-t1"]);
        assert_eq!(items[4].kind, "budget");
        assert_eq!(badge(&items), 6);
        assert_eq!(items[1].preview.as_deref(), Some("pnpm add x"));
    }

    #[test]
    fn trust_dialog_wins_over_waiting() {
        let mut a = agent("a1", "starting", 3);
        a.trust_dialog = true;
        a.tail = Some("\n\nQuick safety check\n\n 1. Yes, I trust this folder\n".into());
        let items = build(&Inputs { agents: &[a], ..Default::default() });
        assert_eq!(items[0].kind, "trust");
        assert_eq!(items[0].preview.as_deref(), Some("Quick safety check\n 1. Yes, I trust this folder"));
    }
}
