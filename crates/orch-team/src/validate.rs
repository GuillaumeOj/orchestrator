//! Inline validation of a team graph (FR-305).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{EdgeKind, RoleDef, Team, valid_slug};

/// One problem, shown inline on the canvas and in the Rules panel. `node`
/// points at the node it belongs to, when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Issue {
    Empty,
    NoLead,
    ManyLeads { nodes: Vec<String> },
    InvalidNodeId { node: String },
    DuplicateNode { node: String },
    UnknownRole { node: String, role: String },
    DanglingEdge { from: String, to: String },
    SelfEdge { node: String },
    DuplicateEdge { from: String, to: String, edge: EdgeKind },
    Unreachable { node: String },
    ReportsToCycle { nodes: Vec<String> },
    ZeroConcurrency { node: String },
    LeadConcurrency { node: String },
}

impl Issue {
    /// The node the issue is drawn on, if any.
    pub fn node(&self) -> Option<&str> {
        match self {
            Issue::InvalidNodeId { node }
            | Issue::DuplicateNode { node }
            | Issue::UnknownRole { node, .. }
            | Issue::SelfEdge { node }
            | Issue::Unreachable { node }
            | Issue::ZeroConcurrency { node }
            | Issue::LeadConcurrency { node } => Some(node),
            Issue::ReportsToCycle { nodes } | Issue::ManyLeads { nodes } => nodes.first().map(String::as_str),
            Issue::DanglingEdge { from, .. } | Issue::DuplicateEdge { from, .. } => Some(from),
            Issue::Empty | Issue::NoLead => None,
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::Empty => write!(f, "the team has no nodes"),
            Issue::NoLead => write!(f, "no lead: mark exactly one node as the lead"),
            Issue::ManyLeads { nodes } => write!(f, "more than one lead: {}", nodes.join(", ")),
            Issue::InvalidNodeId { node } => {
                write!(f, "{node:?} is not a valid node id (a-z, 0-9, - and _, up to 40)")
            }
            Issue::DuplicateNode { node } => write!(f, "two nodes are called {node}"),
            Issue::UnknownRole { node, role } => write!(f, "{node} uses the unknown role {role}"),
            Issue::DanglingEdge { from, to } => write!(f, "an edge {from} → {to} points at a missing node"),
            Issue::SelfEdge { node } => write!(f, "{node} has an edge to itself"),
            Issue::DuplicateEdge { from, to, edge } => write!(f, "{from} → {to} ({edge}) is drawn twice"),
            Issue::Unreachable { node } => write!(f, "{node} is not reachable from the lead"),
            Issue::ReportsToCycle { nodes } => write!(f, "reports-to cycle: {}", nodes.join(" → ")),
            Issue::ZeroConcurrency { node } => write!(f, "{node} has a max concurrency of 0"),
            Issue::LeadConcurrency { node } => write!(f, "the lead {node} runs as one agent (max 1)"),
        }
    }
}

/// Every issue of `team`; empty means the team can be launched.
/// `roles` resolves `role_id`s; pass `None` to skip that check (import).
pub fn validate(team: &Team, roles: Option<&[RoleDef]>) -> Vec<Issue> {
    let mut out = Vec::new();
    if team.nodes.is_empty() {
        out.push(Issue::Empty);
        return out;
    }
    let mut seen = BTreeSet::new();
    for n in &team.nodes {
        if !valid_slug(&n.id) || n.id == "integration" || n.id == "solo" || n.id == "user" {
            out.push(Issue::InvalidNodeId { node: n.id.clone() });
        }
        if !seen.insert(n.id.as_str()) {
            out.push(Issue::DuplicateNode { node: n.id.clone() });
        }
        if let Some(roles) = roles
            && !roles.iter().any(|r| r.id == n.role_id)
        {
            out.push(Issue::UnknownRole { node: n.id.clone(), role: n.role_id.clone() });
        }
        if n.max_concurrency == 0 {
            out.push(Issue::ZeroConcurrency { node: n.id.clone() });
        } else if n.is_lead && n.max_concurrency > 1 {
            out.push(Issue::LeadConcurrency { node: n.id.clone() });
        }
    }
    let leads: Vec<String> = team.nodes.iter().filter(|n| n.is_lead).map(|n| n.id.clone()).collect();
    match leads.len() {
        0 => out.push(Issue::NoLead),
        1 => {}
        _ => out.push(Issue::ManyLeads { nodes: leads.clone() }),
    }
    let mut edges = BTreeSet::new();
    for e in &team.edges {
        if team.node(&e.from).is_none() || team.node(&e.to).is_none() {
            out.push(Issue::DanglingEdge { from: e.from.clone(), to: e.to.clone() });
            continue;
        }
        if e.from == e.to {
            out.push(Issue::SelfEdge { node: e.from.clone() });
            continue;
        }
        // can-message is symmetric: A↔B and B↔A are the same edge.
        let key = if e.kind == EdgeKind::CanMessage && e.to < e.from {
            (e.to.clone(), e.from.clone(), e.kind)
        } else {
            (e.from.clone(), e.to.clone(), e.kind)
        };
        if !edges.insert(key) {
            out.push(Issue::DuplicateEdge { from: e.from.clone(), to: e.to.clone(), edge: e.kind });
        }
    }
    if let [lead] = leads.as_slice() {
        for node in unreachable(team, lead) {
            out.push(Issue::Unreachable { node });
        }
    }
    if let Some(cycle) = reports_to_cycle(team) {
        out.push(Issue::ReportsToCycle { nodes: cycle });
    }
    out
}

/// Nodes with no path to the lead over edges of any type and direction.
fn unreachable(team: &Team, lead: &str) -> Vec<String> {
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &team.edges {
        adj.entry(&e.from).or_default().push(&e.to);
        adj.entry(&e.to).or_default().push(&e.from);
    }
    let mut seen = BTreeSet::from([lead]);
    let mut stack = vec![lead];
    while let Some(n) = stack.pop() {
        for &m in adj.get(n).into_iter().flatten() {
            if seen.insert(m) {
                stack.push(m);
            }
        }
    }
    team.nodes.iter().filter(|n| !seen.contains(n.id.as_str())).map(|n| n.id.clone()).collect()
}

/// The first reports-to cycle found, closed (`[a, b, a]`), if any.
fn reports_to_cycle(team: &Team) -> Option<Vec<String>> {
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in team.edges.iter().filter(|e| e.kind == EdgeKind::ReportsTo && e.from != e.to) {
        next.entry(&e.from).or_default().push(&e.to);
    }
    // 0 = unvisited, 1 = on the stack, 2 = done.
    let mut mark: BTreeMap<&str, u8> = BTreeMap::new();
    fn dfs<'a>(
        n: &'a str,
        next: &BTreeMap<&'a str, Vec<&'a str>>,
        mark: &mut BTreeMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        mark.insert(n, 1);
        path.push(n);
        for &m in next.get(n).into_iter().flatten() {
            match mark.get(m).copied().unwrap_or(0) {
                1 => {
                    let start = path.iter().position(|&p| p == m).unwrap_or(0);
                    let mut cycle: Vec<String> = path[start..].iter().map(|s| s.to_string()).collect();
                    cycle.push(m.to_string());
                    return Some(cycle);
                }
                0 => {
                    if let Some(c) = dfs(m, next, mark, path) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
        path.pop();
        mark.insert(n, 2);
        None
    }
    for n in &team.nodes {
        if mark.get(n.id.as_str()).copied().unwrap_or(0) == 0
            && let Some(c) = dfs(&n.id, &next, &mut mark, &mut Vec::new())
        {
            return Some(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentNode, Edge, RoleOverrides, Scope};

    fn node(id: &str, lead: bool) -> AgentNode {
        AgentNode {
            id: id.into(),
            role_id: id.into(),
            overrides: RoleOverrides::default(),
            position: (0.0, 0.0),
            max_concurrency: 1,
            is_lead: lead,
        }
    }

    fn team(nodes: Vec<AgentNode>, edges: Vec<Edge>) -> Team {
        Team { id: "t".into(), name: "t".into(), scope: Scope::Global, nodes, edges, version: 1 }
    }

    #[test]
    fn a_valid_team_has_no_issues() {
        let t = team(
            vec![node("lead", true), node("back", false), node("front", false)],
            vec![
                Edge::new("back", "lead", EdgeKind::ReportsTo),
                Edge::new("front", "lead", EdgeKind::ReportsTo),
                Edge::new("back", "front", EdgeKind::CanMessage),
            ],
        );
        assert_eq!(validate(&t, None), vec![]);
    }

    #[test]
    fn leads_are_counted() {
        let t = team(vec![node("a", false)], vec![]);
        assert_eq!(validate(&t, None), vec![Issue::NoLead]);
        let t = team(vec![node("a", true), node("b", true)], vec![Edge::new("a", "b", EdgeKind::CanMessage)]);
        assert_eq!(validate(&t, None), vec![Issue::ManyLeads { nodes: vec!["a".into(), "b".into()] }]);
        assert_eq!(validate(&team(vec![], vec![]), None), vec![Issue::Empty]);
    }

    #[test]
    fn unreachable_nodes_and_dangling_edges() {
        let t = team(
            vec![node("lead", true), node("back", false), node("ux", false)],
            vec![Edge::new("back", "lead", EdgeKind::ReportsTo), Edge::new("ui", "lead", EdgeKind::ReportsTo)],
        );
        let issues = validate(&t, None);
        assert!(issues.contains(&Issue::Unreachable { node: "ux".into() }), "{issues:?}");
        assert!(issues.contains(&Issue::DanglingEdge { from: "ui".into(), to: "lead".into() }));
        assert_eq!(Issue::Unreachable { node: "ux".into() }.to_string(), "ux is not reachable from the lead");
    }

    #[test]
    fn reports_to_cycles_are_found() {
        let t = team(
            vec![node("lead", true), node("ui", false), node("ux", false)],
            vec![
                Edge::new("ui", "ux", EdgeKind::ReportsTo),
                Edge::new("ux", "ui", EdgeKind::ReportsTo),
                Edge::new("ui", "lead", EdgeKind::HandsOffTo),
            ],
        );
        let issues = validate(&t, None);
        assert!(
            issues.contains(&Issue::ReportsToCycle { nodes: vec!["ui".into(), "ux".into(), "ui".into()] }),
            "{issues:?}"
        );
        // Other edge types may form cycles.
        let t = team(
            vec![node("lead", true), node("a", false)],
            vec![Edge::new("a", "lead", EdgeKind::Reviews), Edge::new("lead", "a", EdgeKind::Reviews)],
        );
        assert_eq!(validate(&t, None), vec![]);
    }

    #[test]
    fn node_ids_roles_and_concurrency() {
        let mut lead = node("lead", true);
        lead.max_concurrency = 2;
        let mut z = node("z", false);
        z.max_concurrency = 0;
        let t = team(
            vec![lead, node("Bad Id", false), node("integration", false), z, node("lead", false)],
            vec![Edge::new("lead", "lead", EdgeKind::ReportsTo)],
        );
        let issues = validate(&t, Some(&[RoleDef::new("lead", "Lead")]));
        for want in [
            Issue::LeadConcurrency { node: "lead".into() },
            Issue::InvalidNodeId { node: "Bad Id".into() },
            Issue::InvalidNodeId { node: "integration".into() },
            Issue::ZeroConcurrency { node: "z".into() },
            Issue::DuplicateNode { node: "lead".into() },
            Issue::UnknownRole { node: "z".into(), role: "z".into() },
            Issue::SelfEdge { node: "lead".into() },
        ] {
            assert!(issues.contains(&want), "missing {want:?} in {issues:?}");
        }
    }

    #[test]
    fn duplicate_edges_and_symmetric_messages() {
        let t = team(
            vec![node("lead", true), node("a", false)],
            vec![Edge::new("a", "lead", EdgeKind::CanMessage), Edge::new("lead", "a", EdgeKind::CanMessage)],
        );
        assert_eq!(
            validate(&t, None),
            vec![Issue::DuplicateEdge { from: "lead".into(), to: "a".into(), edge: EdgeKind::CanMessage }]
        );
    }
}
