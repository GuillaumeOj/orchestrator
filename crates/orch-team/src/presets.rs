//! Starter roles and team presets (FR-306).

use governor::Tier;
use serde_json::Value;

use crate::model::{AgentNode, Edge, EdgeKind, ReviewMode, RoleDef, RoleOverrides, Scope, Team};

/// The starters' version: bumped whenever a starter role or team changes, so
/// installed unmodified copies are upgraded in place (FR-306). 1 = P2,
/// 2 = P3 design hand-offs, 3 = auto mode by default, the reviewer's
/// `submit_review` and the reviewed full-stack preset (v0.11).
pub const STARTER_VERSION: u32 = 3;

/// JSON without the values a new field with a serde default would add
/// (`null`, `false`, `0`, `""`, `[]`, `{}`), so a fingerprint survives
/// fields added later.
fn canonical(v: Value) -> Option<Value> {
    match v {
        Value::Null | Value::Bool(false) => None,
        Value::Number(n) if n.as_f64() == Some(0.0) => None,
        Value::String(s) if s.is_empty() => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Array(a) => Some(Value::Array(a.into_iter().map(|x| canonical(x).unwrap_or(Value::Null)).collect())),
        Value::Object(m) => {
            let m: serde_json::Map<String, Value> =
                m.into_iter().filter_map(|(k, v)| canonical(v).map(|v| (k, v))).collect();
            (!m.is_empty()).then_some(Value::Object(m))
        }
        other => Some(other),
    }
}

/// FNV-1a 64 of the canonical JSON (stable across Rust versions, unlike `Hash`).
fn fingerprint(v: Value) -> String {
    let mut keys_sorted = canonical(v).unwrap_or(Value::Null);
    sort_keys(&mut keys_sorted);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in keys_sorted.to_string().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Object keys in order whatever the map type (`preserve_order` may be on).
fn sort_keys(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(m).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, mut x) in entries {
                sort_keys(&mut x);
                m.insert(k, x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

/// What a role says, without where it lives, its version or its origin.
pub fn role_fingerprint(r: &RoleDef) -> String {
    let mut r = r.clone();
    (r.version, r.scope, r.origin) = (0, Scope::Global, None);
    fingerprint(serde_json::to_value(&r).unwrap_or(Value::Null))
}

/// What a team says, without where it lives, its version, preset flag or origin.
/// The default review mode is left out, like a default added later: teams
/// saved before it existed read back with it.
pub fn team_fingerprint(t: &Team) -> String {
    let mut t = t.clone();
    (t.version, t.scope, t.preset, t.origin) = (0, Scope::Global, false, None);
    let mut v = serde_json::to_value(&t).unwrap_or(Value::Null);
    if t.review == ReviewMode::default()
        && let Value::Object(m) = &mut v
    {
        m.remove("review");
    }
    fingerprint(v)
}

fn role(id: &str, name: &str, color: u8, prompt: &str, tier: Tier) -> RoleDef {
    let mut r = RoleDef::new(id, name);
    r.color = Some(color);
    r.prompt = prompt.into();
    r.budget_priority = tier;
    r
}

/// Fingerprints of the P2 starters (version 1), for the copies installed
/// before starters recorded their origin: such a copy is unmodified when its
/// fingerprint is the P2 one.
pub const V1_FINGERPRINTS: &[(&str, &str)] = &[
    ("role:lead", "707e399771addec1"),
    ("role:backend", "fbe8ced57f656896"),
    ("role:frontend", "27236530990b6a78"),
    ("role:ui", "8ce094a453780e60"),
    ("role:ux", "a3863e2957920d3e"),
    ("role:reviewer", "8d02ecd0d945b9a6"),
    ("team:solo-dev", "aa0064957110f684"),
    ("team:full-stack", "589ebaa5088655af"),
    ("team:bugfix-squad", "de2c502284384fca"),
    ("team:design-sprint", "fab3a22dc3301e6d"),
];

/// The design hand-off convention (spec §3.2, D9), in the UX and UI prompts.
pub const DESIGN_EXPORTS: &str = "Design hand-offs: always export the frames you hand off as images under `design/` \
     in the feature's docs folder (`<docs folder>design/<name>.png`, or `.svg`), in your worktree, and commit them with \
     mcp__orch__commit. Then call mcp__orch__handoff with `artifact` = that repo path, `kind` = \"design\" and `to` = \
     the node you hand designs to (frontend when the team has one), one call per exported frame; put the Penpot or \
     Figma frame link in the `note`. Always export: a link-only hand-off can't be read by the other agents.";

/// Lead, Backend, Frontend, UI, UX, Reviewer (colors in the order of the
/// design tokens `color.role.1…`).
pub fn starter_roles() -> Vec<RoleDef> {
    let mut lead = role(
        "lead",
        "Lead",
        1,
        "You are the lead of this feature team. You own the integration branch and its worktree. When the team has \
         designers (UX, UI): make the frontend task depend on the design task (`depends_on`), merge the designer's \
         branch into the integration branch before the frontend task starts, and tell frontend to call \
         mcp__orch__sync_integration and read the exported images under the feature's `design/` folder.",
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
    ui.prompt = format!("{}\n\n{DESIGN_EXPORTS}", ui.prompt);
    ui.tools.web = true;
    ui.mcp_refs = vec!["penpot".into()];
    let mut ux = role(
        "ux",
        "UX",
        5,
        "You are the UX designer. You write the flows and the UX brief, and hand them to the UI designer.",
        Tier::Normal,
    );
    ux.prompt = format!("{}\n\n{DESIGN_EXPORTS}", ux.prompt);
    ux.tools.web = true;
    ux.mcp_refs = vec!["penpot".into()];
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
            "You are the reviewer. You review the branches you are asked to review and send concrete findings \
             (file:line, the problem, the fix) with mcp__orch__submit_review; you do not change them yourself.",
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
        preset: true,
        origin: None,
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
            // D2: the design → code flow (J12) hands frames straight to frontend.
            Edge::new("ui", "front", HandsOffTo),
            Edge::new("front", "ui", Reviews),
            Edge::new("back", "front", CanMessage),
        ],
    ))
}

/// The full-stack team with a reviewer node: every task is reviewed by it
/// before the lead merges (review mode `node`, ADR-010).
pub fn full_stack_reviewed() -> (Vec<RoleDef>, Team) {
    use EdgeKind::*;
    let (_, mut t) = full_stack();
    t.id = "full-stack-reviewed".into();
    t.name = "Full-stack team with a reviewer".into();
    t.review = crate::model::ReviewMode::Node;
    t.nodes.push(node("review", "reviewer", 800.0, 240.0, false));
    // The reviewer node reviews everyone (front no longer asks ui).
    t.edges.retain(|e| e.kind != Reviews);
    for n in ["back", "front", "ui", "ux"] {
        t.edges.push(Edge::new(n, "review", Reviews));
    }
    t.edges.push(Edge::new("review", "lead", ReportsTo));
    with_roles(t)
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
    vec![solo_dev(), full_stack(), full_stack_reviewed(), bugfix_squad(), design_sprint()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    #[test]
    fn fingerprints_ignore_place_version_origin_and_new_defaults() {
        let (roles, t) = full_stack();
        let mut t2 = t.clone();
        (t2.version, t2.scope, t2.preset) = (7, Scope::Project("p".into()), false);
        t2.origin = Some(crate::StarterOrigin { starter: "x".into(), version: 1, fingerprint: "f".into() });
        assert_eq!(team_fingerprint(&t), team_fingerprint(&t2));
        t2.nodes[0].position = (1.0, 1.0);
        assert_ne!(team_fingerprint(&t), team_fingerprint(&t2), "a moved node is a change");
        let ui = roles.iter().find(|r| r.id == "ui").unwrap();
        let mut ui2 = ui.clone();
        ui2.version = 4;
        assert_eq!(role_fingerprint(ui), role_fingerprint(&ui2));
        ui2.prompt.push('!');
        assert_ne!(role_fingerprint(ui), role_fingerprint(&ui2));
        // Starters that didn't change since P2 keep their v1 fingerprint.
        let v1 = |k: &str| V1_FINGERPRINTS.iter().find(|(id, _)| *id == k).unwrap().1;
        let mut backend = starter_roles().into_iter().find(|r| r.id == "backend").unwrap();
        assert_ne!(role_fingerprint(&backend), v1("role:backend"), "auto mode by default in v3");
        backend.permission_profile = crate::PermissionProfile::Gated;
        assert_eq!(role_fingerprint(&backend), v1("role:backend"));
        assert_ne!(role_fingerprint(ui), v1("role:ui"), "the UI prompt changed in v2");
        assert_ne!(team_fingerprint(&t), v1("team:full-stack"), "D2 edge");
        assert_eq!(team_fingerprint(&bugfix_squad().1), v1("team:bugfix-squad"), "default review mode left out");
        t2 = t.clone();
        t2.review = ReviewMode::Node;
        assert_ne!(team_fingerprint(&t), team_fingerprint(&t2));
    }

    #[test]
    fn design_starters_use_penpot_and_ui_hands_off_to_front() {
        let (roles, t) = full_stack();
        for id in ["ui", "ux"] {
            let r = roles.iter().find(|r| r.id == id).unwrap();
            assert_eq!(r.mcp_refs, ["penpot"]);
            assert!(r.prompt.contains("design/<name>.png") && r.prompt.contains("kind` = \"design\""));
            assert!(r.prompt.contains("link-only hand-off can't be read"));
        }
        assert!(crate::rules::can_handoff(&t, "ui", "front"));
        assert!(design_sprint().1.preset && t.preset);
    }

    #[test]
    fn every_preset_is_valid_and_carries_its_roles() {
        for (roles, t) in all() {
            assert_eq!(validate(&t, Some(&roles)), vec![], "{}", t.id);
            assert!(roles.iter().all(|r| t.nodes.iter().any(|n| n.role_id == r.id)));
        }
    }
}
