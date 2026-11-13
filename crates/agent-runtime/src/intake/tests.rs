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
        // The first argument after `api` that is not a flag (`--paginate`); a GraphQL call by its operation.
        let mut key = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
        if key == "graphql"
            && let Some(q) = args.iter().find_map(|a| a.strip_prefix("query="))
            && let Some(op) = q.split_whitespace().nth(1).map(|o| o.split('(').next().unwrap_or(o))
        {
            let by_op = format!("graphql:{op}");
            if self.0.lock().unwrap().contains_key(&by_op) {
                key = by_op;
            }
        }
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
        Ok(json!({"issues": {"nodes": r}, "projects": {"nodes": []}}))
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

/// A REST-shaped test issue as the GraphQL tree query returns it (`None`: a pull request).
fn gql_issue(i: &Value) -> Option<Value> {
    if i.get("pull_request").is_some() {
        return None;
    }
    let nodes = |k: &str, key: &str| -> Vec<Value> {
        i[k].as_array().into_iter().flatten().map(|l| json!({key: l[key]})).collect()
    };
    Some(json!({"number": i["number"], "title": i["title"], "body": i["body"], "url": i["html_url"],
        "state": i["state"].as_str().unwrap_or("open").to_uppercase(), "updatedAt": i["updated_at"],
        "labels": {"nodes": nodes("labels", "name")}, "assignees": {"nodes": nodes("assignees", "login")},
        "milestone": i["milestone"], "parent": null, "subIssuesSummary": {"total": 0, "completed": 0}}))
}

impl Fx {
    /// What a user's Refresh does: a full read (the scheduled ones are incremental).
    fn refresh(&self) -> bool {
        lock(&self.rt.intake.incr).clear();
        self.rt.intake_refresh_project(&self.project.id)
    }
    /// `ISSUES` (the REST list of the old readers) answers the tree's issues query.
    fn answer(&self, key: &str, v: Value) {
        if key == ISSUES {
            let nodes: Vec<Value> = v.as_array().unwrap().iter().filter_map(gql_issue).collect();
            let page = json!({"data": {"repository": {"url": "https://github.com/acme/shop",
                "issues": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": nodes}}}});
            let mut g = self.gh.lock().unwrap();
            g.insert("graphql:OrchTreeIssues".into(), Ok(page));
            g.entry("graphql:OrchTreeMilestones".into())
                .or_insert(Ok(json!({"data": {"repository": {"milestones": {"nodes": []}}}})));
            return;
        }
        self.gh.lock().unwrap().insert(key.into(), Ok(v));
    }
    fn fail(&self, key: &str, e: &str) {
        let key = if key == ISSUES { "graphql:OrchTreeIssues" } else { key };
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
    f.refresh();
    assert_eq!(f.refs(), ["#3"]);
    // Hide: the item stays in `items` as hidden and is counted; an edit upstream brings it back.
    f.rt.intake_hide(&id).unwrap();
    let s = f.source();
    assert_eq!((s["count"].as_u64(), s["hidden"].as_u64()), (Some(0), Some(1)));
    assert_eq!(f.items()[0]["state"], "hidden");
    f.refresh();
    assert_eq!(f.items()[0]["state"], "hidden", "unchanged upstream: still hidden");
    f.answer(ISSUES, json!([issue(3, &["ready", "size:L"], "2026-11-02T00:00:00Z")]));
    f.refresh();
    assert_eq!((f.items()[0]["state"].as_str(), f.items()[0]["id"].as_str()), (Some("ready"), Some(id.as_str())));
    f.rt.intake_hide(&id).unwrap();
    f.rt.intake_unhide(&id).unwrap();
    assert_eq!(f.items()[0]["state"], "ready");
    // Read-only towards GitHub.
    for c in f.calls.lock().unwrap().iter() {
        // (the GraphQL queries pass their variables as fields)
        let graphql = c.get(1).is_some_and(|a| a == "graphql");
        assert!(
            !c.iter().any(|a| a == "-X" || a.contains("mutation") || (!graphql && (a == "-f" || a == "-F"))),
            "{c:?}"
        );
    }
}

#[test]
fn an_error_keeps_the_last_good_items() {
    let f = fx();
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.github_label();
    f.rt.db.flush().unwrap();
    f.fail(ISSUES, "To get started with GitHub CLI, please run:  gh auth login");
    f.refresh();
    let s = f.source();
    assert_eq!(s["status"], "error");
    assert_eq!((s["error"]["code"].as_str(), s["error"]["fix"].as_str()), (Some("gh_auth"), Some("integrations")));
    assert!(s["refreshed_at_ms"].as_u64().is_some());
    assert_eq!(f.refs(), ["#1"], "last good items kept");
    f.rt.db.flush().unwrap();
    let errs = f.rt.db.events(0, None, Some("intake.error"), 10).unwrap();
    assert_eq!(errs.len(), 1);
    f.refresh();
    f.rt.db.flush().unwrap();
    assert_eq!(f.rt.db.events(0, None, Some("intake.error"), 10).unwrap().len(), 1, "the same error is logged once");
    // Recovered.
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    f.refresh();
    assert_eq!((f.source()["status"].as_str(), &f.source()["error"]), (Some("ok"), &Value::Null));
}

#[test]
fn linear_items_and_key_errors() {
    let f = fx();
    *f.linear.issues.lock().unwrap() = Ok(vec![
        json!({"id": "u1", "identifier": "ENG-42", "title": "Refunds", "description": "d", "priority": 2,
               "estimate": 5, "sortOrder": 1.0, "url": "https://linear.app/acme/issue/ENG-42/refunds",
               "team": {"key": "ENG"}, "project": null, "state": {"name": "Todo", "type": "unstarted"},
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
    f.refresh();
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
    // The tracker reads the project's base branch: an old source's `branch` is not kept.
    f.rt.intake_source_set(&f.project.id, &json!({"type": "repo_docs", "branch": "nope"})).unwrap();
    assert_eq!(f.rt.db.intake_source(&f.project.id).unwrap().kind(), "repo_docs");
    assert_eq!(f.source()["error"], Value::Null);
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
    for p in [&f.project.id, &p2.id] {
        let mut t = f.rt.db.tracker(p).unwrap();
        t.refresh_secs = 300;
        f.rt.db.set_tracker(p, &t).unwrap();
    }
    let t0 = orch_protocol::now_ms();
    f.rt.intake_step(t0);
    let due = lock(&f.rt.intake.sched).due.clone();
    let mut ids = [f.project.id.clone(), p2.id.clone()];
    ids.sort();
    // The first is refreshed at once (due again in 300 s), the second 2 s later.
    assert!(due[&ids[0]] >= t0 + 300_000 - 1000, "{due:?}");
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
    f.refresh();
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
    Some(IntakeLink { item: Some(i.into()), ..Default::default() })
}

fn brief(d: &str) -> Option<IntakeLink> {
    Some(IntakeLink { brief_decision: Some(d.into()), ..Default::default() })
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
        f.solo(
            "s3",
            Some(IntakeLink { item: Some(two.clone()), brief_decision: Some("brief-1".into()), source: None })
        )
        .is_err()
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
    f.refresh();
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

// ------------------------------------------------------------ tracker (B1)

fn gh_settings(ready: Value) -> Value {
    json!({"tracker": {"kind": "github", "repo": "acme/shop", "project_board": null},
           "ready": ready, "refresh_secs": 600})
}

fn ready_labels(l: &[&str]) -> Value {
    json!({"kind": "github", "labels": l})
}

fn field(e: anyhow::Error) -> String {
    let v: Value = serde_json::from_str(&e.to_string()).unwrap();
    assert_eq!(v["code"], "bad_request");
    v["field"].as_str().unwrap().to_string()
}

#[test]
fn tracker_settings_set_validates_stores_and_refreshes() {
    let f = fx();
    let id = &f.project.id;
    // A new project: Local, nothing to ask.
    let r = f.rt.tracker_settings(id).unwrap();
    assert_eq!(r["settings"]["tracker"], json!({"kind": "local", "backlog_glob": null}));
    assert_eq!(r["migration_notice"], Value::Null);
    // Field errors, nothing stored.
    let bad = |s: Value| field(f.rt.tracker_settings_set(id, &s).unwrap_err());
    assert_eq!(bad(gh_settings(json!({"kind": "linear", "state_types": []}))), "ready");
    let mut s = gh_settings(ready_labels(&["ready"]));
    s["tracker"]["repo"] = json!("nope");
    assert_eq!(bad(s), "repo");
    let mut s = gh_settings(ready_labels(&["ready"]));
    s["refresh_secs"] = json!(7);
    assert_eq!(bad(s), "refresh_secs");
    assert_eq!(bad(json!({"tracker": {"kind": "linear", "teams": []}, "ready": {"kind": "linear"}})), "teams");
    assert_eq!(
        bad(json!({"tracker": {"kind": "local", "backlog_glob": "a/[b"}, "ready": {"kind": "local"}})),
        "backlog_glob"
    );
    assert_eq!(bad(json!({"tracker": {"kind": "bogus"}, "ready": {"kind": "local"}})), "settings");
    assert_eq!(f.rt.db.tracker(id).unwrap().tracker.kind(), "local");
    // Valid: stored, the task backend and the Ready lane follow, refreshed at once.
    f.answer(ISSUES, json!([issue(1, &["ready"], "2026-11-01T00:00:00Z")]));
    let r = f.rt.tracker_settings_set(id, &gh_settings(ready_labels(&["ready"]))).unwrap();
    assert_eq!(r["settings"]["tracker"]["repo"], "acme/shop");
    assert_eq!(
        f.rt.db.project(id).unwrap().unwrap().backend(),
        orch_store::TaskBackendConfig::Github { repo: Some("acme/shop".into()) }
    );
    assert_eq!(f.refs(), ["#1"]);
}

#[test]
fn a_tracker_save_resolves_a_mismatch_and_a_notice_is_dismissed_once() {
    let f = fx();
    let id = f.project.id.clone();
    let n = orch_store::MigrationNotice {
        kind: "mismatch".into(),
        from: json!({}),
        kept: orch_store::Tracker::default(),
        alternative: None,
        dismissed: false,
    };
    f.rt.db.set_tracker_notice(&id, Some(&n)).unwrap();
    assert_eq!(f.rt.tracker_settings(&id).unwrap()["migration_notice"]["kind"], "mismatch");
    let r = f.rt.tracker_settings_set(&id, &json!({"tracker": {"kind": "local"}, "ready": {"kind": "local"}})).unwrap();
    assert_eq!(r["migration_notice"], Value::Null);
    f.rt.tracker_notice_dismiss(&id).unwrap();
    assert!(f.rt.tracker_notice_dismiss("nope").is_err());
}

#[test]
fn tracker_preview_is_a_dry_run_over_the_listing() {
    let f = fx();
    f.answer(
        ISSUES,
        json!([issue(1, &["ready"], "2026-11-01T00:00:00Z"), issue(2, &["ready"], "2026-11-01T00:00:00Z")]),
    );
    let p = f.rt.tracker_preview(&f.project.id, &gh_settings(ready_labels(&["ready"]))).unwrap();
    assert_eq!((p["roots"].as_u64(), p["items"].as_u64(), p["ready"].as_u64()), (Some(2), Some(2), Some(2)));
    assert_eq!(p["sample"][0]["id"], "gh:acme/shop#1");
    assert_eq!(p["sample"][0]["ready"], true);
    assert_eq!(p["error"], Value::Null);
    assert_eq!(f.rt.db.tracker(&f.project.id).unwrap().tracker.kind(), "local", "nothing stored");
    // A source error is in `error`.
    f.fail(ISSUES, "gh auth login required");
    let p = f.rt.tracker_preview(&f.project.id, &gh_settings(ready_labels(&["ready"]))).unwrap();
    assert_eq!(p["ready"], 0);
    assert!(p["error"]["code"].is_string());
    // Invalid settings are a field error.
    assert_eq!(
        field(f.rt.tracker_preview(&f.project.id, &gh_settings(json!({"kind": "local"}))).unwrap_err()),
        "ready"
    );
}

#[test]
fn tracker_options_add_issue_types_and_boards() {
    let f = fx();
    f.answer("repos/acme/shop/labels", json!([{"name": "ready"}, {"name": "orch:ready"}]));
    f.answer("repos/acme/shop/milestones", json!([{"title": "v2", "open_issues": 3}]));
    f.answer(
        "graphql",
        json!({"data": {"repositoryOwner": {"projectsV2": {"nodes": [
            {"number": 4, "title": "Roadmap", "closed": false,
             "field": {"options": [{"name": "Todo"}, {"name": "In progress"}]}}]}},
            "repository": {"issueTypes": {"nodes": [{"name": "Epic"}, {"name": "Feature"}]}}}}),
    );
    let o = f.rt.tracker_options(&f.project.id, &json!({"kind": "github", "repo": "acme/shop"})).unwrap();
    assert_eq!(o["labels"], json!(["ready"]));
    assert_eq!(o["milestones"][0]["title"], "v2");
    assert_eq!(o["issue_types"], json!(["Epic", "Feature"]));
    assert_eq!(
        o["boards"][0],
        json!({"owner": "acme", "number": 4, "title": "Roadmap", "statuses": ["Todo", "In progress"]})
    );
    assert!(o.get("projects").is_none());
    // A bad repo is a field error; a source error comes back in `error`.
    assert_eq!(
        field(f.rt.tracker_options(&f.project.id, &json!({"kind": "github", "repo": "x"})).unwrap_err()),
        "repo"
    );
    f.fail("repos/acme/shop/labels", "HTTP 401: Bad credentials");
    let o = f.rt.tracker_options(&f.project.id, &json!({"kind": "github", "repo": "acme/shop"})).unwrap();
    assert!(o["error"]["code"].is_string());
    // Linear: teams, and projects / labels / states of the picked ones.
    let o = f.rt.tracker_options(&f.project.id, &json!({"kind": "linear", "teams": []})).unwrap();
    assert_eq!(o["teams"], json!([]));
    // Local: how many files match.
    let o = f.rt.tracker_options(&f.project.id, &json!({"kind": "local", "backlog_glob": "*.md"})).unwrap();
    assert!(o["matches"].is_u64());
}

#[test]
fn the_migrated_ready_default_is_checked_once_by_the_refresher_and_a_failed_check_stays_pending() {
    let f = fx();
    let id = f.project.id.clone();
    let mut t = orch_store::TrackerSettings::default()
        .with_backend(&orch_store::TaskBackendConfig::Github { repo: Some("acme/shop".into()) });
    t.ready = orch_store::ReadyFilter::defaults("github");
    f.rt.db.set_tracker(&id, &t).unwrap();
    f.answer(ISSUES, json!([]));
    let mark = || f.rt.db.set_setting(&format!("tracker.ready_pending:{id}"), &json!(true)).unwrap();
    let ready = || f.rt.tracker_settings(&id).unwrap()["settings"]["ready"]["labels"].clone();
    let asks = || f.calls.lock().unwrap().iter().filter(|c| c.iter().any(|a| a.contains("labels/ready"))).count();
    // The settings read asks nothing.
    mark();
    assert_eq!(ready(), json!(["ready"]));
    assert_eq!(asks(), 0, "no network read on a settings read");
    // The check fails: the default stays and stays pending.
    f.fail("repos/acme/shop/labels/ready", "HTTP 500");
    f.refresh();
    assert_eq!(ready(), json!(["ready"]));
    assert!(f.rt.db.tracker_ready_pending(&id).unwrap());
    // The repository has no such label: emptied, once.
    f.fail("repos/acme/shop/labels/ready", "HTTP 404: Not Found");
    f.refresh();
    assert_eq!(ready(), json!([]));
    assert!(!f.rt.db.tracker_ready_pending(&id).unwrap());
    let n = asks();
    f.refresh();
    assert_eq!(asks(), n, "settled: asked no more");
    // With the label: kept.
    f.rt.db.set_tracker(&id, &t).unwrap();
    mark();
    f.answer("repos/acme/shop/labels/ready", json!({"name": "ready"}));
    f.refresh();
    assert_eq!(ready(), json!(["ready"]));
    assert!(!f.rt.db.tracker_ready_pending(&id).unwrap());
}

#[test]
fn the_old_intake_commands_still_answer_through_the_tracker() {
    let f = fx();
    f.github_label();
    let t = f.rt.db.tracker(&f.project.id).unwrap();
    assert_eq!(t.tracker, orch_store::Tracker::Github { repo: Some("acme/shop".into()), project_board: None });
    assert_eq!(t.ready, orch_store::ReadyFilter::defaults("github"));
    let s = f.rt.intake_source(&f.project.id).unwrap();
    assert_eq!(s["source"]["filter"]["names"], json!(["ready"]));
    f.rt.intake_source_set(&f.project.id, &Value::Null).unwrap();
    assert!(f.rt.intake_source(&f.project.id).unwrap()["source"].is_null());
}

// ------------------------------------------------------------ the tree (B2)

fn tree_issue(n: u64, extra: Value) -> Value {
    let mut v = json!({"number": n, "title": format!("Issue {n}"), "body": format!("Body of {n}"),
        "url": format!("https://github.com/acme/shop/issues/{n}"), "state": "OPEN",
        "updatedAt": "2026-11-01T00:00:00Z", "labels": {"nodes": [{"name": "ready"}]},
        "assignees": {"nodes": []}, "milestone": null, "parent": null,
        "subIssuesSummary": {"total": 0, "completed": 0}});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

fn answer_tree(f: &Fx, issues: Vec<Value>, milestones: Vec<Value>) {
    f.answer(
        "graphql:OrchTreeIssues",
        json!({"data": {"repository": {"url": "https://github.com/acme/shop",
            "issues": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": issues}}}}),
    );
    f.answer("graphql:OrchTreeMilestones", json!({"data": {"repository": {"milestones": {"nodes": milestones}}}}));
}

fn github_tracker(f: &Fx) {
    let mut t = f.rt.db.tracker(&f.project.id).unwrap();
    t.tracker = orch_store::Tracker::Github { repo: Some("acme/shop".into()), project_board: None };
    t.ready = orch_store::ReadyFilter::defaults("github");
    f.rt.db.set_tracker(&f.project.id, &t).unwrap();
}

fn milestone(n: u64, title: &str) -> Value {
    json!({"number": n, "title": title, "url": "https://github.com/acme/shop/milestone/3", "dueOn": null,
           "updatedAt": "2026-11-01T00:00:00Z", "open": {"totalCount": 2}, "closed": {"totalCount": 0}})
}

#[test]
fn the_lane_shows_ready_nodes_with_their_breadcrumb_and_actions_and_the_tree_stays_cached() {
    let f = fx();
    github_tracker(&f);
    answer_tree(
        &f,
        vec![
            tree_issue(
                1,
                json!({"milestone": {"number": 3, "title": "v2"}, "labels": {"nodes": []},
                "subIssuesSummary": {"total": 1, "completed": 0}}),
            ),
            tree_issue(2, json!({"parent": {"number": 1}, "milestone": {"number": 3, "title": "v2"}})),
            tree_issue(3, json!({"labels": {"nodes": [{"name": "bug"}]}})),
        ],
        vec![milestone(3, "v2")],
    );
    f.refresh();
    // Only #2 is ready (the label): #1 is its parent, #3 has another label, the milestone is a container.
    assert_eq!(f.refs(), ["#2"]);
    let it = &f.items()[0];
    assert_eq!(it["node"], "gh:acme/shop#2");
    assert_eq!(
        it["breadcrumb"],
        json!([{"id": "gh-milestone:acme/shop/3", "ref": "v2", "title": "v2"},
               {"id": "gh:acme/shop#1", "ref": "#1", "title": "Issue 1"}])
    );
    assert_eq!(it["actions"]["solo"], "ok");
    assert_eq!(it["actions"]["team"], json!({"disabled": "This project has no team yet"}));
    assert_eq!(it["actions"]["program"], "hidden", "a leaf is not run as a program");
    assert_eq!(f.source()["counts"], json!({"items": 4, "roots": 2, "ready": 1}), "milestone and #3 are roots");
    // The whole tree is cached, the lane only shows what is ready.
    assert_eq!(f.rt.db.work_nodes(Some(&f.project.id)).unwrap().len(), 4);
    // The Ready filter is read when the lane is: no refetch.
    let mut t = f.rt.db.tracker(&f.project.id).unwrap();
    t.ready = orch_store::ReadyFilter::Github {
        labels: vec!["bug".into()],
        milestone: None,
        board_status: None,
        issue_type: None,
    };
    let n = f.calls.lock().unwrap().len();
    f.rt.db.set_tracker(&f.project.id, &t).unwrap();
    assert_eq!(f.refs(), ["#3"]);
    assert_eq!(f.calls.lock().unwrap().len(), n);
    assert_eq!(f.rt.db.work_nodes(Some(&f.project.id)).unwrap().len(), 4);
}

#[test]
fn scheduled_reads_are_incremental_and_a_users_refresh_is_full() {
    let f = fx();
    github_tracker(&f);
    answer_tree(&f, vec![tree_issue(1, json!({})), tree_issue(2, json!({}))], vec![]);
    let tree_calls = |f: &Fx| -> Vec<Vec<String>> {
        f.calls.lock().unwrap().iter().filter(|c| c.iter().any(|a| a.contains("OrchTreeIssues"))).cloned().collect()
    };
    f.refresh();
    assert_eq!(f.refs(), ["#1", "#2"]);
    assert!(tree_calls(&f).iter().all(|c| !c.iter().any(|a| a.starts_with("since=") && !a.contains("2026-0"))));
    let before = tree_calls(&f).len();
    // A scheduled read: what changed since (one query, open and closed), merged into the cache.
    answer_tree(&f, vec![tree_issue(2, json!({"title": "Renamed"})), tree_issue(3, json!({}))], vec![]);
    assert!(f.rt.intake_refresh_project(&f.project.id));
    let calls = tree_calls(&f);
    assert_eq!(calls.len(), before + 1);
    let last = calls.last().unwrap();
    assert!(last.iter().any(|a| a == "states[]=OPEN") && last.iter().any(|a| a == "states[]=CLOSED"));
    assert!(last.iter().any(|a| a.starts_with("since=")), "{last:?}");
    assert_eq!(f.refs(), ["#1", "#2", "#3"], "#1 was not listed: an incremental read removes nothing");
    assert_eq!(f.items().iter().find(|i| i["ref"] == "#2").unwrap()["title"], "Renamed");
    // A user's refresh reads everything again: #1 is gone.
    answer_tree(&f, vec![tree_issue(2, json!({})), tree_issue(3, json!({}))], vec![]);
    f.rt.intake_request(Some(&f.project.id));
    f.rt.intake_refresh_project(&f.project.id);
    assert_eq!(f.refs(), ["#2", "#3"]);
}

#[test]
fn offline_keeps_the_cache_and_says_so() {
    let f = fx();
    github_tracker(&f);
    answer_tree(&f, vec![tree_issue(1, json!({}))], vec![]);
    f.refresh();
    assert_eq!(f.refs(), ["#1"]);
    f.fail("graphql:OrchTreeIssues", "dial tcp: lookup api.github.com: no such host; network is unreachable");
    f.refresh();
    let s = f.source();
    assert_eq!((s["offline"].as_bool(), &s["error"], s["status"].as_str()), (Some(true), &Value::Null, Some("ok")));
    assert!(s["refreshed_at_ms"].as_u64().is_some());
    assert_eq!(f.refs(), ["#1"], "the cache is shown");
}

#[test]
fn a_refresh_in_flight_is_joined_not_repeated() {
    let f = fx();
    github_tracker(&f);
    answer_tree(&f, vec![tree_issue(1, json!({}))], vec![]);
    let id = f.project.id.clone();
    lock(&f.rt.intake.sched).inflight.insert(id.clone());
    let rt = f.rt.clone();
    let end = id.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        lock(&rt.intake.sched).inflight.remove(&end);
        rt.intake.wake.notify_all();
    });
    let t0 = std::time::Instant::now();
    assert!(f.rt.intake_join(&id));
    assert!(t0.elapsed() >= std::time::Duration::from_millis(200), "it waited for the refresh in flight");
    h.join().unwrap();
    assert!(f.calls.lock().unwrap().is_empty(), "and did not start another");
    // Nothing in flight: it runs one.
    assert!(f.rt.intake_join(&id));
    assert!(!f.calls.lock().unwrap().is_empty());
    assert_eq!(f.refs(), ["#1"]);
}

#[test]
fn the_preview_reads_the_tree_with_counts_a_breadcrumb_a_warning_and_the_backlog_files() {
    let f = fx();
    answer_tree(
        &f,
        vec![
            tree_issue(
                1,
                json!({"labels": {"nodes": []}, "milestone": {"number": 3, "title": "v2"},
                "subIssuesSummary": {"total": 1, "completed": 0}}),
            ),
            tree_issue(2, json!({"parent": {"number": 1}})),
            tree_issue(3, json!({})),
        ],
        vec![milestone(3, "v2")],
    );
    let p = f.rt.tracker_preview(&f.project.id, &gh_settings(ready_labels(&["ready"]))).unwrap();
    assert_eq!((p["roots"].as_u64(), p["items"].as_u64(), p["ready"].as_u64()), (Some(2), Some(3), Some(2)));
    let s = &p["sample"][0];
    assert_eq!((s["id"].as_str(), s["depth"].as_u64()), (Some("gh:acme/shop#2"), Some(3)));
    assert_eq!(s["breadcrumb"].as_array().unwrap().len(), 2);
    assert_eq!(p["sample"][1]["breadcrumb"], json!([]));
    assert_eq!((p["warning"].clone(), p["error"].clone()), (Value::Null, Value::Null));
    assert_eq!(f.rt.db.tracker(&f.project.id).unwrap().tracker.kind(), "local", "nothing stored");
    // Local: the backlog files, by name.
    let repo = f.project.path.clone();
    std::fs::create_dir_all(repo.join(".orchestrator/backlog")).unwrap();
    commit_file(&repo, ".orchestrator/backlog/01-a.md", "# A\n", "a");
    commit_file(&repo, ".orchestrator/backlog/02-b.md", "# B\n", "b");
    let local =
        json!({"tracker": {"kind": "local", "backlog_glob": ".orchestrator/backlog/*.md"}, "ready": {"kind": "local"}});
    let p = f.rt.tracker_preview(&f.project.id, &local).unwrap();
    assert_eq!((p["ready"].as_u64(), p["items"].as_u64()), (Some(2), Some(2)));
    assert_eq!(p["files"], json!([".orchestrator/backlog/01-a.md", ".orchestrator/backlog/02-b.md"]));
    let o =
        f.rt.tracker_options(&f.project.id, &json!({"kind": "local", "backlog_glob": ".orchestrator/backlog/*.md"}))
            .unwrap();
    assert_eq!((o["matches"].as_u64(), o["files"].clone()), (Some(2), p["files"].clone()));
}

// ------------------------------------------------------------ Work commands (B3)

fn scope(f: &Fx) -> Value {
    json!({"projects": [f.project.id]})
}

fn ids(v: &Value) -> Vec<String> {
    v["nodes"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap().to_string()).collect()
}

/// A tree: milestone v2 › #1 (2 sub-issues: #2 open, #3 closed) and a root #4 (label bug).
fn work_fixture() -> Fx {
    let f = fx();
    github_tracker(&f);
    answer_tree(
        &f,
        vec![
            tree_issue(
                1,
                json!({"milestone": {"number": 3, "title": "v2"}, "labels": {"nodes": []},
                       "subIssuesSummary": {"total": 2, "completed": 1}}),
            ),
            tree_issue(2, json!({"parent": {"number": 1}})),
            tree_issue(3, json!({"parent": {"number": 1}, "state": "CLOSED", "labels": {"nodes": []}})),
            tree_issue(
                4,
                json!({"labels": {"nodes": [{"name": "ready"}, {"name": "bug"}]}, "assignees": {"nodes": [{"login": "ann"}]}}),
            ),
        ],
        vec![milestone(3, "v2")],
    );
    f.refresh();
    f
}

#[test]
fn work_tree_serves_roots_children_filters_and_ancestors_from_the_cache() {
    let f = work_fixture();
    let calls = f.calls.lock().unwrap().len();
    let r = f.rt.work_tree(&scope(&f), &Value::Null, &json!({}), None).unwrap();
    assert_eq!(ids(&r), ["gh-milestone:acme/shop/3", "gh:acme/shop#4"]);
    assert_eq!(r["status"][&f.project.id]["counts"], json!({"items": 5, "roots": 2, "ready": 2}));
    let ms = &r["nodes"][0];
    assert_eq!((ms["kind"].as_str(), ms["depth"].as_u64()), (Some("milestone"), Some(1)));
    assert_eq!(ms["actions"]["solo"], "hidden", "a container starts nothing");
    // Children of the milestone, then of #1: open ones by default, the closed one with the State filter.
    let k = f.rt.work_tree(&scope(&f), &json!(["gh-milestone:acme/shop/3"]), &json!({}), None).unwrap();
    assert_eq!(ids(&k), ["gh:acme/shop#1"]);
    let one = &k["nodes"][0];
    assert_eq!(one["children"], json!({"total": 2, "open": 1, "done": 1}));
    assert_eq!(one["actions"]["program"], "ok");
    assert_eq!(one["actions"]["team"], json!({"disabled": "This project has no team yet"}));
    let c = f.rt.work_tree(&scope(&f), &json!(["gh:acme/shop#1"]), &json!({}), None).unwrap();
    assert_eq!(ids(&c), ["gh:acme/shop#2"]);
    let all = f.rt.work_tree(&scope(&f), &json!(["gh:acme/shop#1"]), &json!({"state": "all"}), None).unwrap();
    assert_eq!(ids(&all), ["gh:acme/shop#2", "gh:acme/shop#3"]);
    let closed = f.rt.work_tree(&scope(&f), &json!(["gh:acme/shop#1"]), &json!({"state": "closed"}), None).unwrap();
    assert_eq!(ids(&closed), ["gh:acme/shop#3"]);
    assert_eq!(
        closed["nodes"][0]["actions"]["solo"],
        json!({"disabled": "Closed in GitHub. Reopen it there to start it."})
    );
    // A filtered tree keeps the ancestors of what matches, flagged.
    let bug = f.rt.work_tree(&scope(&f), &Value::Null, &json!({"labels": ["bug"]}), None).unwrap();
    assert_eq!(ids(&bug), ["gh:acme/shop#4"]);
    let ready = f.rt.work_tree(&scope(&f), &Value::Null, &json!({"only_ready": true}), None).unwrap();
    assert_eq!(ids(&ready), ["gh-milestone:acme/shop/3", "gh:acme/shop#4"], "{ready}");
    assert_eq!(
        (ready["nodes"][0]["ancestor_only"].clone(), ready["nodes"][1].get("ancestor_only")),
        (json!(true), None)
    );
    // `@me` is the tracker's user, resolved at refresh time and cached.
    f.rt.db.set_setting(&super::work::viewer_key(&f.project.id), &json!("ann")).unwrap();
    let mine = f.rt.work_tree(&scope(&f), &Value::Null, &json!({"assignees": ["@me"]}), None).unwrap();
    assert_eq!(ids(&mine), ["gh:acme/shop#4"]);
    // Several parents in one call (restore), and everything came from the cache.
    let two =
        f.rt.work_tree(&scope(&f), &json!(["gh-milestone:acme/shop/3", "gh:acme/shop#1"]), &json!({}), None).unwrap();
    assert_eq!(ids(&two), ["gh:acme/shop#1", "gh:acme/shop#2"]);
    assert_eq!(f.calls.lock().unwrap().len(), calls, "no network read");
}

#[test]
fn work_tree_pages_the_roots_by_cursor() {
    let f = fx();
    github_tracker(&f);
    answer_tree(&f, (1..=130).map(|n| tree_issue(n, json!({}))).collect(), vec![]);
    f.refresh();
    let first = f.rt.work_tree(&scope(&f), &Value::Null, &json!({}), None).unwrap();
    assert_eq!(first["nodes"].as_array().unwrap().len(), 100);
    assert_eq!(first["more"]["roots"], json!({"left": 30, "cursor": "100"}));
    let rest = f.rt.work_tree(&scope(&f), &Value::Null, &json!({}), Some("100")).unwrap();
    assert_eq!(rest["nodes"].as_array().unwrap().len(), 30);
    assert!(rest["more"].as_object().unwrap().is_empty());
    assert_eq!(rest["nodes"][0]["id"], "gh:acme/shop#101");
}

#[test]
fn app_state_rollup_and_actions_follow_what_runs() {
    let f = work_fixture();
    let id = |n: u64| f.rt.db.work_node_rows(&format!("gh:acme/shop#{n}")).unwrap().into_iter().next().unwrap().id;
    // #2 runs as a solo session, #4 as a team run that reached its PR; #1's run merged.
    f.rt.db.intake_mark_started(id(2), "agent", "fix-feed").unwrap();
    let feature = orch_store::Feature {
        id: "f1".into(),
        project_id: f.project.id.clone(),
        slug: "checkout".into(),
        title: "t".into(),
        brief: String::new(),
        base: "main".into(),
        integration_worktree: None,
        allocation: None,
        created_at_ms: 1,
        archived: false,
    };
    f.rt.db.upsert_feature(&feature).unwrap();
    let run = |rid: &str, status, pr: Option<&str>| orch_store::TeamRun {
        id: rid.into(),
        team_id: "t".into(),
        team_scope: Default::default(),
        team_version: 1,
        feature_id: "f1".into(),
        status,
        resume_status: None,
        pr_url: pr.map(str::to_string),
        started_at_ms: 1,
        ended_at_ms: Some(9),
        pr_number: pr.map(|_| 61),
        pr: None,
    };
    f.rt.db.upsert_run(&run("r4", orch_team::RunState::PrOpen, Some("https://github.com/acme/shop/pull/61"))).unwrap();
    f.rt.db.upsert_run(&run("r1", orch_team::RunState::Done, None)).unwrap();
    f.rt.db.intake_mark_started(id(4), "run", "r4").unwrap();
    // The team of run r4 wrote two tasks: counted on the node, never children, listed as fold rows.
    for title in ["API client", "Form"] {
        let mut t = orch_store::Task::new(&f.project.id, "github", title, 1);
        t.team_run_id = Some("r4".into());
        t.node = Some("back".into());
        f.rt.db.upsert_task(&t).unwrap();
    }
    let nodes = |parents: Value| f.rt.work_tree(&scope(&f), &parents, &json!({"state": "all"}), None).unwrap();
    let roots = nodes(Value::Null);
    let four = roots["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "gh:acme/shop#4").unwrap().clone();
    assert_eq!(
        four["app"],
        json!({"state": "pr", "feature": "checkout", "pr": {"number": 61, "url": "https://github.com/acme/shop/pull/61"}})
    );
    assert_eq!((four["actions"]["team"].clone(), four["actions"]["solo"].clone()), (json!("hidden"), json!("hidden")));
    assert_eq!((four["team_tasks"].as_u64(), four["children"]["total"].as_u64()), (Some(2), Some(0)));
    let fold =
        f.rt.work_tree(&scope(&f), &json!(["gh:acme/shop#4"]), &json!({"include_team_tasks": true}), None).unwrap();
    assert_eq!(fold["nodes"].as_array().unwrap().len(), 2);
    let t0 = &fold["nodes"][0];
    assert_eq!(
        (t0["team_task"].as_bool(), t0["run"].as_str(), t0["feature"].as_str(), t0["tracker"].as_str()),
        (Some(true), Some("r4"), Some("checkout"), Some("github"))
    );
    assert_eq!(t0["upstream"]["assignees"], json!(["back"]));
    let plain = f.rt.work_tree(&scope(&f), &json!(["gh:acme/shop#4"]), &json!({}), None).unwrap();
    assert!(plain["nodes"].as_array().unwrap().is_empty(), "the fold is asked for");
    let kids = nodes(json!(["gh:acme/shop#1"]));
    assert_eq!(kids["nodes"][0]["app"], json!({"state": "solo", "agent": "fix-feed", "name": "fix-feed"}));
    // The roll-up of the parent counts its descendants: one running (the solo session).
    let one = &nodes(json!(["gh-milestone:acme/shop/3"]))["nodes"][0];
    assert_eq!(
        one["rollup"],
        json!({"running": 1, "pr": 0, "waiting": 0, "queued": 0, "merged": 0, "failed": 0, "skipped": 0})
    );
    assert_eq!(roots["nodes"][0]["rollup"]["running"], 1, "the milestone sees it too");
    assert_eq!(one["app"], json!({"state": "none"}));
    // A node whose run merged is `done`, can run again, and says what it ran as.
    f.rt.db.intake_mark_started(id(1), "run", "r1").unwrap();
    let again = f.rt.work_node("gh:acme/shop#1", &json!({})).unwrap();
    assert_eq!(again["app"]["state"], "done");
    assert_eq!(again["actions"]["solo"], "ok");
    let pre = f.rt.work_prefill("gh:acme/shop#1", Some("solo")).unwrap();
    assert_eq!(pre["ran_before"]["feature"], "checkout");
    // #4 is at its PR: not startable.
    assert!(f.rt.work_prefill("gh:acme/shop#4", None).is_err());
}

#[test]
fn work_search_ranks_ref_title_body_with_a_snippet_a_breadcrumb_and_a_root() {
    let f = fx();
    github_tracker(&f);
    answer_tree(
        &f,
        vec![
            tree_issue(
                12,
                json!({"title": "Checkout v2", "body": "Overview", "subIssuesSummary": {"total": 1, "completed": 0}}),
            ),
            tree_issue(
                13,
                json!({"title": "Address step", "body": "The postcode lookup **fails** for new addresses",
                "parent": {"number": 12}}),
            ),
            tree_issue(14, json!({"title": "Postcode lookup", "body": "x"})),
            tree_issue(
                15,
                json!({"title": "Other", "body": "Mentions address checkout flow", "labels": {"nodes": [{"name": "bug"}]}}),
            ),
        ],
        vec![],
    );
    f.refresh();
    let s = f.rt.work_search(&scope(&f), "#12", &json!({})).unwrap();
    assert_eq!(ids(&s), ["gh:acme/shop#12"]);
    assert_eq!(s["nodes"][0]["match"], json!({"field": "ref", "snippet": "#12"}));
    let s = f.rt.work_search(&scope(&f), "address", &json!({})).unwrap();
    assert_eq!(ids(&s), ["gh:acme/shop#13", "gh:acme/shop#15"], "title before body");
    assert_eq!(s["nodes"][0]["match"]["field"], "title");
    assert_eq!(s["nodes"][1]["match"]["field"], "body");
    assert!(s["nodes"][1]["match"]["snippet"].as_str().unwrap().contains("address"));
    assert!(s["nodes"][1]["match"]["snippet"].as_str().unwrap().chars().count() <= 82);
    assert_eq!(s["nodes"][0]["breadcrumb"], json!([{"id": "gh:acme/shop#12", "ref": "#12", "title": "Checkout v2"}]));
    // Filters hide matches and say how many; a root restricts the search to its subtree.
    let s = f.rt.work_search(&scope(&f), "address", &json!({"labels": ["bug"]})).unwrap();
    assert_eq!((s["total"].as_u64(), s["hidden_by_filters"].as_u64()), (Some(1), Some(1)));
    let s =
        f.rt.work_search(&json!({"projects": [f.project.id], "root": "gh:acme/shop#12"}), "address", &json!({}))
            .unwrap();
    assert_eq!(ids(&s), ["gh:acme/shop#13"]);
    assert_eq!(f.rt.work_search(&scope(&f), "  ", &json!({})).unwrap()["total"], 0);
    assert_eq!(f.rt.work_search(&scope(&f), "zzz", &json!({})).unwrap()["total"], 0);
}

#[test]
fn work_node_reveal_hide_and_refresh() {
    let f = work_fixture();
    let n = f.rt.work_node("gh:acme/shop#2", &json!({})).unwrap();
    assert_eq!((n["body"].as_str(), n["body_clipped"].as_bool()), (Some("Body of 2"), Some(false)));
    assert_eq!(
        n["breadcrumb"],
        json!([{"id": "gh-milestone:acme/shop/3", "ref": "v2", "title": "v2"}, {"id": "gh:acme/shop#1", "ref": "#1", "title": "Issue 1"}])
    );
    assert_eq!(n["history"], json!([]));
    assert!(n.get("child_nodes").is_none());
    let p = f.rt.work_node("gh:acme/shop#1", &json!({"children": true})).unwrap();
    assert_eq!(p["child_nodes"].as_array().unwrap().len(), 2);
    assert!(f.rt.work_node("gh:nope#1", &json!({})).is_err());
    // Reveal: the chain and the siblings of each link.
    let r = f.rt.work_reveal("gh:acme/shop#2").unwrap();
    assert_eq!(r["chain"], json!(["gh-milestone:acme/shop/3", "gh:acme/shop#1", "gh:acme/shop#2"]));
    let got = ids(&r);
    for want in ["gh:acme/shop#4", "gh:acme/shop#2", "gh:acme/shop#3", "gh-milestone:acme/shop/3"] {
        assert!(got.contains(&want.to_string()), "{want} in {got:?}");
    }
    // Hide: the node leaves the lane, stays in the tree, and comes back.
    assert_eq!(f.refs(), ["#2", "#4"]);
    let state4 = |f: &Fx| f.items().iter().find(|i| i["ref"] == "#4").unwrap()["state"].clone();
    f.rt.work_hide("gh:acme/shop#4").unwrap();
    assert_eq!(state4(&f), "hidden");
    let t = f.rt.work_tree(&scope(&f), &Value::Null, &json!({}), None).unwrap();
    let four = t["nodes"].as_array().unwrap().iter().find(|x| x["id"] == "gh:acme/shop#4").unwrap().clone();
    assert_eq!((four["hidden"].as_bool(), four["actions"]["hide"].as_str()), (Some(true), Some("ok")));
    f.rt.work_unhide("gh:acme/shop#4").unwrap();
    assert_eq!(state4(&f), "ready");
    // Refresh returns at once and asks for a full read.
    lock(&f.rt.intake.incr).insert(f.project.id.clone(), (1, 1));
    f.rt.work_refresh(Some(&f.project.id)).unwrap();
    assert!(lock(&f.rt.intake.incr).get(&f.project.id).is_none());
    assert!(f.rt.work_refresh(Some("nope")).is_err());
}

#[test]
fn work_prefill_adds_origin_scope_blockers_and_the_write_back_line() {
    let f = fx();
    github_tracker(&f);
    answer_tree(
        &f,
        vec![
            tree_issue(12, json!({"title": "Checkout v2", "subIssuesSummary": {"total": 1, "completed": 0}})),
            tree_issue(
                13,
                json!({"title": "Address step", "parent": {"number": 12}, "issueType": {"name": "Bug"},
                    "subIssuesSummary": {"total": 2, "completed": 1},
                    "blockedBy": {"nodes": [{"number": 11, "title": "Postcode API", "state": "OPEN", "url": "u",
                        "repository": {"nameWithOwner": "acme/shop"}}]}}),
            ),
            tree_issue(
                17,
                json!({"title": "Address validation", "parent": {"number": 13}, "issueType": {"name": "Bug"}}),
            ),
            tree_issue(16, json!({"title": "Postcode lookup", "parent": {"number": 13}, "state": "CLOSED"})),
        ],
        vec![],
    );
    f.refresh();
    let p = f.rt.work_prefill("gh:acme/shop#13", Some("team")).unwrap();
    let brief = p["launch"]["brief"].as_str().unwrap();
    assert!(brief.starts_with("# Address step (#13)\n\nIn acme/shop › #12 Checkout v2."), "{brief}");
    assert!(
        brief.contains("## Scope: sub-issues\n- [ ] #17 Address validation (Bug)\n- [x] #16 Postcode lookup (closed)"),
        "{brief}"
    );
    assert!(brief.contains("## Depends on\n- #11 Postcode API (open)"), "{brief}");
    assert_eq!(p["open_sub_issues"], 1);
    assert_eq!(p["blocked_by_open"][0]["ref"], "#11");
    assert_eq!(p["ran_before"], Value::Null);
    assert_eq!(
        p["origin"],
        json!({"id": "gh:acme/shop#13", "ref": "#13", "title": "Address step", "url": "https://github.com/acme/shop/issues/13",
               "breadcrumb": [{"id": "gh:acme/shop#12", "ref": "#12", "title": "Checkout v2"}], "tracker": "github"})
    );
    assert_eq!(p["write_back_line"], "#13 gets the label orch:running and a comment in GitHub.");
    let solo = f.rt.work_prefill("gh:acme/shop#13", Some("solo")).unwrap();
    assert!(solo["session"]["message"].as_str().unwrap().contains("Address step"));
    let mut t = f.rt.db.tracker(&f.project.id).unwrap();
    t.write_back.enabled = false;
    f.rt.db.set_tracker(&f.project.id, &t).unwrap();
    assert_eq!(f.rt.work_prefill("gh:acme/shop#13", None).unwrap()["write_back_line"], Value::Null);
    // A container is not started: it runs as a program.
    assert!(f.rt.work_prefill("gh:nope#1", None).is_err());
}

#[test]
fn a_node_starts_with_its_source_and_leaves_its_history_and_run_source() {
    let f = work_fixture();
    let id = |n: u64| f.rt.db.work_node_rows(&format!("gh:acme/shop#{n}")).unwrap().into_iter().next().unwrap();
    let link = IntakeLink { source: Some("gh:acme/shop#2".into()), ..Default::default() };
    // The claim accepts a node, hidden or not, and records it as started.
    let claim = f.rt.intake_claim(&f.project.id, Some(&link)).unwrap();
    f.rt.intake_started(claim, &f.project.id, "agent", "fix-feed", "fix-feed");
    let r = id(2);
    assert_eq!((r.state, r.started_ref.as_deref()), (IntakeState::Started, Some("fix-feed")));
    let h = f.rt.db.work_history(&f.project.id, "gh:acme/shop#2").unwrap();
    assert_eq!(h.iter().map(|e| e.what.as_str()).collect::<Vec<_>>(), ["started"]);
    // Already running: stale. A container, an unknown node or another project's: refused.
    assert!(f.rt.intake_claim(&f.project.id, Some(&link)).is_err());
    let bad = |n: &str| IntakeLink { source: Some(n.into()), ..Default::default() };
    assert!(f.rt.intake_claim(&f.project.id, Some(&bad("gh-milestone:acme/shop/3"))).is_err());
    assert!(f.rt.intake_claim(&f.project.id, Some(&bad("gh:nope#9"))).is_err());
    assert!(f.rt.intake_claim("other", Some(&bad("gh:acme/shop#4"))).is_err());
    // `run.source` of the session: node, breadcrumb, tracker, write-back, closed / deleted flags.
    let o = f.rt.intake_origins();
    let s = &o.src_agents["fix-feed"];
    assert_eq!(
        (s["node"].as_str(), s["ref"].as_str(), s["tracker"].as_str(), s["write_back"].as_str()),
        (Some("gh:acme/shop#2"), Some("#2"), Some("github"), Some("on"))
    );
    assert_eq!(s["breadcrumb"].as_array().unwrap().len(), 2);
    assert_eq!(
        (s["closed_upstream"].as_bool(), s["deleted"].as_bool(), s["program"].clone()),
        (Some(false), Some(false), Value::Null)
    );
    // Killed before any outcome: back, with its history.
    f.rt.intake_on_session_killed("fix-feed");
    assert_eq!(id(2).state, IntakeState::Ready);
    let h = f.rt.db.work_history(&f.project.id, "gh:acme/shop#2").unwrap();
    assert_eq!(h.iter().map(|e| e.what.as_str()).collect::<Vec<_>>(), ["started", "returned"]);
    // Run events of a started run are history too.
    let feature = orch_store::Feature {
        id: "f2".into(),
        project_id: f.project.id.clone(),
        slug: "address".into(),
        title: "t".into(),
        brief: String::new(),
        base: "main".into(),
        integration_worktree: None,
        allocation: None,
        created_at_ms: 1,
        archived: false,
    };
    f.rt.db.upsert_feature(&feature).unwrap();
    f.rt.db
        .upsert_run(&orch_store::TeamRun {
            id: "r9".into(),
            team_id: "t".into(),
            team_scope: Default::default(),
            team_version: 1,
            feature_id: "f2".into(),
            status: orch_team::RunState::Spec,
            resume_status: None,
            pr_url: None,
            started_at_ms: 1,
            ended_at_ms: None,
            pr_number: None,
            pr: None,
        })
        .unwrap();
    f.rt.db.intake_mark_started(id(4).id, "run", "r9").unwrap();
    f.rt.run_event("r9", None, "run.state", json!({"from": "spec_gate", "to": "planning", "why": "spec approved"}));
    f.rt.run_event("r9", None, "run.pr_opened", json!({"url": "https://github.com/acme/shop/pull/7"}));
    f.rt.run_event("r9", None, "run.state", json!({"from": "pr_gate", "to": "done"}));
    f.rt.run_event("r9", None, "task.created", json!({}));
    let h = f.rt.db.work_history(&f.project.id, "gh:acme/shop#4").unwrap();
    assert_eq!(h.iter().map(|e| e.what.as_str()).collect::<Vec<_>>(), ["spec_approved", "pr_opened", "merged"]);
}

#[test]
fn missing_children_are_fetched_once_with_a_per_parent_error_and_no_cap() {
    let f = fx();
    github_tracker(&f);
    answer_tree(
        &f,
        vec![
            tree_issue(1, json!({"labels": {"nodes": []}, "subIssuesSummary": {"total": 3, "completed": 1}})),
            tree_issue(2, json!({"parent": {"number": 1}})),
        ],
        vec![],
    );
    f.refresh();
    let kids = |f: &Fx, filter: Value| f.rt.work_tree(&scope(f), &json!(["gh:acme/shop#1"]), &filter, None).unwrap();
    let sub = |n: u64, state: &str| {
        let mut i = tree_issue(n, json!({"labels": {"nodes": []}}));
        i["state"] = json!(state);
        i
    };
    let subs = |nodes: Vec<Value>| {
        json!({"data": {"repository": {"issue": {"subIssues": {"pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": nodes}}}}})
    };
    let asks =
        |f: &Fx| f.calls.lock().unwrap().iter().filter(|c| c.iter().any(|a| a.contains("OrchTreeSubIssues"))).count();
    // The read of the tree could not complete the children: the fetch fails, the error is the parent's.
    f.fail("graphql:OrchTreeSubIssues", "HTTP 502: bad gateway");
    let before = asks(&f);
    let r = kids(&f, json!({"state": "all"}));
    assert_eq!(ids(&r), ["gh:acme/shop#2"]);
    assert!(r["more"]["gh:acme/shop#1"]["error"]["code"].is_string(), "{r}");
    // It answers: the missing ones are cached (every state), no error, and the parent's counts are whole.
    f.answer("graphql:OrchTreeSubIssues", subs(vec![sub(2, "OPEN"), sub(3, "CLOSED"), sub(5, "OPEN")]));
    let r = kids(&f, json!({"state": "all"}));
    assert_eq!(ids(&r), ["gh:acme/shop#2", "gh:acme/shop#3", "gh:acme/shop#5"]);
    assert!(r["more"].as_object().unwrap().is_empty(), "{r}");
    let asked = asks(&f);
    assert!(asked > before);
    // Once per revision: the next call reads the cache.
    kids(&f, json!({}));
    let n = f.rt.work_node("gh:acme/shop#1", &json!({"children": true})).unwrap();
    assert_eq!(n["child_nodes"].as_array().unwrap().len(), 3);
    assert_eq!(n["children"], json!({"total": 3, "open": 2, "done": 1}));
    assert_eq!(asks(&f), asked);
}

// ------------------------------------------------------------ programs (B4)

fn dep(n: u64) -> Value {
    json!({"number": n, "title": format!("Issue {n}"), "state": "OPEN", "url": "u",
           "repository": {"nameWithOwner": "acme/shop"}})
}

/// Checkout v2 (#12): #13 Address step (+ #17 below it) blocks #14 Payment step; #15 Summary; #16 closed.
fn program_fixture() -> Fx {
    let f = fx();
    github_tracker(&f);
    program_tree(&f, vec![]);
    f.refresh();
    f
}

fn program_tree(f: &Fx, extra: Vec<Value>) {
    let t = |n: u64, title: &str, x: Value| {
        let mut v = tree_issue(n, x);
        v["title"] = json!(title);
        v["labels"] = json!({"nodes": []});
        v
    };
    let mut issues = vec![
        t(12, "Checkout v2", json!({"subIssuesSummary": {"total": 5, "completed": 1}})),
        t(13, "Address step", json!({"parent": {"number": 12}, "subIssuesSummary": {"total": 1, "completed": 0}})),
        t(17, "Validation", json!({"parent": {"number": 13}})),
        t(14, "Payment step", json!({"parent": {"number": 12}, "blockedBy": {"nodes": [dep(13)]}})),
        t(15, "Order summary", json!({"parent": {"number": 12}})),
        t(16, "Old", json!({"parent": {"number": 12}, "state": "CLOSED"})),
    ];
    issues.extend(extra);
    answer_tree(f, issues, vec![]);
}

fn plan(f: &Fx, roots: Value, grouping: &str, opts: Value) -> Value {
    f.rt.program_plan(&roots, grouping, &opts).unwrap()
}

fn feats(v: &Value) -> Vec<(String, Vec<String>)> {
    v["program"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["slug"].as_str().unwrap().to_string(),
                f["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i.as_str().unwrap().rsplit('#').next().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

const ROOT: &str = "gh:acme/shop#12";

#[test]
fn the_planner_groups_orders_and_flags() {
    let f = program_fixture();
    let v = plan(&f, json!([ROOT]), "child", json!({}));
    let p = &v["program"];
    assert_eq!(
        feats(&v),
        [
            ("address-step".to_string(), vec!["13".to_string(), "17".into()]),
            ("payment-step".into(), vec!["14".into()]),
            ("order-summary".into(), vec!["15".into()]),
        ]
    );
    assert_eq!(
        (p["title"].as_str(), p["root_ref"].as_str(), p["status"].as_str()),
        (Some("Checkout v2"), Some("#12"), Some("draft"))
    );
    assert_eq!(
        (p["max_parallel"].as_u64(), p["grouping"].as_str(), p["sequential"].as_bool()),
        (Some(2), Some("child"), Some(false))
    );
    // #13 blocks #14 in GitHub: 2 waits for 1; the others are first.
    assert_eq!(
        p["edges"],
        json!([{"from": "pf-1", "to": "pf-2", "source": "tracker", "items": ["gh:acme/shop#13", "gh:acme/shop#14"]}])
    );
    assert_eq!(p["features"][1]["depends_on"], json!(["pf-1"]));
    assert_eq!(p["features"][0]["depends_on"], json!([]));
    // The user's own values: solo (no team), the brief carries the scope, the base is the project's.
    assert_eq!((p["features"][0]["mode"].as_str(), p["features"][0]["base"].as_str()), (Some("solo"), Some("main")));
    assert!(p["features"][0]["brief"].as_str().unwrap().contains("- [ ] #17 Validation"));
    assert!(p["features"][0]["estimate"]["high_pp"].as_f64().unwrap() > 0.0);
    assert_eq!(p["left_out"]["closed"], json!(["gh:acme/shop#16"]));
    let kinds: Vec<&str> = v["issues"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["closed"], "advisory only: {v}");
    assert!(v["issues"].as_array().unwrap().iter().all(|i| i["blocking"] == false));
    // Other groupings.
    let g = plan(&f, json!([ROOT]), "grandchild", json!({}));
    assert_eq!(
        feats(&g).iter().map(|f| f.1.clone()).collect::<Vec<_>>(),
        [vec!["17".to_string()], vec!["14".into()], vec!["15".into()]]
    );
    let s = plan(&f, json!([ROOT]), "single", json!({}));
    assert_eq!(feats(&s).len(), 1);
    assert_eq!(feats(&s)[0].1, ["13", "17", "14", "15"]);
    // One after the other: a chain in tree order, one at a time.
    let q = plan(&f, json!([ROOT]), "child", json!({"sequential": true, "max_parallel": 4}));
    assert_eq!(q["program"]["max_parallel"], 1);
    assert_eq!(
        q["program"]["edges"].as_array().unwrap().iter().map(|e| e["source"].as_str().unwrap()).collect::<Vec<_>>(),
        ["sequential", "sequential"]
    );
    assert_eq!(q["program"]["features"][2]["depends_on"], json!(["pf-2"]));
    // A cut stays.
    let c = plan(&f, json!([ROOT]), "child", json!({"cut": [["pf-1", "pf-2"]]}));
    assert_eq!(c["program"]["edges"], json!([]));
    assert_eq!(c["program"]["cut"], json!([["pf-1", "pf-2"]]));
    // Several siblings: a title from their parent; non-siblings are refused.
    let m = plan(&f, json!(["gh:acme/shop#13", "gh:acme/shop#14"]), "child", json!({}));
    assert_eq!(
        (m["program"]["title"].as_str(), m["program"]["root_ref"].as_str()),
        (Some("Checkout v2 · 2 selected"), Some("#12"))
    );
    assert!(f.rt.program_plan(&json!(["gh:acme/shop#17", "gh:acme/shop#14"]), "child", &json!({})).is_err());
    assert!(f.rt.program_plan(&json!([]), "child", &json!({})).is_err());
    assert!(f.rt.program_plan(&json!([ROOT]), "diagonal", &json!({})).is_err());
}

#[test]
fn a_cycle_blocks_and_names_its_edges() {
    let f = fx();
    github_tracker(&f);
    let t = |n: u64, title: &str, x: Value| {
        let mut v = tree_issue(n, x);
        v["title"] = json!(title);
        v
    };
    answer_tree(
        &f,
        vec![
            t(1, "Epic", json!({"subIssuesSummary": {"total": 2, "completed": 0}})),
            t(2, "Two", json!({"parent": {"number": 1}, "blockedBy": {"nodes": [dep(3)]}})),
            t(3, "Three", json!({"parent": {"number": 1}, "blockedBy": {"nodes": [dep(2)]}})),
        ],
        vec![],
    );
    f.refresh();
    let v = plan(&f, json!(["gh:acme/shop#1"]), "child", json!({}));
    let c = v["issues"].as_array().unwrap().iter().find(|i| i["kind"] == "cycle").unwrap();
    assert_eq!(c["blocking"], true);
    assert_eq!(c["features"].as_array().unwrap().len(), 2);
    assert_eq!(c["edges"].as_array().unwrap().len(), 2);
    assert!(c["message"].as_str().unwrap().contains("wait on each other"));
    // It refuses to start; cutting one edge fixes it.
    let r = f.rt.program_start(&v["program"]).unwrap();
    assert!(r["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "cycle"));
    assert!(f.rt.db.programs(None).unwrap().is_empty(), "nothing stored");
    let mut d = v["program"].clone();
    d["cut"] = json!([["pf-2", "pf-1"]]);
    let ok = f.rt.program_start(&d).unwrap();
    assert!(ok["program"]["id"].is_string(), "{ok}");
}

/// Starts a feature without a terminal: a run in the store, the first item marked.
fn fake_start(f: &Fx) {
    *crate::intake::lock(&f.rt.intake.test_starter) = Some(Box::new(|rt, feat| {
        if feat.slug == "boom" {
            anyhow::bail!("no team named boom");
        }
        let id = format!("run-{}", feat.slug);
        let fid = format!("f-{}", feat.slug);
        rt.db.upsert_feature(&orch_store::Feature {
            id: fid.clone(),
            project_id: rt.db.projects(false)?[0].id.clone(),
            slug: feat.slug.clone(),
            title: feat.title.clone(),
            brief: String::new(),
            base: "main".into(),
            integration_worktree: None,
            allocation: None,
            created_at_ms: 1,
            archived: false,
        })?;
        rt.db.upsert_run(&orch_store::TeamRun {
            id: id.clone(),
            team_id: "t".into(),
            team_scope: Default::default(),
            team_version: 1,
            feature_id: fid,
            status: orch_team::RunState::Executing,
            resume_status: None,
            pr_url: None,
            started_at_ms: 1,
            ended_at_ms: None,
            pr_number: None,
            pr: None,
        })?;
        let first = rt.db.work_node_rows(&feat.items[0])?.remove(0);
        rt.db.intake_mark_started(first.id, "run", &id)?;
        Ok(("run", id))
    }));
}

fn set_run(f: &Fx, slug: &str, status: orch_team::RunState) {
    let mut run = f.rt.db.run(&format!("run-{slug}")).unwrap().unwrap();
    run.status = status;
    run.ended_at_ms = Some(9);
    f.rt.db.upsert_run(&run).unwrap();
}

fn state(f: &Fx, id: &str) -> Vec<(String, String)> {
    let p = f.rt.program(id).unwrap();
    p["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["slug"].as_str().unwrap().into(), x["state"].as_str().unwrap().into()))
        .collect()
}

fn started(f: &Fx) -> String {
    let v = plan(f, json!([ROOT]), "child", json!({}));
    let r = f.rt.program_start(&v["program"]).unwrap();
    r["program"]["id"].as_str().unwrap().to_string()
}

fn s(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn start_validates_the_draft_keeps_the_users_edits_and_runs_in_dependency_order() {
    let f = program_fixture();
    fake_start(&f);
    let mut d = plan(&f, json!([ROOT]), "child", json!({}))["program"].clone();
    // The user's edits: a slug, a brief, a mode / team, a skip.
    d["features"][0]["slug"] = json!("addr");
    d["features"][0]["brief"] = json!("my brief");
    d["features"][2]["skip"] = json!(true);
    // Invalid values are refused with their field.
    let mut bad = d.clone();
    bad["features"][1]["slug"] = json!("addr");
    assert!(f.rt.program_start(&bad).unwrap_err().to_string().contains("\"field\":\"slug\""));
    bad["features"][1]["slug"] = json!("Bad Slug");
    assert!(f.rt.program_start(&bad).is_err());
    // Team mode with no team is blocking.
    let mut noteam = d.clone();
    noteam["features"][1]["mode"] = json!("team");
    noteam["features"][1]["team"] = Value::Null;
    let r = f.rt.program_start(&noteam).unwrap();
    assert!(r["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "no_team" && i["blocking"] == true), "{r}");
    // A closed item meanwhile is left out of its feature, not an error.
    let r = f.rt.program_start(&d).unwrap();
    let p = &r["program"];
    let id = p["id"].as_str().unwrap().to_string();
    assert_eq!(p["status"], "running");
    assert_eq!(p["features"][0]["slug"], "addr", "the user's slug is kept");
    assert_eq!(p["features"][0]["brief"], "my brief", "and the edited brief");
    // pf-1 runs at once (no dependency), pf-2 waits for it, pf-3 is skipped.
    assert_eq!(state(&f, &id), [s("addr", "running"), s("payment-step", "waiting"), s("order-summary", "skipped")]);
    assert_eq!(f.rt.program(&id).unwrap()["features"][1]["depends_on"], json!(["pf-1"]));
    // Every item of the first feature is linked to its run; history says the program queued them.
    for n in [13, 17] {
        let r = f.rt.db.work_node_rows(&format!("gh:acme/shop#{n}")).unwrap().remove(0);
        assert_eq!((r.state, r.started_ref.as_deref()), (IntakeState::Started, Some("run-addr")), "#{n}");
    }
    let h = f.rt.db.work_history(&f.project.id, "gh:acme/shop#17").unwrap();
    assert_eq!(h.iter().map(|e| e.what.as_str()).collect::<Vec<_>>(), ["program_queued", "started"]);
    assert_eq!(h[0].program.as_ref().unwrap()["title"], "Checkout v2");
    // It merges: the dependent starts, then everything done closes the program with a decision.
    set_run(&f, "addr", orch_team::RunState::Done);
    assert_eq!(state(&f, &id), [s("addr", "merged"), s("payment-step", "running"), s("order-summary", "skipped")]);
    set_run(&f, "payment-step", orch_team::RunState::Done);
    let done = f.rt.program(&id).unwrap();
    assert_eq!(done["status"], "done");
    assert!(done["done_at_ms"].is_u64());
    let ds: Vec<Value> =
        f.rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "program").map(|i| i.payload.clone()).collect());
    assert_eq!(ds.iter().map(|d| d["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["program_done"]);
    // Idempotent.
    f.rt.programs_tick();
    assert_eq!(f.rt.program(&id).unwrap()["status"], "done");
    // The same root cannot start again while running: a fresh program is fine once done.
    assert!(f.rt.program_plan(&json!([ROOT]), "child", &json!({})).is_ok());
}

#[test]
fn at_most_max_parallel_run_pause_skip_cancel_and_a_failed_feature_waits_the_others() {
    let f = program_fixture();
    fake_start(&f);
    let mut d = plan(&f, json!([ROOT]), "child", json!({"max_parallel": 1}))["program"].clone();
    d["features"][1]["depends_on"] = json!([]);
    d["edges"] = json!([]);
    d["features"][0]["slug"] = json!("boom");
    let id = f.rt.program_start(&d).unwrap()["program"]["id"].as_str().unwrap().to_string();
    // One at once: the first fails to start (its own error), the next one takes its place.
    let p = f.rt.program(&id).unwrap();
    assert_eq!(p["features"][0]["state"], "failed");
    assert_eq!(p["features"][0]["error"], "no team named boom");
    assert_eq!(state(&f, &id)[1..], [s("payment-step", "running"), s("order-summary", "planned")]);
    assert_eq!(p["status"], "running", "a failed feature does not stop the program");
    let failed: Vec<Value> =
        f.rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "program").map(|i| i.payload.clone()).collect());
    assert_eq!(failed.iter().map(|d| d["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["program_feature_failed"]);
    f.rt.programs_tick();
    assert_eq!(failed.len(), 1, "posted once");
    // The decision: open the feature, or skip it (its dependents stop waiting).
    let n = f.rt.store.read(|s| s.inbox.iter().find(|i| i.kind == "program").map(|i| i.id).unwrap());
    assert_eq!(f.rt.program_decision_act(n, "open").unwrap()["open"], "feature");
    assert_eq!(f.rt.program_decision_act(n, "skip").unwrap()["effect"], "skipped");
    assert_eq!(f.rt.program(&id).unwrap()["features"][0]["state"], "skipped");
    // Pause: nothing new starts while paused; running ones go on.
    f.rt.program_update(&id, &json!({"pause": true})).unwrap();
    set_run(&f, "payment-step", orch_team::RunState::Done);
    assert_eq!(state(&f, &id)[1..], [s("payment-step", "merged"), s("order-summary", "planned")]);
    assert_eq!(f.rt.program(&id).unwrap()["status"], "paused");
    f.rt.program_update(&id, &json!({"resume": true})).unwrap();
    assert_eq!(state(&f, &id)[2], s("order-summary", "running"));
    // Only a feature that has not started can be skipped; max_parallel is clamped; a cancel keeps the running ones.
    assert!(f.rt.program_update(&id, &json!({"skip": "pf-2"})).is_err());
    assert_eq!(f.rt.program_update(&id, &json!({"max_parallel": 9})).unwrap()["max_parallel"], 5);
    let c = f.rt.program_update(&id, &json!({"cancel": true})).unwrap();
    assert_eq!(c["status"], "cancelled");
    assert_eq!(c["features"][2]["state"], "running", "it goes on as a normal feature");
    assert!(f.rt.program_update(&id, &json!({"pause": true})).is_err(), "over");
    assert!(f.rt.program_update("prg-nope", &json!({"pause": true})).is_err());
}

#[test]
fn now_carries_programs_whole_then_as_deltas_and_the_items_leave_the_lane() {
    let f = program_fixture();
    fake_start(&f);
    assert_eq!(f.rt.programs_snapshot(), json!([]));
    assert!(f.rt.programs_delta().is_none());
    let id = started(&f);
    let d = f.rt.programs_delta().unwrap();
    let row = &d["upsert"][0];
    assert_eq!(
        (row["id"].as_str(), row["status"].as_str(), row["root_ref"].as_str()),
        (Some(id.as_str()), Some("running"), Some("#12"))
    );
    assert_eq!(
        row["counts"],
        json!({"merged": 0, "running": 2, "pr": 0, "waiting": 1, "queued": 0, "failed": 0, "skipped": 0})
    );
    assert_eq!(row["features"][1]["waiting_on"], json!(["address-step"]));
    assert_eq!(row["features"][0]["state"], "running");
    assert!(f.rt.programs_delta().is_none(), "nothing moved");
    set_run(&f, "address-step", orch_team::RunState::Done);
    f.rt.programs_tick();
    let d = f.rt.programs_delta().unwrap();
    assert_eq!(d["upsert"][0]["counts"]["merged"], 1);
    // The lane shows no item of a program in progress.
    assert!(f.items().iter().all(|i| i["project"] != f.project.id.as_str()) || f.refs().is_empty(), "{:?}", f.refs());
    // Done: removed from the list, and an outcome line says so.
    set_run(&f, "payment-step", orch_team::RunState::Done);
    set_run(&f, "order-summary", orch_team::RunState::Done);
    f.rt.programs_tick();
    let d = f.rt.programs_delta().unwrap();
    assert_eq!((d["upsert"].as_array().unwrap().len(), d["remove"].as_array().unwrap().len()), (0, 1));
    f.rt.db.flush().unwrap();
    let evs = f.rt.db.events(0, None, Some("program.done"), 5).unwrap();
    assert_eq!(evs.len(), 1);
    let tz = governor::jiff::tz::TimeZone::UTC;
    let o = crate::digest::outcome(&evs[0], None, &crate::digest::Names::default(), 0, &tz).unwrap();
    assert_eq!((o.kind, o.text.as_str()), ("program", "Program Checkout v2 done · 3 merged"));
}

#[test]
fn work_shows_program_states_roll_ups_and_roots_and_origins_carry_the_program() {
    let f = program_fixture();
    fake_start(&f);
    let id = started(&f);
    let scope = json!({"projects": [f.project.id]});
    let kids = f.rt.work_tree(&scope, &json!([ROOT]), &json!({"state": "all"}), None).unwrap();
    let app = |n: u64| {
        kids["nodes"].as_array().unwrap().iter().find(|x| x["id"] == format!("gh:acme/shop#{n}")).unwrap().clone()
    };
    assert_eq!(app(13)["app"]["state"], "running");
    // #14 waits for address-step in the program; #15 runs too (at most 2 at once).
    assert_eq!(
        app(14)["app"],
        json!({"state": "queued", "program": {"id": id, "title": "Checkout v2"}, "waiting_on": ["address-step"]})
    );
    // The root is the program's, with its progress; its action opens the program.
    let roots = f.rt.work_tree(&scope, &Value::Null, &json!({}), None).unwrap();
    let r = roots["nodes"].as_array().unwrap().iter().find(|x| x["id"] == ROOT).unwrap();
    assert_eq!(
        r["app"],
        json!({"state": "program_root", "program": {"id": id, "title": "Checkout v2"}, "merged": 0, "total": 3})
    );
    assert_eq!(r["actions"]["program"], json!({"open_program": id}));
    // The roll-up counts the descendants by the word list.
    assert_eq!(
        r["rollup"],
        json!({"running": 3, "pr": 0, "waiting": 1, "queued": 0, "merged": 0, "failed": 0, "skipped": 0})
    );
    // Origins: the run, its source and the program it belongs to.
    let o = f.rt.intake_origins();
    assert_eq!(
        o.src_runs["run-address-step"]["program"],
        json!({"id": id, "title": "Checkout v2", "index": 1, "total": 3})
    );
    assert_eq!(o.runs["run-address-step"]["program"]["index"], 1);
    assert_eq!(o.src_runs["run-address-step"]["node"], "gh:acme/shop#13");
    let tags = f.rt.program_of_starts();
    assert_eq!(tags["run-order-summary"].0["index"], 3);
}

#[test]
fn upstream_changes_become_decisions_for_the_program() {
    let f = program_fixture();
    fake_start(&f);
    let id = started(&f);
    let decisions = |f: &Fx| -> Vec<(String, String)> {
        f.rt.store.read(|s| {
            s.inbox
                .iter()
                .filter(|i| i.kind == "program" && i.open())
                .map(|i| {
                    (i.payload["kind"].as_str().unwrap().to_string(), i.payload["key"].as_str().unwrap().to_string())
                })
                .collect()
        })
    };
    assert!(decisions(&f).is_empty());
    let t = |n: u64, title: &str, x: Value| {
        let mut v = tree_issue(n, x);
        v["title"] = json!(title);
        v["labels"] = json!({"nodes": []});
        v
    };
    // A child is added under the root (#18), #15 is closed, #15 is now blocked by #14.
    program_tree(&f, vec![]);
    let mut issues = vec![
        t(12, "Checkout v2", json!({"subIssuesSummary": {"total": 6, "completed": 1}})),
        t(13, "Address step", json!({"parent": {"number": 12}})),
        t(
            14,
            "Payment step",
            json!({"parent": {"number": 12}, "state": "CLOSED", "blockedBy": {"nodes": [dep(13), dep(15)]}}),
        ),
        t(15, "Order summary", json!({"parent": {"number": 12}})),
        t(18, "Gift cards", json!({"parent": {"number": 12}})),
    ];
    issues.push(t(17, "Validation", json!({"parent": {"number": 13}})));
    answer_tree(&f, issues, vec![]);
    f.refresh();
    let d = decisions(&f);
    assert!(d.contains(&("program_new_item".into(), "new:gh:acme/shop#18".into())), "{d:?}");
    assert!(d.contains(&("program_item_closed".into(), "closed:pf-2".into())), "{d:?}");
    assert!(d.contains(&("program_dependency_added".into(), "dep:pf-3>pf-2".into())), "{d:?}");
    let p = f.rt.program(&id).unwrap();
    assert_eq!(p["new_items"], json!(["gh:acme/shop#18"]));
    assert!(p["features"][1]["flags"].as_array().unwrap().contains(&json!("closed_upstream")));
    assert!(p["features"][1]["flags"].as_array().unwrap().contains(&json!("new_dependency")));
    // A second refresh posts nothing new.
    f.refresh();
    assert_eq!(decisions(&f).len(), d.len());
    // Add it: the card says where to go; the edit adds a feature and closes the card.
    let n =
        f.rt.store.read(|s| s.inbox.iter().find(|i| i.payload["key"] == "new:gh:acme/shop#18").map(|i| i.id).unwrap());
    assert_eq!(f.rt.program_decision_act(n, "add").unwrap()["open"], "program_add");
    let p =
        f.rt.program_update(&id, &json!({"add_item": "gh:acme/shop#18", "mode": "solo", "after": ["pf-1"]})).unwrap();
    assert_eq!(p["features"].as_array().unwrap().len(), 4);
    assert_eq!(p["features"][3]["slug"], "gift-cards");
    assert_eq!(p["features"][3]["depends_on"], json!(["pf-1"]));
    assert!(p["new_items"].as_array().unwrap().is_empty());
    assert!(!decisions(&f).contains(&("program_new_item".into(), "new:gh:acme/shop#18".into())));
    // Skip the closed one on the card; ignore leaves an item out for good.
    let n = f.rt.store.read(|s| s.inbox.iter().find(|i| i.payload["key"] == "closed:pf-2").map(|i| i.id).unwrap());
    f.rt.program_decision_act(n, "skip").unwrap();
    assert_eq!(f.rt.program(&id).unwrap()["features"][1]["state"], "skipped");
}
