use std::sync::{Arc, Mutex};

use anyhow::bail;
use serde_json::{Value, json};

use super::*;

/// GitHub in memory: issues, labels, comments; `offline` fails every call.
/// Every write moves a clock of ten minutes, so `since` filters for real.
#[derive(Default)]
struct World {
    issues: Vec<Value>,
    labels: Vec<String>,
    comments: Vec<(String, String)>,
    clock: i64,
    offline: bool,
    calls: usize,
    /// Paged list calls: (path, pages returned).
    lists: Vec<(String, usize)>,
    /// PATCHes: (issue, the fields sent; `labels` if any label was).
    patches: Vec<(u64, Vec<String>)>,
    /// Single issue reads.
    gets: usize,
}

impl World {
    fn tick(&mut self) -> String {
        self.clock += 1;
        stamp(self.clock)
    }
}

fn stamp(clock: i64) -> String {
    let base: jiff::Timestamp = "2026-11-02T10:00:00Z".parse().unwrap();
    (base + jiff::SignedDuration::from_mins(10 * clock)).to_string()
}

fn labels_json<S: AsRef<str>>(names: &[S]) -> Value {
    json!(names.iter().map(|l| json!({"name": l.as_ref()})).collect::<Vec<_>>())
}

#[derive(Clone, Default)]
struct FakeGh(Arc<Mutex<World>>);

impl FakeGh {
    fn edit(&self, number: u64, f: impl FnOnce(&mut Value)) {
        let mut w = self.0.lock().unwrap();
        let stamp = w.tick();
        let i = w.issues.iter_mut().find(|i| i["number"] == json!(number)).unwrap();
        f(i);
        i["updated_at"] = json!(stamp);
    }
    /// An issue made on GitHub (not by the app).
    fn add(&self, number: u64, title: &str, state: &str, names: &[&str]) {
        let mut w = self.0.lock().unwrap();
        let stamp = w.tick();
        w.issues.push(json!({"number": number, "title": title, "body": "", "state": state,
            "labels": labels_json(names), "updated_at": stamp}));
    }
    /// The last PATCH: its issue and the fields it sent, sorted.
    fn last_patch(&self) -> (u64, Vec<String>) {
        let (n, mut sent) = self.0.lock().unwrap().patches.last().cloned().unwrap();
        sent.sort();
        (n, sent)
    }
    /// The path of the last paged list.
    fn last_list(&self) -> String {
        self.0.lock().unwrap().lists.last().unwrap().0.clone()
    }
    fn label_names(&self, number: u64) -> Vec<String> {
        self.issue(number)["labels"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().into()).collect()
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
        if !matches!(a.as_slice(), ["api", "--paginate", ..] | ["api", _]) {
            w.tick();
        }
        let stamp = stamp(w.clock);
        match a.as_slice() {
            ["repo", "view", ..] => Ok(json!({"nameWithOwner": "me/shop"}).to_string()),
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
                    "labels": labels_json(&labels), "updated_at": stamp,
                    "html_url": format!("https://github.com/me/shop/issues/{number}")});
                w.issues.push(issue.clone());
                Ok(issue.to_string())
            }
            ["api", "-X", "PATCH", path, ..] => {
                let number: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                let mut sent: Vec<String> = f.iter().map(|(k, _)| k.clone()).collect();
                if !labels.is_empty() {
                    sent.push("labels".into());
                }
                w.patches.push((number, sent));
                let i = w.issues.iter_mut().find(|i| i["number"] == json!(number)).unwrap();
                for (k, v) in &f {
                    i[k.as_str()] = json!(v);
                }
                if !labels.is_empty() {
                    i["labels"] = labels_json(&labels);
                }
                i["updated_at"] = json!(stamp);
                Ok(i.to_string())
            }
            ["api", "--paginate", path] => {
                // GET repos/me/shop/issues?state=…&labels=…&since=…&per_page=…
                let (p, q) = path.split_once('?').unwrap();
                assert_eq!(p, "repos/me/shop/issues");
                let q: std::collections::HashMap<&str, &str> =
                    q.split('&').map(|kv| kv.split_once('=').unwrap()).collect();
                let per_page: usize = q["per_page"].parse().unwrap();
                assert_eq!(per_page, 100);
                let since: Option<jiff::Timestamp> = q.get("since").map(|s| s.parse().unwrap());
                let items: Vec<Value> = w
                    .issues
                    .iter()
                    .filter(|i| q["state"] == "all" || i["state"] == json!(q["state"]))
                    .filter(|i| {
                        q.get("labels").is_none_or(|l| i["labels"].as_array().unwrap().contains(&json!({"name": l})))
                    })
                    .filter(|i| {
                        since.is_none_or(|s| i["updated_at"].as_str().unwrap().parse::<jiff::Timestamp>().unwrap() >= s)
                    })
                    .cloned()
                    .collect();
                // `gh --paginate`: one array per page, back to back.
                let pages: Vec<String> = items.chunks(per_page).map(|c| Value::Array(c.to_vec()).to_string()).collect();
                let out = if pages.is_empty() { "[]".to_string() } else { pages.join("\n") };
                w.lists.push((path.to_string(), pages.len().max(1)));
                Ok(out)
            }
            ["api", path] => {
                w.gets += 1;
                let number: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                match w.issues.iter().find(|i| i["number"] == json!(number)) {
                    Some(i) => Ok(i.to_string()),
                    None => bail!("HTTP 404: Not Found"),
                }
            }
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
    gh.add(3, "From GitHub", "open", &["orch", "orch:ready"]);
    gh.edit(3, |i| i["body"] = json!("Blocked by: #2"));
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
    assert_eq!(lin.label_names("ENG-2"), ["orch", "orch:node:front"]);
    // Moving a task in the app moves the issue; labels added in Linear stay.
    lin.0.lock().unwrap().labels.push(("lbl-bug".into(), "bug".into()));
    lin.edit("ENG-2", |i| i.labels.push("lbl-bug".into()));
    svc.sync(2).unwrap();
    svc.set_status(b.id, TaskStatus::Review, 3).unwrap();
    assert_eq!(lin.issue("ENG-2").state, "st-review");
    assert!(lin.label_names("ENG-2").contains(&"bug".to_string()));
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
    lin.add("ENG-9", "From Linear", "st-todo", &["orch"]);
    lin.0.lock().unwrap().relations.push(("rel-9".into(), "ENG-2".into(), "ENG-9".into()));
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
    // The states the mapping doesn't name (by type), for the Board.
    let rep = svc.sync(4).unwrap();
    assert_eq!(rep.unmapped_states, ["Backlog", "Todo", "In Progress", "Doing it", "Done", "Canceled"]);
    assert_eq!(rep.summary()["unmapped_states"][0], "Backlog");
}

fn github_setup() -> (tempfile::TempDir, Store, Project, FakeGh) {
    let (d, s, p) = setup(json!({"type": "github", "repo": "me/shop"}));
    (d, s, p, FakeGh::default())
}

fn svc<'a>(s: &'a Store, p: &Project, gh: &FakeGh) -> TaskService<'a> {
    TaskService::new(s, p, Some(Box::new(gh.clone())))
}

fn by_issue(s: &Store, n: u64) -> Task {
    s.task_by_external("p1", "github", &n.to_string()).unwrap().unwrap()
}

/// Acceptance 2: more than 500 issues, most without `orch`, read page by
/// page; the closed ones without `orch` are left on GitHub; reading writes
/// nothing.
#[test]
fn github_reads_every_open_issue_paged() {
    let (_d, s, p, gh) = github_setup();
    for n in 1..=600 {
        gh.add(n, &format!("Issue {n}"), "open", if n == 5 { &["orch:review", "bug"] } else { &["bug"] });
    }
    for n in 601..=700 {
        gh.add(n, "Old managed", "closed", &["orch"]);
    }
    for n in 701..=750 {
        gh.add(n, "Old", "closed", &["bug"]);
    }
    let calls = gh.0.lock().unwrap().calls;
    let rep = svc(&s, &p, &gh).sync(1).unwrap();
    assert_eq!(rep.imported.len(), 700);
    assert!(rep.errors.is_empty(), "{rep:?}");
    let w = gh.0.lock().unwrap();
    assert_eq!(w.calls - calls, 2, "two paged reads, no write");
    assert_eq!(
        w.lists,
        [
            ("repos/me/shop/issues?state=open&per_page=100".to_string(), 6),
            ("repos/me/shop/issues?state=closed&labels=orch&per_page=100".to_string(), 1)
        ]
    );
    drop(w);
    assert_eq!(by_issue(&s, 1).status, TaskStatus::Backlog);
    assert_eq!(by_issue(&s, 5).status, TaskStatus::Review);
    assert_eq!(by_issue(&s, 650).status, TaskStatus::Done);
    assert!(s.task_by_external("p1", "github", "720").unwrap().is_none());
    assert_eq!(gh.label_names(1), ["bug"], "import adds no label");
}

/// Acceptance 3 and 5: a status change sends only labels and state (a
/// foreign issue becomes managed, its labels kept); a title/body edit only
/// title and body; nothing changed, nothing sent.
#[test]
fn github_writes_only_what_changed() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "Foreign", "open", &["bug"]);
    let svc = svc(&s, &p, &gh);
    svc.sync(1).unwrap();
    let t = by_issue(&s, 1);
    gh.edit(1, |i| i["body"] = json!("Written on GitHub\r\n"));

    let out = svc.set_status(t.id, TaskStatus::InProgress, 2).unwrap();
    assert!(out.remote_error.is_none(), "{:?}", out.remote_error);
    let (n, sent) = gh.last_patch();
    assert_eq!((n, sent), (1, vec!["labels".to_string(), "state".to_string()]));
    assert_eq!(gh.label_names(1), ["orch", "orch:in_progress", "bug"]);
    assert_eq!(gh.issue(1)["body"], json!("Written on GitHub\r\n"), "the body is not rewritten");
    let after = by_issue(&s, 1);
    assert_eq!(after.body, "Written on GitHub", "the remote body is taken, not hidden by the new stamp");
    assert_eq!(svc.sync(2).unwrap().updated, Vec::<u64>::new());
    svc.set_status(t.id, TaskStatus::Done, 3).unwrap();
    assert_eq!(gh.issue(1)["state"], json!("closed"));
    svc.set_status(t.id, TaskStatus::Ready, 4).unwrap();
    assert_eq!(gh.issue(1)["state"], json!("open"));
    assert_eq!(gh.label_names(1), ["orch", "orch:ready", "bug"]);

    // Title and body: no label read, no labels/state sent.
    let gets = gh.0.lock().unwrap().gets;
    let mut t = s.task(t.id).unwrap().unwrap();
    (t.title, t.body) = ("Renamed".into(), "New body".into());
    svc.update(t.clone(), 5).unwrap();
    let (_, sent) = gh.last_patch();
    assert_eq!(sent, ["body", "title"]);
    assert_eq!(gh.0.lock().unwrap().gets, gets);
    assert_eq!((gh.issue(1)["title"].clone(), gh.issue(1)["state"].clone()), (json!("Renamed"), json!("open")));

    // A change GitHub does not show: no call at all.
    let calls = gh.0.lock().unwrap().calls;
    let mut t = s.task(t.id).unwrap().unwrap();
    t.branch = Some("orch/x".into());
    let out = svc.update(t, 6).unwrap();
    assert!(out.remote_error.is_none());
    assert_eq!(gh.0.lock().unwrap().calls, calls);
}

/// Acceptance 4: after the first sync only what changed since is read; a
/// close, an edit and a reopen come back; a closed issue the app never
/// tracked, without `orch`, is not imported.
#[test]
fn github_incremental_sync() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "Foreign", "open", &[]);
    gh.add(2, "Managed", "open", &["orch", "orch:ready"]);
    let svc = svc(&s, &p, &gh);
    assert_eq!(svc.sync(1).unwrap().imported.len(), 2);
    gh.edit(1, |i| i["title"] = json!("Foreign v2"));
    gh.edit(2, |i| i["state"] = json!("closed"));
    gh.add(3, "Closed elsewhere", "closed", &["wontfix"]);
    gh.add(4, "New", "open", &[]);
    let rep = svc.sync(2).unwrap();
    let last = gh.last_list();
    // The cursor (issue 2's first stamp) minus a minute.
    assert_eq!(last, format!("repos/me/shop/issues?state=all&since={}&per_page=100", "2026-11-02T10:19:00Z"));
    assert_eq!(rep.imported.len(), 1);
    assert_eq!(rep.updated.len(), 2);
    assert_eq!(by_issue(&s, 1).title, "Foreign v2");
    assert_eq!(by_issue(&s, 2).status, TaskStatus::Done);
    assert!(s.task_by_external("p1", "github", "3").unwrap().is_none());
    assert_eq!(by_issue(&s, 4).status, TaskStatus::Backlog);
    gh.edit(2, |i| i["state"] = json!("open"));
    let rep = svc.sync(3).unwrap();
    assert_eq!(rep.updated, [by_issue(&s, 2).id]);
    assert_eq!(by_issue(&s, 2).status, TaskStatus::Ready);
    assert_eq!(svc.sync(4).unwrap(), SyncReport::default(), "nothing changed");
}

/// Acceptance 7: an offline status change is pushed at the next sync when
/// the issue did not change (so it is not in the incremental list); when it
/// did, the remote wins.
#[test]
fn github_pending_change_not_in_the_incremental_list() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "Old", "open", &[]);
    gh.add(2, "Newer", "open", &[]);
    let svc = svc(&s, &p, &gh);
    svc.sync(1).unwrap();
    let t = by_issue(&s, 1);
    gh.0.lock().unwrap().offline = true;
    assert!(svc.set_status(t.id, TaskStatus::InProgress, 2).unwrap().remote_error.is_some());
    gh.0.lock().unwrap().offline = false;
    let rep = svc.sync(3).unwrap();
    assert_eq!(rep.pushed, [t.id]);
    let (n, sent) = gh.last_patch();
    assert_eq!((n, sent), (1, ["body", "labels", "state", "title"].map(String::from).to_vec()));
    assert_eq!(gh.label_names(1), ["orch", "orch:in_progress"]);
    assert!(!by_issue(&s, 1).remote_updated_at.unwrap().starts_with(PENDING));

    // Changed on both sides: GitHub wins.
    gh.0.lock().unwrap().offline = true;
    svc.set_status(t.id, TaskStatus::Review, 4).unwrap();
    gh.0.lock().unwrap().offline = false;
    gh.edit(1, |i| i["title"] = json!("Theirs"));
    let rep = svc.sync(5).unwrap();
    assert_eq!(rep.conflicts, [t.id]);
    let t = by_issue(&s, 1);
    assert_eq!((t.title.as_str(), t.status), ("Theirs", TaskStatus::InProgress));
}

/// A full read (no cursor) does not list a tracked issue closed without
/// `orch`: a pending task on it is read alone, then pushed or adopted.
#[test]
fn github_pending_change_not_in_the_full_list() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "Foreign", "open", &[]);
    gh.add(2, "Foreign too", "open", &[]);
    let svc = svc(&s, &p, &gh);
    svc.sync(1).unwrap();
    gh.edit(1, |i| i["state"] = json!("closed"));
    gh.edit(2, |i| i["state"] = json!("closed"));
    svc.sync(2).unwrap();
    let (a, b) = (by_issue(&s, 1), by_issue(&s, 2));
    assert_eq!((a.status, b.status), (TaskStatus::Done, TaskStatus::Done));
    // Both reopened in the app while offline; #2 then edited on GitHub.
    gh.0.lock().unwrap().offline = true;
    svc.set_status(a.id, TaskStatus::Ready, 3).unwrap();
    svc.set_status(b.id, TaskStatus::Ready, 3).unwrap();
    gh.0.lock().unwrap().offline = false;
    gh.edit(2, |i| i["title"] = json!("Theirs"));
    s.set_task_sync_cursor("p1", "github:me/old", "2026-11-02T10:00:00Z").unwrap();
    let rep = svc.sync(4).unwrap();
    assert!(gh.last_list().contains("state=closed&labels=orch"), "a full read");
    assert_eq!((rep.pushed, rep.conflicts), (vec![a.id], vec![b.id]));
    assert_eq!(gh.issue(1)["state"], json!("open"));
    assert_eq!(by_issue(&s, 2).status, TaskStatus::Done);
}

/// Acceptance 8: the cursor survives a restart; a new checkout (or a
/// cursor of another repository) means a full read again.
#[test]
fn github_cursor_survives_a_restart() {
    let (d, s, mut p, gh) = github_setup();
    gh.add(1, "One", "open", &[]);
    svc(&s, &p, &gh).sync(1).unwrap();
    assert_eq!(s.task_sync_cursor("p1").unwrap(), Some(("github:me/shop".into(), stamp(1))));
    s.flush().unwrap();
    drop(s);
    let s = Store::open(&d.path().join("o.db")).unwrap();
    svc(&s, &p, &gh).sync(2).unwrap();
    assert!(gh.last_list().contains("since="), "{}", gh.last_list());
    p.path = d.path().join("moved");
    s.upsert_project(&p).unwrap();
    svc(&s, &p, &gh).sync(3).unwrap();
    assert!(gh.last_list().contains("state=closed&labels=orch"), "a full read: {}", gh.last_list());
    s.set_task_sync_cursor("p1", "github:me/old", &stamp(1)).unwrap();
    svc(&s, &p, &gh).sync(4).unwrap();
    assert!(gh.last_list().contains("state=closed&labels=orch"), "a full read: {}", gh.last_list());
    assert_eq!(svc(&s, &p, &gh).github_repo().unwrap().as_deref(), Some("me/shop"));
}

/// A full read (no cursor) does not list a tracked issue closed on GitHub
/// without `orch`: a task still open in the app is read alone and closed.
#[test]
fn github_full_read_finds_a_tracked_issue_closed_elsewhere() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "Foreign", "open", &[]);
    let svc = svc(&s, &p, &gh);
    svc.sync(1).unwrap();
    gh.edit(1, |i| i["state"] = json!("closed"));
    s.set_task_sync_cursor("p1", "github:me/old", &stamp(1)).unwrap();
    let rep = svc.sync(2).unwrap();
    assert!(gh.last_list().contains("state=closed&labels=orch"), "a full read");
    assert_eq!(rep.updated, [by_issue(&s, 1).id]);
    assert_eq!(by_issue(&s, 1).status, TaskStatus::Done);
}

/// A cursor that does not parse is replaced by the listing's, not kept.
#[test]
fn github_unparsable_cursor_is_replaced() {
    let (_d, s, p, gh) = github_setup();
    gh.add(1, "One", "open", &[]);
    s.set_task_sync_cursor("p1", "github:me/shop", "garbage").unwrap();
    svc(&s, &p, &gh).sync(1).unwrap();
    assert_eq!(s.task_sync_cursor("p1").unwrap(), Some(("github:me/shop".into(), stamp(1))));
}

/// The Board asks for the repository without running `gh`: unknown until a
/// first call resolved it (or configured up front).
#[test]
fn known_github_repo_never_runs_gh() {
    let (_d, s, p, gh) = github_setup();
    assert_eq!(svc(&s, &p, &gh).known_github_repo().as_deref(), Some("me/shop"), "configured");
    let (_d, s, mut p, gh) = github_setup();
    p.task_backend = json!({"type": "github"});
    s.upsert_project(&p).unwrap();
    let backends = Backends::default();
    let service = || backends.service(&s, &p, || Some(Box::new(gh.clone())), || Box::new(linear::Unconfigured));
    assert_eq!(service().known_github_repo(), None);
    assert_eq!(gh.0.lock().unwrap().calls, 0, "no gh call");
    service().github_repo().unwrap();
    assert_eq!(service().known_github_repo().as_deref(), Some("me/shop"), "resolved once");
}

fn linear_setup(project: Option<&str>) -> (tempfile::TempDir, Store, Project, linear::fake::FakeLinear) {
    let mut backend = json!({"type": "linear", "team": "ENG"});
    if let Some(p) = project {
        backend["project"] = json!(p);
    }
    let (d, s, p) = setup(backend);
    (d, s, p, linear::fake::FakeLinear::default())
}

fn lin_svc<'a>(s: &'a Store, p: &Project, lin: &linear::fake::FakeLinear) -> TaskService<'a> {
    let project = p.task_backend["project"].as_str().map(str::to_string);
    let remote = linear::LinearIssues::new(lin.clone(), "ENG", project, Default::default());
    TaskService::with_remote(s, p, "linear", Arc::new(remote))
}

fn by_key(s: &Store, key: &str) -> Task {
    s.task_by_external("p1", "linear", key).unwrap().unwrap()
}

/// The variables and the issue ids of the last `OrchIssues` page.
fn last_linear_list(lin: &linear::fake::FakeLinear) -> (Value, Vec<String>) {
    lin.0.lock().unwrap().lists.last().cloned().unwrap()
}

/// The last `OrchIssueUpdate` input.
fn last_linear_update(lin: &linear::fake::FakeLinear) -> Value {
    lin.0.lock().unwrap().updates.last().cloned().unwrap().1
}

/// fix-linear-pull §3.4 (1): the first sync imports the team's open issues
/// without `orch`, each in its state's column, and the closed `orch` ones;
/// completed or canceled issues without `orch` stay in Linear; reading
/// writes nothing.
#[test]
fn linear_first_sync_reads_every_open_issue() {
    let (_d, s, p, lin) = linear_setup(None);
    lin.add("ENG-1", "Idea", "st-backlog", &["bug"]);
    lin.add("ENG-2", "Next", "st-todo", &[]);
    lin.add("ENG-3", "Doing", "st-progress", &[]);
    lin.add("ENG-4", "Reviewing", "st-review", &[]);
    lin.add("ENG-5", "Shipped", "st-done", &[]);
    lin.add("ENG-6", "Dropped", "st-canceled", &[]);
    lin.add("ENG-7", "Ours, done", "st-done", &["orch"]);
    let rep = lin_svc(&s, &p, &lin).sync(1).unwrap();
    assert_eq!(rep.imported.len(), 5, "{rep:?}");
    let col = |k: &str| by_key(&s, k).status;
    assert_eq!(
        [col("ENG-1"), col("ENG-2"), col("ENG-3"), col("ENG-4"), col("ENG-7")],
        [TaskStatus::Backlog, TaskStatus::Ready, TaskStatus::InProgress, TaskStatus::Review, TaskStatus::Done]
    );
    for k in ["ENG-5", "ENG-6"] {
        assert!(s.task_by_external("p1", "linear", k).unwrap().is_none(), "{k} is history");
    }
    assert_eq!(lin.0.lock().unwrap().mutations, Vec::<String>::new(), "reading never writes");
    let (vars, _) = last_linear_list(&lin);
    assert_eq!(vars["filter"]["or"][0]["state"]["type"]["nin"], json!(["completed", "canceled"]));
    assert_eq!(vars["filter"]["or"][1]["labels"]["name"]["eq"], "orch");
    assert!(vars["filter"].get("updatedAt").is_none());
}

/// §3.4 (2): more than the old cap of 1 250 issues (50 pages) is read.
#[test]
fn linear_reads_past_the_old_page_cap() {
    let (_d, s, p, lin) = linear_setup(None);
    for n in 1..=1_300 {
        lin.add(&format!("ENG-{n}"), &format!("Issue {n}"), "st-todo", &[]);
    }
    let rep = lin_svc(&s, &p, &lin).sync(1).unwrap();
    assert_eq!(rep.imported.len(), 1_300);
    assert_eq!(lin.0.lock().unwrap().lists.len(), 52, "pages of 25");
    assert_eq!(by_key(&s, "ENG-1300").title, "Issue 1300");
}

/// §3.4 (3): the next sync asks only for what changed since the cursor
/// (minus a minute); an edit, a close and a reopen in Linear come back, a
/// closed issue never tracked and without `orch` is not imported. The
/// cursor is stored with scope `linear:ENG`.
#[test]
fn linear_incremental_sync() {
    let (_d, s, p, lin) = linear_setup(None);
    // ENG-3 is older than the overlap of a minute before the cursor.
    lin.add("ENG-3", "C", "st-todo", &[]);
    lin.wait(300);
    lin.add("ENG-1", "A", "st-todo", &[]);
    lin.add("ENG-2", "B", "st-progress", &[]);
    let svc = lin_svc(&s, &p, &lin);
    assert_eq!(svc.sync(1).unwrap().imported.len(), 3);
    let newest = linear::fake::stamp(lin.issue("ENG-2").updated);
    let cursor = newest.to_string();
    assert_eq!(s.task_sync_cursor("p1").unwrap(), Some(("linear:ENG".to_string(), cursor.clone())));

    lin.wait(300);
    lin.edit("ENG-1", |i| i.title = "A v2".into());
    lin.edit("ENG-2", |i| i.state = "st-done".into());
    lin.add("ENG-4", "Closed elsewhere", "st-done", &[]);
    lin.add("ENG-5", "New", "st-backlog", &[]);
    let rep = svc.sync(2).unwrap();
    let (vars, ids) = last_linear_list(&lin);
    let from = newest - jiff::SignedDuration::from_secs(60);
    assert_eq!(vars["filter"]["updatedAt"]["gt"], from.to_string());
    assert!(vars["filter"].get("or").is_none(), "all states");
    assert_eq!(ids, ["ENG-1", "ENG-2", "ENG-4", "ENG-5"], "ENG-3 did not change");
    assert_eq!(rep.updated, [by_key(&s, "ENG-1").id, by_key(&s, "ENG-2").id]);
    assert_eq!(rep.imported, [by_key(&s, "ENG-5").id]);
    assert_eq!(by_key(&s, "ENG-1").title, "A v2");
    assert_eq!(by_key(&s, "ENG-2").status, TaskStatus::Done);
    assert!(s.task_by_external("p1", "linear", "ENG-4").unwrap().is_none());
    assert_ne!(s.task_sync_cursor("p1").unwrap().unwrap().1, cursor, "the cursor moves");

    lin.edit("ENG-2", |i| i.state = "st-todo".into());
    let rep = svc.sync(3).unwrap();
    assert_eq!(rep.updated, [by_key(&s, "ENG-2").id]);
    assert_eq!(by_key(&s, "ENG-2").status, TaskStatus::Ready);
    assert_eq!(svc.sync(4).unwrap(), SyncReport::default(), "nothing changed");
}

/// §3.4 (4): moving a foreign card sends the state and the labels (adds
/// `orch`, keeps the others) but no title or description; editing its
/// title sends the content but no state.
#[test]
fn linear_writes_only_what_changed() {
    let (_d, s, p, lin) = linear_setup(None);
    lin.add("ENG-1", "Foreign", "st-todo", &["bug"]);
    let svc = lin_svc(&s, &p, &lin);
    svc.sync(1).unwrap();
    // Edited in Linear since the sync: a status change must not undo it.
    lin.edit("ENG-1", |i| i.description = "Edited in Linear".into());
    let t = by_key(&s, "ENG-1");
    let out = svc.set_status(t.id, TaskStatus::InProgress, 2).unwrap();
    assert!(out.remote_error.is_none(), "{:?}", out.remote_error);
    let inp = last_linear_update(&lin);
    assert_eq!(inp["stateId"], "st-progress");
    assert!(inp.get("title").is_none() && inp.get("description").is_none(), "{inp}");
    assert_eq!(lin.label_names("ENG-1"), ["orch", "bug"]);
    let i = lin.issue("ENG-1");
    assert_eq!((i.title.as_str(), i.description.as_str()), ("Foreign", "Edited in Linear"));
    assert_eq!(out.value.body, "Edited in Linear", "the task takes what it did not send");

    let mut t = by_key(&s, "ENG-1");
    t.title = "Renamed".into();
    let calls = lin.0.lock().unwrap().calls.len();
    let out = svc.update(t, 3).unwrap();
    assert!(out.remote_error.is_none(), "{:?}", out.remote_error);
    assert_eq!(lin.0.lock().unwrap().calls[calls..], ["OrchIssueUpdate"], "a content write reads nothing first");
    let inp = last_linear_update(&lin);
    assert_eq!((inp["title"].as_str(), inp["description"].as_str()), (Some("Renamed"), Some("Edited in Linear")));
    assert!(inp.get("stateId").is_none() && inp.get("labelIds").is_none(), "{inp}");
    assert_eq!(lin.issue("ENG-1").state, "st-progress");
    assert_eq!(svc.sync(4).unwrap(), SyncReport::default(), "our own writes are not remote changes");
}

/// §3.4 (5): a status change made offline is pushed at the next sync when
/// the issue did not change (so it is not in the incremental list); when
/// it did, Linear wins.
#[test]
fn linear_pending_change_not_in_the_incremental_list() {
    let (_d, s, p, lin) = linear_setup(None);
    lin.add("ENG-1", "Old", "st-todo", &[]);
    lin.wait(300);
    lin.add("ENG-2", "Other", "st-todo", &[]);
    let svc = lin_svc(&s, &p, &lin);
    svc.sync(1).unwrap();
    lin.0.lock().unwrap().offline = true;
    let (a, b) = (by_key(&s, "ENG-1").id, by_key(&s, "ENG-2").id);
    assert!(svc.set_status(a, TaskStatus::InProgress, 2).unwrap().remote_error.is_some());
    assert!(svc.set_status(b, TaskStatus::Review, 3).unwrap().remote_error.is_some());
    lin.0.lock().unwrap().offline = false;
    lin.edit("ENG-2", |i| i.title = "Changed in Linear".into());
    let rep = svc.sync(4).unwrap();
    let (_, ids) = last_linear_list(&lin);
    assert_eq!(ids, ["ENG-2"], "ENG-1 did not change");
    assert_eq!((rep.pushed, rep.conflicts), (vec![a], vec![b]));
    assert_eq!(lin.issue("ENG-1").state, "st-progress");
    let t = by_key(&s, "ENG-2");
    assert_eq!((t.title.as_str(), t.status), ("Changed in Linear", TaskStatus::Ready), "Linear wins");
}

/// §3.4 (6): with a project, only its issues are listed (and created
/// there); the cursor's scope names it.
#[test]
fn linear_project_scopes_the_listing() {
    let (_d, s, p, lin) = linear_setup(Some("proj-1"));
    lin.add("ENG-1", "In the project", "st-todo", &[]);
    lin.add("ENG-2", "Elsewhere", "st-todo", &[]);
    lin.edit("ENG-1", |i| i.project = Some("proj-1".into()));
    let svc = lin_svc(&s, &p, &lin);
    let rep = svc.sync(1).unwrap();
    assert_eq!(rep.imported, [by_key(&s, "ENG-1").id]);
    assert!(s.task_by_external("p1", "linear", "ENG-2").unwrap().is_none());
    assert_eq!(last_linear_list(&lin).0["filter"]["project"]["id"]["eq"], "proj-1");
    assert_eq!(s.task_sync_cursor("p1").unwrap().unwrap().0, "linear:ENG/proj-1");
    let t = svc.create(Task::new("", "", "New one", 2)).unwrap().value;
    let key = t.external_id.unwrap();
    assert_eq!(lin.issue(&key).project.as_deref(), Some("proj-1"));
}
