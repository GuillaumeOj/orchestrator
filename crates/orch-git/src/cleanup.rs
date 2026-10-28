//! Branches and worktrees, orphans included, and safe cleanup (FR-104, J11).
//! Unmerged or uncommitted work is never deleted without an explicit
//! [`Confirm::DiscardUnmerged`].

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::git::{git, git_ok, real};
use crate::worktree::{self, WorktreeRow, WorktreeRowState};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub head: String,
    pub upstream: Option<String>,
    /// Every commit of the branch is reachable from the base.
    pub merged: bool,
    /// Commits on the branch that the base does not have.
    pub ahead_of_base: u32,
    pub behind_base: u32,
    /// The worktree that has it checked out (main checkout included).
    pub worktree: Option<PathBuf>,
    /// An `orch/…` branch created by the app.
    pub orch: bool,
    pub last_commit_s: i64,
}

/// Local branches compared with `base`.
pub fn branches(repo: &Path, base: &str) -> Result<Vec<BranchInfo>> {
    let wts = worktree::list(repo)?;
    let out = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)%1f%(objectname)%1f%(upstream:short)%1f%(committerdate:unix)",
            "refs/heads",
        ],
    )?;
    let mut v = vec![];
    for line in out.lines() {
        let f: Vec<&str> = line.split('\x1f').collect();
        if f.len() < 4 {
            continue;
        }
        let name = f[0].to_string();
        let (ahead, behind) = ahead_behind(repo, &name, base);
        v.push(BranchInfo {
            merged: is_merged(repo, &name, base),
            head: f[1].into(),
            upstream: Some(f[2].to_string()).filter(|s| !s.is_empty()),
            ahead_of_base: ahead,
            behind_base: behind,
            worktree: wts.iter().find(|w| w.branch.as_deref() == Some(name.as_str())).map(|w| real(&w.path)),
            orch: name.starts_with("orch/"),
            last_commit_s: f[3].parse().unwrap_or(0),
            name,
        });
    }
    Ok(v)
}

/// Every commit of `tip` is reachable from `base` (false when either doesn't resolve).
pub fn is_merged(repo: &Path, tip: &str, base: &str) -> bool {
    git_ok(repo, &["merge-base", "--is-ancestor", tip, base])
}

/// (commits on `branch` not in `base`, commits on `base` not in `branch`).
pub fn ahead_behind(repo: &Path, branch: &str, base: &str) -> (u32, u32) {
    git(repo, &["rev-list", "--left-right", "--count", &format!("{branch}...{base}")])
        .ok()
        .and_then(|s| {
            let mut it = s.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .unwrap_or((0, 0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeVerdict {
    /// Known to git and to the app, folder present.
    Ok,
    /// The app's row has no git worktree or no folder (moved or removed outside the app).
    Missing,
    /// In the app's worktree root, unknown to the app.
    Unknown,
    /// Git still lists it but its folder is gone.
    Prunable,
    /// The main checkout.
    Main,
    /// A worktree outside the app's root (the user's own).
    External,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub locked: bool,
    pub verdict: WorktreeVerdict,
    pub row: Option<WorktreeRow>,
    pub base: String,
    /// Uncommitted + untracked files.
    pub dirty: usize,
    /// Commits not in the base (unmerged work).
    pub unmerged_commits: u32,
    pub merged: bool,
    pub in_progress: Vec<String>,
    /// True when [`remove_worktree`] would succeed without confirmation.
    pub safe_to_remove: bool,
}

/// Every worktree git knows plus every app row, with orphans flagged.
pub fn worktrees(repo: &Path, rows: &[WorktreeRow], default_base: &str) -> Result<Vec<WorktreeInfo>> {
    collect(repo, rows, default_base, None)
}

/// [`worktrees`] for the one at `path` (canonical): git is asked about it alone.
pub fn worktree(repo: &Path, path: &Path, rows: &[WorktreeRow], default_base: &str) -> Result<Option<WorktreeInfo>> {
    Ok(collect(repo, rows, default_base, Some(path))?.pop())
}

fn collect(repo: &Path, rows: &[WorktreeRow], default_base: &str, only: Option<&Path>) -> Result<Vec<WorktreeInfo>> {
    let wanted = |p: &Path| only.is_none_or(|o| o == p);
    let main = real(repo);
    let root = worktree::worktree_root(repo);
    let git_wts = worktree::list(repo)?;
    // Canonicalized once (not per pair of worktree and row).
    let live_rows: Vec<(PathBuf, &WorktreeRow)> =
        rows.iter().filter(|r| r.state != WorktreeRowState::Removed).map(|r| (real(&r.path), r)).collect();
    let git_paths: Vec<PathBuf> = git_wts.iter().map(|w| real(&w.path)).collect();
    let mut out = vec![];
    for (w, p) in git_wts.iter().zip(git_paths.iter().cloned()) {
        if !wanted(&p) {
            continue;
        }
        let row = live_rows.iter().find(|(rp, _)| *rp == p).map(|(_, r)| (*r).clone());
        let verdict = if p == main {
            WorktreeVerdict::Main
        } else if w.prunable || !p.is_dir() {
            WorktreeVerdict::Prunable
        } else if row.is_some() {
            WorktreeVerdict::Ok
        } else if p.starts_with(&root) {
            WorktreeVerdict::Unknown
        } else {
            WorktreeVerdict::External
        };
        out.push(describe(repo, w, p, verdict, row, default_base));
    }
    for (p, r) in live_rows {
        if wanted(&p) && !git_paths.contains(&p) {
            out.push(WorktreeInfo {
                path: p,
                branch: Some(r.branch.clone()),
                head: None,
                locked: false,
                verdict: WorktreeVerdict::Missing,
                row: Some(r.clone()),
                base: r.base.clone(),
                dirty: 0,
                unmerged_commits: ahead_behind(repo, &r.branch, &r.base).0,
                merged: worktree::branch_exists(repo, &r.branch) && is_merged(repo, &r.branch, &r.base),
                in_progress: vec![],
                safe_to_remove: false,
            });
        }
    }
    Ok(out)
}

fn describe(
    repo: &Path,
    w: &worktree::GitWorktree,
    path: PathBuf,
    verdict: WorktreeVerdict,
    row: Option<WorktreeRow>,
    default_base: &str,
) -> WorktreeInfo {
    let base = row.as_ref().map(|r| r.base.clone()).unwrap_or_else(|| default_base.to_string());
    let insp = if path.is_dir() { worktree::inspect(&path) } else { Default::default() };
    let branch = w.branch.clone();
    let (unmerged, merged) = match &branch {
        Some(b) => (ahead_behind(repo, b, &base).0, is_merged(repo, b, &base)),
        None => (0, false),
    };
    let safe = matches!(verdict, WorktreeVerdict::Ok | WorktreeVerdict::Unknown | WorktreeVerdict::Prunable)
        && insp.dirty == 0
        && insp.in_progress.is_empty()
        && (merged || branch.is_none() && verdict == WorktreeVerdict::Prunable);
    WorktreeInfo {
        path,
        branch,
        head: w.head.clone(),
        locked: w.locked,
        verdict,
        row,
        base,
        dirty: insp.dirty,
        unmerged_commits: unmerged,
        merged,
        in_progress: insp.in_progress,
        safe_to_remove: safe,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirm {
    /// Refuse when anything would be lost.
    Safe,
    /// The user confirmed: remove even with uncommitted or unmerged work.
    DiscardUnmerged,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoveOutcome {
    pub worktree_removed: bool,
    pub branch_deleted: bool,
    pub log: Vec<String>,
}

/// Removes one worktree (and optionally its branch). With `Confirm::Safe` it
/// refuses when the worktree has uncommitted changes, an operation in
/// progress, or commits that `base` does not have. The main checkout is
/// never removed.
pub fn remove_worktree(
    repo: &Path,
    path: &Path,
    base: &str,
    delete_branch: bool,
    confirm: Confirm,
) -> Result<RemoveOutcome> {
    let p = real(path);
    if p == real(repo) {
        bail!("refusing to remove the main checkout");
    }
    let git_wt = worktree::list(repo)?.into_iter().find(|w| real(&w.path) == p);
    let branch = git_wt.as_ref().and_then(|w| w.branch.clone());
    let mut out = RemoveOutcome::default();
    if confirm == Confirm::Safe {
        if p.is_dir() {
            let i = worktree::inspect(&p);
            if !i.in_progress.is_empty() {
                bail!("{}: {} in progress (needs confirmation)", p.display(), i.in_progress.join("/"));
            }
            if i.dirty > 0 {
                bail!("{}: {} uncommitted file(s) (needs confirmation)", p.display(), i.dirty);
            }
        }
        // The branch, or a detached HEAD, must be reachable from the base. A
        // base that doesn't resolve counts as unmerged (never "0 ahead").
        let tip = branch.clone().or_else(|| git_wt.as_ref().and_then(|w| w.head.clone()));
        if let Some(t) = &tip {
            if !is_merged(repo, t, base) {
                let (ahead, _) = ahead_behind(repo, t, base);
                bail!("{t}: not merged into {base} ({ahead} commit(s) ahead; needs confirmation)");
            }
        }
    }
    if let Some(w) = &git_wt {
        let ps = p.display().to_string();
        if w.locked {
            git(repo, &["worktree", "unlock", &ps])?;
            out.log.push("unlocked".into());
        }
        if p.is_dir() {
            let mut args = vec!["worktree", "remove", ps.as_str()];
            if confirm == Confirm::DiscardUnmerged {
                args.insert(2, "--force");
            }
            git(repo, &args)?;
        } else {
            git(repo, &["worktree", "prune"])?;
        }
        out.worktree_removed = true;
        out.log.push(format!("removed worktree {ps}"));
    } else if p.is_dir() && confirm == Confirm::DiscardUnmerged {
        // Not a git worktree any more: a stray folder in our root.
        std::fs::remove_dir_all(&p)?;
        out.worktree_removed = true;
        out.log.push(format!("removed stray folder {}", p.display()));
    }
    if delete_branch {
        if let Some(b) = branch {
            let flag = if confirm == Confirm::DiscardUnmerged { "-D" } else { "-d" };
            match git(repo, &["branch", flag, &b]) {
                Ok(_) => {
                    out.branch_deleted = true;
                    out.log.push(format!("deleted branch {b}"));
                }
                Err(e) => out.log.push(format!("kept branch {b}: {e}")),
            }
        }
    }
    Ok(out)
}

/// Deletes a branch with no worktree. Unmerged branches need confirmation.
pub fn delete_branch(repo: &Path, branch: &str, base: &str, confirm: Confirm) -> Result<()> {
    if worktree::list(repo)?.iter().any(|w| w.branch.as_deref() == Some(branch)) {
        bail!("{branch} is checked out in a worktree: remove the worktree first");
    }
    if confirm == Confirm::Safe && !is_merged(repo, branch, base) {
        bail!("{branch} is not merged into {base} (needs confirmation)");
    }
    // `-d` would check against HEAD/upstream; the check against `base` is done above.
    git(repo, &["branch", "-D", branch])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{MemLedger, commit_file, git as tgit, scratch_repo};
    use orch_protocol::WorktreeKindSpec;

    fn solo(store: &MemLedger, repo: &Path, slug: &str) -> WorktreeRow {
        worktree::ensure(store, repo, &WorktreeKindSpec::Solo { slug: slug.into() }, None, Some(slug)).unwrap()
    }

    #[test]
    fn lists_orphans_and_cleans_up_safely() {
        let (_d, repo) = scratch_repo();
        let store = MemLedger::default();
        let merged = solo(&store, &repo, "merged");
        let work = solo(&store, &repo, "work");
        let dirty = solo(&store, &repo, "dirty");
        commit_file(&work.path, "w.txt", "w", "unmerged work");
        std::fs::write(dirty.path.join("d.txt"), "x").unwrap();
        // A worktree in our root the app doesn't know, and a row whose worktree is gone.
        let (stray, sb) = worktree::layout(&repo, &WorktreeKindSpec::Solo { slug: "stray".into() }).unwrap();
        tgit(&repo, &["worktree", "add", "-q", "-b", &sb, stray.to_str().unwrap(), "main"]);
        let ghost = WorktreeRow {
            path: worktree::worktree_root(&repo).join("solo/ghost"),
            repo: repo.clone(),
            branch: "orch/solo/ghost".into(),
            base: "main".into(),
            agent_id: None,
            state: WorktreeRowState::Ready,
            created_at_ms: 0,
        };
        let rows: Vec<WorktreeRow> =
            store.0.lock().unwrap().values().cloned().chain(std::iter::once(ghost.clone())).collect();

        let infos = worktrees(&repo, &rows, "main").unwrap();
        let get = |p: &Path| infos.iter().find(|i| i.path == real(p)).unwrap().clone();
        assert_eq!(get(&repo).verdict, WorktreeVerdict::Main);
        assert_eq!(get(&merged.path).verdict, WorktreeVerdict::Ok);
        assert!(get(&merged.path).safe_to_remove);
        assert_eq!(get(&work.path).unmerged_commits, 1);
        assert!(!get(&work.path).safe_to_remove);
        assert_eq!(get(&dirty.path).dirty, 1);
        assert!(!get(&dirty.path).safe_to_remove);
        assert_eq!(get(&stray).verdict, WorktreeVerdict::Unknown);
        assert_eq!(infos.iter().find(|i| i.path == ghost.path).unwrap().verdict, WorktreeVerdict::Missing);

        let br = branches(&repo, "main").unwrap();
        let b = |n: &str| br.iter().find(|b| b.name == n).unwrap().clone();
        assert!(b("orch/solo/merged").merged && b("orch/solo/merged").orch);
        assert_eq!(b("orch/solo/work").ahead_of_base, 1);
        assert_eq!(b("orch/solo/work").worktree, Some(real(&work.path)));

        // Unmerged and dirty work is refused without confirmation.
        assert!(
            remove_worktree(&repo, &work.path, "main", true, Confirm::Safe)
                .unwrap_err()
                .to_string()
                .contains("not merged")
        );
        assert!(
            remove_worktree(&repo, &dirty.path, "main", true, Confirm::Safe)
                .unwrap_err()
                .to_string()
                .contains("uncommitted")
        );
        assert!(work.path.is_dir() && dirty.path.is_dir());
        assert!(remove_worktree(&repo, &repo, "main", false, Confirm::DiscardUnmerged).is_err());
        // Merged and locked: unlocked, removed, branch deleted.
        let o = remove_worktree(&repo, &merged.path, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && o.branch_deleted, "{o:?}");
        assert!(!merged.path.exists());
        // Confirmed: unmerged work goes.
        let o = remove_worktree(&repo, &work.path, "main", true, Confirm::DiscardUnmerged).unwrap();
        assert!(o.worktree_removed && o.branch_deleted);
        assert!(!worktree::branch_exists(&repo, "orch/solo/work"));
        // Branch without a worktree.
        tgit(&repo, &["branch", "orch/solo/lonely"]);
        delete_branch(&repo, "orch/solo/lonely", "main", Confirm::Safe).unwrap();
        tgit(&repo, &["branch", "orch/solo/lonely2"]);
        let ext = repo.parent().unwrap().join("ext-wt");
        tgit(&repo, &["worktree", "add", "-q", ext.to_str().unwrap(), "orch/solo/lonely2"]);
        assert!(delete_branch(&repo, "orch/solo/lonely2", "main", Confirm::Safe).is_err());
        tgit(&repo, &["worktree", "remove", "--force", ext.to_str().unwrap()]);
        // A detached HEAD with its own commits, and a base that doesn't exist, need confirmation.
        let det = solo(&store, &repo, "det");
        tgit(&det.path, &["checkout", "-q", "--detach"]);
        commit_file(&det.path, "d.txt", "d", "detached work");
        assert!(
            remove_worktree(&repo, &det.path, "main", false, Confirm::Safe)
                .unwrap_err()
                .to_string()
                .contains("not merged")
        );
        let clean = solo(&store, &repo, "clean");
        assert!(remove_worktree(&repo, &clean.path, "no-such-base", true, Confirm::Safe).is_err());
        assert!(det.path.is_dir() && clean.path.is_dir());
        // A prunable entry (folder deleted by hand) is pruned.
        std::fs::remove_dir_all(&stray).unwrap();
        let infos = worktrees(&repo, &[], "main").unwrap();
        assert!(infos.iter().any(|i| i.verdict == WorktreeVerdict::Prunable));
        remove_worktree(&repo, &stray, "main", false, Confirm::Safe).unwrap();
        assert!(!worktree::list(&repo).unwrap().iter().any(|w| real(&w.path) == real(&stray)));
    }
}
