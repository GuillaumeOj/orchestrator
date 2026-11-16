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
    serde_json::from_str::<Value>(&e.to_string())
        .map(|v| v["code"].as_str().unwrap_or("").to_string())
        .unwrap_or_else(|_| e.to_string())
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

#[test]
fn feature_set_prs_follows_the_rules_and_tells_the_lead() {
    let (_d, rt, _rd, _repo, pid) = setup();
    let run = approved(&rt, &pid, &["#1"], PrMode::Single);
    let t = create(&rt, json!({"title": "API", "node": "back"}));
    let mut done = rt.db.task(t.id).unwrap().unwrap();
    done.status = orch_team::TaskStatus::Done;
    rt.db.upsert_task(&done).unwrap();
    let set = |issues: Option<&[&str]>, mode: Option<PrMode>| {
        rt.feature_set_prs("fx", issues.map(|l| l.iter().map(|s| s.to_string()).collect()), mode)
    };
    assert_eq!(code(set(None, Some(PrMode::PerIssue)).unwrap_err()), "per_issue_needs_two");
    assert_eq!(code(set(Some(&["nope"]), None).unwrap_err()), "bad_ref");
    // Two issues and one PR per issue: the lead is told, with the task to tag.
    let f = set(Some(&["#1", "#2", "2"]), Some(PrMode::PerIssue)).unwrap();
    assert_eq!((f["pr_mode"].as_str(), f["issues"].as_array().unwrap().len()), (Some("per_issue"), 2));
    let turn = queued(&rt, LEAD).into_iter().find(|b| b.contains("changed the pull requests")).unwrap();
    assert!(turn.contains("One PR per issue: #1, #2."), "{turn}");
    assert!(turn.contains(&format!("{} (id {}, API)", t.handle(), t.id)), "{turn}");
    rt.db.flush().unwrap();
    let ev = rt.db.run_events(&run.id, 100).unwrap();
    let e = ev.iter().find(|e| e.kind == "feature.prs").unwrap();
    assert_eq!(e.detail, json!({"issues": ["#1", "#2"], "pr_mode": "per_issue", "by": "user"}));
    // An issue with an open PR stays; the mode is locked once a PR exists.
    let mut r = rt.db.run(&run.id).unwrap().unwrap();
    r.prs = vec![RunPr {
        issue: "#1".into(),
        branch: "orch/fx/issue/1".into(),
        number: 7,
        url: "u".into(),
        pr: Some(PrStatus { state: "open".into(), ..Default::default() }),
        ..Default::default()
    }];
    rt.db.upsert_run(&r).unwrap();
    let e = set(Some(&["#2"]), None).unwrap_err();
    assert!(e.to_string().contains("\"ref\":\"#1\""), "{e}");
    assert_eq!(code(e), "issue_has_pr");
    assert_eq!(code(set(None, Some(PrMode::Single)).unwrap_err()), "pr_busy");
    // Down to one issue is fine once a PR exists (the locked one stays).
    let f = set(Some(&["#1"]), None).unwrap();
    assert_eq!(f["issues"].as_array().unwrap().len(), 1);
    assert!(set(Some(&["#1", "#3"]), None).is_ok(), "adding an issue is fine");
    // A done run is finished.
    let mut r = rt.db.run(&run.id).unwrap().unwrap();
    r.status = RunState::Done;
    rt.db.upsert_run(&r).unwrap();
    assert_eq!(code(set(Some(&["#1"]), None).unwrap_err()), "finished");
}

#[test]
fn issue_suggest_reads_the_intake_cache() {
    let (_d, rt, _rd, _repo, pid) = setup();
    assert_eq!(rt.issue_suggest(&pid, "x").unwrap(), json!([]));
}

fn head_message(rt: &Runtime, agent: &str) -> String {
    let wt = rt.store.read(|s| s.agents[agent].worktree.clone());
    orch_git::git(&wt, &["log", "-1", "--format=%B"]).unwrap()
}

/// `git interpret-trailers --parse` of a message.
fn trailers(msg: &str) -> String {
    use std::io::Write;
    let mut c = std::process::Command::new("git")
        .args(["interpret-trailers", "--parse"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(msg.as_bytes()).unwrap();
    String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()
}

#[test]
fn tasks_and_lead_commits_carry_a_feature_issue() {
    let (_d, rt, _rd, _repo, pid) = setup();
    approved(&rt, &pid, &["#1", "ENG-2"], PrMode::Single);
    let t = create(&rt, json!({"title": "API", "node": "back", "issue": "1"}));
    assert_eq!(t.issue.as_deref(), Some("#1"));
    assert!(
        call(&rt, LEAD, "create_task", json!({"title": "x", "node": "back", "issue": "#9"}))
            .1
            .contains("not an issue of this feature")
    );
    ok(&rt, LEAD, "update_task", json!({"id": t.id, "issue": "eng-2"}));
    assert_eq!(rt.db.task(t.id).unwrap().unwrap().issue.as_deref(), Some("ENG-2"));
    ok(&rt, LEAD, "update_task", json!({"id": t.id, "issue": ""}));
    assert_eq!(rt.db.task(t.id).unwrap().unwrap().issue, None, "cleared");
    // The lead's commit gets the trailer; a ref not on the feature is refused.
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    std::fs::write(wt.join("docs.md"), "x").unwrap();
    assert!(
        call(&rt, LEAD, "commit", json!({"message": "docs", "issue": "#5"})).1.contains("not an issue of this feature")
    );
    ok(&rt, LEAD, "commit", json!({"message": "docs", "issue": "#1"}));
    let msg = head_message(&rt, LEAD);
    assert!(trailers(&msg).lines().any(|l| l == "Orch-Issue: #1"), "{msg:?}");
    // A specialist does not tag commits or tasks with an issue.
    rt.team_tick();
    let back = "fx-back-1";
    let bwt = rt.store.read(|s| s.agents[back].worktree.clone());
    std::fs::write(bwt.join("a.rs"), "a").unwrap();
    assert!(call(&rt, back, "commit", json!({"message": "a", "issue": "#1"})).1.contains("only the lead"));
    assert!(call(&rt, back, "update_task", json!({"id": t.id, "issue": "#1"})).1.contains("only the lead"));
}

#[test]
fn request_pr_in_single_mode_closes_the_issues_and_locks_the_mode() {
    let (_d, rt, _rd, _repo, pid) = setup();
    let run = approved(&rt, &pid, &["#1", "#2"], PrMode::Single);
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    assert!(call(&rt, LEAD, "request_pr", json!({"title": "T", "issue": "#1"})).1.contains("this feature has one PR"));
    ok(&rt, LEAD, "request_pr", json!({"title": "T", "body": "Fixes #1"}));
    let gate = rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
    assert_eq!(gate.payload["body"], "Fixes #1\n\nCloses #2");
    assert_eq!((gate.payload["run"].as_str(), gate.payload["issue"].clone()), (Some(run.id.as_str()), Value::Null));
    let v = rt.run_json(&run.id).unwrap();
    assert_eq!(
        v["run"]["prs_view"],
        json!([{"issue": null, "branch": "orch/fx/integration", "state": "pending",
        "number": null, "url": null, "pr": null, "gate": gate.id}])
    );
    // team_status names the issues and their PR.
    let (err, text, data) = rt.on_mcp(
        &orch_protocol::McpMsg {
            agent_id: LEAD.into(),
            tool: "team_status".into(),
            arguments: json!({}),
            deadline_ms: 1,
        },
        None,
    );
    assert!(!err, "{text}");
    assert!(
        text.contains("Pull requests: One PR for the feature; it closes #1, #2.")
            && text.contains("- #2: no task · PR waiting for the user"),
        "{text}"
    );
    assert_eq!(data["pr_mode"], "single");
    assert_eq!(data["issues"][0], json!({"ref": "#1", "tasks": [], "pr": {"number": null, "state": "pending"}}));
    assert!(data["members"].is_array());
    // Declined: the mode stays locked (a PR was asked for).
    rt.decide(gate.id, InboxDecision::Reject, None, Some("not yet")).unwrap();
    assert_eq!(code(rt.feature_set_prs("fx", None, Some(PrMode::PerIssue)).unwrap_err()), "pr_busy");
}

#[test]
fn request_pr_per_issue_is_refused_until_per_issue_prs_exist() {
    let (_d, rt, _rd, _repo, pid) = setup();
    let run = approved(&rt, &pid, &["#1", "#2"], PrMode::PerIssue);
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    let (err, text) = call(&rt, LEAD, "request_pr", json!({"title": "T", "issue": "#1"}));
    assert!(err && text.contains("cannot open per-issue PRs yet"), "{text}");
    assert!(rt.store.read(|s| !s.inbox.iter().any(|i| i.pending() && i.kind == "gate")));
}

/// `back` does task `t` (one file), reports it (with its self-review).
fn report(rt: &Runtime, file: &str) {
    let back = "fx-back-1";
    write_commit(rt, back, file, file);
    ok(rt, back, "report_done", json!({"summary": "s"}));
    ok(rt, back, "report_done", json!({"summary": "s", "review": {"fixed": [], "unfixed": []}}));
}

#[test]
fn per_issue_merges_take_the_reported_head_with_a_task_trailer() {
    let (_d, rt, _rd, _repo, pid) = setup();
    approved(&rt, &pid, &["#1", "#2"], PrMode::PerIssue);
    let t = create(&rt, json!({"title": "API", "node": "back", "issue": "#1"}));
    rt.team_tick();
    report(&rt, "a.rs");
    let reported = rt.db.task(t.id).unwrap().unwrap().reported_sha.unwrap();
    let bwt = rt.store.read(|s| s.agents["fx-back-1"].worktree.clone());
    assert_eq!(orch_git::git(&bwt, &["rev-parse", "HEAD"]).unwrap().trim(), reported);
    // The owner moves on before the merge.
    write_commit(&rt, "fx-back-1", "b.rs", "b");
    let e = call(&rt, LEAD, "merge", json!({"branch": "orch/fx/back-1"})).1;
    assert!(e.contains("`task` is required"), "{e}");
    let e = call(&rt, LEAD, "merge", json!({"branch": "orch/fx/issue/1", "task": t.id})).1;
    assert!(e.contains("only branches of this feature"), "{e}");
    ok(&rt, LEAD, "merge", json!({"branch": "orch/fx/back-1", "task": t.id}));
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    assert!(wt.join("a.rs").is_file() && !wt.join("b.rs").exists(), "only the reported work");
    let msg = head_message(&rt, LEAD);
    assert!(msg.starts_with("Merge orch/fx/back-1\n\n"), "{msg:?}");
    assert!(trailers(&msg).lines().any(|l| l == format!("Orch-Task: {}", t.id)), "{msg:?}");
}

#[test]
fn per_issue_refuses_a_task_never_reported_and_single_mode_tags_its_merge() {
    let (_d, rt, _rd, _repo, pid) = setup();
    let run = approved(&rt, &pid, &["#1", "#2"], PrMode::PerIssue);
    let t = create(&rt, json!({"title": "API", "node": "back"}));
    rt.team_tick();
    write_commit(&rt, "fx-back-1", "a.rs", "a");
    // In review with no reported head (reported before heads were recorded).
    let mut old = rt.db.task(t.id).unwrap().unwrap();
    old.status = orch_team::TaskStatus::Review;
    rt.db.upsert_task(&old).unwrap();
    let e = call(&rt, LEAD, "merge", json!({"branch": "orch/fx/back-1", "task": t.id})).1;
    assert!(e.contains("has not been reported yet"), "{e}");
    old.status = orch_team::TaskStatus::InProgress;
    rt.db.upsert_task(&old).unwrap();
    // Single mode: the tip, with the trailer.
    let mut f = rt.db.feature(&run.feature_id).unwrap().unwrap();
    f.pr_mode = PrMode::Single;
    rt.db.upsert_feature(&f).unwrap();
    report(&rt, "b.rs");
    ok(&rt, LEAD, "merge", json!({"branch": "orch/fx/back-1", "task": t.id}));
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    assert!(wt.join("a.rs").is_file() && wt.join("b.rs").is_file());
    assert!(trailers(&head_message(&rt, LEAD)).contains(&format!("Orch-Task: {}", t.id)));
}
