//! allow-launch-team-on-update-pr B1: a launch on a PR (spec §5.3), the
//! branch it works on (merge guard, protected branch), and D2 on delete.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use orch_git::fixtures::{git as g, scratch_repo};
use orch_protocol::InboxDecision;
use orch_store::PrLink;
use orch_team::RunState;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::runtime::tests::test_cfg;
use crate::team::tests::{call, ok};
use crate::team::{GhFactory, LaunchSpec};

const HEAD: &str = "renovate/foo";
const LEAD: &str = "pr-42-lead-1";

/// PR #42 as GitHub has it; its head is `HEAD` in the bare origin.
struct Pr {
    origin: PathBuf,
    state: &'static str,
    cross_repo: bool,
    head_ref: String,
    views: u32,
}

struct FakeGh(Arc<Mutex<Pr>>);

impl orch_tasks::github::Gh for FakeGh {
    fn run(&self, args: &[String]) -> Result<String> {
        let mut p = self.0.lock().unwrap();
        if args[..2] != ["pr", "view"] || args.get(4).map(String::as_str) != Some(orch_git::gh::PR_HEAD_FIELDS) {
            bail!("unexpected gh {args:?}");
        }
        p.views += 1;
        let n: u64 = args[2].parse()?;
        let sha = orch_git::git(&p.origin, &["rev-parse", &p.head_ref]).map(|s| s.trim().to_string()).unwrap_or_default();
        Ok(json!({"number": n, "url": format!("https://github.com/o/r/pull/{n}"), "state": p.state,
                  "headRefName": p.head_ref, "headRefOid": sha, "baseRefName": "main",
                  "isCrossRepository": p.cross_repo, "author": {"login": "app/renovate"}})
        .to_string())
    }
}

struct Fx {
    _d: tempfile::TempDir,
    _rd: tempfile::TempDir,
    rt: Arc<Runtime>,
    pr: Arc<Mutex<Pr>>,
    repo: PathBuf,
    pid: String,
    /// A clone of origin to push to the PR from (the updater).
    bot: PathBuf,
}

impl Fx {
    fn origin_sha(&self) -> String {
        g(&self.pr.lock().unwrap().origin, &["rev-parse", HEAD]).trim().to_string()
    }

    /// The updater pushes one more commit to the PR.
    fn bot_push(&self, file: &str) {
        std::fs::write(self.bot.join(file), file).unwrap();
        g(&self.bot, &["add", file]);
        g(&self.bot, &["-c", "user.name=B", "-c", "user.email=b@example.invalid", "commit", "-qm", file]);
        g(&self.bot, &["push", "-q", "origin", HEAD]);
    }
}

/// A project whose origin has PR #42 on `renovate/foo` (one commit past main),
/// never fetched into the main checkout.
fn fx() -> Fx {
    let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let (rd, repo) = scratch_repo();
    let origin = rd.path().join("origin.git");
    g(rd.path(), &["clone", "-q", "--bare", repo.to_str().unwrap(), origin.to_str().unwrap()]);
    g(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    let bot = rd.path().join("bot");
    g(rd.path(), &["clone", "-q", origin.to_str().unwrap(), bot.to_str().unwrap()]);
    g(&bot, &["checkout", "-q", "-b", HEAD]);
    let pr = Arc::new(Mutex::new(Pr { origin, state: "OPEN", cross_repo: false, head_ref: HEAD.into(), views: 0 }));
    let mut cfg = test_cfg(d.path(), false);
    let h = pr.clone();
    cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(h.clone())))));
    let rt = Runtime::new(cfg).unwrap();
    rt.overwatch.manual.store(true, std::sync::atomic::Ordering::SeqCst);
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    let (roles, team) = orch_team::presets::bugfix_squad();
    for r in &roles {
        rt.db.upsert_role(r, 1).unwrap();
    }
    rt.db.save_team(&team, &roles, 1).unwrap();
    let f = Fx { _d: d, _rd: rd, rt, pr, repo, pid: p.id, bot };
    f.bot_push("dep.txt");
    f
}

fn link() -> PrLink {
    PrLink {
        number: 42,
        url: "https://github.com/o/r/pull/42".into(),
        head_ref: HEAD.into(),
        base_ref: "main".into(),
        head_sha_at_link: String::new(),
        author: Some("app/renovate".into()),
    }
}

fn spec(fx: &Fx, slug: &str) -> LaunchSpec {
    LaunchSpec {
        project_id: fx.pid.clone(),
        team_id: "bugfix-squad".into(),
        slug: slug.into(),
        title: "PR #42 · bump foo".into(),
        brief: "Make PR #42 pass CI".into(),
        base: Some("ignored".into()),
        pr: Some(link()),
        ..Default::default()
    }
}

/// The JSON of a refused launch.
fn refusal(fx: &Fx, s: &LaunchSpec) -> Value {
    let e = fx.rt.launch(s).unwrap_err();
    serde_json::from_str(&e.to_string()).unwrap_or_else(|_| panic!("not a coded error: {e:#}"))
}

fn rev(dir: &Path, r: &str) -> String {
    g(dir, &["rev-parse", r]).trim().to_string()
}

#[test]
fn a_pr_launch_checks_out_the_head_branch_and_links_the_feature() {
    let fx = fx();
    let run = fx.rt.launch(&spec(&fx, "pr-42")).unwrap();
    let f = fx.rt.db.feature(&run.feature_id).unwrap().unwrap();
    assert_eq!((f.branch.as_deref(), f.base.as_str(), f.integration_branch()), (Some(HEAD), "main", HEAD.into()));
    let l = f.pr_link.clone().unwrap();
    assert_eq!((l.number, l.head_ref.as_str(), l.base_ref.as_str()), (42, HEAD, "main"));
    assert_eq!(l.head_sha_at_link, fx.origin_sha(), "the head fetched at launch");
    assert_eq!(l.author.as_deref(), Some("app/renovate"));
    // D1/G1: docs and designs in the app folder whatever the project setting.
    let loc = f.docs_location.clone().unwrap();
    assert_eq!((loc.docs.kind, loc.designs.kind), (orch_team::LocKind::App, orch_team::LocKind::App));
    // The run is on its PR from the start, and follows the usual states (D5).
    assert_eq!((run.status, run.pr_number, run.pr_url.as_deref()), (RunState::Spec, Some(42), Some(l.url.as_str())));
    // The integration worktree at the usual path, on the PR branch at origin's head, tracking it.
    let wt = f.integration_worktree.clone().unwrap();
    assert!(wt.ends_with("pr-42/integration"), "{}", wt.display());
    assert_eq!(orch_git::worktree::current_branch(&wt).unwrap(), HEAD);
    assert_eq!(rev(&wt, "HEAD"), fx.origin_sha());
    assert_eq!(g(&wt, &["rev-parse", "--abbrev-ref", "@{u}"]).trim(), format!("origin/{HEAD}"));
    let row = fx.rt.store.read(|s| s.worktrees.get(&wt).cloned()).unwrap();
    assert_eq!((row.branch.as_str(), row.base.as_str()), (HEAD, "main"), "the row keeps the PR base");
    // api-contract §4: the real branch in the feature's JSON.
    let v = crate::pr_link::feature_json(&f);
    assert_eq!((v["branch"].as_str(), v["pr_link"]["number"].as_u64()), (Some(HEAD), Some(42)));
    assert_eq!(fx.pr.lock().unwrap().views, 1, "re-read once (G2)");

    // Members branch from it; their branches stay `orch/<slug>/<node>-<n>`.
    let docs = crate::feature_docs::app_base(&fx.rt.cfg.paths.root, &fx.pid, "pr-42").join("docs");
    std::fs::write(docs.join("spec.md"), "# PR 42\n").unwrap();
    ok(&fx.rt, LEAD, "request_spec_approval", json!({"summary": "s"}));
    let gate = fx.rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
    fx.rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
    ok(&fx.rt, LEAD, "create_task", json!({"title": "Fix the API", "node": "back"}));
    fx.rt.team_tick();
    let back = fx.rt.store.read(|s| s.agents["pr-42-back-1"].clone());
    assert_eq!(orch_git::worktree::current_branch(&back.worktree).unwrap(), "orch/pr-42/back-1");
    assert_eq!(rev(&back.worktree, "HEAD"), fx.origin_sha());

    // §5.4 merge guard: the integration branch is never a source, whatever its name.
    let (err, text) = call(&fx.rt, LEAD, "merge", json!({"branch": HEAD}));
    assert!(err && text.contains("only branches of this feature (orch/pr-42/<node>-<n>)"), "{text}");
    // policy/rt.rs: a specialist can't merge into `renovate/foo`.
    let cmd = format!("git checkout {HEAD} && git merge orch/pr-42/back-1");
    let v = fx.rt.hard_check(&back, "Bash", &json!({"command": cmd}), None).expect("denied");
    assert!(format!("{v:?}").contains(crate::policy::hard::WHY_MERGE_PROTECTED), "{v:?}");
    let own = "git checkout orch/pr-42/back-1 && git merge main";
    assert!(fx.rt.hard_check(&back, "Bash", &json!({"command": own}), None).is_none());
}

#[test]
fn a_local_head_behind_the_pr_is_fast_forwarded_and_one_ahead_refused() {
    let fx = fx();
    g(&fx.repo, &["fetch", "-q", "origin", HEAD]);
    g(&fx.repo, &["branch", "-q", HEAD, &format!("origin/{HEAD}")]);
    fx.bot_push("dep2.txt");
    let run = fx.rt.launch(&spec(&fx, "pr-42")).unwrap();
    let f = fx.rt.db.feature(&run.feature_id).unwrap().unwrap();
    assert_eq!(rev(&f.integration_worktree.unwrap(), "HEAD"), fx.origin_sha(), "fast-forwarded");

    // A local branch with a commit the PR lacks.
    let fx = self::fx();
    g(&fx.repo, &["checkout", "-q", "-b", HEAD]);
    std::fs::write(fx.repo.join("mine.txt"), "m").unwrap();
    g(&fx.repo, &["add", "mine.txt"]);
    g(&fx.repo, &["commit", "-qm", "mine"]);
    g(&fx.repo, &["checkout", "-q", "main"]);
    let before = rev(&fx.repo, HEAD);
    let e = refusal(&fx, &spec(&fx, "pr-42"));
    assert_eq!(e["code"], "local_ahead");
    assert_eq!(e["details"]["commits"], 1, "{e}");
    assert!(e["message"].as_str().unwrap().contains("local renovate/foo has 1 commit(s) that PR #42 does not have"));
    assert_eq!(rev(&fx.repo, HEAD), before, "left as it was");
    assert!(fx.rt.db.feature(&format!("{}-pr-42", fx.pid)).unwrap().is_none(), "nothing stored");
}

#[test]
fn refusals_come_before_anything_is_stored() {
    let fx = fx();
    let none = |fx: &Fx| fx.rt.db.features(Some(&fx.pid)).unwrap().is_empty() && fx.rt.db.runs(None, false).unwrap().is_empty();
    // Per-issue mode.
    let s = LaunchSpec { pr_mode: Some(orch_store::PrMode::PerIssue), ..spec(&fx, "pr-42") };
    assert_eq!(refusal(&fx, &s)["code"], "per_issue");
    assert_eq!(fx.pr.lock().unwrap().views, 0, "refused before gh");
    // A fork, a closed and a merged PR.
    fx.pr.lock().unwrap().cross_repo = true;
    assert_eq!(refusal(&fx, &spec(&fx, "pr-42"))["code"], "fork");
    fx.pr.lock().unwrap().cross_repo = false;
    fx.pr.lock().unwrap().state = "CLOSED";
    assert_eq!(refusal(&fx, &spec(&fx, "pr-42"))["message"], "PR #42 is closed");
    fx.pr.lock().unwrap().state = "MERGED";
    let e = refusal(&fx, &spec(&fx, "pr-42"));
    assert_eq!((e["code"].as_str(), e["message"].as_str()), (Some("pr_closed"), Some("PR #42 is merged")));
    fx.pr.lock().unwrap().state = "OPEN";
    // The head checked out in the user's main checkout.
    g(&fx.repo, &["fetch", "-q", "origin", HEAD]);
    g(&fx.repo, &["checkout", "-q", "-b", HEAD, &format!("origin/{HEAD}")]);
    let e = refusal(&fx, &spec(&fx, "pr-42"));
    assert_eq!(e["code"], "branch_checked_out");
    assert_eq!(e["details"]["path"], json!(fx.repo));
    assert!(e["message"].as_str().unwrap().ends_with("switch that checkout to another branch first"), "{e}");
    g(&fx.repo, &["checkout", "-q", "main"]);
    // A head the fetch can't find.
    fx.pr.lock().unwrap().head_ref = "renovate/gone".into();
    assert_eq!(refusal(&fx, &spec(&fx, "pr-42"))["code"], "fetch_failed");
    fx.pr.lock().unwrap().head_ref = HEAD.into();
    assert!(none(&fx), "no feature, no run");
    assert!(!fx.repo.parent().unwrap().join("demo.worktrees/pr-42").exists(), "no worktree");
}

#[test]
fn one_active_feature_per_pr_and_a_finished_one_holding_the_branch_is_named() {
    let fx = fx();
    let run = fx.rt.launch(&spec(&fx, "pr-42")).unwrap();
    // D3: another launch on the same PR.
    let e = refusal(&fx, &spec(&fx, "pr-42-again"));
    assert_eq!(e["code"], "active_feature");
    assert_eq!(e["feature"], json!({"id": run.feature_id, "slug": "pr-42", "run_status": "spec"}));
    // The same slug is a new feature only.
    fx.rt.set_run_state(&run.id, RunState::Failed, "test").unwrap();
    assert_eq!(refusal(&fx, &spec(&fx, "pr-42"))["code"], "slug_taken");
    // F1: finished, its worktree still holds the branch.
    let e = refusal(&fx, &spec(&fx, "pr-42-again"));
    assert_eq!(e["code"], "branch_checked_out");
    assert_eq!(e["details"]["feature"]["slug"], "pr-42");
    assert_eq!(e["details"]["feature"]["run_status"], "failed");
    assert!(e["message"].as_str().unwrap().contains("(feature pr-42): delete that feature first"), "{e}");
    // Deleted: the launch goes on, the pushed branch went with it.
    fx.rt.delete_feature(&run.feature_id, true, false).unwrap();
    let again = fx.rt.launch(&spec(&fx, "pr-42-again")).unwrap();
    assert_eq!(fx.rt.db.feature(&again.feature_id).unwrap().unwrap().integration_branch(), HEAD);
}

#[test]
fn deleting_a_linked_feature_keeps_unpushed_commits_and_never_touches_the_remote() {
    let fx = fx();
    let run = fx.rt.launch(&spec(&fx, "pr-42")).unwrap();
    let plan = fx.rt.feature_delete_plan(&run.feature_id).unwrap();
    assert_eq!(plan, json!({"branch": HEAD, "linked": true, "unpushed": 0}));
    let wt = fx.rt.db.feature(&run.feature_id).unwrap().unwrap().integration_worktree.unwrap();
    std::fs::write(wt.join("fix.txt"), "fix").unwrap();
    ok(&fx.rt, LEAD, "commit", json!({"message": "fix the build"}));
    let remote = fx.origin_sha();
    assert_eq!(fx.rt.feature_delete_plan("pr-42").unwrap()["unpushed"], 1);
    let out = fx.rt.delete_feature(&run.feature_id, true, true).unwrap();
    assert_eq!(out["kept"], json!([{"branch": HEAD, "commits": 1}]), "{out}");
    assert!(orch_git::worktree::branch_exists(&fx.repo, HEAD), "kept even with discard");
    assert!(!wt.exists(), "the worktree went");
    assert_eq!(fx.origin_sha(), remote, "nothing on the remote");
    // An unlinked feature: the plan names its branch, nothing kept.
    let other = fx.rt.launch(&LaunchSpec { pr: None, slug: "plain".into(), base: None, ..spec(&fx, "plain") }).unwrap();
    let plan = fx.rt.feature_delete_plan(&other.feature_id).unwrap();
    assert_eq!(plan, json!({"branch": "orch/plain/integration", "linked": false, "unpushed": 0}));
    let f = fx.rt.db.feature(&other.feature_id).unwrap().unwrap();
    assert_eq!((f.branch, f.pr_link, other.pr_number), (None, None, None));
    assert_eq!(fx.rt.delete_feature(&other.feature_id, true, false).unwrap()["kept"], json!([]));
}
