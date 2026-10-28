use std::sync::{Arc, Mutex};

use anyhow::bail;
use serde_json::{Value, json};

use super::*;

/// GitHub in memory: issues, labels, comments; `offline` fails every call.
#[derive(Default)]
struct World {
    issues: Vec<Value>,
    labels: Vec<String>,
    comments: Vec<(String, String)>,
    clock: u64,
    offline: bool,
    calls: usize,
}

#[derive(Clone, Default)]
struct FakeGh(Arc<Mutex<World>>);

impl FakeGh {
    fn edit(&self, number: u64, f: impl FnOnce(&mut Value)) {
        let mut w = self.0.lock().unwrap();
        w.clock += 1;
        let stamp = format!("t{}", w.clock);
        let i = w.issues.iter_mut().find(|i| i["number"] == json!(number)).unwrap();
        f(i);
        i["updatedAt"] = json!(stamp);
    }
    fn issue(&self, number: u64) -> Value {
        self.0.lock().unwrap().issues.iter().find(|i| i["number"] == json!(number)).unwrap().clone()
    }
}

fn fields(args: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    let mut f = vec![];
    let mut labels = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-f"
            && let Some(kv) = it.next()
        {
            let (k, v) = kv.split_once('=').unwrap();
            if k == "labels[]" {
                labels.push(v.to_string());
            } else {
                f.push((k.to_string(), v.to_string()));
            }
        }
    }
    (f, labels)
}

impl Gh for FakeGh {
    fn run(&self, args: &[String]) -> anyhow::Result<String> {
        let mut w = self.0.lock().unwrap();
        w.calls += 1;
        if w.offline {
            bail!("gh: network unreachable");
        }
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        let (f, labels) = fields(args);
        let get = |k: &str| f.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
        w.clock += 1;
        let stamp = format!("t{}", w.clock);
        match a.as_slice() {
            ["api", "-X", "POST", path, ..] if path.ends_with("/labels") => {
                let n = get("name").unwrap();
                if w.labels.contains(&n) {
                    bail!("HTTP 422: already_exists");
                }
                w.labels.push(n);
                Ok("{}".into())
            }
            ["api", "-X", "POST", path, ..] if path.ends_with("/comments") => {
                let n = path.split('/').nth(4).unwrap().to_string();
                w.comments.push((n, get("body").unwrap()));
                Ok("{}".into())
            }
            ["api", "-X", "POST", path, ..] if path.ends_with("/issues") => {
                assert_eq!(*path, "repos/me/shop/issues");
                let number = w.issues.len() as u64 + 1;
                let issue = json!({"number": number, "title": get("title"), "body": get("body"), "state": "open",
                    "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(), "updated_at": stamp});
                w.issues.push(issue.clone());
                Ok(issue.to_string())
            }
            ["api", "-X", "PATCH", path, ..] => {
                let number: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                let i = w.issues.iter_mut().find(|i| i["number"] == json!(number)).unwrap();
                i["title"] = json!(get("title"));
                i["body"] = json!(get("body"));
                i["state"] = json!(get("state"));
                i["labels"] = json!(labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>());
                i["updated_at"] = json!(stamp);
                i["updatedAt"] = json!(stamp);
                Ok(i.to_string())
            }
            ["api", path] => {
                let number: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                Ok(w.issues.iter().find(|i| i["number"] == json!(number)).unwrap().to_string())
            }
            ["issue", "list", ..] => Ok(Value::Array(
                w.issues
                    .iter()
                    .map(|i| {
                        let mut i = i.clone();
                        if i.get("updatedAt").is_none() {
                            i["updatedAt"] = i["updated_at"].clone();
                        }
                        i
                    })
                    .collect(),
            )
            .to_string()),
            other => bail!("unexpected gh call {other:?}"),
        }
    }
}

fn setup(backend: Value) -> (tempfile::TempDir, Store, Project) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("o.db")).unwrap();
    let p = Project {
        id: "p1".into(),
        name: "shop".into(),
        path: d.path().to_path_buf(),
        default_branch: "main".into(),
        remotes: vec![],
        worktree_root: d.path().join("w"),
        task_backend: backend,
        track_orchestrator_dir: true,
        settings: json!({}),
        created_at_ms: 1,
        last_active_at_ms: 1,
        last_fetch_at_ms: None,
        last_fetch_error: None,
        archived: false,
    };
    s.upsert_project(&p).unwrap();
    (d, s, p)
}

#[test]
fn local_backend_crud() {
    let (_d, s, p) = setup(json!({"type": "local"}));
    let svc = TaskService::new(&s, &p, None);
    assert_eq!(svc.kind(), "local");
    let t = svc.create(Task::new("x", "x", "Session endpoint", 1)).unwrap().value;
    assert!(t.id > 0 && t.backend == "local" && t.project_id == "p1");
    let t = svc.set_status(t.id, TaskStatus::InProgress, 2).unwrap().value;
    assert_eq!(s.task(t.id).unwrap().unwrap().status, TaskStatus::InProgress);
    let t = svc.note(t.id, "back", "halfway", 3).unwrap().value;
    assert_eq!(t.notes.len(), 1);
    assert_eq!(svc.sync(4).unwrap(), SyncReport::default());
}

#[test]
fn github_write_through_and_import() {
    let (_d, s, p) = setup(json!({"type": "github", "repo": "me/shop"}));
    let gh = FakeGh::default();
    let svc = TaskService::new(&s, &p, Some(Box::new(gh.clone())));
    let a = svc.create(Task::new("", "", "Session endpoint", 1)).unwrap();
    assert!(a.remote_error.is_none());
    let a = a.value;
    assert_eq!(a.external_id.as_deref(), Some("1"));
    let mut b = Task::new("", "", "Form", 1);
    b.depends_on = vec![a.id];
    b.node = Some("front".into());
    let b = svc.create(b).unwrap().value;
    let issue = gh.issue(2);
    assert_eq!(issue["body"], json!("Blocked by: #1"));
    assert!(issue["labels"].as_array().unwrap().contains(&json!({"name": "orch:node:front"})));
    // Done closes the issue; labels the app doesn't manage stay.
    gh.edit(1, |i| i["labels"].as_array_mut().unwrap().push(json!({"name": "bug"})));
    svc.set_status(a.id, TaskStatus::Done, 5).unwrap();
    assert!(gh.issue(1)["labels"].as_array().unwrap().contains(&json!({"name": "bug"})));
    assert_eq!(gh.issue(1)["state"], json!("closed"));
    svc.note(b.id, "front", "started", 6).unwrap();
    assert_eq!(gh.0.lock().unwrap().comments, vec![("2".to_string(), "**front**: started".to_string())]);
    // Nothing changed remotely: a sync changes nothing.
    let rep = svc.sync(7).unwrap();
    assert!(rep.updated.is_empty() && rep.conflicts.is_empty() && rep.imported.is_empty(), "{rep:?}");
    // An issue opened on GitHub by hand, with the orch label, is imported.
    gh.0.lock().unwrap().issues.push(json!({"number": 3, "title": "From GitHub", "body": "Blocked by: #2",
        "state": "open", "labels": [{"name": "orch"}, {"name": "orch:ready"}], "updatedAt": "x1"}));
    let rep = svc.sync(8).unwrap();
    assert_eq!(rep.imported.len(), 1);
    let imported = s.task(rep.imported[0]).unwrap().unwrap();
    assert_eq!(imported.status, TaskStatus::Ready);
    assert_eq!(imported.depends_on, vec![b.id]);
}

#[test]
fn remote_changes_and_conflicts() {
    let (_d, s, p) = setup(json!({"type": "github", "repo": "me/shop"}));
    let gh = FakeGh::default();
    let svc = TaskService::new(&s, &p, Some(Box::new(gh.clone())));
    let t = svc.create(Task::new("", "", "Session", 1)).unwrap().value;
    // Remote edit only: adopted.
    gh.edit(1, |i| {
        i["title"] = json!("Session v2");
        i["labels"] = json!([{"name": "orch"}, {"name": "orch:review"}]);
    });
    let rep = svc.sync(2).unwrap();
    assert_eq!(rep.updated, vec![t.id]);
    let t = s.task(t.id).unwrap().unwrap();
    assert_eq!((t.title.as_str(), t.status), ("Session v2", TaskStatus::Review));

    // Local edit while offline: kept locally, pushed at the next sync.
    gh.0.lock().unwrap().offline = true;
    let out = svc.set_status(t.id, TaskStatus::Done, 3).unwrap();
    assert!(out.remote_error.is_some());
    assert!(out.value.remote_updated_at.as_deref().unwrap().starts_with(PENDING));
    gh.0.lock().unwrap().offline = false;
    let rep = svc.sync(4).unwrap();
    assert_eq!(rep.pushed, vec![t.id]);
    assert_eq!(gh.issue(1)["state"], json!("closed"));

    // Offline local edit + a remote edit: the remote wins and it is reported.
    gh.0.lock().unwrap().offline = true;
    let mut local = s.task(t.id).unwrap().unwrap();
    local.title = "Mine".into();
    svc.update(local, 5).unwrap();
    gh.0.lock().unwrap().offline = false;
    gh.edit(1, |i| i["title"] = json!("Theirs"));
    let rep = svc.sync(6).unwrap();
    assert_eq!(rep.conflicts, vec![t.id]);
    let t = s.task(t.id).unwrap().unwrap();
    assert_eq!(t.title, "Theirs");
    assert!(!t.remote_updated_at.unwrap().starts_with(PENDING));
}

#[test]
fn create_while_offline_is_pushed_later() {
    let (_d, s, p) = setup(json!({"type": "github", "repo": "me/shop"}));
    let gh = FakeGh::default();
    gh.0.lock().unwrap().offline = true;
    let svc = TaskService::new(&s, &p, Some(Box::new(gh.clone())));
    let t = svc.create(Task::new("", "", "Later", 1)).unwrap();
    assert!(t.remote_error.is_some() && t.value.external_id.is_none());
    assert!(svc.sync(2).is_err(), "listing fails while offline");
    gh.0.lock().unwrap().offline = false;
    let rep = svc.sync(3).unwrap();
    assert_eq!(rep.pushed, vec![t.value.id]);
    assert_eq!(s.task(t.value.id).unwrap().unwrap().external_id.as_deref(), Some("1"));
    assert!(rep.imported.is_empty(), "the pushed issue is not imported twice: {rep:?}");
}

#[test]
fn backends_keep_one_github_backend_per_project() {
    let (_d, s, mut p) = setup(json!({"type": "github", "repo": "me/shop"}));
    let gh = FakeGh::default();
    let made = Arc::new(Mutex::new(0));
    let backends = Backends::default();
    let service = |p: &Project| {
        let (gh, made) = (gh.clone(), made.clone());
        backends.service(
            &s,
            p,
            move || {
                *made.lock().unwrap() += 1;
                Some(Box::new(gh) as Box<dyn Gh>)
            },
            || Box::new(linear::Unconfigured),
        )
    };
    service(&p).create(Task::new("", "", "One", 1)).unwrap();
    let after_first = gh.0.lock().unwrap().calls;
    service(&p).create(Task::new("", "", "Two", 2)).unwrap();
    // The second write only creates its issue: labels were made once.
    assert_eq!(gh.0.lock().unwrap().calls - after_first, 1);
    assert_eq!(*made.lock().unwrap(), 1);
    // Another repository: a new backend.
    p.task_backend = json!({"type": "github", "repo": "me/other"});
    assert_eq!(service(&p).kind(), "github");
    assert_eq!(*made.lock().unwrap(), 2);
    // Local projects need no backend.
    p.task_backend = json!({"type": "local"});
    assert_eq!(service(&p).kind(), "local");
}

/// Spec §6 item 4 on a Linear-backed project (FR-501, FR-503): app changes
/// reach Linear (issue, state, node label, `blocks` relation, comment),
/// Linear changes come back, and on a conflict the remote wins.
#[test]
fn linear_two_way_sync_and_the_remote_wins() {
    use crate::linear::fake::FakeLinear;
    use crate::linear::{LinearIssues, Unconfigured};
    let (_d, s, p) = setup(json!({"type": "linear", "team": "ENG"}));
    let lin = FakeLinear::default();
    let remote: Arc<dyn Remote> = Arc::new(LinearIssues::new(lin.clone(), "ENG", None, Default::default()));
    let svc = TaskService::with_remote(&s, &p, "linear", remote.clone());
    let a = svc.create(Task::new("", "", "Session endpoint", 1)).unwrap();
    assert!(a.remote_error.is_none(), "{:?}", a.remote_error);
    let a = a.value;
    assert_eq!((a.backend.as_str(), a.external_id.as_deref()), ("linear", Some("ENG-1")));
    assert_eq!((a.status, lin.issue("ENG-1").state.as_str()), (TaskStatus::Ready, "st-todo"));
    let mut b = Task::new("", "", "Form", 1);
    (b.depends_on, b.node, b.status) = (vec![a.id], Some("front".into()), TaskStatus::Ready);
    let b = svc.create(b).unwrap().value;
    assert_eq!(lin.issue("ENG-2").state, "st-todo");
    assert_eq!(lin.blockers("ENG-2"), ["ENG-1"]);
    let names = |id: &str| -> Vec<String> {
        let w = lin.0.lock().unwrap();
        let i = w.issues.iter().find(|i| i.id == id).unwrap();
        i.labels.iter().filter_map(|l| w.labels.iter().find(|(x, _)| x == l).map(|(_, n)| n.clone())).collect()
    };
    assert_eq!(names("ENG-2"), ["orch", "orch:node:front"]);
    // Moving a task in the app moves the issue; labels added in Linear stay.
    lin.0.lock().unwrap().labels.push(("lbl-bug".into(), "bug".into()));
    lin.edit("ENG-2", |i| i.labels.push("lbl-bug".into()));
    svc.sync(2).unwrap();
    svc.set_status(b.id, TaskStatus::Review, 3).unwrap();
    assert_eq!(lin.issue("ENG-2").state, "st-review");
    assert!(names("ENG-2").contains(&"bug".to_string()));
    svc.note(b.id, "front", "ready for review", 4).unwrap();
    assert_eq!(lin.0.lock().unwrap().comments, [("ENG-2".to_string(), "**front**: ready for review".to_string())]);
    assert_eq!(svc.sync(5).unwrap(), SyncReport::default(), "nothing changed remotely");

    // Changed in Linear: it comes back.
    lin.edit("ENG-1", |i| {
        i.title = "Session endpoint v2".into();
        i.state = "st-progress".into();
    });
    let rep = svc.sync(6).unwrap();
    assert_eq!(rep.updated, [a.id]);
    let a2 = s.task(a.id).unwrap().unwrap();
    assert_eq!((a2.title.as_str(), a2.status), ("Session endpoint v2", TaskStatus::InProgress));

    // Changed on both sides (the app's change failed to reach Linear): Linear wins.
    lin.0.lock().unwrap().rate_limited = 1;
    let out = svc.set_status(a.id, TaskStatus::Done, 7).unwrap();
    assert!(out.remote_error.unwrap().contains("rate limit"));
    lin.edit("ENG-1", |i| i.state = "st-review".into());
    // The back-off holds the next sync off; once it is over, the remote wins.
    assert!(svc.sync(8).is_err());
    let fresh: Arc<dyn Remote> = Arc::new(LinearIssues::new(lin.clone(), "ENG", None, Default::default()));
    let svc = TaskService::with_remote(&s, &p, "linear", fresh);
    let rep = svc.sync(9).unwrap();
    assert_eq!(rep.conflicts, [a.id]);
    assert_eq!(s.task(a.id).unwrap().unwrap().status, TaskStatus::Review);
    assert_eq!(rep.summary()["pulled"], 1);

    // An issue made in Linear with the orch label is imported, its blocker resolved.
    {
        let mut w = lin.0.lock().unwrap();
        let orch = w.labels.iter().find(|(_, n)| n == "orch").unwrap().0.clone();
        w.issues.push(crate::linear::fake::FakeIssue {
            id: "ENG-9".into(),
            title: "From Linear".into(),
            description: "d".into(),
            state: "st-todo".into(),
            labels: vec![orch],
            updated: 99,
        });
        w.relations.push(("rel-9".into(), "ENG-2".into(), "ENG-9".into()));
    }
    let rep = svc.sync(10).unwrap();
    let t = s.task(rep.imported[0]).unwrap().unwrap();
    assert_eq!((t.status, t.depends_on.clone(), t.backend.as_str()), (TaskStatus::Ready, vec![b.id], "linear"));

    // A task left local (D8 "leave") is never pushed.
    let mut l = Task::new("p1", "local", "Old local task", 1);
    l.id = s.upsert_task(&l).unwrap();
    let before = lin.0.lock().unwrap().issues.len();
    svc.set_status(l.id, TaskStatus::InProgress, 11).unwrap();
    svc.sync(12).unwrap();
    assert_eq!(lin.0.lock().unwrap().issues.len(), before);
    let _ = Unconfigured;
}

/// A custom mapping sends a column to the mapped state.
#[test]
fn linear_state_mapping_per_project() {
    use crate::linear::LinearIssues;
    use crate::linear::fake::FakeLinear;
    let (_d, s, p) = setup(json!({"type": "linear", "team": "ENG"}));
    let lin = FakeLinear::default();
    // "In Progress" is the team's review column here; "Todo" stays Ready.
    let mapping = [("st-progress".to_string(), TaskStatus::Review)].into_iter().collect();
    let svc =
        TaskService::with_remote(&s, &p, "linear", Arc::new(LinearIssues::new(lin.clone(), "ENG", None, mapping)));
    let mut t = Task::new("", "", "x", 1);
    t.status = TaskStatus::Review;
    svc.create(t).unwrap();
    assert_eq!(lin.issue("ENG-1").state, "st-progress");
}

/// Review fixes (T-9): a state added in Linear later still syncs (its
/// column from its type) and is found for writing after one more read of
/// the team; an existing `orch` label anywhere in the workspace is reused.
#[test]
fn linear_new_states_and_existing_labels() {
    use crate::linear::LinearIssues;
    use crate::linear::fake::FakeLinear;
    let (_d, s, p) = setup(json!({"type": "linear", "team": "ENG"}));
    let lin = FakeLinear::default();
    {
        // 60 labels, the `orch` one last (a team listing would stop at 50).
        let mut w = lin.0.lock().unwrap();
        for i in 0..59 {
            w.labels.push((format!("lbl-x{i}"), format!("area-{i}")));
        }
        w.labels.push(("lbl-orch".into(), "orch".into()));
    }
    let mapping = [("st-review".to_string(), TaskStatus::InProgress), ("st-qa".to_string(), TaskStatus::Review)]
        .into_iter()
        .collect();
    let svc =
        TaskService::with_remote(&s, &p, "linear", Arc::new(LinearIssues::new(lin.clone(), "ENG", None, mapping)));
    let t = svc.create(Task::new("", "", "x", 1)).unwrap();
    assert!(t.remote_error.is_none(), "{:?}", t.remote_error);
    let t = t.value;
    assert!(!lin.0.lock().unwrap().labels_created.contains(&"orch".to_string()), "the existing label is used");
    assert_eq!(lin.issue("ENG-1").labels, ["lbl-orch"]);

    // "QA" appears in Linear and the issue moves there: the sync reads it by type.
    lin.0.lock().unwrap().extra_states.push(("st-qa".into(), "QA".into(), "started".into(), 3.5));
    lin.0.lock().unwrap().extra_states.push(("st-new".into(), "Doing it".into(), "started".into(), 3.6));
    lin.edit("ENG-1", |i| i.state = "st-new".into());
    let rep = svc.sync(2).unwrap();
    assert_eq!(rep.updated, [t.id]);
    assert_eq!(s.task(t.id).unwrap().unwrap().status, TaskStatus::InProgress);
    // Review is mapped to the new "QA" state: found after one more read of the team.
    let out = svc.set_status(t.id, TaskStatus::Review, 3).unwrap();
    assert!(out.remote_error.is_none(), "{:?}", out.remote_error);
    assert_eq!(lin.issue("ENG-1").state, "st-qa");
}
