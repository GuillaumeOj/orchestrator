//! Intake end to end in the runtime: fake `gh`, fake Linear transport and a
//! temp git repo (P4.5 spec §5).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use orch_git::fixtures::{commit_file, scratch_repo};
use orch_tasks::linear::{GqlError, Graphql};
use serde_json::{Value, json};

use super::*;
use crate::linear::LinearFactory;
use crate::team::GhFactory;

type Answers = Arc<Mutex<BTreeMap<String, Result<Value, String>>>>;
type Calls = Arc<Mutex<Vec<Vec<String>>>>;

/// `gh` answering by API path prefix; every call recorded.
struct FakeGh(Answers, Calls);

impl orch_tasks::github::Gh for FakeGh {
    fn run(&self, args: &[String]) -> anyhow::Result<String> {
        self.1.lock().unwrap().push(args.to_vec());
        // The first argument after `api` that is not a flag (`--paginate`).
        let key = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
        let a = self.0.lock().unwrap();
        match a.iter().filter(|(k, _)| key.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()) {
            Some((_, Ok(v))) => Ok(v.to_string()),
            Some((_, Err(e))) => Err(anyhow::anyhow!("gh {}: {e}", args.join(" "))),
            None => Err(anyhow::anyhow!("gh {}: HTTP 404: Not Found", args.join(" "))),
        }
    }
}

/// Linear answering every issues query with `issues`, or failing.
#[derive(Clone)]
struct FakeLinear {
    issues: Arc<Mutex<Result<Vec<Value>, GqlError>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Graphql for FakeLinear {
    fn call(&self, query: &str, _vars: &Value) -> Result<Value, GqlError> {
        self.calls.lock().unwrap().push(query.to_string());
        let r = self.issues.lock().unwrap().clone()?;
        Ok(json!({"issues": {"nodes": r}}))
    }
}

struct Fx {
    _d: tempfile::TempDir,
    _rd: tempfile::TempDir,
    rt: Arc<Runtime>,
    project: Project,
    gh: Answers,
    calls: Calls,
    linear: FakeLinear,
}

fn fx() -> Fx {
    let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let mut cfg = crate::runtime::tests::test_cfg(d.path(), true);
    let (gh, calls): (Answers, Calls) = Default::default();
    let (g, c) = (gh.clone(), calls.clone());
    cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(g.clone(), c.clone())))));
    let linear = FakeLinear { issues: Arc::new(Mutex::new(Ok(vec![]))), calls: Arc::default() };
    let l = linear.clone();
    cfg.linear = Some(LinearFactory(Arc::new(move || Box::new(l.clone()))));
    // Stand-ins for the helpers and `claude`: a solo session starts and waits.
    let bin = cfg.paths.bin_dir();
    std::fs::create_dir_all(&bin).unwrap();
    for h in ["orch-hook", "orch-status", "orch-mcp"] {
        stub(&bin.join(h), "exit 0");
    }
    cfg.claude = d.path().join("claude");
    stub(&cfg.claude, "[ \"$1\" = --version ] && { echo '2.1.302 (Claude Code)'; exit 0; }\nsleep 20");
    let rt = Runtime::new(cfg).unwrap();
    // The Overwatch is driven by hand (no background wake).
    rt.overwatch.manual.store(true, std::sync::atomic::Ordering::SeqCst);
    let (rd, repo) = scratch_repo();
    let project = orch_git::project::register(&rt.db, &repo).unwrap();
    Fx { _d: d, _rd: rd, rt, project, gh, calls, linear }
}

fn stub(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn issue(n: u64, labels: &[&str], updated: &str) -> Value {
    json!({"number": n, "title": format!("Issue {n}"), "body": format!("Body of {n}"), "state": "open",
           "html_url": format!("https://github.com/acme/shop/issues/{n}"), "updated_at": updated,
           "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(), "assignees": []})
}

const ISSUES: &str = "repos/acme/shop/issues";

impl Fx {
    fn answer(&self, key: &str, v: Value) {
        self.gh.lock().unwrap().insert(key.into(), Ok(v));
    }
    fn fail(&self, key: &str, e: &str) {
        self.gh.lock().unwrap().insert(key.into(), Err(e.into()));
    }
    fn github_label(&self) {
        let src = json!({"type": "github", "repo": "acme/shop", "filter": {"kind": "label", "names": ["ready"]}});
        self.rt.intake_source_set(&self.project.id, &src).unwrap();
    }
    fn items(&self) -> Vec<Value> {
        self.rt.intake_view()["items"].as_array().unwrap().clone()
    }
    fn refs(&self) -> Vec<String> {
        self.items().iter().map(|i| i["ref"].as_str().unwrap().to_string()).collect()
    }
    fn source(&self) -> Value {
        self.rt.intake_view()["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["project"] == self.project.id.as_str())
            .unwrap()
            .clone()
    }
}

#[test]
fn github_items_reach_now_and_follow_the_source() {
    let f = fx();
    // No source yet: one `none` row, no items, no gh call.
    assert_eq!(f.source()["status"], "none");
    f.rt.intake_step(1);
    assert!(f.calls.lock().unwrap().is_empty(), "no refresh for a project with none");
    let mut pr = issue(4, &["ready"], "2026-11-01T00:00:00Z");
    pr["pull_request"] = json!({});
    f.answer(
        ISSUES,
        json!([
            issue(1, &["ready"], "2026-11-01T00:00:00Z"),
            issue(2, &["ready", "orch"], "2026-11-01T00:00:00Z"),
            issue(3, &["ready", "size:L"], "2026-11-01T00:00:00Z"),
            pr
        ]),
    );
    let r = {
        let src = json!({"type": "github", "repo": "acme/shop", "filter": {"kind": "label", "names": ["ready"]}});
        f.rt.intake_source_set(&f.project.id, &src).unwrap()
    };
    assert_eq!(r["preview"]["count"], 2);
    assert_eq!(r["preview"]["items"][0]["ref"], "#1");
    assert_eq!(f.refs(), ["#1", "#3"], "orch-labelled and PRs are not intake");
    let it = &f.items()[1];
    assert_eq!(
        (it["size"].as_str(), it["mode"].as_str(), it["can_team"].as_bool()),
        (Some("L"), Some("solo"), Some(false))
    );
    // No history, no team: the defaults, solo suggested, size L doubles them.
    assert_eq!(it["estimate"]["solo"], json!({"low_pp": 1.8, "high_pp": 4.2, "basis": "default", "samples": 0}));
    assert_eq!((it["estimate"]["size"].as_str(), it["estimate"]["room_scope"].as_str()), (Some("L"), Some("week")));
    assert_eq!(it["excerpt"], "Body of 3");
    let s = f.source();
    assert_eq!(
        (s["status"].as_str(), s["count"].as_u64(), s["label"].as_str()),
        (Some("ok"), Some(2), Some("acme/shop · label ready"))
    );
    assert!(s["url"].as_str().unwrap().contains("label:%22ready%22"));
    // The full item.
    let id = it["id"].as_str().unwrap().to_string();
    let full = f.rt.intake_item(&id).unwrap();
    assert_eq!((full["body"].as_str(), full["truncated"].as_bool()), (Some("Body of 3"), Some(false)));
    assert_eq!(full["assignees"], json!([]));
    // A closed issue leaves the lane at the next refresh.
    f.answer(ISSUES, json!([issue(3, &["ready", "size:L"], "2026-11-01T00:00:00Z")]));
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!(f.refs(), ["#3"]);
    // Hide: the item stays in `items` as hidden and is counted; an edit upstream brings it back.
    f.rt.intake_hide(&id).unwrap();
    let s = f.source();
    assert_eq!((s["count"].as_u64(), s["hidden"].as_u64()), (Some(0), Some(1)));
    assert_eq!(f.items()[0]["state"], "hidden");
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!(f.items()[0]["state"], "hidden", "unchanged upstream: still hidden");
    f.answer(ISSUES, json!([issue(3, &["ready", "size:L"], "2026-11-02T00:00:00Z")]));
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!((f.items()[0]["state"].as_str(), f.items()[0]["id"].as_str()), (Some("ready"), Some(id.as_str())));
    f.rt.intake_hide(&id).unwrap();
    f.rt.intake_unhide(&id).unwrap();
    assert_eq!(f.items()[0]["state"], "ready");
    // Read-only towards GitHub.
    for c in f.calls.lock().unwrap().iter() {
        assert!(!c.iter().any(|a| a == "-X" || a == "-f" || a == "-F" || a.contains("mutation")), "{c:?}");
    }
}

#[test]
fn an_error_keeps_the_last_good_items() {
    let f = fx();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    f.rt.db.flush().unwrap();
    f.fail(ISSUES, "To get started with GitHub CLI, please run:  gh auth login");
    f.rt.intake_refresh_project(&f.project.id);
    let s = f.source();
    assert_eq!(s["status"], "error");
    assert_eq!((s["error"]["code"].as_str(), s["error"]["fix"].as_str()), (Some("gh_auth"), Some("integrations")));
    assert!(s["refreshed_at_ms"].as_u64().is_some());
    assert_eq!(f.refs(), ["#1"], "last good items kept");
    f.rt.db.flush().unwrap();
    let errs = f.rt.db.events(0, None, Some("intake.error"), 10).unwrap();
    assert_eq!(errs.len(), 1);
    f.rt.intake_refresh_project(&f.project.id);
    f.rt.db.flush().unwrap();
    assert_eq!(f.rt.db.events(0, None, Some("intake.error"), 10).unwrap().len(), 1, "the same error is logged once");
    // Recovered.
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!((f.source()["status"].as_str(), &f.source()["error"]), (Some("ok"), &Value::Null));
}

#[test]
fn linear_items_and_key_errors() {
    let f = fx();
    *f.linear.issues.lock().unwrap() = Ok(vec![
        json!({"id": "u1", "identifier": "ENG-42", "title": "Refunds", "description": "d", "priority": 2,
               "estimate": 5, "sortOrder": 1.0, "url": "https://linear.app/acme/issue/ENG-42/refunds",
               "team": {"key": "ENG"}, "project": null,
               "updatedAt": "2026-11-01T00:00:00Z", "labels": {"nodes": [{"name": "next"}]}}),
        json!({"id": "u2", "identifier": "ENG-43", "title": "Task", "description": "", "priority": 1,
               "updatedAt": "2026-11-01T00:00:00Z", "labels": {"nodes": [{"name": "orch"}]}}),
    ]);
    let src = json!({"type": "linear", "team": "ENG", "label": "next"});
    f.rt.intake_source_set(&f.project.id, &src).unwrap();
    assert_eq!(f.refs(), ["ENG-42"]);
    assert_eq!(f.items()[0]["size"], "L");
    assert_eq!(f.source()["url"], "https://linear.app/acme/team/ENG/active");
    let full = f.rt.intake_item(f.items()[0]["id"].as_str().unwrap()).unwrap();
    assert_eq!((full["priority"].as_str(), full["points"].as_f64()), (Some("High"), Some(5.0)));
    *f.linear.issues.lock().unwrap() = Err(GqlError::Unauthorized("no Linear API key: add it".into()));
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!(f.source()["error"]["code"], "linear_key");
    assert_eq!(f.refs(), ["ENG-42"]);
    for q in f.linear.calls.lock().unwrap().iter() {
        assert!(!q.trim_start().starts_with("mutation"), "read-only: {q}");
    }
    // A preview stores nothing.
    *f.linear.issues.lock().unwrap() = Ok(vec![]);
    let p = f.rt.intake_preview(&f.project.id, &src).unwrap();
    assert_eq!((p["count"].as_u64(), &p["error"]), (Some(0), &Value::Null));
    assert_eq!(f.refs(), ["ENG-42"]);
}

#[test]
fn repo_docs_follow_the_branch_tip() {
    let f = fx();
    let repo = f.project.path.clone();
    std::fs::create_dir_all(repo.join(".orchestrator/backlog")).unwrap();
    commit_file(
        &repo,
        ".orchestrator/backlog/01-checkout.md",
        "---\nsize: L\nmode: team\nteam: web\n---\n# Checkout v2\n",
        "a",
    );
    commit_file(&repo, ".orchestrator/backlog/02-old.md", "---\nstatus: done\n---\n# Old\n", "b");
    let r = f.rt.intake_source_set(&f.project.id, &json!({"type": "repo_docs"})).unwrap();
    assert_eq!(r["preview"]["count"], 1);
    let it = &f.items()[0];
    assert_eq!(
        (it["ref"].as_str(), it["title"].as_str(), it["mode"].as_str(), it["team"].as_str(), it["url"].clone()),
        (Some(".orchestrator/backlog/01-checkout.md"), Some("Checkout v2"), Some("team"), Some("web"), Value::Null)
    );
    assert_eq!(f.source()["hidden"], 0, "status: done is absent, not hidden");
    assert_eq!(f.source()["url"], Value::Null);
    // Nothing moved: the tip check asks for nothing.
    f.rt.intake_step(orch_protocol::now_ms());
    f.rt.intake_tip_check();
    assert_ne!(lock(&f.rt.intake.sched).due.get(&f.project.id), Some(&0));
    // A commit on main: due at once, and the next pass reads it.
    commit_file(&repo, ".orchestrator/backlog/03-new.md", "# New one\n", "c");
    f.rt.intake_tip_check();
    assert_eq!(lock(&f.rt.intake.sched).due.get(&f.project.id), Some(&0));
    f.rt.intake_step(orch_protocol::now_ms());
    assert_eq!(f.refs().len(), 2);
    // Answered: due at the interval again, not refreshed on every pass.
    assert!(lock(&f.rt.intake.sched).due[&f.project.id] > orch_protocol::now_ms());
    // Options: the count the glob matches; a missing branch is a source error.
    assert_eq!(f.rt.intake_options(&f.project.id, &json!({"type": "repo_docs"})).unwrap()["matches"], 3);
    let o = f.rt.intake_options(&f.project.id, &json!({"type": "repo_docs", "branch": "nope"})).unwrap();
    assert_eq!(o["error"]["code"], "branch_missing");
    f.rt.intake_source_set(&f.project.id, &json!({"type": "repo_docs", "branch": "nope"})).unwrap();
    assert_eq!(f.source()["error"]["code"], "branch_missing");
}

#[test]
fn validation_errors_name_the_field() {
    let f = fx();
    let err = |v: Value| -> Value {
        let e = f.rt.intake_source_set(&f.project.id, &v).unwrap_err();
        serde_json::from_str(&e.to_string()).unwrap()
    };
    assert_eq!(
        err(json!({"type": "github", "repo": "x", "filter": {"kind": "label", "names": ["a"]}}))["field"],
        "repo"
    );
    assert_eq!(err(json!({"type": "repo_docs", "glob": "a/["}))["field"], "glob");
    assert_eq!(err(json!({"type": "repo_docs", "glob": "../*.md"}))["field"], "glob");
    assert_eq!(err(json!({"type": "linear", "team": ""}))["code"], "bad_request");
    assert_eq!(err(json!({"type": "jira"}))["field"], "type");
    assert!(
        f.rt.intake_preview(&f.project.id, &json!({"type": "github", "filter": {"kind": "label", "names": []}}))
            .is_err()
    );
    assert_eq!(f.rt.db.intake_source(&f.project.id).unwrap(), IntakeSource::None, "nothing stored");
    // None clears a source.
    f.rt.intake_source_set(&f.project.id, &json!({"type": "repo_docs"})).unwrap();
    f.rt.intake_source_set(&f.project.id, &Value::Null).unwrap();
    assert_eq!(f.rt.intake_source(&f.project.id).unwrap()["source"], Value::Null);
}

#[test]
fn options_take_a_partial_source() {
    let f = fx();
    f.answer("repos/acme/shop/labels", json!([{"name": "ready"}, {"name": "orch:ready"}]));
    f.answer("repos/acme/shop/milestones", json!([]));
    f.fail("graphql", "GraphQL: Your token has not been granted the required scopes: read:project");
    let o = f.rt.intake_options(&f.project.id, &json!({"type": "github", "repo": "acme/shop"})).unwrap();
    assert_eq!(o["labels"], json!(["ready"]));
    assert_eq!(o["error"]["code"], "gh_scope", "a scope error is in `error`, not a failure");
    assert!(f.rt.intake_options(&f.project.id, &json!({"type": "linear"})).is_err());
}

#[test]
fn the_refresher_staggers_the_start_and_waits_its_interval() {
    let f = fx();
    let (_rd2, repo2) = scratch_repo();
    let p2 = orch_git::project::register(&f.rt.db, &repo2).unwrap();
    for p in [&f.project.id, &p2.id] {
        f.rt.db.set_intake_source(p, &IntakeSource::RepoDocs { glob: "*.md".into(), branch: None }).unwrap();
    }
    f.rt.db.set_setting(REFRESH_SECS_KEY, &json!(120)).unwrap();
    let t0 = orch_protocol::now_ms();
    f.rt.intake_step(t0);
    let due = lock(&f.rt.intake.sched).due.clone();
    let mut ids = [f.project.id.clone(), p2.id.clone()];
    ids.sort();
    // The first is refreshed at once (due again in 120 s), the second 2 s later.
    assert!(due[&ids[0]] >= t0 + 120_000 - 1000, "{due:?}");
    assert_eq!(due[&ids[1]], t0 + STAGGER_MS);
    f.rt.intake_step(t0 + STAGGER_MS);
    assert!(lock(&f.rt.intake.sched).due[&ids[1]] > t0 + 100_000);
    // In flight: a second refresh of the same project is not started.
    lock(&f.rt.intake.sched).inflight.insert(ids[0].clone());
    f.rt.db.flush().unwrap();
    let before = f.rt.db.events(0, None, Some("intake.refreshed"), 50).unwrap().len();
    f.rt.intake_refresh_project(&ids[0]);
    f.rt.db.flush().unwrap();
    assert_eq!(f.rt.db.events(0, None, Some("intake.refreshed"), 50).unwrap().len(), before);
    let v = f.rt.intake_view();
    let s = v["sources"].as_array().unwrap().iter().find(|s| s["project"] == ids[0].as_str()).unwrap();
    assert_eq!(s["status"], "refreshing");
    lock(&f.rt.intake.sched).inflight.clear();
    // A request makes it due now.
    f.rt.intake_refresh(Some(&ids[0])).unwrap();
    assert_eq!(lock(&f.rt.intake.sched).due[&ids[0]], 0);
    assert!(f.rt.intake_refresh(Some("nope")).is_err());
}

#[test]
fn now_carries_intake_whole_and_only_on_change() {
    let f = fx();
    let events: Arc<Mutex<Vec<Value>>> = Arc::default();
    let e = events.clone();
    f.rt.subscribe_now(Arc::new(move |v: &Value| e.lock().unwrap().push(v.clone())));
    let snap = f.rt.now_snapshot();
    assert_eq!(snap["intake"]["sources"][0]["status"], "none");
    f.rt.now_pump(true);
    events.lock().unwrap().clear();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    f.rt.now_pump(false);
    let with: Vec<Value> = events.lock().unwrap().iter().filter(|v| v.get("intake").is_some()).cloned().collect();
    assert_eq!(with.last().unwrap()["intake"]["items"][0]["ref"], "#1");
    events.lock().unwrap().clear();
    // Nothing changed: the forced pump sends no intake.
    f.rt.now_pump(true);
    assert!(events.lock().unwrap().iter().all(|v| v.get("intake").is_none()));
    // The same answer again: only the refresh time moves ("updated just now").
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.rt.intake_refresh_project(&f.project.id);
    f.rt.now_pump(false);
    let sent: Vec<Value> = events.lock().unwrap().iter().filter_map(|v| v.get("intake").cloned()).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["items"], with.last().unwrap()["intake"]["items"]);
    assert_eq!(f.rt.now_snapshot()["intake"]["items"][0]["ref"], "#1");
}

// ------------------------------------------------------------ P4.5 back 2: starts, briefs, report

impl Fx {
    /// The bugfix squad as the project's default team.
    fn with_team(&self) {
        let (roles, team) = orch_team::presets::bugfix_squad();
        for r in &roles {
            self.rt.db.upsert_role(r, 1).unwrap();
        }
        self.rt.db.save_team(&team, &roles, 1).unwrap();
        let id = team.id.clone();
        self.rt
            .db
            .update_project_settings(&self.project.id, move |e| {
                e.settings.default_team = Some(id);
                Ok(())
            })
            .unwrap();
    }
    fn item_id(&self, r: &str) -> String {
        self.rt.intake_view()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["ref"] == r)
            .map(|i| i["id"].as_str().unwrap().to_string())
            .unwrap()
    }
    fn events(&self, kind: &str) -> Vec<Value> {
        self.rt.db.flush().unwrap();
        self.rt.db.events(0, None, Some(kind), 50).unwrap().into_iter().rev().map(|e| e.detail).collect()
    }
    fn launch(&self, slug: &str, intake: Option<IntakeLink>) -> anyhow::Result<orch_store::TeamRun> {
        self.rt.launch(&crate::team::LaunchSpec {
            project_id: self.project.id.clone(),
            team_id: "bugfix-squad".into(),
            slug: slug.into(),
            title: "T".into(),
            brief: "b".into(),
            intake,
            ..Default::default()
        })
    }
    /// `start_solo` without a PTY (the sandbox has none): the same claim and
    /// record around a stored, not running, session.
    fn solo(&self, id: &str, intake: Option<IntakeLink>) -> anyhow::Result<()> {
        let claim = self.rt.intake_claim(&self.project.id, intake.as_ref())?;
        let mut rec = crate::runtime::tests::test_agent(&self.project.path);
        rec.id = id.into();
        self.rt.store.write(|s| s.agents.insert(id.into(), rec)).unwrap();
        self.rt.intake_started(claim, &self.project.id, "agent", id, id);
        Ok(())
    }
    fn propose(&self, args: Value) -> Result<Value, String> {
        let mut a = json!({"project": self.project.id, "title": "Partial refunds", "brief": "Refund a line.",
                           "mode": "team"});
        for (k, v) in args.as_object().unwrap() {
            a[k] = v.clone();
        }
        self.rt.ow_propose_brief(&a).map_err(|e| format!("{}: {}", e.code, e.message))
    }
}

fn item(i: &str) -> Option<IntakeLink> {
    Some(IntakeLink { item: Some(i.into()), brief_decision: None })
}

fn brief(d: &str) -> Option<IntakeLink> {
    Some(IntakeLink { item: None, brief_decision: Some(d.into()) })
}

fn stale_of(e: anyhow::Error) -> Value {
    serde_json::from_str(&e.to_string()).unwrap_or_else(|_| panic!("not a JSON error: {e:#}"))
}

#[test]
fn prefill_builds_the_launch_and_session_forms() {
    let f = fx();
    f.with_team();
    f.answer(ISSUES, json!([issue(12, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    let id = f.item_id("#12");
    let v = f.rt.intake_view();
    assert_eq!((v["items"][0]["mode"].as_str(), v["items"][0]["team"].as_str()), (Some("team"), Some("bugfix-squad")));
    let l = f.rt.intake_prefill(Some(&id), None, None).unwrap();
    let l = &l["launch"];
    assert_eq!(
        (l["team_id"].as_str(), l["slug"].as_str(), l["title"].as_str()),
        (Some("bugfix-squad"), Some("issue-12"), Some("Issue 12"))
    );
    let b = l["brief"].as_str().unwrap();
    assert!(b.starts_with("# Issue 12\n\nFrom GitHub issue #12 · https://github.com/acme/shop/issues/12"), "{b}");
    assert!(b.ends_with("Body of 12"));
    assert_eq!((l["base"].clone(), l["off_hours"].as_bool()), (Value::Null, Some(false)));
    let s = f.rt.intake_prefill(Some(&id), None, Some("solo")).unwrap();
    let s = &s["session"];
    assert_eq!(s["name"], "issue-12");
    let m = s["message"].as_str().unwrap();
    assert!(m.starts_with("Brief from the user:\n\n# Issue 12"), "{m}");
    assert!(m.ends_with("When done, say so; the item is https://github.com/acme/shop/issues/12"), "{m}");
    // A slug taken by a feature: the next one.
    f.launch("issue-12", None).unwrap();
    assert_eq!(f.rt.intake_prefill(Some(&id), None, None).unwrap()["launch"]["slug"], "issue-12-2");
    assert!(f.rt.intake_prefill(Some(&id), None, Some("pair")).is_err());
    assert!(f.rt.intake_prefill(None, None, None).is_err());
}

#[test]
fn a_team_run_and_a_solo_session_carry_their_item_and_give_it_back() {
    let f = fx();
    f.with_team();
    f.answer(
        ISSUES,
        json!([
            issue(1, &["ready"], "2026-11-01T00:00:00Z"),
            issue(2, &["ready"], "2026-11-01T00:00:00Z"),
            issue(3, &["ready"], "2026-11-01T00:00:00Z")
        ]),
    );
    f.github_label();
    let (one, two, three) = (f.item_id("#1"), f.item_id("#2"), f.item_id("#3"));
    // Team run from #1: it leaves the lane, the run row says where it came from.
    let run = f.launch("one", item(&one)).unwrap();
    assert_eq!(f.refs(), ["#2", "#3"]);
    let started = f.events("intake.started");
    assert_eq!((started[0]["item"].as_str(), started[0]["mode"].as_str()), (Some(one.as_str()), Some("team")));
    assert!(started[0]["est_pp"].as_f64().is_some());
    f.rt.now_pump(true);
    let snap = f.rt.now_snapshot();
    let row = snap["runs"].as_array().unwrap().iter().find(|r| r["run"] == run.id.as_str()).unwrap().clone();
    assert_eq!(
        row["origin"],
        json!({"kind": "intake", "item": one, "source": "github", "ref": "#1",
                                     "url": "https://github.com/acme/shop/issues/1"})
    );
    // Started already: stale, naming the run's feature (F11).
    let e = stale_of(f.launch("again", item(&one)).unwrap_err());
    assert_eq!(e, json!({"code": "stale", "message": "already started", "started": {"kind": "run", "name": "one"}}));
    assert!(f.rt.db.feature_by_slug(&f.project.id, "again").unwrap().is_none(), "nothing launched");
    // A hidden item can't start (F12); an item of another project neither.
    f.rt.intake_hide(&three).unwrap();
    assert_eq!(stale_of(f.solo("s3", item(&three)).unwrap_err())["message"], "not ready");
    assert!(
        f.solo("s3", Some(IntakeLink { item: Some(two.clone()), brief_decision: Some("brief-1".into()) })).is_err()
    );
    // Solo session from #2.
    f.solo("fix-two", item(&two)).unwrap();
    assert!(f.refs().is_empty() || f.refs() == ["#3"]);
    assert_eq!(f.source()["count"], 0);
    // Killed before any outcome: #2 is ready again.
    f.rt.kill("fix-two").unwrap();
    assert_eq!(f.source()["count"], 1);
    assert_eq!(f.events("intake.returned")[0], json!({"item": two, "why": "session_killed"}));
    // The run deleted before any outcome: #1 too.
    let fid = f.rt.db.feature_by_slug(&f.project.id, "one").unwrap().unwrap().id;
    f.rt.delete_feature(&fid, false, false).unwrap();
    assert_eq!(f.source()["count"], 2);
    // A start with no intake is untracked.
    f.launch("other", None).unwrap();
    let u = f.events("intake.untracked_start");
    assert!(u.iter().any(|e| e["ref"] == "other" && e["kind"] == "run"), "{u:?}");
}

#[test]
fn a_session_with_an_outcome_keeps_its_item() {
    let f = fx();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    let one = f.item_id("#1");
    f.solo("fix-one", item(&one)).unwrap();
    f.rt.event("fix-one", "turn.done", json!({"summary": "done"}));
    f.rt.db.flush().unwrap();
    f.rt.kill("fix-one").unwrap();
    assert_eq!(f.source()["count"], 0, "it finished a turn: still started");
    // The report counts it.
    let r = f.rt.intake_report(true).unwrap();
    assert_eq!(r["week"]["intake"]["started"], 1);
    assert_eq!(r["projects"][0]["started"], 1);
    let text = super::report::render(&r, &jiff::tz::TimeZone::UTC);
    assert!(text.contains("This week: started from intake: 1 of 1"), "{text}");
    assert!(text.contains("acme/shop · label ready · ok · 0 ready"), "{text}");
}

#[test]
fn propose_brief_validates_replaces_and_launches_only_on_the_users_submit() {
    let f = fx();
    f.with_team();
    // Validation (api-contract §4).
    assert!(f.propose(json!({"project": "nope"})).unwrap_err().starts_with("bad_request"));
    assert!(f.propose(json!({"title": ""})).is_err());
    assert!(f.propose(json!({"title": "x".repeat(81)})).is_err());
    assert!(f.propose(json!({"brief": "x".repeat(8001)})).is_err());
    assert!(f.propose(json!({"mode": "pair"})).is_err());
    assert!(f.propose(json!({"team": "nope"})).is_err());
    assert!(f.propose(json!({"slug": "Bad Slug"})).is_err());
    f.launch("taken", None).unwrap();
    assert!(f.propose(json!({"slug": "taken"})).unwrap_err().contains("taken"));
    // A planning message, then a brief in its thread.
    let msg = f.rt.chat_send_with("Plan a feature", None, Some(&json!({"plan": f.project.id}))).unwrap();
    let first = f.propose(json!({"reply_to": msg.id()})).unwrap()["decision"].as_str().unwrap().to_string();
    f.rt.now_pump(true);
    let d = |id: &str| {
        f.rt.now_snapshot()["decisions"].as_array().unwrap().iter().find(|d| d["id"] == id).cloned().unwrap()
    };
    let card = d(&first);
    assert_eq!((card["kind"].as_str(), card["level"].as_u64()), (Some("brief"), Some(4)));
    assert_eq!(card["thread"], msg.id().as_str());
    assert_eq!(card["title"], "Partial refunds");
    let ids: Vec<&str> = card["actions"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["launch", "edit", "dismiss"]);
    assert_eq!(card["context"][2]["type"], "cost");
    assert_eq!(card["payload"]["slug"], "partial-refunds");
    assert_eq!(f.rt.now_snapshot()["counts"]["global"]["decisions"], 0, "not in needs-you");
    // A revision in the same thread replaces it.
    let second = f.propose(json!({"reply_to": msg.id(), "brief": "Refund a line, v2."})).unwrap()["decision"]
        .as_str()
        .unwrap()
        .to_string();
    f.rt.now_pump(true);
    let old = d(&first);
    assert_eq!(
        (old["lifecycle"]["effect"].as_str(), old["lifecycle"]["decided_by"].as_str()),
        (Some("revised"), Some("overwatch"))
    );
    // Launch and Edit open the dialog; nothing starts.
    let n: u64 = second.trim_start_matches("brief-").parse().unwrap();
    let r = f.rt.brief_act(n, "launch").unwrap();
    assert_eq!((r["open"].as_str(), r["intake"]["brief_decision"].as_str()), (Some("launch"), Some(second.as_str())));
    assert_eq!(r["prefill"]["launch"]["brief"], "Refund a line, v2.");
    assert_eq!(f.rt.brief_act(n, "edit").unwrap()["focus"], "brief");
    assert!(f.rt.db.feature_by_slug(&f.project.id, "partial-refunds").unwrap().is_none());
    // The revised one can't be launched.
    assert_eq!(stale_of(f.launch("x", brief(&first)).unwrap_err())["message"], "brief closed");
    // The user's submit: the run starts, the card closes as launched.
    let run = f.launch("partial-refunds", brief(&second)).unwrap();
    f.rt.now_pump(true);
    let done = d(&second);
    assert_eq!(
        (
            done["lifecycle"]["effect"].as_str(),
            done["lifecycle"]["effect_detail"].as_str(),
            done["lifecycle"]["decided_by"].as_str()
        ),
        (Some("launched"), Some("partial-refunds"), Some("user"))
    );
    let ev = f.events("intake.planned_started");
    assert_eq!((ev[0]["decision"].as_str(), ev[0]["mode"].as_str()), (Some(second.as_str()), Some("team")));
    let snap = f.rt.now_snapshot();
    let row = snap["runs"].as_array().unwrap().iter().find(|r| r["run"] == run.id.as_str()).unwrap().clone();
    assert_eq!(row["origin"], json!({"kind": "brief", "decision": second, "title": "Partial refunds"}));
    // Dismiss.
    let third = f.propose(json!({"title": "Other"})).unwrap()["decision"].as_str().unwrap().to_string();
    let n3: u64 = third.trim_start_matches("brief-").parse().unwrap();
    assert_eq!(f.rt.brief_act(n3, "dismiss").unwrap()["effect"], "dismissed");
    assert!(f.rt.brief_act(n3, "launch").is_err());
    // A removed project's open brief goes stale.
    let fourth = f.propose(json!({"title": "Fourth"})).unwrap()["decision"].as_str().unwrap().to_string();
    f.rt.briefs_stale_for(&f.project.id);
    let n4: u64 = fourth.trim_start_matches("brief-").parse().unwrap();
    assert_eq!(f.rt.lifecycle(n4)["state"], "stale");
}

#[test]
fn a_planning_message_carries_the_plan_to_the_overwatch() {
    let f = fx();
    std::fs::create_dir_all(f.project.path.join("docs")).unwrap();
    commit_file(&f.project.path, "docs/README.md", "# Docs\n", "docs");
    f.launch_free_feature();
    let e = f.rt.chat_send_with("Plan a feature for it: ", None, Some(&json!({"plan": f.project.id}))).unwrap();
    assert_eq!(e.meta.as_ref().unwrap()["plan"]["project"], f.project.id.as_str());
    let mut queued = f.rt.ow().queue.clone();
    queued.extend(f.rt.overwatch_pending());
    let ev = queued.into_iter().find(|ev| ev.chat.as_deref() == Some(&e.id())).unwrap();
    let plan = ev.plan.clone().unwrap();
    assert_eq!(plan["name"], f.project.name.as_str());
    assert!(plan["docs_dir"].as_str().unwrap().ends_with("/docs"));
    assert_eq!((plan["intake"].clone(), plan["recent_briefs"].clone()), (Value::Null, json!(["Old feature"])));
    assert!(ev.line(&jiff::tz::TimeZone::UTC).contains(" · plan {"), "{}", ev.line(&jiff::tz::TimeZone::UTC));
    assert!(f.rt.chat_send_with("x", None, Some(&json!({"plan": "nope"}))).is_err());
    assert!(f.rt.chat_send_with("x", None, Some(&json!({"plan": 3}))).is_err());
}

impl Fx {
    fn launch_free_feature(&self) {
        self.rt
            .db
            .upsert_feature(&orch_store::Feature {
                id: format!("{}-old", self.project.id),
                project_id: self.project.id.clone(),
                slug: "old".into(),
                title: "Old feature".into(),
                brief: String::new(),
                base: "main".into(),
                integration_worktree: None,
                allocation: None,
                created_at_ms: 1,
                archived: false,
            })
            .unwrap();
    }
}

/// Spec §5: no refresh, Overwatch call or card action starts a session or
/// a run; only the user's submit (`launch` / `start_solo`) does.
#[test]
fn no_path_starts_work_without_the_users_command() {
    let f = fx();
    f.with_team();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    let agents = || f.rt.store.read(|s| s.agents.len());
    let (a0, r0) = (agents(), f.rt.db.runs(None, false).unwrap().len());
    // Refreshes, the refresher's passes, the lane's commands.
    f.rt.intake_refresh(None).unwrap();
    f.rt.intake_step(orch_protocol::now_ms());
    f.rt.intake_refresh_project(&f.project.id);
    let id = f.item_id("#1");
    f.rt.intake_item(&id).unwrap();
    f.rt.intake_prefill(Some(&id), None, None).unwrap();
    f.rt.intake_prefill(Some(&id), None, Some("solo")).unwrap();
    // The Overwatch's proposal and every card action.
    let d = f.propose(json!({})).unwrap()["decision"].as_str().unwrap().to_string();
    let n: u64 = d.trim_start_matches("brief-").parse().unwrap();
    f.rt.brief_act(n, "launch").unwrap();
    f.rt.brief_act(n, "edit").unwrap();
    f.rt.now_pump(true);
    f.rt.now_snapshot();
    assert_eq!((agents(), f.rt.db.runs(None, false).unwrap().len()), (a0, r0), "nothing started");
    assert_eq!(f.source()["count"], 1, "the item is still ready");
    f.rt.db.flush().unwrap();
    let starts: Vec<_> = f.rt.db.events(0, None, Some("intake."), 100).unwrap();
    assert!(starts.iter().all(|e| !e.kind.contains("start")), "{starts:?}");
}

/// The real `start_solo` (needs a PTY: judged by CI where the sandbox has none).
#[test]
fn start_solo_checks_and_records_its_item() {
    let f = fx();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    let one = f.item_id("#1");
    let spec = |id: &str| orch_protocol::AgentSpec {
        agent_id: id.into(),
        repo: f.project.path.display().to_string(),
        ..Default::default()
    };
    f.rt.start_solo(&spec("fix-one"), &f.project.id, item(&one).as_ref()).unwrap();
    assert_eq!(f.source()["count"], 0);
    let e = stale_of(f.rt.start_solo(&spec("fix-again"), &f.project.id, item(&one).as_ref()).unwrap_err());
    assert_eq!(e["started"], json!({"kind": "agent", "name": "fix-one"}));
    assert!(f.rt.store.read(|s| !s.agents.contains_key("fix-again")), "a stale start starts nothing");
    f.rt.kill("fix-one").unwrap();
    assert_eq!(f.source()["count"], 1, "killed before any outcome: back in the lane");
}
