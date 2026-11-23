//! allow-launch-team-on-update-pr B3: the push to the linked PR (§5.5), the
//! watcher from launch (§5.6), `rebase_base onto pr_head` (§5.4) and the
//! launch leftovers of B2's review.

use std::time::Duration;

use orch_git::fixtures::git as g;
use orch_protocol::{InboxDecision, now_ms};
use orch_store::InboxItem;
use orch_team::RunState;
use serde_json::{Value, json};

use crate::pr_link::tests::{Fx, HEAD, LEAD, fx, refusal, rev, spec};
use crate::runtime::Runtime;
use crate::store::TurnStatus;
use crate::team::LaunchSpec;
use crate::team::tests::{call, ok};

const MIN: u64 = 60_000;

/// The lead's queued turns that contain `needle`.
fn told(rt: &Runtime, needle: &str) -> usize {
    rt.store.read(|s| {
        s.turns
            .iter()
            .filter(|t| t.agent_id == LEAD && t.status == TurnStatus::Queued && t.body.contains(needle))
            .count()
    })
}

fn pending_gate(rt: &Runtime) -> Option<InboxItem> {
    rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned())
}

fn item(rt: &Runtime, id: u64) -> InboxItem {
    rt.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned().unwrap())
}

fn wait(what: &str, f: impl Fn() -> bool) {
    let t = std::time::Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_of(fx: &Fx, id: &str) -> orch_store::TeamRun {
    fx.rt.db.run(id).unwrap().unwrap()
}

/// PR #42 launched and its spec approved: the run is `planning`.
fn approved(fx: &Fx) -> orch_store::TeamRun {
    let run = fx.rt.launch(&spec(fx, "pr-42")).unwrap();
    let docs = crate::feature_docs::app_base(&fx.rt.cfg.paths.root, &fx.pid, "pr-42").join("docs");
    std::fs::write(docs.join("spec.md"), "# PR 42\n").unwrap();
    ok(&fx.rt, LEAD, "request_spec_approval", json!({"summary": "s"}));
    fx.rt.decide(pending_gate(&fx.rt).unwrap().id, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(run_of(fx, &run.id).status, RunState::Planning);
    run
}

fn lead_wt(rt: &Runtime) -> std::path::PathBuf {
    rt.store.read(|s| s.agents[LEAD].worktree.clone())
}

fn commit(rt: &Runtime, file: &str) {
    std::fs::write(lead_wt(rt).join(file), file).unwrap();
    ok(rt, LEAD, "commit", json!({"message": format!("add {file}")}));
}

fn events(fx: &Fx, run: &str, kind: &str) -> Vec<Value> {
    fx.rt.db.flush().unwrap();
    fx.rt.db.run_events(run, 500).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.detail).collect()
}

#[test]
fn a_linked_feature_pushes_the_team_commits_to_its_pr_and_never_opens_one() {
    let fx = fx();
    let rt = &fx.rt;
    let run = approved(&fx);
    let linked_at = fx.origin_sha();
    // Watched from launch: the PR's state shows before any push; the run stays.
    rt.pr_tick(now_ms() + MIN);
    let r = run_of(&fx, &run.id);
    let pr = r.pr.clone().unwrap();
    assert_eq!((r.status, pr.state.as_str(), pr.linked, pr.foreign_head), (RunState::Planning, "open", true, false));
    assert_eq!(pr.head_sha, linked_at);
    let f = rt.db.feature(&run.feature_id).unwrap().unwrap();
    let rows = crate::issues::prs_view(&r, &f, &[]);
    assert_eq!((rows.len(), rows[0].state.as_str(), rows[0].number), (1, "open", Some(42)));

    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    let (err, text) = call(rt, LEAD, "request_pr", json!({}));
    assert!(err && text.contains("No commit to push to PR #42"), "{text}");
    // D1 guard: a doc committed by hand never goes to someone else's PR.
    std::fs::create_dir_all(lead_wt(rt).join(".orchestrator/features/pr-42")).unwrap();
    commit(rt, ".orchestrator/features/pr-42/notes.md");
    let (err, text) = call(rt, LEAD, "request_pr", json!({}));
    assert!(err && text.contains(".orchestrator/features/pr-42/ is in the diff: PR #42 is someone else's"), "{text}");
    g(&lead_wt(rt), &["reset", "-q", "--hard", "HEAD~1"]);

    commit(rt, "fix.txt");
    let text = ok(rt, LEAD, "request_pr", json!({}));
    assert!(text.contains("push 1 commit(s) of renovate/foo to PR #42"), "{text}");
    assert_eq!(run_of(&fx, &run.id).status, RunState::PrGate);
    let gate = pending_gate(rt).unwrap();
    let p = &gate.payload;
    assert_eq!((p["action"].as_str(), p["linked"]["number"].as_u64()), (Some("push_pr"), Some(42)));
    assert_eq!((p["lease_sha"].as_str(), p["force"].as_bool()), (Some(linked_at.as_str()), Some(false)));
    assert_eq!(p["commits"].as_array().unwrap().len(), 1);
    assert_eq!(p["commits"][0]["subject"], "add fix.txt");
    let files: Vec<&str> = p["diff"]["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(files, ["fix.txt"], "the team's commits only, not the PR's own");
    assert_eq!(gate.body, format!("{LEAD}: push 1 commit(s) to PR #42 ({HEAD})"));
    let patch = rt.decision_patch(gate.id, None).unwrap();
    assert!(patch.contains("fix.txt") && !patch.contains("dep.txt"), "{patch}");
    // The PR row carries its push gate.
    let r = run_of(&fx, &run.id);
    let rows = crate::issues::prs_view(&r, &f, &rt.pr_gates(true)[&run.id]);
    assert_eq!((rows[0].state.as_str(), rows[0].gate), ("open", Some(gate.id)));

    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    let local = rev(&lead_wt(rt), "HEAD");
    wait("the push", || {
        let r = run_of(&fx, &run.id);
        r.status == RunState::PrOpen && r.pr.is_some_and(|p| p.pushed_sha.as_deref() == Some(local.as_str()))
    });
    assert_eq!(fx.origin_sha(), local, "a fast-forward of the PR branch");
    assert_eq!(fx.pr.lock().unwrap().created, 0, "no gh pr create");
    let r = run_of(&fx, &run.id);
    assert_eq!((r.pr_number, r.pr_url.as_deref()), (Some(42), Some("https://github.com/o/r/pull/42")));
    let pr = r.pr.unwrap();
    assert!(pr.linked && !pr.foreign_head && pr.head_sha == local);
    assert_eq!(item(rt, gate.id).effect.as_deref(), Some("pr_pushed"));
    assert_eq!(told(rt, "ORCH: the user approved; pushed 1 commit(s) to PR #42"), 1);
    let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
    assert!(audit.iter().any(|a| a.action == "pr_push_linked" && a.decision == "ok"));

    // pr_open: the usual update path pushes to the same PR.
    commit(rt, "more.txt");
    ok(rt, LEAD, "request_pr", json!({}));
    let gate = pending_gate(rt).unwrap();
    assert_eq!(
        (gate.payload["action"].as_str(), gate.payload["pr_number"].as_u64()),
        (Some("push_pr_update"), Some(42))
    );
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    let local = rev(&lead_wt(rt), "HEAD");
    wait("the update", || fx.origin_sha() == local);
    assert_eq!(fx.pr.lock().unwrap().created, 0);

    // Merged → done.
    fx.pr.lock().unwrap().state = "MERGED";
    rt.pr_tick(now_ms() + 10 * MIN);
    assert_eq!(run_of(&fx, &run.id).status, RunState::Done);
    assert_eq!(told(rt, "ORCH: PR #42 was merged. The feature is done"), 1);
}

#[test]
fn a_head_moved_outside_the_app_is_told_once_brought_in_and_never_overwritten() {
    let fx = fx();
    let rt = &fx.rt;
    let run = approved(&fx);
    commit(rt, "fix.txt");
    fx.bot_push("dep2.txt");
    let theirs = fx.origin_sha();
    let t0 = now_ms();
    rt.pr_tick(t0 + MIN);
    let pr = run_of(&fx, &run.id).pr.unwrap();
    assert_eq!((pr.foreign_head, pr.foreign_head_sha.as_deref()), (true, Some(theirs.as_str())));
    let short = &theirs[..12];
    let turn = format!("ORCH: PR #42's head moved to {short} (pushed outside the app, probably by app/renovate)");
    assert_eq!(told(rt, &turn), 1);
    rt.pr_tick(t0 + 2 * MIN);
    assert_eq!(told(rt, "head moved"), 1, "once per head");
    assert_eq!(run_of(&fx, &run.id).status, RunState::Planning, "nothing else acted on before pr_open");

    // The push is refused while the branch lacks their commit.
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    let (err, text) = call(rt, LEAD, "request_pr", json!({}));
    assert!(err && text.contains(&format!("PR #42 has commits the branch does not have ({short})")), "{text}");
    // `onto` must be a known value.
    assert!(call(rt, LEAD, "rebase_base", json!({"onto": "main"})).1.contains("onto is \"base\" or \"pr_head\""));
    let text = ok(rt, LEAD, "rebase_base", json!({"onto": "pr_head"}));
    assert!(text.contains(&format!("Rebased {HEAD} onto origin/{HEAD}")), "{text}");
    let wt = lead_wt(rt);
    assert!(orch_git::cleanup::is_merged(&wt, &theirs, "HEAD") && wt.join("fix.txt").is_file());
    let ev = events(&fx, &run.id, "git.rebase");
    assert_eq!(ev[0]["onto"], format!("origin/{HEAD}"));
    wait("the foreign head cleared", || run_of(&fx, &run.id).pr.is_some_and(|p| !p.foreign_head));

    // Asked, then the bot pushes again before the approval: refused, theirs kept.
    ok(rt, LEAD, "request_pr", json!({}));
    let gate = pending_gate(rt).unwrap();
    assert_eq!(gate.payload["lease_sha"].as_str(), Some(theirs.as_str()));
    fx.bot_push("dep3.txt");
    let again = fx.origin_sha();
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    wait("the refusal", || item(rt, gate.id).status == orch_store::InboxStatus::Failed);
    let it = item(rt, gate.id);
    assert_eq!(it.payload["outcome"]["code"], "pr_moved", "{:?}", it.payload);
    assert!(it.error.unwrap().contains(&format!("PR #42 moved to {}", &again[..12])));
    assert_eq!(fx.origin_sha(), again, "never overwritten");
    assert_eq!(run_of(&fx, &run.id).status, RunState::Integrating);
    assert_eq!(told(rt, "ORCH: the user approved, but the push to PR #42 was refused"), 1);
    let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
    assert!(audit.iter().any(|a| a.action == "pr_push_linked" && a.decision == "failed"));
}

#[test]
fn a_branch_only_behind_the_pr_head_fast_forwards_and_unlinked_features_have_no_pr_head() {
    let fx = fx();
    let rt = &fx.rt;
    let run = approved(&fx);
    let before = rev(&lead_wt(rt), "HEAD");
    fx.bot_push("dep2.txt");
    // From planning on (G4).
    let text = ok(rt, LEAD, "rebase_base", json!({"onto": "pr_head"}));
    assert!(text.contains("Rebased"), "{text}");
    let after = rev(&lead_wt(rt), "HEAD");
    assert_eq!(after, fx.origin_sha(), "fast-forwarded to the PR head");
    assert_eq!(g(&lead_wt(rt), &["rev-list", "--count", &format!("{before}..{after}")]).trim(), "1");
    assert!(ok(rt, LEAD, "rebase_base", json!({"onto": "pr_head"})).contains("nothing to rebase"));
    // The base keeps its states.
    assert!(call(rt, LEAD, "rebase_base", json!({})).1.contains("while integrating"));
    let _ = run;

    let plain = rt.launch(&LaunchSpec { pr: None, slug: "plain".into(), base: None, ..spec(&fx, "plain") }).unwrap();
    let (err, text) = call(rt, "plain-lead-1", "rebase_base", json!({"onto": "pr_head"}));
    assert!(err && text.contains("is for a feature on someone else's PR"), "{text}");
    let _ = plain;
}

#[test]
fn a_closed_linked_pr_keeps_the_run_refuses_the_push_and_clears_when_reopened() {
    let fx = fx();
    let rt = &fx.rt;
    let run = approved(&fx);
    let t0 = now_ms();
    rt.pr_tick(t0 + MIN);
    fx.pr.lock().unwrap().state = "CLOSED";
    rt.pr_tick(t0 + 2 * MIN);
    rt.pr_tick(t0 + 3 * MIN);
    assert_eq!(run_of(&fx, &run.id).status, RunState::Planning, "the run keeps its state");
    assert_eq!(told(rt, "ORCH: PR #42 was closed outside the app. Ask the user"), 1);
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    commit(rt, "fix.txt");
    let (err, text) = call(rt, LEAD, "request_pr", json!({"title": "x"}));
    assert!(err && text.contains("PR #42 is closed: ask the user"), "{text}");
    assert_eq!(run_of(&fx, &run.id).status, RunState::Integrating);

    fx.pr.lock().unwrap().state = "OPEN";
    rt.pr_tick(t0 + 4 * MIN);
    assert_eq!(told(rt, "ORCH: PR #42 is open again"), 1);
    assert_eq!(run_of(&fx, &run.id).pr.unwrap().state, "open");
    ok(rt, LEAD, "request_pr", json!({}));
    let gate = pending_gate(rt).unwrap();

    // Closed between the request and the approval: the gate fails `pr_closed`.
    fx.pr.lock().unwrap().state = "CLOSED";
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    wait("the refusal", || item(rt, gate.id).status == orch_store::InboxStatus::Failed);
    assert_eq!(item(rt, gate.id).payload["outcome"]["code"], "pr_closed");
    assert_eq!(fx.pr.lock().unwrap().created, 0);

    // Closed once `pr_open`: back to integrating, and still no new PR.
    fx.pr.lock().unwrap().state = "OPEN";
    rt.pr_tick(t0 + 5 * MIN);
    ok(rt, LEAD, "request_pr", json!({}));
    rt.decide(pending_gate(rt).unwrap().id, InboxDecision::Approve, None, None).unwrap();
    wait("pr_open", || run_of(&fx, &run.id).status == RunState::PrOpen);
    fx.pr.lock().unwrap().state = "CLOSED";
    rt.pr_tick(t0 + 6 * MIN);
    assert_eq!(run_of(&fx, &run.id).status, RunState::Integrating);
    assert_eq!(told(rt, "was closed outside the app"), 2, "told for each closing the watcher saw");
    assert!(call(rt, LEAD, "request_pr", json!({"title": "x"})).1.contains("PR #42 is closed"));
    // Reopened after the app had pushed: back to pr_open, followed again.
    fx.pr.lock().unwrap().state = "OPEN";
    rt.pr_tick(t0 + 7 * MIN);
    assert_eq!(run_of(&fx, &run.id).status, RunState::PrOpen);
    assert_eq!(told(rt, "ORCH: PR #42 is open again: the run is back to pr_open"), 1);

    // Merged while a push waits: the run is done and the push gate goes.
    fx.pr.lock().unwrap().state = "OPEN";
    rt.pr_tick(t0 + 7 * MIN);
    commit(rt, "late.txt");
    ok(rt, LEAD, "request_pr", json!({}));
    let gate = pending_gate(rt).unwrap();
    fx.pr.lock().unwrap().state = "MERGED";
    rt.pr_tick(t0 + 8 * MIN);
    assert_eq!(run_of(&fx, &run.id).status, RunState::Done);
    assert_eq!(item(rt, gate.id).status, orch_store::InboxStatus::Dismissed);
}

#[test]
fn a_linked_feature_launched_again_rereads_its_pr_and_a_deleted_slug_is_never_reused() {
    let fx = fx();
    let rt = &fx.rt;
    let run = rt.launch(&spec(&fx, "pr-42")).unwrap();
    rt.set_run_state(&run.id, RunState::Failed, "test").unwrap();
    // Launched again without `pr`: the link kept, the PR re-read.
    let again = LaunchSpec { pr: None, base: None, ..spec(&fx, "pr-42") };
    let views = fx.pr.lock().unwrap().views;
    fx.pr.lock().unwrap().state = "MERGED";
    assert_eq!(refusal(&fx, &again)["code"], "pr_merged");
    fx.pr.lock().unwrap().state = "CLOSED";
    assert_eq!(refusal(&fx, &again)["code"], "pr_closed");
    fx.pr.lock().unwrap().state = "OPEN";
    fx.pr.lock().unwrap().cross_repo = true;
    assert_eq!(refusal(&fx, &again)["code"], "fork");
    fx.pr.lock().unwrap().cross_repo = false;
    assert_eq!(fx.pr.lock().unwrap().views, views + 3);
    let per = LaunchSpec { pr_mode: Some(orch_store::PrMode::PerIssue), ..again.clone() };
    assert_eq!(refusal(&fx, &per)["code"], "per_issue");
    assert_eq!(rt.db.runs(Some(&run.feature_id), false).unwrap().len(), 1, "no run made");

    // Deleted with its worktree kept: the branch's holder is named.
    rt.delete_feature(&run.feature_id, false, false).unwrap();
    let e = refusal(&fx, &spec(&fx, "pr-42-b"));
    assert_eq!(e["code"], "branch_checked_out");
    assert!(e["message"].as_str().unwrap().contains("(deleted feature pr-42, its worktree kept)"), "{e}");
    assert_eq!(e["details"]["feature"]["slug"], "pr-42");
    // The deleted slug onto a PR: refused before anything is written.
    let e = refusal(&fx, &spec(&fx, "pr-42"));
    assert_eq!(
        (e["code"].as_str(), e["message"].as_str()),
        (Some("slug_taken"), Some("feature pr-42 existed (deleted): pick another slug"))
    );
    assert_eq!(rt.db.runs(Some(&run.feature_id), false).unwrap().len(), 1, "no run made");
    assert!(rt.db.feature(&run.feature_id).unwrap().unwrap().archived);
}

#[test]
fn a_rebase_onto_the_base_is_force_pushed_under_the_lease() {
    let fx = fx();
    let rt = &fx.rt;
    let run = approved(&fx);
    rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
    commit(rt, "fix.txt");
    let linked_at = fx.origin_sha();
    // main moves on origin; the team rebases its branch (the PR's commits too) onto it.
    g(&fx.bot, &["checkout", "-q", "main"]);
    std::fs::write(fx.bot.join("m.txt"), "m").unwrap();
    g(&fx.bot, &["add", "m.txt"]);
    g(&fx.bot, &["-c", "user.name=B", "-c", "user.email=b@example.invalid", "commit", "-qm", "main moves"]);
    g(&fx.bot, &["push", "-q", "origin", "main"]);
    g(&fx.bot, &["checkout", "-q", HEAD]);
    assert!(ok(rt, LEAD, "rebase_base", json!({})).contains("onto origin/main"));
    assert!(!orch_git::cleanup::is_merged(&lead_wt(rt), &linked_at, "HEAD"), "the PR's commits rewritten");
    ok(rt, LEAD, "request_pr", json!({}));
    let gate = pending_gate(rt).unwrap();
    assert_eq!(
        (gate.payload["force"].as_bool(), gate.payload["lease_sha"].as_str()),
        (Some(true), Some(linked_at.as_str()))
    );
    // The PR's and the team's commits, not the base's new one.
    let subjects: Vec<&str> =
        gate.payload["commits"].as_array().unwrap().iter().filter_map(|c| c["subject"].as_str()).collect();
    assert_eq!(subjects.len(), 2, "{subjects:?}");
    assert!(!subjects.contains(&"main moves"), "{subjects:?}");
    rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    let local = rev(&lead_wt(rt), "HEAD");
    wait("the force push", || fx.origin_sha() == local);
    wait("pr_open", || run_of(&fx, &run.id).status == RunState::PrOpen);
    let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
    assert!(audit.iter().any(|a| a.action == "pr_push_linked" && a.decision == "ok" && a.detail["force"] == true));
}
