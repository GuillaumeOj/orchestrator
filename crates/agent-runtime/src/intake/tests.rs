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
        let key = args.get(1).cloned().unwrap_or_default();
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
    let rt = Runtime::new(cfg).unwrap();
    let (rd, repo) = scratch_repo();
    let project = orch_git::project::register(&rt.db, &repo).unwrap();
    Fx { _d: d, _rd: rd, rt, project, gh, calls, linear }
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
    assert_eq!(it["estimate"], Value::Null);
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
