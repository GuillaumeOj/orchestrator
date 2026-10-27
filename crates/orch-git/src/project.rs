//! Project registry (FR-101, FR-109, FR-111, J1, J11): register a folder or
//! clone a URL; guided removal that keeps unmerged work.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use orch_store::{Project, Remote, Store};
use serde::{Deserialize, Serialize};

use crate::cleanup::{self, BranchInfo, Confirm, WorktreeInfo, WorktreeVerdict};
use crate::git::{git, git_deadline, real};
use crate::worktree::{self, WorktreeLedger, WorktreeRowState};

/// `git remote -v` (fetch URLs).
pub fn remotes(repo: &Path) -> Vec<Remote> {
    let mut v: Vec<Remote> = vec![];
    for line in git(repo, &["remote", "-v"]).unwrap_or_default().lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 && f[2] == "(fetch)" && !v.iter().any(|r| r.name == f[0]) {
            v.push(Remote { name: f[0].into(), url: f[1].into() });
        }
    }
    v
}

/// The default branch: `origin/HEAD` when known, else the checked-out branch.
pub fn detect_default_branch(repo: &Path) -> String {
    git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .and_then(|s| s.trim().strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| worktree::default_branch(repo))
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "project".into() } else { s.chars().take(48).collect() }
}

/// Registers a local folder (its main checkout). Idempotent: registering the
/// same checkout again returns the existing project (un-archived).
pub fn register(store: &Store, path: &Path) -> Result<Project> {
    let top = worktree::toplevel(path).with_context(|| format!("{} is not a git repository", path.display()))?;
    let top = real(&top);
    // A linked worktree is not a project: register its main checkout.
    let main = real(&worktree::main_checkout(&top)?);
    let now = orch_store::now_ms();
    if let Some(mut p) = store.project_by_path(&main)? {
        p.archived = false;
        p.remotes = remotes(&main);
        p.last_active_at_ms = now;
        store.upsert_project(&p)?;
        return Ok(p);
    }
    let name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
    let base = slug(&name);
    let mut id = base.clone();
    let mut n = 2;
    while store.project(&id)?.is_some() {
        id = format!("{base}-{n}");
        n += 1;
    }
    let p = Project {
        id,
        name,
        default_branch: detect_default_branch(&main),
        remotes: remotes(&main),
        worktree_root: worktree::worktree_root(&main),
        task_backend: serde_json::json!({"type": "local"}),
        track_orchestrator_dir: true,
        settings: serde_json::json!({}),
        created_at_ms: now,
        last_active_at_ms: now,
        last_fetch_at_ms: None,
        last_fetch_error: None,
        archived: false,
        path: main,
    };
    store.upsert_project(&p)?;
    Ok(p)
}

/// Clones `url` into `parent/<name>` (name from the URL unless given), then registers it.
pub fn clone(store: &Store, url: &str, parent: &Path, name: Option<&str>, deadline: Duration) -> Result<Project> {
    if url.starts_with('-') {
        bail!("invalid URL {url:?}");
    }
    let name = match name {
        Some(n) => n.to_string(),
        None => url
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .map(|s| s.trim_end_matches(".git").to_string())
            .filter(|s| !s.is_empty())
            .context("cannot name the clone from the URL")?,
    };
    if !crate::valid_id(&name.replace('.', "_")) {
        bail!("invalid folder name {name:?}");
    }
    let dest = parent.join(&name);
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    std::fs::create_dir_all(parent)?;
    git_deadline(parent, &["clone", "--quiet", "--", url, &dest.display().to_string()], deadline)?;
    register(store, &dest)
}

/// What J11 shows before removing a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemovalPlan {
    pub project: Project,
    /// The app's worktrees and orphans (main checkout and external worktrees excluded).
    pub worktrees: Vec<WorktreeInfo>,
    /// `orch/…` branches.
    pub branches: Vec<BranchInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum History {
    /// Settings, rules and the transcript index are deleted (events, usage and audit stay).
    Delete,
    /// The project is hidden and everything is kept.
    Archive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemovalChoices {
    /// Remove worktrees whose work is merged and clean (default true).
    pub remove_merged: bool,
    /// Worktrees the user explicitly confirmed to discard, unmerged work included.
    pub discard: Vec<PathBuf>,
    pub history: History,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemovalReport {
    pub removed: Vec<PathBuf>,
    /// Kept worktrees and why (unmerged work is never deleted silently).
    pub kept: Vec<(PathBuf, String)>,
    pub log: Vec<String>,
    pub history: Option<History>,
}

pub fn removal_plan(store: &Store, ledger: &dyn WorktreeLedger, project_id: &str) -> Result<RemovalPlan> {
    let project = store.project(project_id)?.with_context(|| format!("no project {project_id}"))?;
    let rows = ledger.rows_of(&project.path);
    let worktrees = cleanup::worktrees(&project.path, &rows, &project.default_branch)?
        .into_iter()
        .filter(|w| !matches!(w.verdict, WorktreeVerdict::Main | WorktreeVerdict::External))
        .collect();
    let branches = cleanup::branches(&project.path, &project.default_branch)?.into_iter().filter(|b| b.orch).collect();
    Ok(RemovalPlan { project, worktrees, branches })
}

/// Carries out J11 after the caller stopped the project's agents. The main
/// checkout and its `.orchestrator/` are never touched.
pub fn remove(
    store: &Store,
    ledger: &dyn WorktreeLedger,
    project_id: &str,
    choices: &RemovalChoices,
) -> Result<RemovalReport> {
    let plan = removal_plan(store, ledger, project_id)?;
    let repo = plan.project.path.clone();
    let mut report = RemovalReport::default();
    for w in &plan.worktrees {
        let confirmed = choices.discard.iter().any(|d| real(d) == w.path);
        let confirm = if confirmed { Confirm::DiscardUnmerged } else { Confirm::Safe };
        let remove = confirmed || (choices.remove_merged && w.safe_to_remove);
        if !remove {
            let why = if w.dirty > 0 {
                format!("{} uncommitted file(s)", w.dirty)
            } else if w.unmerged_commits > 0 {
                format!("{} unmerged commit(s)", w.unmerged_commits)
            } else if w.verdict == WorktreeVerdict::Missing {
                "missing on disk".into()
            } else {
                "kept".into()
            };
            report.kept.push((w.path.clone(), why));
            continue;
        }
        let base = w.base.clone();
        if w.verdict == WorktreeVerdict::Missing {
            report.removed.push(w.path.clone());
        } else {
            match cleanup::remove_worktree(&repo, &w.path, &base, true, confirm) {
                Ok(o) => {
                    report.log.extend(o.log);
                    report.removed.push(w.path.clone());
                }
                Err(e) => {
                    report.kept.push((w.path.clone(), format!("{e:#}")));
                    continue;
                }
            }
        }
        if let Some(mut row) = w.row.clone() {
            row.state = WorktreeRowState::Removed;
            ledger.put_row(row)?;
        }
    }
    match choices.history {
        History::Archive => {
            let mut p = plan.project.clone();
            p.archived = true;
            store.upsert_project(&p)?;
        }
        History::Delete => store.delete_project(&plan.project.id)?,
    }
    report.history = Some(choices.history);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{commit_file, git as tgit, scratch_repo};
    use orch_protocol::WorktreeKindSpec;

    #[test]
    fn register_is_idempotent_and_resolves_worktrees_to_main() {
        let (d, repo) = scratch_repo();
        let store = Store::open(&d.path().join("orch.db")).unwrap();
        tgit(&repo, &["remote", "add", "origin", "git@example.invalid:me/demo.git"]);
        let p = register(&store, &repo.join(".")).unwrap();
        assert_eq!((p.id.as_str(), p.default_branch.as_str()), ("demo", "main"));
        assert_eq!(p.remotes[0].url, "git@example.invalid:me/demo.git");
        assert_eq!(p.worktree_root, worktree::worktree_root(&repo));
        let row = worktree::ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: "x".into() }, None, None).unwrap();
        assert_eq!(register(&store, &row.path).unwrap().id, "demo", "a worktree registers its main checkout");
        assert_eq!(store.projects(true).unwrap().len(), 1);
        assert!(register(&store, d.path()).is_err());
    }

    #[test]
    fn clone_registers() {
        let (d, repo) = scratch_repo();
        let store = Store::open(&d.path().join("orch.db")).unwrap();
        let p = clone(&store, repo.to_str().unwrap(), &d.path().join("clones"), None, Duration::from_secs(30)).unwrap();
        assert_eq!(p.name, "demo");
        assert_eq!(p.remotes[0].name, "origin");
        assert!(clone(&store, "--upload-pack=x", d.path(), Some("y"), Duration::from_secs(5)).is_err());
    }

    #[test]
    fn j11_keeps_unmerged_work_unless_confirmed() {
        let (d, repo) = scratch_repo();
        let store = Store::open(&d.path().join("orch.db")).unwrap();
        let p = register(&store, &repo).unwrap();
        let mk =
            |s: &str| worktree::ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: s.into() }, None, None).unwrap();
        let merged = mk("merged");
        let work = mk("work");
        let wip = mk("wip");
        commit_file(&work.path, "w.txt", "w", "unmerged");
        commit_file(&wip.path, "v.txt", "v", "unmerged too");
        std::fs::create_dir_all(repo.join(".orchestrator")).unwrap();
        std::fs::write(repo.join(".orchestrator/spec.md"), "keep").unwrap();

        let plan = removal_plan(&store, &store, &p.id).unwrap();
        assert_eq!(plan.worktrees.len(), 3);
        assert_eq!(plan.branches.len(), 3);
        let r = remove(
            &store,
            &store,
            &p.id,
            &RemovalChoices { remove_merged: true, discard: vec![wip.path.clone()], history: History::Archive },
        )
        .unwrap();
        assert!(r.removed.contains(&merged.path) && r.removed.contains(&wip.path), "{r:?}");
        assert_eq!(r.kept.len(), 1);
        assert_eq!(r.kept[0].0, work.path);
        assert!(r.kept[0].1.contains("unmerged"));
        assert!(work.path.is_dir() && !merged.path.exists() && !wip.path.exists());
        assert!(repo.join(".orchestrator/spec.md").exists());
        assert!(store.project(&p.id).unwrap().unwrap().archived);
        assert_eq!(store.worktree(&merged.path).unwrap().unwrap().state, WorktreeRowState::Removed);
        // Delete: the project row goes, the kept worktree stays on disk.
        remove(
            &store,
            &store,
            &p.id,
            &RemovalChoices { remove_merged: true, discard: vec![], history: History::Delete },
        )
        .unwrap();
        assert!(store.project(&p.id).unwrap().is_none());
        assert!(work.path.is_dir());
    }
}
