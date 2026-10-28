//! Properties of validation and rules over random graphs.

use orch_team::rules::{can_assign, can_message, can_request_review};
use orch_team::{AgentNode, Edge, EdgeKind, RoleOverrides, Scope, Team, validate};
use proptest::prelude::*;

const IDS: [&str; 5] = ["lead", "a", "b", "c", "d"];

fn arb_team() -> impl Strategy<Value = Team> {
    let edge = (0..5usize, 0..5usize, 0..4usize).prop_map(|(f, t, k)| Edge::new(IDS[f], IDS[t], EdgeKind::ALL[k]));
    (1..=5usize, proptest::collection::vec(edge, 0..12)).prop_map(|(n, edges)| Team {
        id: "t".into(),
        name: "t".into(),
        scope: Scope::Global,
        review: Default::default(),
        nodes: IDS[..n]
            .iter()
            .enumerate()
            .map(|(i, id)| AgentNode {
                id: id.to_string(),
                role_id: "r".into(),
                overrides: RoleOverrides::default(),
                position: (0.0, 0.0),
                max_concurrency: 1,
                is_lead: i == 0,
            })
            .collect(),
        edges,
        version: 1,
        preset: false,
        origin: None,
    })
}

proptest! {
    #[test]
    fn rules_are_consistent(team in arb_team()) {
        let _ = validate(&team, None);
        for x in &team.nodes {
            for y in &team.nodes {
                let (a, b) = (x.id.as_str(), y.id.as_str());
                if a == b {
                    prop_assert!(!can_message(&team, a, b));
                    continue;
                }
                // can-message and reviews edges talk both ways.
                let both_ways = team.edges.iter().any(|e| {
                    matches!(e.kind, EdgeKind::CanMessage | EdgeKind::Reviews | EdgeKind::ReportsTo)
                        && ((e.from == a && e.to == b) || (e.from == b && e.to == a))
                });
                if both_ways {
                    prop_assert!(can_message(&team, a, b) && can_message(&team, b, a));
                }
                // Whoever may assign or ask a review may also message.
                if can_assign(&team, a, b) || can_request_review(&team, a, b) {
                    prop_assert!(can_message(&team, a, b));
                }
                // The lead talks to everyone, everyone to the lead.
                if team.is_lead(a) || team.is_lead(b) {
                    prop_assert!(can_message(&team, a, b));
                }
            }
        }
    }

    #[test]
    fn edges_only_to_the_lead_are_always_valid(n in 1..=5usize) {
        let mut team = arb_team_fixed(n);
        team.edges = IDS[1..n].iter().map(|id| Edge::new(id, "lead", EdgeKind::ReportsTo)).collect();
        prop_assert_eq!(validate(&team, None), vec![]);
    }
}

fn arb_team_fixed(n: usize) -> Team {
    Team {
        id: "t".into(),
        name: "t".into(),
        scope: Scope::Global,
        review: Default::default(),
        nodes: IDS[..n]
            .iter()
            .enumerate()
            .map(|(i, id)| AgentNode {
                id: id.to_string(),
                role_id: "r".into(),
                overrides: RoleOverrides::default(),
                position: (0.0, 0.0),
                max_concurrency: 1,
                is_lead: i == 0,
            })
            .collect(),
        edges: vec![],
        version: 1,
        preset: false,
        origin: None,
    }
}
