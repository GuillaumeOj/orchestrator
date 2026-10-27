#![doc = include_str!("../README.md")]

pub mod github;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use orch_store::{Project, Store, Task, TaskBackendConfig, TaskNote};
use orch_team::TaskStatus;

use crate::github::{Gh, GhCli, GithubIssues, Issue, IssueWrite};

impl Gh for Box<dyn Gh> {
    fn run(&self, args: &[String]) -> Result<String> {
        (**self).run(args)
    }
}

/// `remote_updated_at` of a task whose last change did not reach the remote:
/// `pending:<the remote's updatedAt we last saw>`.
const PENDING: &str = "pending:";

/// A result plus the remote's failure, if any: the local change is kept and
/// pushed again at the next sync.
#[derive(Debug)]
pub struct Outcome<T> {
    pub value: T,
    pub remote_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncReport {
    pub imported: Vec<u64>,
    pub updated: Vec<u64>,
    pub pushed: Vec<u64>,
    /// Changed on both sides since the last sync: the remote won (FR-503).
    pub conflicts: Vec<u64>,
    pub errors: Vec<String>,
}

impl SyncReport {
    /// Counts for the UI and the event log.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"imported": self.imported.len(), "updated": self.updated.len(), "pushed": self.pushed.len(),
                           "conflicts": self.conflicts.len(), "errors": self.errors})
    }
}

enum Backend {
    Local,
    Github(Arc<GithubIssues<Box<dyn Gh>>>),
}

type GithubBackend = Arc<GithubIssues<Box<dyn Gh>>>;

fn github(project: &Project, repo: Option<String>, gh: Option<Box<dyn Gh>>) -> GithubBackend {
    Arc::new(GithubIssues::new(gh.unwrap_or_else(|| Box::new(GhCli::new(project.path.clone()))), repo))
}

/// One backend per project, kept across calls: a GitHub backend resolves
/// its repository and creates its labels once, not on every task write. A
/// project whose backend setting or checkout changes gets a new one.
#[derive(Default)]
pub struct Backends {
    by_project: Mutex<HashMap<String, (TaskBackendConfig, PathBuf, GithubBackend)>>,
}

impl Backends {
    /// The tasks of `project`. `gh` makes the `gh` runner when a GitHub
    /// backend is created (`None` = the real `gh`).
    pub fn service<'a>(
        &self,
        store: &'a Store,
        project: &Project,
        gh: impl FnOnce() -> Option<Box<dyn Gh>>,
    ) -> TaskService<'a> {
        let cfg = project.backend();
        let TaskBackendConfig::Github { repo } = &cfg else {
            return TaskService { store, project: project.clone(), backend: Backend::Local };
        };
        let mut m = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let gh = match m.get(&project.id) {
            Some((c, path, b)) if *c == cfg && *path == project.path => b.clone(),
            _ => {
                let b = github(project, repo.clone(), gh());
                m.insert(project.id.clone(), (cfg.clone(), project.path.clone(), b.clone()));
                b
            }
        };
        TaskService { store, project: project.clone(), backend: Backend::Github(gh) }
    }
}

/// The tasks of one project over its backend (ADR-004). Every task is
/// mirrored in the store; with GitHub, writes go through to the issue at once
/// and [`TaskService::sync`] brings remote changes back.
pub struct TaskService<'a> {
    store: &'a Store,
    project: Project,
    backend: Backend,
}

impl<'a> TaskService<'a> {
    /// `gh` replaces the real `gh` (tests).
    pub fn new(store: &'a Store, project: &Project, gh: Option<Box<dyn Gh>>) -> TaskService<'a> {
        let backend = match project.backend() {
            TaskBackendConfig::Local => Backend::Local,
            TaskBackendConfig::Github { repo } => Backend::Github(github(project, repo, gh)),
        };
        TaskService { store, project: project.clone(), backend }
    }

    pub fn kind(&self) -> &'static str {
        match self.backend {
            Backend::Local => "local",
            Backend::Github(_) => "github",
        }
    }

    fn write_of<'t>(&self, t: &'t Task) -> IssueWrite<'t> {
        let blocked_by = t
            .depends_on
            .iter()
            .filter_map(|id| self.store.task(*id).ok().flatten())
            .filter_map(|d| d.external_id)
            .collect();
        IssueWrite { title: &t.title, body: &t.body, status: t.status, node: t.node.as_deref(), blocked_by }
    }

    fn pending(t: &Task) -> Option<String> {
        t.remote_updated_at.as_deref().and_then(|s| s.strip_prefix(PENDING)).map(str::to_string)
    }

    fn mark_pending(t: &mut Task) {
        if Self::pending(t).is_none() {
            t.remote_updated_at = Some(format!("{PENDING}{}", t.remote_updated_at.clone().unwrap_or_default()));
        }
    }

    /// Pushes `t` to the remote (create or update) and records what it said.
    fn push(&self, gh: &GithubIssues<Box<dyn Gh>>, t: &mut Task) -> Result<()> {
        let w = self.write_of(t);
        let issue = match &t.external_id {
            None => gh.create(&w)?,
            Some(n) => {
                let keep = gh.get(n)?.other_labels;
                gh.update(n, &w, &keep)?
            }
        };
        t.external_id = Some(issue.number);
        t.remote_updated_at = Some(issue.updated_at);
        Ok(())
    }

    fn save_and_push(&self, mut t: Task) -> Result<Outcome<Task>> {
        t.backend = self.kind().into();
        t.project_id = self.project.id.clone();
        t.id = self.store.upsert_task(&t)?;
        let mut remote_error = None;
        if let Backend::Github(gh) = &self.backend {
            match self.push(gh, &mut t) {
                Ok(()) => {}
                Err(e) => {
                    Self::mark_pending(&mut t);
                    remote_error = Some(format!("{e:#}"));
                }
            }
            self.store.upsert_task(&t)?;
        }
        Ok(Outcome { value: t, remote_error })
    }

    /// Creates a task (`t.id` is ignored).
    pub fn create(&self, mut t: Task) -> Result<Outcome<Task>> {
        t.id = 0;
        t.external_id = None;
        t.remote_updated_at = None;
        self.save_and_push(t)
    }

    /// Saves a changed task and pushes it.
    pub fn update(&self, t: Task, now_ms: u64) -> Result<Outcome<Task>> {
        if t.id == 0 {
            return Err(anyhow!("update of a task with no id"));
        }
        let mut t = t;
        t.updated_at_ms = now_ms;
        self.save_and_push(t)
    }

    /// Sets a task's status (and agent), the common case.
    pub fn set_status(&self, id: u64, status: TaskStatus, now_ms: u64) -> Result<Outcome<Task>> {
        let mut t = self.store.task(id)?.ok_or_else(|| anyhow!("no task {id}"))?;
        t.status = status;
        self.update(t, now_ms)
    }

    /// Adds a note; on GitHub it is also a comment.
    pub fn note(&self, id: u64, by: &str, text: &str, now_ms: u64) -> Result<Outcome<Task>> {
        let mut t = self.store.task(id)?.ok_or_else(|| anyhow!("no task {id}"))?;
        t.notes.push(TaskNote { ts_ms: now_ms, by: by.into(), text: text.into() });
        t.updated_at_ms = now_ms;
        self.store.upsert_task(&t)?;
        let mut remote_error = None;
        if let (Backend::Github(gh), Some(n)) = (&self.backend, t.external_id.clone()) {
            match gh.comment(&n, &format!("**{by}**: {text}")) {
                Err(e) => remote_error = Some(format!("{e:#}")),
                // A comment moves the issue's updatedAt: record it, or the next
                // sync takes our own comment for a remote change (and a later
                // pending change for a conflict).
                Ok(()) if Self::pending(&t).is_none() => {
                    if let Ok(issue) = gh.get(&n) {
                        t.remote_updated_at = Some(issue.updated_at);
                        self.store.upsert_task(&t)?;
                    }
                }
                Ok(()) => {}
            }
        }
        Ok(Outcome { value: t, remote_error })
    }

    /// Brings the remote back (FR-503). New issues are imported; an issue
    /// changed since the last sync replaces the local copy; a local change
    /// that never reached the remote is pushed again, unless the remote
    /// changed too: then **the remote wins** and the task is listed in
    /// `conflicts` (the caller tells the user).
    pub fn sync(&self, now_ms: u64) -> Result<SyncReport> {
        let Backend::Github(gh) = &self.backend else { return Ok(SyncReport::default()) };
        let mut rep = SyncReport::default();
        let issues = gh.list()?;
        let local = self.store.tasks(Some(&self.project.id), None, None)?;
        // Local tasks the remote never got.
        for t in local.iter().filter(|t| t.backend == "github" && t.external_id.is_none()) {
            let mut t = t.clone();
            match self.push(gh, &mut t) {
                Ok(()) => {
                    self.store.upsert_task(&t)?;
                    rep.pushed.push(t.id);
                }
                Err(e) => rep.errors.push(format!("{}: {e:#}", t.handle())),
            }
        }
        // Adopted issues whose `Blocked by:` may name an issue imported later
        // in this pass: resolved again once every issue is in the store.
        let mut adopted: Vec<(u64, &Issue)> = Vec::new();
        for issue in &issues {
            match self.store.task_by_external(&self.project.id, "github", &issue.number)? {
                None => {
                    let mut t = Task::new(&self.project.id, "github", &issue.title, now_ms);
                    t.external_id = Some(issue.number.clone());
                    self.adopt(&mut t, issue);
                    t.id = self.store.upsert_task(&t)?;
                    rep.imported.push(t.id);
                    adopted.push((t.id, issue));
                }
                Some(mut t) => match Self::pending(&t) {
                    Some(seen) if seen == issue.updated_at || seen.is_empty() && issue.updated_at.is_empty() => {
                        t.remote_updated_at = Some(seen);
                        match self.push(gh, &mut t) {
                            Ok(()) => {
                                self.store.upsert_task(&t)?;
                                rep.pushed.push(t.id);
                            }
                            Err(e) => rep.errors.push(format!("{}: {e:#}", t.handle())),
                        }
                    }
                    Some(_) => {
                        self.adopt(&mut t, issue);
                        t.updated_at_ms = now_ms;
                        self.store.upsert_task(&t)?;
                        rep.conflicts.push(t.id);
                        adopted.push((t.id, issue));
                    }
                    None if t.remote_updated_at.as_deref() != Some(issue.updated_at.as_str()) => {
                        self.adopt(&mut t, issue);
                        t.updated_at_ms = now_ms;
                        self.store.upsert_task(&t)?;
                        rep.updated.push(t.id);
                        adopted.push((t.id, issue));
                    }
                    None => {}
                },
            }
        }
        for (id, issue) in adopted {
            if issue.blocked_by.is_empty() {
                continue;
            }
            let Some(mut t) = self.store.task(id)? else { continue };
            let deps = self.deps_of(issue);
            if deps != t.depends_on {
                t.depends_on = deps;
                self.store.upsert_task(&t)?;
            }
        }
        Ok(rep)
    }

    fn adopt(&self, t: &mut Task, issue: &Issue) {
        t.title = issue.title.clone();
        t.body = issue.body.clone();
        t.status = issue.status;
        t.node = issue.node.clone().or(t.node.take());
        t.depends_on = self.deps_of(issue);
        t.remote_updated_at = Some(issue.updated_at.clone());
    }

    /// An issue's `Blocked by:` as local task ids (the ones in the store).
    fn deps_of(&self, issue: &Issue) -> Vec<u64> {
        issue
            .blocked_by
            .iter()
            .filter_map(|n| self.store.task_by_external(&self.project.id, "github", n).ok().flatten())
            .map(|d| d.id)
            .collect()
    }
}

#[cfg(test)]
mod tests;
