use orch_protocol::InboxDecision;
use orch_store::{PrMode, PrStatus, RunPr, Task};
use orch_team::RunState;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::team::LaunchSpec;
use crate::team::tests::{call, ok, queued, setup, write_commit};

const LEAD: &str = "fx-lead-1";

fn spec(pid: &str, issues: &[&str], mode: PrMode) -> LaunchSpec {
    LaunchSpec {
        project_id: pid.into(),
        team_id: "bugfix-squad".into(),
        slug: "fx".into(),
        title: "Fix X".into(),
        issues: issues.iter().map(|s| s.to_string()).collect(),
        pr_mode: Some(mode),
        ..Default::default()
    }
}

/// A launched run whose spec is approved.
fn approved(rt: &Runtime, pid: &str, issues: &[&str], mode: PrMode) -> orch_store::TeamRun {
    let run = rt.launch(&spec(pid, issues, mode)).unwrap();
    write_commit(rt, LEAD, ".orchestrator/features/fx/spec.md", "# Fix X\n");
    ok(rt, LEAD, "request_spec_approval", json!({"summary": "s"}));
    let gate = rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    run
}

fn code(e: anyhow::Error) -> String {
    serde_json::from_str::<Value>(&e.to_string()).map(|v| v["code"].as_str().unwrap_or("").to_string()).unwrap_or_else(|_| e.to_string())
}

fn create(rt: &Runtime, args: Value) -> Task {
    let (err, text, data) = rt.on_mcp(
        &orch_protocol::McpMsg { agent_id: LEAD.into(), tool: "create_task".into(), arguments: args, deadline_ms: 1 },
        None,
    );
    assert!(!err, "{text}");
    serde_json::from_value(data).unwrap()
}

#[test]
fn launch_validates_and_stores_the_issues_and_mode() {
    let (_d, rt, _rd, _repo, pid) = setup();
    assert_eq!(code(rt.launch(&spec(&pid, &["x/y#1"], PrMode::Single)).unwrap_err()), "bad_ref");
    assert_eq!(code(rt.launch(&spec(&pid, &["#1"], PrMode::PerIssue)).unwrap_err()), "per_issue_needs_two");
    let many: Vec<String> = (1..=11).map(|n| n.to_string()).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(code(rt.launch(&spec(&pid, &refs, PrMode::Single)).unwrap_err()), "too_many");
    assert!(rt.db.feature_by_slug(&pid, "fx").unwrap().is_none(), "nothing created");
    let run = rt.launch(&spec(&pid, &["1", "#1", "eng-7"], PrMode::PerIssue)).unwrap();
    let f = rt.db.feature(&run.feature_id).unwrap().unwrap();
    assert_eq!((f.issue_refs(), f.pr_mode), (vec!["#1".to_string(), "ENG-7".to_string()], PrMode::PerIssue));
    assert_eq!(f.issues[1].key, "eng-7");
    let prompt = rt.store.read(|s| s.agents[LEAD].role.prompt.clone()).unwrap_or_default();
    assert!(prompt.contains("Pull requests: One PR per issue: #1, ENG-7."), "{prompt}");
    let v = rt.run_json(&run.id).unwrap();
    assert_eq!(v["run"]["prs_view"].as_array().unwrap().len(), 2);
    assert_eq!(v["run"]["prs_view"][0]["state"], "none");
    assert_eq!(v["feature"]["issues"][0]["ref"], "#1");
}
