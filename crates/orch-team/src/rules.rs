//! What each node may do, from the edges (FR-304), and the Rules panel text
//! (FR-305). The orchestrator MCP server checks every recipient with these
//! (ADR-009 decision 5).
//!
//! Readings of FR-304 (simplest reading taken):
//! - every node may message the lead, and the lead may message, assign to,
//!   hand off to and ask a review from every node;
//! - reports-to A → B: A messages B, B messages and assigns tasks to A;
//! - hands-off-to A → B: A hands artifacts to B and may message B;
//! - reviews A → B: A asks B for reviews; both may message each other;
//! - can-message: both ways.

use std::collections::BTreeSet;

use crate::model::{EdgeKind, Team};

/// The pseudo-node of the user (FR-409): may message anyone, never assigned.
pub const USER: &str = "user";

fn has(team: &Team, from: &str, to: &str, kind: EdgeKind) -> bool {
    team.edges.iter().any(|e| e.kind == kind && e.from == from && e.to == to)
}

fn exists(team: &Team, id: &str) -> bool {
    team.node(id).is_some()
}

pub fn can_message(team: &Team, from: &str, to: &str) -> bool {
    if from == to || !exists(team, to) {
        return false;
    }
    if from == USER {
        return true;
    }
    if !exists(team, from) {
        return false;
    }
    if team.is_lead(from) || team.is_lead(to) {
        return true;
    }
    has(team, from, to, EdgeKind::ReportsTo)
        || has(team, to, from, EdgeKind::ReportsTo)
        || has(team, from, to, EdgeKind::HandsOffTo)
        || has(team, from, to, EdgeKind::Reviews)
        || has(team, to, from, EdgeKind::Reviews)
        || has(team, from, to, EdgeKind::CanMessage)
        || has(team, to, from, EdgeKind::CanMessage)
}

/// `assigner` may give a task to `assignee`.
pub fn can_assign(team: &Team, assigner: &str, assignee: &str) -> bool {
    exists(team, assigner)
        && exists(team, assignee)
        && (team.is_lead(assigner) || has(team, assignee, assigner, EdgeKind::ReportsTo))
}

pub fn can_handoff(team: &Team, from: &str, to: &str) -> bool {
    from != to
        && exists(team, from)
        && exists(team, to)
        && (team.is_lead(from) || team.is_lead(to) || has(team, from, to, EdgeKind::HandsOffTo))
}

pub fn can_request_review(team: &Team, from: &str, to: &str) -> bool {
    from != to
        && exists(team, from)
        && exists(team, to)
        && (team.is_lead(from) || team.is_lead(to) || has(team, from, to, EdgeKind::Reviews))
}

/// The node a specialist reports to: its first reports-to target, else the lead.
pub fn reports_to(team: &Team, node: &str) -> Option<String> {
    team.edges
        .iter()
        .find(|e| e.kind == EdgeKind::ReportsTo && e.from == node)
        .map(|e| e.to.clone())
        .or_else(|| team.lead().map(|l| l.id.clone()))
        .filter(|t| t != node)
}

fn list(set: BTreeSet<String>) -> String {
    set.into_iter().collect::<Vec<_>>().join(", ")
}

/// One Rules-panel line per node, in node order: `(node, "reports to lead · …")`.
pub fn rules_text(team: &Team) -> Vec<(String, String)> {
    let others = |n: &str| -> Vec<String> { team.nodes.iter().filter(|m| m.id != n).map(|m| m.id.clone()).collect() };
    team.nodes
        .iter()
        .map(|n| {
            let id = n.id.as_str();
            let mut parts = Vec::new();
            if n.is_lead {
                parts.push("messages everyone".to_string());
                let assigns: BTreeSet<String> = others(id).into_iter().collect();
                if !assigns.is_empty() {
                    parts.push(format!("assigns to {}", list(assigns)));
                }
                parts.push("merges into the integration branch".into());
            } else {
                let up: BTreeSet<String> = team
                    .edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::ReportsTo && e.from == id)
                    .map(|e| e.to.clone())
                    .collect();
                if !up.is_empty() {
                    parts.push(format!("reports to {}", list(up)));
                }
                let assigns: BTreeSet<String> = others(id).into_iter().filter(|m| can_assign(team, id, m)).collect();
                if !assigns.is_empty() {
                    parts.push(format!("assigns to {}", list(assigns)));
                }
                let hands: BTreeSet<String> = team
                    .edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::HandsOffTo && e.from == id)
                    .map(|e| e.to.clone())
                    .collect();
                if !hands.is_empty() {
                    parts.push(format!("hands off to {}", list(hands)));
                }
                let reviews: BTreeSet<String> = team
                    .edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Reviews && e.from == id)
                    .map(|e| e.to.clone())
                    .collect();
                if !reviews.is_empty() {
                    parts.push(format!("asks reviews from {}", list(reviews)));
                }
                let msg: BTreeSet<String> =
                    others(id).into_iter().filter(|m| can_message(team, id, m) && !team.is_lead(m)).collect();
                if !msg.is_empty() {
                    parts.push(format!("can message {}", list(msg)));
                }
            }
            (n.id.clone(), parts.join(" · "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    #[test]
    fn full_stack_rules() {
        let (_, t) = presets::full_stack();
        assert!(can_message(&t, "back", "lead") && can_message(&t, "lead", "back"));
        assert!(can_message(&t, "back", "front") && can_message(&t, "front", "back"));
        // front reviews ui: both ways.
        assert!(can_message(&t, "front", "ui") && can_message(&t, "ui", "front"));
        // ux hands off to ui: one way.
        assert!(can_message(&t, "ux", "ui") && !can_message(&t, "ui", "ux"));
        assert!(!can_message(&t, "back", "ux"));
        assert!(can_handoff(&t, "ux", "ui") && !can_handoff(&t, "ui", "ux"));
        assert!(can_request_review(&t, "front", "ui") && !can_request_review(&t, "ui", "front"));
        assert!(can_assign(&t, "lead", "ux") && !can_assign(&t, "back", "front"));
        assert!(can_message(&t, USER, "back") && !can_message(&t, "back", USER));
        assert!(!can_message(&t, "back", "back") && !can_message(&t, "ghost", "lead"));
        assert_eq!(reports_to(&t, "back").as_deref(), Some("lead"));
        assert_eq!(reports_to(&t, "lead"), None);
    }

    #[test]
    fn text_lines() {
        let (_, t) = presets::full_stack();
        let lines = rules_text(&t);
        let get = |n: &str| lines.iter().find(|(k, _)| k == n).unwrap().1.clone();
        assert_eq!(
            get("lead"),
            "messages everyone · assigns to back, front, ui, ux · merges into the integration branch"
        );
        assert_eq!(get("front"), "reports to lead · asks reviews from ui · can message back, ui");
        assert_eq!(get("ux"), "reports to lead · hands off to ui · can message ui");
        assert_eq!(get("ui"), "reports to lead · can message front");
    }
}
