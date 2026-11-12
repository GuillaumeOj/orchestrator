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
    try_ahead_behind(repo, branch, base).unwrap_or((0, 0))
}

/// Like [`ahead_behind`], `None` when a side doesn't resolve.
pub fn try_ahead_behind(repo: &Path, branch: &str, base: &str) -> Option<(u32, u32)> {
    let s = git(repo, &["rev-list", "--left-right", "--count", &format!("{branch}...{base}")]).ok()?;
    let mut it = s.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
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
    /// A worktree outside the app's root (the user's own, `outside`), folder
    /// present. Removable like the app's, but its unmerged branch is kept.
    External,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub locked: bool,
    /// Git's `locked <reason>`.
    pub lock_reason: Option<String>,
    /// Not under the app's worktree root (never the main checkout): the
    /// user's own, whose unmerged branch is never deleted.
    pub outside: bool,
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
        let outside = p != main && !p.starts_with(&root);
        out.push(describe(repo, w, p, verdict, outside, row, default_base));
    }
    for (p, r) in live_rows {
        if wanted(&p) && !git_paths.contains(&p) {
            out.push(WorktreeInfo {
                outside: !p.starts_with(&root),
                path: p,
                branch: Some(r.branch.clone()),
                head: None,
                locked: false,
                lock_reason: None,
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
    outside: bool,
    row: Option<WorktreeRow>,
    default_base: &str,
) -> WorktreeInfo {
    let base = row.as_ref().map(|r| r.base.clone()).unwrap_or_else(|| default_base.to_string());
    let insp = if path.is_dir() { worktree::inspect(&path) } else { Default::default() };
    let branch = w.branch.clone();
    // A detached HEAD's own commits count as unmerged work too.
    let (unmerged, merged) = match branch.as_ref().or(w.head.as_ref()) {
        Some(t) => (ahead_behind(repo, t, &base).0, is_merged(repo, t, &base)),
        None => (0, false),
    };
    let clean = insp.dirty == 0 && insp.in_progress.is_empty();
    let safe = if outside {
        // The user's: an unmerged branch is kept, so only another tool's
        // lock, files on disk or a detached HEAD's own commits would be lost.
        !w.foreign_lock() && clean && (branch.is_some() || merged)
    } else {
        matches!(verdict, WorktreeVerdict::Ok | WorktreeVerdict::Unknown | WorktreeVerdict::Prunable)
            && clean
            && (merged || branch.is_none() && verdict == WorktreeVerdict::Prunable)
    };
    WorktreeInfo {
        path,
        branch,
        head: w.head.clone(),
        locked: w.locked,
        lock_reason: w.lock_reason.clone(),
        outside,
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
    /// The branch was kept because it is not merged into the base (a
    /// worktree outside the app's root, or a prune).
    pub kept_branch: Option<String>,
    pub log: Vec<String>,
}

/// Removes one worktree (and optionally its branch). With `Confirm::Safe` it
/// refuses when the worktree has uncommitted changes, an operation in
/// progress, or commits that `base` does not have. The main checkout is
/// never removed.
///
/// A worktree outside the app's root (the user's own) keeps its branch
/// unless that branch is merged into `base`, even with
/// `Confirm::DiscardUnmerged`, so only a detached HEAD's commits count as
/// unmerged work; a locked one needs the confirmation. A folder that is not
/// a git worktree any more is deleted only inside the app's root.
pub fn remove_worktree(
    repo: &Path,
    path: &Path,
    base: &str,
    delete_branch: bool,
    confirm: Confirm,
) -> Result<RemoveOutcome> {
    remove_worktree_of(repo, path, base, None, delete_branch, confirm)
}

/// [`remove_worktree`] with the branch the app recorded for it: when git no
/// longer knows the worktree, that branch is the one deleted (never the base,
/// never one checked out in another worktree).
pub fn remove_worktree_of(
    repo: &Path,
    path: &Path,
    base: &str,
    row_branch: Option<&str>,
    delete_branch: bool,
    confirm: Confirm,
) -> Result<RemoveOutcome> {
    remove(repo, path, base, row_branch, delete_branch, confirm, false)
}

/// Clears a worktree whose folder is gone, wherever it is: git's entry goes
/// (lock or not: nothing on disk is left to lose), and its branch only when
/// merged into `base`. A detached HEAD with commits `base` lacks is refused.
pub fn prune_worktree(repo: &Path, path: &Path, base: &str, row_branch: Option<&str>) -> Result<RemoveOutcome> {
    // Only a folder that is really gone: one that can't be read (permissions,
    // a volume not mounted) is not pruned.
    match real(path).try_exists() {
        Ok(false) => {}
        Ok(true) => bail!("{}: its folder is still there", path.display()),
        Err(e) => bail!("{}: can't tell whether its folder is gone: {e}", path.display()),
    }
    remove(repo, path, base, row_branch, true, Confirm::Safe, true)
}

fn remove(
    repo: &Path,
    path: &Path,
    base: &str,
    row_branch: Option<&str>,
    delete_branch: bool,
    confirm: Confirm,
    prune: bool,
) -> Result<RemoveOutcome> {
    let p = real(path);
    let main = real(repo);
    if p == main {
        bail!("refusing to remove the main checkout");
    }
    let root = worktree::worktree_root(repo);
    let outside = !p.starts_with(&root);
    // The user's branch (outside), or any on a prune: kept unless merged.
    let keep_unmerged = outside || prune;
    let wts = worktree::list(repo)?;
    let git_wt = wts.iter().find(|w| real(&w.path) == p).cloned();
    let mut branch = git_wt.as_ref().and_then(|w| w.branch.clone());
    if git_wt.is_none()
        && let Some(b) = row_branch
        && b != base
        && worktree::branch_exists(repo, b)
        && !wts.iter().any(|w| w.branch.as_deref() == Some(b))
    {
        branch = Some(b.to_string());
    }
    let mut out = RemoveOutcome::default();
    if confirm == Confirm::Safe {
        if outside
            && !prune
            && let Some(w) = git_wt.as_ref().filter(|w| w.foreign_lock())
        {
            let why = w.lock_reason.as_ref().map(|r| format!(" ({r})")).unwrap_or_default();
            bail!("{} is locked{why}: needs confirmation", p.display());
        }
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
        // base that doesn't resolve counts as unmerged (never "0 ahead"). A
        // kept branch keeps its commits: only a detached HEAD is checked.
        let tip = match &branch {
            Some(_) if keep_unmerged => None,
            Some(b) => Some(b.clone()),
            None => git_wt.as_ref().and_then(|w| w.head.clone()),
        };
        if let Some(t) = &tip
            && !is_merged(repo, t, base)
        {
            let (ahead, _) = ahead_behind(repo, t, base);
            bail!("{t}: not merged into {base} ({ahead} commit(s) ahead; needs confirmation)");
        }
    }
    if let Some(w) = &git_wt {
        let ps = p.display().to_string();
        if w.locked {
            git(repo, &["worktree", "unlock", &ps])?;
            out.log.push("unlocked".into());
        }
        let mut args = vec!["worktree", "remove", ps.as_str()];
        if confirm == Confirm::DiscardUnmerged {
            args.insert(2, "--force");
        }
        if let Err(e) = git(repo, &args) {
            if p.is_dir() {
                // Not removed: give another tool its lock back.
                if w.locked {
                    let mut lock = vec!["worktree", "lock"];
                    if let Some(r) = &w.lock_reason {
                        lock.extend(["--reason", r.as_str()]);
                    }
                    lock.push(ps.as_str());
                    let _ = git(repo, &lock);
                }
                return Err(e);
            }
            // An older git that can't remove a worktree without its folder.
            git(repo, &["worktree", "prune"])?;
        }
        out.worktree_removed = true;
        out.log.push(format!("removed worktree {ps}"));
    } else if p.is_dir() && confirm == Confirm::DiscardUnmerged && !outside {
        // Not a git worktree any more: a stray folder in our root.
        std::fs::remove_dir_all(&p)?;
        out.worktree_removed = true;
        out.log.push(format!("removed stray folder {}", p.display()));
    }
    if delete_branch && let Some(b) = branch {
        if b == base {
            // Never the base itself (`-d` would delete it when HEAD has it).
            out.log.push(format!("kept branch {b}: it is the base"));
        } else if keep_unmerged && !is_merged(repo, &b, base) {
            out.log.push(format!("kept branch {b}: not merged into {base}"));
            out.kept_branch = Some(b);
        } else {
            // Merged into the base (checked above) or confirmed: `-D`, since
            // `-d` would check HEAD or the upstream, not the base.
            match git(repo, &["branch", "-D", &b]) {
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
    /// `git worktree add -b <branch> <path> main` outside the sibling folder.
    fn outside(repo: &Path, path: &Path, branch: &str) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        tgit(repo, &["worktree", "add", "-q", "-b", branch, path.to_str().unwrap(), "main"]);
        real(path)
    }

    fn info(repo: &Path, path: &Path) -> WorktreeInfo {
        let p = real(path);
        worktrees(repo, &[], "main").unwrap().into_iter().find(|i| i.path == p).unwrap()
    }

    fn listed(repo: &Path, path: &Path) -> bool {
        let p = real(path);
        worktrees(repo, &[], "main").unwrap().iter().any(|i| i.path == p)
    }

    #[test]
    fn outside_worktrees_are_removed_and_keep_their_unmerged_branch() {
        let (_d, repo) = scratch_repo();
        let elsewhere = repo.parent().unwrap().join("mine");
        // Clean and merged: safe; the merged branch goes.
        let merged = outside(&repo, &elsewhere.join("merged"), "merged");
        let i = info(&repo, &merged);
        assert_eq!((i.verdict, i.outside, i.safe_to_remove), (WorktreeVerdict::External, true, true), "{i:?}");
        assert_eq!(i.lock_reason, None);
        let o = remove_worktree(&repo, &merged, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && o.branch_deleted && o.kept_branch.is_none(), "{o:?}");
        assert!(!merged.exists() && !worktree::branch_exists(&repo, "merged"));
        assert!(!listed(&repo, &merged));
        // Unmerged: still safe, removed without discard, the branch is kept (even with discard).
        for (n, confirm) in [("work", Confirm::Safe), ("work2", Confirm::DiscardUnmerged)] {
            let work = outside(&repo, &elsewhere.join(n), n);
            commit_file(&work, "w.txt", "w", "unmerged work");
            let i = info(&repo, &work);
            assert!(i.safe_to_remove && i.unmerged_commits == 1, "{i:?}");
            let o = remove_worktree(&repo, &work, "main", true, confirm).unwrap();
            assert!(o.worktree_removed && !o.branch_deleted, "{o:?}");
            assert_eq!(o.kept_branch.as_deref(), Some(n));
            assert!(o.log.iter().any(|l| l == &format!("kept branch {n}: not merged into main")), "{o:?}");
            assert!(!work.exists() && worktree::branch_exists(&repo, n));
        }
        // Dirty: refused without discard, removed with it.
        let dirty = outside(&repo, &elsewhere.join("dirty"), "dirty");
        std::fs::write(dirty.join("d.txt"), "x").unwrap();
        assert!(!info(&repo, &dirty).safe_to_remove);
        let e = remove_worktree(&repo, &dirty, "main", true, Confirm::Safe).unwrap_err().to_string();
        assert!(e.contains("uncommitted"), "{e}");
        assert!(dirty.join("d.txt").exists());
        assert!(remove_worktree(&repo, &dirty, "main", true, Confirm::DiscardUnmerged).unwrap().worktree_removed);
        assert!(!dirty.exists());
        // Folder gone: prunable, outside, safe; removed and gone from the list.
        let gone = outside(&repo, &elsewhere.join("gone"), "gone");
        std::fs::remove_dir_all(&gone).unwrap();
        let i = info(&repo, &gone);
        assert_eq!((i.verdict, i.outside, i.safe_to_remove), (WorktreeVerdict::Prunable, true, true), "{i:?}");
        let o = remove_worktree(&repo, &gone, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && o.branch_deleted, "{o:?}");
        assert!(!listed(&repo, &gone));
        // Locked (another tool uses it): needs the confirmation, which unlocks and removes.
        let locked = outside(&repo, &elsewhere.join("locked"), "locked");
        tgit(&repo, &["worktree", "lock", "--reason", "claude agent", locked.to_str().unwrap()]);
        let i = info(&repo, &locked);
        assert!(i.locked && !i.safe_to_remove, "{i:?}");
        assert_eq!(i.lock_reason.as_deref(), Some("claude agent"));
        let e = remove_worktree(&repo, &locked, "main", true, Confirm::Safe).unwrap_err().to_string();
        assert_eq!(e, format!("{} is locked (claude agent): needs confirmation", locked.display()));
        assert!(locked.is_dir());
        let o = remove_worktree(&repo, &locked, "main", true, Confirm::DiscardUnmerged).unwrap();
        assert!(o.worktree_removed && o.log[0] == "unlocked", "{o:?}");
        assert!(!locked.exists());
        // A detached HEAD with its own commits needs the confirmation.
        let det = outside(&repo, &elsewhere.join("det"), "det");
        tgit(&det, &["checkout", "-q", "--detach"]);
        commit_file(&det, "d.txt", "d", "detached work");
        let i = info(&repo, &det);
        assert!(!i.safe_to_remove && i.unmerged_commits == 1 && !i.merged, "{i:?}");
        let e = remove_worktree(&repo, &det, "main", true, Confirm::Safe).unwrap_err().to_string();
        assert!(e.contains("not merged"), "{e}");
        assert!(det.is_dir());
        // Never the base branch, merged or not.
        tgit(&repo, &["checkout", "-q", "-b", "other"]);
        let on_main = real(&elsewhere.join("on-main"));
        tgit(&repo, &["worktree", "add", "-q", on_main.to_str().unwrap(), "main"]);
        let o = remove_worktree(&repo, &on_main, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && !o.branch_deleted && o.kept_branch.is_none(), "{o:?}");
        assert!(worktree::branch_exists(&repo, "main"));
        // Nor from the sibling folder (`-d` would delete it: HEAD contains it).
        let sib = worktree::worktree_root(&repo).join("solo/on-main");
        std::fs::create_dir_all(sib.parent().unwrap()).unwrap();
        tgit(&repo, &["worktree", "add", "-q", sib.to_str().unwrap(), "main"]);
        let o = remove_worktree(&repo, &sib, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && !o.branch_deleted, "{o:?}");
        assert!(worktree::branch_exists(&repo, "main"));
    }

    #[test]
    fn a_nested_worktree_is_outside_and_a_plain_folder_is_never_deleted() {
        let (_d, repo) = scratch_repo();
        // Claude Code's `<repo>/.claude/worktrees/x`, inside the main checkout.
        let nested = outside(&repo, &repo.join(".claude/worktrees/x"), "claude/x");
        let i = info(&repo, &nested);
        assert_eq!((i.verdict, i.outside, i.safe_to_remove), (WorktreeVerdict::External, true, true), "{i:?}");
        assert!(!info(&repo, &repo).outside, "the main checkout is not outside");
        let o = remove_worktree(&repo, &nested, "main", true, Confirm::Safe).unwrap();
        assert!(o.worktree_removed && o.branch_deleted, "{o:?}");
        assert!(!nested.exists() && repo.join("README.md").exists());
        let e = remove_worktree(&repo, &repo, "main", true, Confirm::DiscardUnmerged).unwrap_err();
        assert_eq!(e.to_string(), "refusing to remove the main checkout");
        // A folder that is not a worktree of the repo stays, even confirmed.
        let plain = repo.parent().unwrap().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("keep.txt"), "x").unwrap();
        let o = remove_worktree(&repo, &plain, "main", true, Confirm::DiscardUnmerged).unwrap();
        assert!(!o.worktree_removed, "{o:?}");
        assert!(plain.join("keep.txt").exists());
    }

    #[test]
    fn prune_clears_gone_worktrees_and_keeps_unmerged_branches() {
        let (_d, repo) = scratch_repo();
        let store = MemLedger::default();
        // The app's own, unmerged, folder gone: git's entry goes, the branch stays.
        let work = solo(&store, &repo, "work");
        commit_file(&work.path, "w.txt", "w", "unmerged work");
        std::fs::remove_dir_all(&work.path).unwrap();
        let o = prune_worktree(&repo, &work.path, "main", Some(&work.branch)).unwrap();
        assert!(o.worktree_removed && !o.branch_deleted, "{o:?}");
        assert_eq!(o.kept_branch.as_deref(), Some("orch/solo/work"));
        assert!(!listed(&repo, &work.path));
        // Locked and outside, folder gone: no lock check (nothing left on disk), merged branch goes.
        let gone = outside(&repo, &repo.parent().unwrap().join("mine/gone"), "gone");
        tgit(&repo, &["worktree", "lock", "--reason", "claude agent", gone.to_str().unwrap()]);
        std::fs::remove_dir_all(&gone).unwrap();
        let o = prune_worktree(&repo, &gone, "main", None).unwrap();
        assert!(o.worktree_removed && o.branch_deleted, "{o:?}");
        assert!(!listed(&repo, &gone));
        // A folder still there is not pruned.
        let here = solo(&store, &repo, "here");
        assert!(prune_worktree(&repo, &here.path, "main", None).unwrap_err().to_string().contains("still there"));
        assert!(here.path.is_dir());
    }
}
