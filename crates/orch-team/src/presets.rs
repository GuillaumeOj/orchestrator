//! Starter roles and team presets (FR-306).

use governor::Tier;

use crate::model::{AgentNode, Edge, EdgeKind, RoleDef, RoleOverrides, Scope, Team};

fn role(id: &str, name: &str, color: u8, prompt: &str, tier: Tier) -> RoleDef {
    let mut r = RoleDef::new(id, name);
    r.color = Some(color);
    r.prompt = prompt.into();
    r.budget_priority = tier;
    r
}

/// Lead, Backend, Frontend, UI, UX, Reviewer (colors in the order of the
/// design tokens `color.role.1…`).
pub fn starter_roles() -> Vec<RoleDef> {
    let mut lead = role(
        "lead",
        "Lead",
        1,
        "You are the lead of this feature team. You own the integration branch and its worktree.",
        Tier::High,
    );
    lead.model = Some("opus".into());
    lead.effort = Some("high".into());
    let mut ui = role(
        "ui",
        "UI",
        4,
        "You are the UI designer. You turn the UX brief into screens in the design tool and hand the frames to the \
         frontend.",
        Tier::Normal,
    );
    ui.tools.web = true;
    let mut ux = role(
        "ux",
        "UX",
        5,
        "You are the UX designer. You write the flows and the UX brief, and hand them to the UI designer.",
        Tier::Normal,
    );
    ux.tools.web = true;
    vec![
        lead,
        role(
            "backend",
            "Backend",
            2,
            "You are the backend engineer. You implement the server side of your tasks, with tests.",
            Tier::Normal,
        ),
        role(
            "frontend",
            "Frontend",
            3,
            "You are the frontend engineer. You implement the client side of your tasks, with tests, from the API \
             contract and the design hand-offs.",
            Tier::Normal,
        ),
        ui,
        ux,
        role(
            "reviewer",
            "Reviewer",
            7,
            "You are the reviewer. You review the branches you are asked to review and reply with concrete findings; \
             you do not change them yourself.",
            Tier::Normal,
        ),
    ]
}

fn node(id: &str, role: &str, x: f64, y: f64, lead: bool) -> AgentNode {
    AgentNode {
        id: id.into(),
        role_id: role.into(),
        overrides: RoleOverrides::default(),
        position: (x, y),
        max_concurrency: 1,
        is_lead: lead,
    }
}

fn team(id: &str, name: &str, nodes: Vec<AgentNode>, edges: Vec<Edge>) -> Team {
    Team {
        id: id.into(),
        name: name.into(),
        scope: Scope::Global,
        nodes,
        edges,
        version: 1,
        review: Default::default(),
    }
}

fn with_roles(t: Team) -> (Vec<RoleDef>, Team) {
    let roles = starter_roles().into_iter().filter(|r| t.nodes.iter().any(|n| n.role_id == r.id)).collect();
    (roles, t)
}

pub fn solo_dev() -> (Vec<RoleDef>, Team) {
    with_roles(team("solo-dev", "Solo dev", vec![node("lead", "lead", 0.0, 0.0, true)], vec![]))
}

pub fn full_stack() -> (Vec<RoleDef>, Team) {
    use EdgeKind::*;
    with_roles(team(
        "full-stack",
        "Full-stack feature team",
        vec![
            node("lead", "lead", 280.0, 0.0, true),
            node("back", "backend", 0.0, 240.0, false),
            node("front", "frontend", 220.0, 240.0, false),
            node("ui", "ui", 440.0, 240.0, false),
            node("ux", "ux", 620.0, 370.0, false),
        ],
        vec![
            Edge::new("back", "lead", ReportsTo),
            Edge::new("front", "lead", ReportsTo),
            Edge::new("ui", "lead", ReportsTo),
            Edge::new("ux", "lead", ReportsTo),
            Edge::new("ux", "ui", HandsOffTo),
            Edge::new("front", "ui", Reviews),
            Edge::new("back", "front", CanMessage),
        ],
    ))
}

pub fn bugfix_squad() -> (Vec<RoleDef>, Team) {
    use EdgeKind::*;
    with_roles(team(
        "bugfix-squad",
        "Bugfix squad",
        vec![
            node("lead", "lead", 110.0, 0.0, true),
            node("back", "backend", 0.0, 240.0, false),
            node("front", "frontend", 220.0, 240.0, false),
        ],
        vec![
            Edge::new("back", "lead", ReportsTo),
            Edge::new("front", "lead", ReportsTo),
            Edge::new("back", "front", Reviews),
            Edge::new("front", "back", Reviews),
        ],
    ))
}

pub fn design_sprint() -> (Vec<RoleDef>, Team) {
    use EdgeKind::*;
    with_roles(team(
        "design-sprint",
        "Design sprint",
        vec![
            node("lead", "lead", 110.0, 0.0, true),
            node("ux", "ux", 0.0, 240.0, false),
            node("ui", "ui", 220.0, 240.0, false),
        ],
        vec![Edge::new("ux", "lead", ReportsTo), Edge::new("ui", "lead", ReportsTo), Edge::new("ux", "ui", HandsOffTo)],
    ))
}

/// Every starter preset, in library order.
pub fn all() -> Vec<(Vec<RoleDef>, Team)> {
    vec![solo_dev(), full_stack(), bugfix_squad(), design_sprint()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    #[test]
    fn every_preset_is_valid_and_carries_its_roles() {
        for (roles, t) in all() {
            assert_eq!(validate(&t, Some(&roles)), vec![], "{}", t.id);
            assert!(roles.iter().all(|r| t.nodes.iter().any(|n| n.role_id == r.id)));
        }
    }
}
