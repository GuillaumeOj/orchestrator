//! allow-launch-team-on-update-pr B2: the PR list rows and the launch prefill.

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use orch_git::fixtures::scratch_repo;
use serde_json::{Value, json};

use super::*;
use crate::runtime::Runtime;
use crate::runtime::tests::test_cfg;
use crate::team::GhFactory;

#[test]
fn slug_rule() {
    assert_eq!(
        pr_slug(42, "chore(deps): update rust crate tokio to v1.48"),
        "pr-42-chore-deps-update-rust-crate-tokio"
    );
    assert_eq!(pr_slug(7, "Fix it"), "pr-7-fix-it");
    assert_eq!(pr_slug(7, "!!!"), "pr-7");
    // cut inside a word → that word is dropped; at a boundary → kept
    assert_eq!(pr_slug(1, "aaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbb"), "pr-1-aaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(pr_slug(1, &"x".repeat(80)), format!("pr-1-{}", "x".repeat(35)));
}

#[test]
fn log_helpers() {
    assert_eq!(
        actions_ids("https://github.com/o/r/actions/runs/123/job/456"),
        Some(("123".into(), Some("456".into())))
    );
    assert_eq!(actions_ids("https://github.com/o/r/actions/runs/123"), Some(("123".into(), None)));
    assert_eq!(
        actions_ids("https://github.com/o/r/actions/runs/123/job/"),
        Some(("123".into(), None)),
        "no empty job id"
    );
    assert_eq!(actions_ids("https://ci.example.com/build/1"), None);
    assert_eq!(log_text("test\tRun cargo\t2025-01-02T03:04:05.1234567Z error: boom"), "error: boom");
    assert_eq!(log_text("no tabs here"), "no tabs here");
    assert_eq!(tail("a\nb\nc\n\n", 2), ("b\nc".into(), true));
    assert_eq!(tail("a\nb", 5), ("a\nb".into(), false));
    assert_eq!(tail_bytes("aaaa\nbbbb\ncccc", 8), ("cccc".into(), true));
    assert!(fenced("x\n~~~\ny").starts_with("  ~~~~\n  x"));
}

/// What the fake `gh` answers.
struct Gh {
    pr: Mutex<Value>,
    calls: Mutex<Vec<String>>,
}

#[derive(Debug)]
struct Online;

impl orch_git::fetch::Conditions for Online {
    fn battery(&self) -> Option<u8> {
        None
    }
    fn online(&self) -> bool {
        true
    }
}

struct FakeGh(Arc<Gh>);

impl orch_tasks::github::Gh for FakeGh {
    fn run(&self, args: &[String]) -> Result<String> {
        self.0.calls.lock().unwrap().push(args.join(" "));
        match (args[0].as_str(), args.get(1).map(String::as_str)) {
            ("pr", Some("view")) => Ok(self.0.pr.lock().unwrap().to_string()),
            ("pr", Some("list")) => {
                let mut p = self.0.pr.lock().unwrap().clone();
                p["state"] = json!("OPEN");
                Ok(json!([p]).to_string())
            }
            ("run", Some("view")) if args.iter().any(|a| a == "456") => Ok((1..=80)
                .map(|i| format!("test\tRun cargo test\t2025-01-02T03:04:05.0000000Z line {i}"))
                .collect::<Vec<_>>()
                .join("\n")),
            ("run", Some("view")) => Ok(String::new()),
            ("api", _) => {
                Ok(json!([{"path": "src/a.rs", "start_line": 3, "annotation_level": "failure", "message": "bad"}])
                    .to_string())
            }
            _ => bail!("unexpected gh {args:?}"),
        }
    }
}

fn pr_json() -> Value {
    json!({
        "number": 42, "url": "https://github.com/o/r/pull/42", "state": "OPEN",
        "title": "chore(deps): update rust crate tokio to v1.48", "body": "Bumps tokio.",
        "headRefName": "renovate/tokio-1.x", "headRefOid": "9f2c", "baseRefName": "main",
        "isCrossRepository": false, "author": {"login": "renovate[bot]", "is_bot": true},
        "isDraft": false,
        "statusCheckRollup": [
            {"__typename": "CheckRun", "name": "test", "workflowName": "ci", "status": "COMPLETED",
             "conclusion": "FAILURE", "detailsUrl": "https://github.com/o/r/actions/runs/123/job/456"},
            {"__typename": "CheckRun", "name": "lint", "workflowName": "ci", "status": "COMPLETED",
             "conclusion": "FAILURE", "detailsUrl": "https://github.com/o/r/actions/runs/123/job/789"},
            {"__typename": "CheckRun", "name": "build", "status": "IN_PROGRESS", "conclusion": ""},
            {"__typename": "CheckRun", "name": "fmt", "status": "COMPLETED", "conclusion": "SUCCESS"}
        ]
    })
}

fn fx() -> (Arc<Runtime>, String, Arc<Gh>, tempfile::TempDir, tempfile::TempDir) {
    let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let (rd, repo) = scratch_repo();
    let gh = Arc::new(Gh { pr: Mutex::new(pr_json()), calls: Mutex::default() });
    let mut cfg = test_cfg(d.path(), false);
    let h = gh.clone();
    cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(h.clone())))));
    cfg.conditions = Arc::new(Online);
    let rt = Runtime::new(cfg).unwrap();
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    let (roles, team) = orch_team::presets::bugfix_squad();
    for r in &roles {
        rt.db.upsert_role(r, 1).unwrap();
    }
    rt.db.save_team(&team, &roles, 1).unwrap();
    (rt, p.id, gh, d, rd)
}

fn code(e: anyhow::Error) -> Value {
    serde_json::from_str(&e.to_string()).unwrap()
}

#[test]
fn prefill_brief_and_notes() {
    let (rt, pid, gh, _d, _rd) = fx();
    let v = rt.pr_prefill(&pid, 42).unwrap();
    let l = &v["launch"];
    assert_eq!(l["slug"], "pr-42-chore-deps-update-rust-crate-tokio");
    assert_eq!(l["title"], "PR #42 · chore(deps): update rust crate tokio to v1.48");
    assert_eq!(
        (l["base"].as_str(), l["pr_mode"].is_null(), l["team_id"].as_str()),
        (Some("main"), true, Some("bugfix-squad"))
    );
    assert_eq!(l["pr"]["head_sha"], "9f2c");
    assert_eq!(l["pr"]["author"], "renovate[bot]");
    assert_eq!(v["origin"], json!({"kind": "pr", "label": "PR #42", "url": "https://github.com/o/r/pull/42"}));
    // 80 lines → tail of 60 (cut); the second job had no log → annotations
    assert_eq!(v["notes"], json!({"failing": 2, "pending": 1, "logs": "cut"}));
    assert_eq!(v["warnings"], json!([]));
    let b = l["brief"].as_str().unwrap();
    assert!(b.starts_with("Make PR #42 pass CI: chore(deps)"), "{b}");
    assert!(b.contains("renovate/tokio-1.x → main · opened by renovate[bot]"));
    assert!(b.contains("do not open another pull request"));
    assert!(b.contains("- ci / test · https://github.com/o/r/actions/runs/123/job/456\n  ~~~\n  line 21\n"));
    assert!(!b.contains("line 20\n"));
    assert!(b.contains("  src/a.rs:3: bad"), "annotations for the job without a log: {b}");
    assert!(b.ends_with("## PR description\nBumps tokio.\n"));
    // the prefill round-trips as a launch link
    let link: orch_store::PrLink = serde_json::from_value(l["pr"].clone()).unwrap();
    assert_eq!((link.number, link.head_sha_at_link.as_str()), (42, "9f2c"));
    assert!(gh.calls.lock().unwrap().iter().any(|c| c == "run view 123 --log-failed --job 456"));
}

#[test]
fn prefill_without_failure_and_unknown_author() {
    let (rt, pid, gh, _d, _rd) = fx();
    {
        let mut p = gh.pr.lock().unwrap();
        p["statusCheckRollup"] = json!([{"__typename": "CheckRun", "name": "b", "status": "IN_PROGRESS"}]);
        p["author"] = Value::Null;
        p["body"] = json!("");
    }
    let v = rt.pr_prefill(&pid, 42).unwrap();
    let b = v["launch"]["brief"].as_str().unwrap();
    assert!(b.contains("opened by an unknown author"));
    assert!(b.contains("## Failing checks\nNo check is failing right now (1 pending).\n"), "{b}");
    assert!(b.ends_with("No description.\n"));
    assert_eq!((v["notes"]["logs"].as_str(), v["launch"]["pr"]["author"].is_null()), (Some("none"), true));
}

#[test]
fn prefill_refusals() {
    let (rt, pid, gh, _d, _rd) = fx();
    for (state, want) in [("CLOSED", "pr_closed"), ("MERGED", "pr_merged")] {
        gh.pr.lock().unwrap()["state"] = json!(state);
        assert_eq!(code(rt.pr_prefill(&pid, 42).unwrap_err())["code"], want);
    }
    {
        let mut p = gh.pr.lock().unwrap();
        p["state"] = json!("OPEN");
        p["isCrossRepository"] = json!(true);
        p["headRepositoryOwner"] = json!({"login": "someone"});
    }
    let e = code(rt.pr_prefill(&pid, 42).unwrap_err());
    assert_eq!(
        (e["code"].as_str(), e["message"].as_str()),
        (Some("fork"), Some("Fork PR (from someone): the app pushes to origin only."))
    );
    assert_eq!(code(rt.pr_prefill(&pid, 42).unwrap_err()), e);
    assert!(rt.pr_prefill("nope", 42).is_err());
}

#[test]
fn list_rows_carry_author_fork_and_launch() {
    let (rt, pid, gh, _d, _rd) = fx();
    let p = rt.project(&pid).unwrap();
    let rows = rt.pr_list(&p).unwrap();
    let r = &rows[0];
    assert_eq!(r["author"], json!({"login": "renovate[bot]", "is_bot": true}));
    assert_eq!((r["cross_repo"].as_bool(), r["head_sha"].as_str()), (Some(false), Some("9f2c")));
    assert_eq!((r["feature"].is_null(), &r["launch"]), (true, &json!({"ok": true})));
    assert!(r.get("fork_owner").is_none());
    assert_eq!(r["checks"]["failing"], 2);
    {
        let mut x = gh.pr.lock().unwrap();
        x["isCrossRepository"] = json!(true);
        x["headRepositoryOwner"] = json!({"login": "someone"});
        x["author"] = json!({"login": "app/renovate"});
    }
    let r = &rt.pr_list(&p).unwrap()[0];
    assert_eq!(
        r["launch"],
        json!({"ok": false, "code": "fork", "reason": "Fork PR (from someone): the app pushes to origin only."})
    );
    assert_eq!((r["fork_owner"].as_str(), r["author"]["is_bot"].as_bool()), (Some("someone"), Some(true)));
    // project_detail embeds the same rows
    let d = rt.project_detail(&pid, true).unwrap();
    assert_eq!(d["prs"][0]["number"], 42);
}

#[test]
fn list_row_links_the_feature_and_blocks_an_active_one() {
    let (rt, pid, _gh, _d, _rd) = fx();
    let p = rt.project(&pid).unwrap();
    let f = orch_store::Feature {
        id: "f_1".into(),
        project_id: pid.clone(),
        slug: "pr-42-x".into(),
        title: "t".into(),
        brief: String::new(),
        base: "main".into(),
        integration_worktree: None,
        allocation: None,
        created_at_ms: 1,
        archived: false,
        issues: vec![],
        pr_mode: Default::default(),
        docs_location: None,
        branch: Some("renovate/tokio-1.x".into()),
        pr_link: Some(orch_store::PrLink { number: 42, ..Default::default() }),
    };
    rt.db.upsert_feature(&f).unwrap();
    // a feature without a run counts as not active (contract §1)
    let r = &rt.pr_list(&p).unwrap()[0];
    assert_eq!(r["feature"], json!({"id": "f_1", "slug": "pr-42-x", "run_status": null}));
    assert_eq!(r["launch"], json!({"ok": true}));
}
