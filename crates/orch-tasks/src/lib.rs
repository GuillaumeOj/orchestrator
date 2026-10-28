#![doc = include_str!("../README.md")]

pub mod github;
pub mod linear;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use orch_store::{Project, Store, Task, TaskBackendConfig, TaskNote};
use orch_team::TaskStatus;

use crate::github::{Gh, GhCli, GithubIssues, Issue, IssueWrite};
use crate::linear::{Graphql, LinearIssues};

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
    /// Remote states the project's mapping does not name (Linear).
    pub unmapped_states: Vec<String>,
}

impl SyncReport {
    /// Counts for the UI and the event log.
    pub fn summary(&self) -> serde_json::Value {
        let pulled = self.imported.len() + self.updated.len() + self.conflicts.len();
        serde_json::json!({"pushed": self.pushed.len(), "pulled": pulled, "imported": self.imported.len(),
                           "updated": self.updated.len(), "conflicts": self.conflicts.len(), "errors": self.errors,
                           "unmapped_states": self.unmapped_states})
    }
}

/// A remote task backend (GitHub Issues, Linear): the issue operations the
/// sync needs. `Issue.number` is the remote's id.
pub trait Remote: Send + Sync {
    fn list_issues(&self) -> Result<Vec<Issue>>;
    fn create_issue(&self, w: &IssueWrite) -> Result<Issue>;
    /// Replaces the managed fields; labels the app does not manage are kept.
    fn update_issue(&self, id: &str, w: &IssueWrite) -> Result<Issue>;
    fn get_issue(&self, id: &str) -> Result<Issue>;
    fn add_comment(&self, id: &str, text: &str) -> Result<()>;
    /// Remote states the project's own mapping does not name (they get the
    /// column of their type); empty without a mapping.
    fn unmapped_states(&self) -> Vec<String> {
        Vec::new()
    }
}

impl<G: Gh> Remote for GithubIssues<G> {
    fn list_issues(&self) -> Result<Vec<Issue>> {
        self.list()
    }
    fn create_issue(&self, w: &IssueWrite) -> Result<Issue> {
        self.create(w)
    }
    fn update_issue(&self, id: &str, w: &IssueWrite) -> Result<Issue> {
        let keep = self.get(id)?.other_labels;
        self.update(id, w, &keep)
    }
    fn get_issue(&self, id: &str) -> Result<Issue> {
        self.get(id)
    }
    fn add_comment(&self, id: &str, text: &str) -> Result<()> {
        self.comment(id, text)
    }
}

impl<T: Graphql> Remote for LinearIssues<T> {
    fn list_issues(&self) -> Result<Vec<Issue>> {
        self.list()
    }
    fn create_issue(&self, w: &IssueWrite) -> Result<Issue> {
        self.create(w)
    }
    fn update_issue(&self, id: &str, w: &IssueWrite) -> Result<Issue> {
        self.update(id, w)
    }
    fn get_issue(&self, id: &str) -> Result<Issue> {
        self.get(id)
    }
    fn add_comment(&self, id: &str, text: &str) -> Result<()> {
        self.comment(id, text)
    }
    fn unmapped_states(&self) -> Vec<String> {
        self.unmapped_states()
    }
}

#[derive(Clone)]
enum Backend {
    Local,
    Remote(&'static str, Arc<dyn Remote>),
}

fn github(project: &Project, repo: Option<String>, gh: Option<Box<dyn Gh>>) -> Arc<dyn Remote> {
    Arc::new(GithubIssues::new(gh.unwrap_or_else(|| Box::new(GhCli::new(project.path.clone()))), repo))
}

fn remote(
    project: &Project,
    gh: impl FnOnce() -> Option<Box<dyn Gh>>,
    linear: impl FnOnce() -> Box<dyn Graphql>,
) -> Backend {
    match project.backend() {
        TaskBackendConfig::Local => Backend::Local,
        TaskBackendConfig::Github { repo } => Backend::Remote("github", github(project, repo, gh())),
        TaskBackendConfig::Linear { team, project: lp, states } => {
            Backend::Remote("linear", Arc::new(LinearIssues::new(linear(), &team, lp, states)))
        }
    }
}

/// One backend per project, kept across calls: a remote backend resolves
/// its repository or team and its labels once, not on every task write, and
/// keeps its rate-limit back-off. A project whose backend setting or
/// checkout changes gets a new one.
#[derive(Default)]
pub struct Backends {
    by_project: Mutex<HashMap<String, (TaskBackendConfig, PathBuf, Backend)>>,
}

impl Backends {
    /// The tasks of `project`. `gh` makes the `gh` runner when a GitHub
    /// backend is created (`None` = the real `gh`); `linear` the GraphQL
    /// transport of a Linear one.
    pub fn service<'a>(
        &self,
        store: &'a Store,
        project: &Project,
        gh: impl FnOnce() -> Option<Box<dyn Gh>>,
        linear: impl FnOnce() -> Box<dyn Graphql>,
    ) -> TaskService<'a> {
        let cfg = project.backend();
        if cfg == TaskBackendConfig::Local {
            return TaskService { store, project: project.clone(), backend: Backend::Local };
        }
        let mut m = self.by_project.lock().unwrap_or_else(|e| e.into_inner());
        let backend = match m.get(&project.id) {
            Some((c, path, b)) if *c == cfg && *path == project.path => b.clone(),
            _ => {
                let b = remote(project, gh, linear);
                m.insert(project.id.clone(), (cfg.clone(), project.path.clone(), b.clone()));
                b
            }
        };
        TaskService { store, project: project.clone(), backend }
    }
}

/// The tasks of one project over its backend (ADR-004). Every task is
/// mirrored in the store; with a remote, writes go through to the issue at
/// once and [`TaskService::sync`] brings remote changes back. A task whose
/// `backend` is not the project's (left out when the backend changed, D8)
/// stays local and is never pushed.
pub struct TaskService<'a> {
    store: &'a Store,
    project: Project,
    backend: Backend,
}

impl<'a> TaskService<'a> {
    /// `gh` replaces the real `gh` (tests).
    pub fn new(store: &'a Store, project: &Project, gh: Option<Box<dyn Gh>>) -> TaskService<'a> {
        let backend = remote(project, || gh, || Box::new(linear::Unconfigured));
        TaskService { store, project: project.clone(), backend }
    }

    /// Over a given remote (tests, and a Linear transport chosen by the caller).
    pub fn with_remote(store: &'a Store, project: &Project, kind: &'static str, r: Arc<dyn Remote>) -> TaskService<'a> {
        TaskService { store, project: project.clone(), backend: Backend::Remote(kind, r) }
    }

    pub fn kind(&self) -> &'static str {
        match self.backend {
            Backend::Local => "local",
            Backend::Remote(k, _) => k,
        }
    }

    /// The remote of a task the project's backend owns.
    fn remote_of(&self, t: &Task) -> Option<&Arc<dyn Remote>> {
        match &self.backend {
            Backend::Remote(k, r) if t.backend == *k => Some(r),
            _ => None,
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
    fn push(&self, gh: &dyn Remote, t: &mut Task) -> Result<()> {
        let w = self.write_of(t);
        let issue = match &t.external_id {
            None => gh.create_issue(&w)?,
            Some(n) => gh.update_issue(n, &w)?,
        };
        t.external_id = Some(issue.number);
        t.remote_updated_at = Some(issue.updated_at);
        Ok(())
    }

    fn save_and_push(&self, mut t: Task) -> Result<Outcome<Task>> {
        if t.id == 0 || t.backend.is_empty() {
            t.backend = self.kind().into();
        }
        t.project_id = self.project.id.clone();
        t.id = self.store.upsert_task(&t)?;
        let mut remote_error = None;
        if let Some(gh) = self.remote_of(&t).cloned() {
            match self.push(gh.as_ref(), &mut t) {
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
        if let (Some(gh), Some(n)) = (self.remote_of(&t).cloned(), t.external_id.clone()) {
            match gh.add_comment(&n, &format!("**{by}**: {text}")) {
                Err(e) => remote_error = Some(format!("{e:#}")),
                // A comment moves the issue's updatedAt: record it, or the next
                // sync takes our own comment for a remote change (and a later
                // pending change for a conflict).
                Ok(()) if Self::pending(&t).is_none() => {
                    if let Ok(issue) = gh.get_issue(&n) {
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
        let Backend::Remote(kind, gh) = &self.backend else { return Ok(SyncReport::default()) };
        let (kind, gh) = (*kind, gh.as_ref());
        let mut rep = SyncReport::default();
        let issues = gh.list_issues()?;
        let local = self.store.tasks(Some(&self.project.id), None, None)?;
        // Local tasks the remote never got.
        for t in local.iter().filter(|t| t.backend == kind && t.external_id.is_none()) {
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
            match self.store.task_by_external(&self.project.id, kind, &issue.number)? {
                None => {
                    let mut t = Task::new(&self.project.id, kind, &issue.title, now_ms);
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
        rep.unmapped_states = gh.unmapped_states();
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
            .filter_map(|n| self.store.task_by_external(&self.project.id, self.kind(), n).ok().flatten())
            .map(|d| d.id)
            .collect()
    }
}

#[cfg(test)]
mod tests;
