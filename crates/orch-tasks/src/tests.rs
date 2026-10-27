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
        backends.service(&s, p, move || {
            *made.lock().unwrap() += 1;
            Some(Box::new(gh) as Box<dyn Gh>)
        })
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
