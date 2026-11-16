//! One PR per issue, end to end (feature allow-multiple-prs §3.3–3.6): a
//! fake `gh` over a bare `origin`, the real builds, gates and pushes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use orch_git::fixtures::{commit_file, git as g, scratch_repo};
use orch_protocol::{InboxDecision, now_ms};
use orch_store::{InboxItem, PrMode, RunPr, TeamRun};
use orch_team::RunState;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::runtime::tests::test_cfg;
use crate::store::TurnStatus;
use crate::team::tests::{call, ok};
use crate::team::{GhFactory, LaunchSpec};

const LEAD: &str = "fx-lead-1";
const MIN: u64 = 60_000;

struct Pr {
    number: u64,
    head: String,
    body: String,
    state: &'static str,
}

/// GitHub as the fake `gh` sees it: PRs by number, their heads in `origin`.
struct Hub {
    origin: PathBuf,
    prs: Vec<Pr>,
}

struct FakeGh(Arc<Mutex<Hub>>);

fn arg<'a>(args: &'a [String], flag: &str) -> &'a str {
    args.iter().position(|a| a == flag).map_or("", |i| args[i + 1].as_str())
}

impl orch_tasks::github::Gh for FakeGh {
    fn run(&self, args: &[String]) -> Result<String> {
        let mut h = self.0.lock().unwrap();
        match (args[0].as_str(), args[1].as_str()) {
            ("pr", "create") => {
                let number = 12 + h.prs.len() as u64;
                let (head, body) = (arg(args, "--head").to_string(), arg(args, "--body").to_string());
                h.prs.push(Pr { number, head, body, state: "OPEN" });
                Ok(format!("Creating…\nhttps://github.com/o/r/pull/{number}\n"))
            }
            ("pr", "view") => {
                let n: u64 = args[2].parse()?;
                let Some(p) = h.prs.iter().find(|p| p.number == n) else { bail!("no PR {n}") };
                let head = g(&h.origin, &["rev-parse", &p.head]).trim().to_string();
                Ok(json!({"number": n, "url": format!("https://github.com/o/r/pull/{n}"), "state": p.state,
                          "headRefOid": head, "baseRefName": "main", "mergeable": "MERGEABLE",
                          "mergeStateStatus": "CLEAN", "statusCheckRollup": []})
                .to_string())
            }
            _ => bail!("unexpected gh {args:?}"),
        }
    }
}

#[derive(Debug, Default)]
struct Online;

impl orch_git::fetch::Conditions for Online {
    fn battery(&self) -> Option<u8> {
        None
    }
    fn online(&self) -> bool {
        true
    }
}

struct Fx {
    _d: tempfile::TempDir,
    _rd: tempfile::TempDir,
    rt: Arc<Runtime>,
    hub: Arc<Mutex<Hub>>,
    run: String,
    /// A clone of `origin` to move `main` from.
    other: PathBuf,
}

fn refused(rt: &Runtime, args: Value) -> String {
    let (err, text) = call(rt, LEAD, "request_pr", args);
    assert!(err, "request_pr was not refused: {text}");
    text
}

fn told(rt: &Runtime, needle: &str) -> usize {
    rt.store.read(|s| {
        s.turns
            .iter()
            .filter(|t| t.agent_id == LEAD && t.status == TurnStatus::Queued && t.body.contains(needle))
            .count()
    })
}

fn wait(what: &str, f: impl Fn() -> bool) {
    let t = std::time::Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run(fx: &Fx) -> TeamRun {
    fx.rt.db.run(&fx.run).unwrap().unwrap()
}

fn pr_of(fx: &Fx, issue: &str) -> RunPr {
    run(fx).prs.iter().rev().find(|p| p.issue == issue).cloned().unwrap_or_else(|| panic!("no PR for {issue}"))
}

fn gates(rt: &Runtime) -> Vec<InboxItem> {
    rt.store.read(|s| s.inbox.iter().filter(|i| i.pending() && i.kind == "gate").cloned().collect())
}

fn gate_of(rt: &Runtime, issue: &str) -> InboxItem {
    gates(rt).into_iter().find(|i| i.payload["issue"] == issue).unwrap_or_else(|| panic!("no gate for {issue}"))
}

fn origin_rev(fx: &Fx, rev: &str) -> String {
    g(&fx.hub.lock().unwrap().origin, &["rev-parse", rev]).trim().to_string()
}

fn origin_files(fx: &Fx, branch: &str) -> Vec<String> {
    let o = fx.hub.lock().unwrap().origin.clone();
    g(&o, &["ls-tree", "-r", "--name-only", branch]).lines().map(str::to_string).collect()
}

fn events(fx: &Fx, kind: &str) -> Vec<Value> {
    fx.rt.db.flush().unwrap();
    let mut v: Vec<Value> =
        fx.rt.db.run_events(&fx.run, 500).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.detail).collect();
    v.reverse();
    v
}

fn infos(rt: &Runtime) -> Vec<InboxItem> {
    rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "info").cloned().collect())
}

/// A `per_issue` feature (#1, #2) at its integration step, its spec
/// committed by the lead (shared), with a bare `origin`.
fn integrating() -> Fx {
    let d = tempfile::Builder::new().prefix("orchpi").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let (rd, repo) = scratch_repo();
    let origin = rd.path().join("origin.git");
    g(rd.path(), &["clone", "-q", "--bare", repo.to_str().unwrap(), origin.to_str().unwrap()]);
    g(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    g(&repo, &["fetch", "-q", "origin"]);
    let other = rd.path().join("other");
    g(rd.path(), &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()]);
    let hub = Arc::new(Mutex::new(Hub { origin, prs: vec![] }));
    let mut cfg = test_cfg(d.path(), false);
    let h = hub.clone();
    cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(h.clone())))));
    cfg.conditions = Arc::new(Online);
    let rt = Runtime::new(cfg).unwrap();
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    let (roles, team) = orch_team::presets::bugfix_squad();
    for r in &roles {
        rt.db.upsert_role(r, 1).unwrap();
    }
    rt.db.save_team(&team, &roles, 1).unwrap();
    let run = rt
        .launch(&LaunchSpec {
            project_id: p.id,
            team_id: "bugfix-squad".into(),
            slug: "fx".into(),
            title: "Fix X".into(),
            issues: vec!["#1".into(), "#2".into()],
            pr_mode: Some(PrMode::PerIssue),
            ..Default::default()
        })
        .unwrap();
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    std::fs::create_dir_all(wt.join(".orchestrator/features/fx")).unwrap();
    std::fs::write(wt.join(".orchestrator/features/fx/spec.md"), "# Fix X\n").unwrap();
    ok(&rt, LEAD, "commit", json!({"message": "spec"}));
    ok(&rt, LEAD, "request_spec_approval", json!({"summary": "s"}));
    let spec = gates(&rt).pop().unwrap();
    rt.decide(spec.id, InboxDecision::Approve, None, None).unwrap();
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    Fx { _d: d, _rd: rd, rt, hub, run: run.id, other }
}

/// A task of `issue` on `node`: dispatched, `file` written, reported and merged.
fn task(rt: &Runtime, node: &str, issue: &str, file: &str, body: &str) -> orch_store::Task {
    let agent = format!("fx-{node}-1");
    let text =
        ok(rt, LEAD, "create_task", json!({"title": format!("{file} for {issue}"), "node": node, "issue": issue}));
    rt.team_tick();
    let id = rt.store.read(|s| s.agents[&agent].team.as_ref().unwrap().task.unwrap());
    let wt = rt.store.read(|s| s.agents[&agent].worktree.clone());
    std::fs::write(wt.join(file), body).unwrap();
    ok(rt, &agent, "commit", json!({"message": format!("edit {file}")}));
    ok(rt, &agent, "report_done", json!({"summary": "s"}));
    ok(rt, &agent, "report_done", json!({"summary": "s", "review": {"fixed": [], "unfixed": []}}));
    ok(rt, LEAD, "merge", json!({"branch": format!("orch/fx/{node}-1"), "task": id}));
    let t = rt.db.task(id).unwrap().unwrap();
    assert!(text.contains(&t.handle()) || text.contains(&id.to_string()), "{text}");
    t
}

fn approve(rt: &Runtime, issue: &str) -> InboxItem {
    let gate = gate_of(rt, issue);
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    gate
}

/// Both issues' PRs asked, approved and open.
fn open_both(fx: &Fx) {
    let rt = &fx.rt;
    ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One", "body": "First."}));
    ok(rt, LEAD, "request_pr", json!({"issue": "2", "title": "Two"}));
    approve(rt, "#1");
    approve(rt, "#2");
    wait("both PRs open and checked", || {
        let r = run(fx);
        r.prs.len() == 2 && r.prs.iter().all(|p| p.pr.is_some())
    });
}

#[test]
fn two_issues_get_their_own_prs_updates_and_the_run_ends_when_both_merge() {
    let fx = integrating();
    let rt = &fx.rt;
    let t1 = task(rt, "back", "#1", "api.rs", "fn api() {}\n");
    let t2 = task(rt, "front", "#2", "ui.ts", "export {}\n");

    // Two gates side by side, each with its issue's diff plus the shared spec.
    let text = ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One", "body": "First."}));
    assert!(text.contains("orch/fx/issue/1") && text.contains("1 commit(s) of issue #1, 1 shared"), "{text}");
    assert_eq!(run(&fx).status, RunState::PrGate);
    assert!(refused(rt, json!({"issue": "#1", "title": "Again"})).contains("already waiting"));
    ok(rt, LEAD, "request_pr", json!({"issue": "2", "title": "Two"}));
    let (g1, g2) = (gate_of(rt, "#1"), gate_of(rt, "#2"));
    assert_eq!(
        (g1.payload["action"].clone(), g1.payload["branch"].clone()),
        (json!("push_pr"), json!("orch/fx/issue/1"))
    );
    assert_eq!(g1.payload["body"], "First.\n\nCloses #1");
    assert_eq!(g2.payload["body"], "Closes #2");
    let paths = |g: &InboxItem| -> Vec<String> {
        g.payload["diff"]["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(paths(&g1), [".orchestrator/features/fx/spec.md", "api.rs"]);
    assert_eq!(paths(&g2), [".orchestrator/features/fx/spec.md", "ui.ts"]);
    let patch = rt.decision_patch(g2.id, None).unwrap();
    assert!(patch.contains("ui.ts") && !patch.contains("api.rs"), "{patch}");
    for k in ["fork_sha", "integration_head", "head"] {
        assert!(orch_git::git::valid_sha(g1.payload[k].as_str().unwrap()), "{k}");
    }
    let build = events(&fx, "pr.build");
    assert_eq!(build[0], json!({"issue": "#1", "ok": true, "commits": 1, "shared": 1, "skipped": 0}));
    let view = rt.run_json(&fx.run).unwrap()["run"]["prs_view"].clone();
    assert_eq!((view[0]["state"].clone(), view[0]["gate"].clone()), (json!("pending"), json!(g1.id)));

    // Approved: two PRs from the issue branches, each with only its work.
    approve(rt, "#1");
    approve(rt, "#2");
    wait("both PRs open and checked", || {
        let r = run(&fx);
        r.prs.len() == 2 && r.prs.iter().all(|p| p.pr.is_some())
    });
    let r = run(&fx);
    assert_eq!(r.status, RunState::PrOpen);
    assert_eq!(r.pr_number, None, "the single PR fields stay empty");
    let (p1, p2) = (pr_of(&fx, "#1"), pr_of(&fx, "#2"));
    assert_eq!((p1.number, p1.branch.as_str(), p2.number), (12, "orch/fx/issue/1", 13));
    assert_eq!(origin_files(&fx, "orch/fx/issue/1"), [".orchestrator/features/fx/spec.md", "README.md", "api.rs"]);
    assert_eq!(origin_files(&fx, "orch/fx/issue/2"), [".orchestrator/features/fx/spec.md", "README.md", "ui.ts"]);
    assert_eq!(p1.built_sha, origin_rev(&fx, "orch/fx/issue/1"));
    assert_eq!(p1.pr.as_ref().unwrap().pushed_sha.as_deref(), Some(p1.built_sha.as_str()));
    {
        let h = fx.hub.lock().unwrap();
        assert_eq!((h.prs[0].head.as_str(), h.prs[0].body.as_str()), ("orch/fx/issue/1", "First.\n\nCloses #1"));
    }
    let opened = events(&fx, "pr.opened");
    assert_eq!(opened[0], json!({"number": 12, "url": "https://github.com/o/r/pull/12", "issue": "#1"}));
    let view = rt.run_json(&fx.run).unwrap()["run"]["prs_view"].clone();
    assert_eq!((view[0]["state"].clone(), view[1]["number"].clone()), (json!("open"), json!(13)));

    // A fix of #2: one update commit on PR 2 only; PR 1 is up to date.
    assert!(refused(rt, json!({"issue": "#2"})).contains("PR #13 (issue #2) is up to date"));
    let t3 = task(rt, "front", "#2", "ui2.ts", "export const x = 1;\n");
    rt.pr_refresh("fx").unwrap();
    assert_eq!((pr_of(&fx, "#1").pr.unwrap().unpushed, pr_of(&fx, "#2").pr.unwrap().unpushed), (0, 1));
    assert!(refused(rt, json!({"issue": "#1"})).contains("PR #12 (issue #1) is up to date"));
    let text = ok(rt, LEAD, "request_pr", json!({"issue": "#2"}));
    assert!(text.contains("push 1 commit(s) of orch/fx/issue/2 to PR #13"), "{text}");
    let gate = gate_of(rt, "#2");
    let before = origin_rev(&fx, "orch/fx/issue/2");
    assert_eq!(gate.payload["action"], "push_pr_update");
    assert_eq!((gate.payload["force"].clone(), gate.payload["lease_sha"].clone()), (json!(false), json!(before)));
    assert_eq!(gate.payload["commits"].as_array().unwrap().len(), 1);
    assert_eq!(gate.payload["commits"][0]["subject"], format!("Update for #2: {}", t3.handle()));
    approve(rt, "#2");
    wait("the update pushed", || pr_of(&fx, "#2").pr.unwrap().pushed_sha.as_deref() != Some(before.as_str()));
    let after = origin_rev(&fx, "orch/fx/issue/2");
    assert_eq!(origin_rev(&fx, "orch/fx/issue/2^"), before, "one commit on top of the PR");
    assert!(origin_files(&fx, "orch/fx/issue/2").contains(&"ui2.ts".to_string()));
    assert_eq!(origin_rev(&fx, "orch/fx/issue/1"), p1.built_sha, "PR 1 untouched");
    assert_eq!(pr_of(&fx, "#2").built_sha, after);
    assert_eq!(events(&fx, "pr.pushed")[0], json!({"number": 13, "commits": 1, "force": false, "issue": "#2"}));
    assert_eq!(told(rt, "pushed 1 commit(s) to PR #13 (issue #2)"), 1);

    // PR 1 merged: still pr_open, the lead told one issue is left.
    fx.hub.lock().unwrap().prs[0].state = "MERGED";
    let t0 = now_ms();
    rt.pr_tick(t0 + MIN);
    assert_eq!(run(&fx).status, RunState::PrOpen);
    assert_eq!(told(rt, "ORCH: PR #12 (issue #1) was merged; 1 issue(s) left: #2."), 1);
    assert_eq!(events(&fx, "pr.merged"), [json!({"number": 12, "issue": "#1"})]);
    let info = infos(rt);
    assert_eq!(info.len(), 1);
    assert_eq!(
        (info[0].payload["action"].clone(), info[0].payload["issue"].clone()),
        (json!("pr_merged"), json!("#1"))
    );
    assert!(refused(rt, json!({"issue": "#1"})).contains("was merged"));

    // PR 2 merged: done, "Feature fx merged" once, even checked again.
    fx.hub.lock().unwrap().prs[1].state = "MERGED";
    rt.pr_tick(t0 + 2 * MIN);
    let r = run(&fx);
    assert_eq!(r.status, RunState::Done);
    assert!(events(&fx, "run.state").iter().any(|e| e["why"] == "every issue PR merged" && e["to"] == "done"));
    assert_eq!(told(rt, "every issue PR is merged. The feature is done"), 1);
    rt.pr_tick(t0 + 3 * MIN);
    rt.pr_refresh("fx").unwrap();
    rt.reevaluate_run_prs(&fx.run, None).unwrap();
    let merged: Vec<_> = infos(rt).into_iter().filter(|i| i.body == "Feature fx merged").collect();
    assert_eq!(merged.len(), 1, "once");
    assert_eq!(rt.run_json(&fx.run).unwrap()["run"]["prs_view"][1]["state"], "merged");
    let _ = (t1, t2);
}

#[test]
fn a_task_built_on_another_issue_s_work_is_refused_with_its_task_and_files() {
    let fx = integrating();
    let rt = &fx.rt;
    task(rt, "back", "#1", "x.rs", "one\n");
    let t2 = task(rt, "front", "#2", "x.rs", "two\n");
    let text = refused(rt, json!({"issue": "#2", "title": "Two"}));
    assert!(text.contains(&format!("task {} (node front", t2.handle())), "{text}");
    assert!(text.contains("conflicts in x.rs") && text.contains("mcp__orch__update_task"), "{text}");
    assert!(gates(rt).is_empty(), "nothing requested");
    let ev = events(&fx, "pr.build");
    assert_eq!(ev[0]["ok"], false);
    assert_eq!(ev[0]["conflict"]["files"], json!(["x.rs"]));
    assert_eq!(ev[0]["conflict"]["task"], json!(t2.handle()));
    assert!(
        g(&rt.store.read(|s| s.agents[LEAD].worktree.clone()), &["branch", "--list", "orch/fx/issue/2"])
            .trim()
            .is_empty(),
        "nothing written"
    );
    // #1 alone builds; re-tagging the task to #1 makes #2's PR shared work only.
    ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One"}));
    ok(rt, LEAD, "update_task", json!({"id": t2.id, "issue": "#1"}));
    ok(rt, LEAD, "request_pr", json!({"issue": "#2", "title": "Two"}));
}

#[test]
fn after_rebase_base_each_issue_pr_is_rebuilt_and_force_pushed_with_its_lease() {
    let fx = integrating();
    let rt = &fx.rt;
    task(rt, "back", "#1", "api.rs", "fn api() {}\n");
    task(rt, "front", "#2", "ui.ts", "export {}\n");
    open_both(&fx);
    // A gate of either issue waiting: no rebase under it.
    task(rt, "back", "#1", "api2.rs", "fn b() {}\n");
    ok(rt, LEAD, "request_pr", json!({"issue": "#1"}));
    assert!(call(rt, LEAD, "rebase_base", json!({})).1.contains("waiting for the user"));
    let gate = gate_of(rt, "#1");
    rt.decide(gate.id, InboxDecision::Reject, None, None).unwrap();
    assert_eq!(run(&fx).status, RunState::PrOpen, "a declined update leaves the PRs open");

    commit_file(&fx.other, "m1.txt", "m1", "main: m1");
    g(&fx.other, &["push", "-q", "origin", "main"]);
    let text = ok(rt, LEAD, "rebase_base", json!({}));
    assert!(text.contains("Rebased orch/fx/integration onto origin/main"), "{text}");
    rt.pr_refresh("fx").unwrap();
    assert!(pr_of(&fx, "#1").pr.unwrap().rewritten && pr_of(&fx, "#2").pr.unwrap().rewritten);

    let p1 = pr_of(&fx, "#1");
    let lease = p1.pr.as_ref().unwrap().pushed_sha.clone().unwrap();
    let text = ok(rt, LEAD, "request_pr", json!({"issue": "#1"}));
    assert!(text.contains("force-push orch/fx/issue/1"), "{text}");
    let gate = gate_of(rt, "#1");
    assert_eq!((gate.payload["force"].clone(), gate.payload["lease_sha"].clone()), (json!(true), json!(lease)));
    assert_ne!(gate.payload["fork_sha"], json!(p1.fork_sha), "a new fork point");
    approve(rt, "#1");
    wait("the force push", || !events(&fx, "pr.pushed").is_empty());
    assert_eq!(events(&fx, "pr.pushed")[0]["force"], true);
    let files = origin_files(&fx, "orch/fx/issue/1");
    assert!(files.contains(&"m1.txt".to_string()) && files.contains(&"api2.rs".to_string()), "{files:?}");
    assert!(!files.contains(&"ui.ts".to_string()));
    wait("checked again", || pr_of(&fx, "#1").pr.is_some_and(|p| !p.rewritten));
    assert!(pr_of(&fx, "#2").pr.unwrap().rewritten, "PR 2 waits for its own request");
}

#[test]
fn a_stale_issue_gate_and_a_closed_pr_follow_the_run_state() {
    let fx = integrating();
    let rt = &fx.rt;
    task(rt, "back", "#1", "api.rs", "fn api() {}\n");
    ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One"}));
    // The integration branch moves before the approval (the lead's own commit, tagged #2): stale.
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    std::fs::write(wt.join("ui.ts"), "export {}\n").unwrap();
    ok(rt, LEAD, "commit", json!({"message": "ui", "issue": "#2"}));
    let gate = gate_of(rt, "#1");
    let r = rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(r["effect"], "stale");
    assert!(r["stale_why"].as_str().unwrap().contains("orch/fx/integration"), "{r}");
    ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One"}));
    approve(rt, "#1");
    wait("PR 1 open", || run(&fx).prs.first().is_some_and(|p| p.pr.is_some()));
    // Declining #2's gate keeps the run pr_open (PR 1 is open).
    ok(rt, LEAD, "request_pr", json!({"issue": "#2", "title": "Two"}));
    assert!(origin_files(&fx, "orch/fx/issue/1").iter().all(|f| f != "ui.ts"), "the Orch-Issue commit is #2's");
    let local = g(&wt, &["ls-tree", "-r", "--name-only", "orch/fx/issue/2"]);
    assert!(local.contains("ui.ts") && !local.contains("api.rs"), "{local}");
    rt.decide(gate_of(rt, "#2").id, InboxDecision::Reject, None, None).unwrap();
    assert_eq!(run(&fx).status, RunState::PrOpen);
    // An update pushed to PR 1: its branch on GitHub now has a commit no rebuild makes.
    let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
    std::fs::write(wt.join("api2.rs"), "fn b() {}\n").unwrap();
    ok(rt, LEAD, "commit", json!({"message": "api2", "issue": "#1"}));
    ok(rt, LEAD, "request_pr", json!({"issue": "#1"}));
    approve(rt, "#1");
    wait("the update pushed", || !events(&fx, "pr.pushed").is_empty());
    let updated = origin_rev(&fx, "orch/fx/issue/1");
    // PR 1 closed: no PR open → integrating; a new PR replaces it.
    fx.hub.lock().unwrap().prs[0].state = "CLOSED";
    rt.pr_tick(now_ms() + MIN);
    assert_eq!(run(&fx).status, RunState::Integrating);
    assert!(events(&fx, "run.state").iter().any(|e| e["why"] == "no PR open"));
    assert_eq!(told(rt, "ORCH: PR #12 (issue #1) was closed without being merged"), 1);
    ok(rt, LEAD, "request_pr", json!({"issue": "#1", "title": "One again"}));
    let gate = approve(rt, "#1");
    assert_eq!(gate.payload["lease_sha"], json!(updated), "the closed PR's branch, replaced under its lease");
    wait("a new PR 1", || run(&fx).prs.iter().any(|p| p.number == 13 && p.pr.is_some()));
    assert_eq!(origin_rev(&fx, "orch/fx/issue/1"), gate.payload["head"].as_str().unwrap());
    assert_ne!(origin_rev(&fx, "orch/fx/issue/1"), updated);
    let r = run(&fx);
    assert_eq!((r.status, r.prs.len()), (RunState::PrOpen, 1), "the closed one is replaced");
}

#[test]
fn github_s_update_branch_merge_on_an_issue_pr_is_replaced_by_a_force_push() {
    let fx = integrating();
    let rt = &fx.rt;
    task(rt, "back", "#1", "api.rs", "fn api() {}\n");
    task(rt, "front", "#2", "ui.ts", "export {}\n");
    open_both(&fx);
    // main moves; "Update branch" merges it into PR 1 on GitHub.
    commit_file(&fx.other, "m1.txt", "m1", "main: m1");
    g(&fx.other, &["push", "-q", "origin", "main"]);
    let o = &fx.other;
    let id = ["-c", "user.name=O", "-c", "user.email=o@example.invalid"];
    g(o, &["fetch", "-q", "origin"]);
    g(o, &["checkout", "-q", "-B", "pr", "origin/orch/fx/issue/1"]);
    g(o, &[&id[..], &["merge", "-q", "--no-ff", "-m", "Merge main into the PR", "origin/main"]].concat());
    g(o, &["push", "-q", "origin", "pr:orch/fx/issue/1"]);
    g(o, &["checkout", "-q", "main"]);
    let merge = origin_rev(&fx, "orch/fx/issue/1");
    rt.pr_refresh("fx").unwrap();
    assert_eq!(pr_of(&fx, "#1").pr.unwrap().head_sha, merge);
    task(rt, "back", "#1", "api2.rs", "fn b() {}\n");
    ok(rt, LEAD, "request_pr", json!({"issue": "#1"}));
    let gate = approve(rt, "#1");
    let p = &gate.payload;
    assert_eq!(
        (p["force"].clone(), p["lease_sha"].clone(), p["foreign_replaced"].clone()),
        (json!(true), json!(merge), json!(1))
    );
    wait("the force push", || !events(&fx, "pr.pushed").is_empty());
    assert_eq!(origin_rev(&fx, "orch/fx/issue/1"), p["head"].as_str().unwrap());
    // A real commit of theirs is never dropped.
    std::fs::write(o.join("theirs.txt"), "t").unwrap();
    g(o, &["fetch", "-q", "origin"]);
    g(o, &["checkout", "-q", "-B", "pr", "origin/orch/fx/issue/1"]);
    g(o, &["add", "theirs.txt"]);
    g(o, &[&id[..], &["commit", "-q", "-m", "their fix"]].concat());
    g(o, &["push", "-q", "origin", "pr:orch/fx/issue/1"]);
    wait("checked after the push", || {
        pr_of(&fx, "#1").pr.is_some_and(|s| s.pushed_sha.as_deref() == p["head"].as_str())
    });
    rt.pr_refresh("fx").unwrap();
    task(rt, "back", "#1", "api3.rs", "fn c() {}\n");
    let text = refused(rt, json!({"issue": "#1"}));
    assert!(text.contains("pushed outside the app") && text.contains("their fix"), "{text}");
}
