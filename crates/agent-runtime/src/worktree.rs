//! Worktree service (FR-105, ADR-003): a sibling folder `<parent>/<repo>.worktrees/`,
//! relative paths, `git worktree lock` while in use, and an idempotent
//! creation = add + lock + DB row.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use orch_protocol::WorktreeKindSpec;

use crate::paths::{real, valid_id};
use crate::store::{Store, WorktreeRow, WorktreeRowState};

/// Runs git with no prompts and returns stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("git {args:?}"))?;
    if !out.status.success() {
        bail!("git {} in {}: {}", args.join(" "), dir.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `<parent>/<repo>.worktrees`.
pub fn worktree_root(repo: &Path) -> PathBuf {
    let repo = real(repo);
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    repo.parent().unwrap_or(Path::new("/")).join(format!("{name}.worktrees"))
}

/// Path and branch for a worktree kind (ADR-003 naming: no ref is both a file and a folder).
pub fn layout(repo: &Path, kind: &WorktreeKindSpec) -> Result<(PathBuf, String)> {
    let root = worktree_root(repo);
    let check = |s: &str| if valid_id(s) { Ok(()) } else { bail!("invalid name {s:?}") };
    Ok(match kind {
        WorktreeKindSpec::Solo { slug } => {
            check(slug)?;
            (root.join("solo").join(slug), format!("orch/solo/{slug}"))
        }
        WorktreeKindSpec::Integration { feature } => {
            check(feature)?;
            if feature == "solo" {
                bail!("\"solo\" is reserved");
            }
            (root.join(feature).join("integration"), format!("orch/{feature}/integration"))
        }
        WorktreeKindSpec::Team { feature, role, n } => {
            check(feature)?;
            check(role)?;
            if feature == "solo" || role == "integration" {
                bail!("reserved name");
            }
            (root.join(feature).join(format!("{role}-{n}")), format!("orch/{feature}/{role}-{n}"))
        }
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitWorktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub locked: bool,
    pub prunable: bool,
    pub bare: bool,
}

/// `git worktree list --porcelain`.
pub fn list(repo: &Path) -> Result<Vec<GitWorktree>> {
    Ok(parse_porcelain(&git(repo, &["worktree", "list", "--porcelain"])?))
}

pub fn parse_porcelain(out: &str) -> Vec<GitWorktree> {
    let mut res: Vec<GitWorktree> = vec![];
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            res.push(GitWorktree { path: PathBuf::from(p), ..Default::default() });
            continue;
        }
        let Some(cur) = res.last_mut() else { continue };
        let (k, v) = line.split_once(' ').unwrap_or((line, ""));
        match k {
            "HEAD" => cur.head = Some(v.into()),
            "branch" => cur.branch = Some(v.trim_start_matches("refs/heads/").into()),
            "locked" => cur.locked = true,
            "prunable" => cur.prunable = true,
            "bare" => cur.bare = true,
            _ => {}
        }
    }
    res
}

/// `git rev-parse --git-common-dir`, absolute.
pub fn common_dir(path: &Path) -> Result<PathBuf> {
    let out = git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Ok(PathBuf::from(out.trim()))
}

pub fn toplevel(path: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(git(path, &["rev-parse", "--show-toplevel"])?.trim()))
}

pub fn default_branch(repo: &Path) -> Result<String> {
    Ok(git(repo, &["symbolic-ref", "--short", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "main".into()))
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok()
}

/// Creates (or finishes creating) a worktree. Each step is idempotent:
/// 1. DB row `creating`; 2. `git worktree add` unless git already has it;
/// 3. `git worktree lock` unless locked; 4. row `ready`.
pub fn ensure(
    store: &Store,
    repo: &Path,
    kind: &WorktreeKindSpec,
    base: Option<&str>,
    agent_id: Option<&str>,
) -> Result<WorktreeRow> {
    let repo = real(repo);
    let (path, branch) = layout(&repo, kind)?;
    let base = match base {
        Some(b) => b.to_string(),
        None => default_branch(&repo)?,
    };
    if let Some(row) = store.read(|s| s.worktrees.get(&path).cloned()) {
        if row.state == WorktreeRowState::Ready && path.is_dir() {
            lock(&repo, &path, row.agent_id.as_deref())?;
            return Ok(row);
        }
    }
    let row = WorktreeRow {
        path: path.clone(),
        repo: repo.clone(),
        branch,
        base,
        agent_id: agent_id.map(str::to_string),
        state: WorktreeRowState::Creating,
        created_at_ms: orch_protocol::now_ms(),
    };
    store.write(|s| s.worktrees.insert(path.clone(), row.clone()))?;
    complete(store, &row)
}

/// Steps 2–4 for a row in `creating` (also used by the reconciler).
pub fn complete(store: &Store, row: &WorktreeRow) -> Result<WorktreeRow> {
    let repo = &row.repo;
    // Relative links survive a joint move of the repo and its sibling folder.
    git(repo, &["config", "worktree.useRelativePaths", "true"])?;
    let known = list(repo)?.into_iter().any(|w| real(&w.path) == real(&row.path));
    if !known {
        if let Some(parent) = row.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let p = row.path.display().to_string();
        if branch_exists(repo, &row.branch) {
            git(repo, &["-c", "worktree.useRelativePaths=true", "worktree", "add", "-q", &p, &row.branch])?;
        } else {
            git(
                repo,
                &["-c", "worktree.useRelativePaths=true", "worktree", "add", "-q", "-b", &row.branch, &p, &row.base],
            )?;
        }
    }
    lock(repo, &row.path, row.agent_id.as_deref())?;
    let mut done = row.clone();
    done.state = WorktreeRowState::Ready;
    store.write(|s| s.worktrees.insert(done.path.clone(), done.clone()))?;
    Ok(done)
}

/// `git worktree lock`; already locked is fine.
pub fn lock(repo: &Path, path: &Path, agent_id: Option<&str>) -> Result<()> {
    let locked = list(repo)?.into_iter().any(|w| real(&w.path) == real(path) && w.locked);
    if !locked {
        let reason = format!("orchestrator agent {}", agent_id.unwrap_or("-"));
        git(repo, &["worktree", "lock", "--reason", &reason, &path.display().to_string()])?;
    }
    Ok(())
}

/// Every worktree of the repo except the main checkout and `except`.
pub fn others(repo: &Path, except: &Path) -> Vec<PathBuf> {
    let main = real(repo);
    let except = real(except);
    list(repo).unwrap_or_default().into_iter().map(|w| real(&w.path)).filter(|p| *p != main && *p != except).collect()
}

/// Half-done git operations and locks in one worktree (J10 steps 2.3–2.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspection {
    pub exists: bool,
    pub gitdir: Option<PathBuf>,
    /// `MERGE_HEAD`, `REBASE_HEAD`, `rebase-merge`, `rebase-apply`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`.
    pub in_progress: Vec<String>,
    pub index_lock: bool,
    pub unmerged: usize,
    pub dirty: usize,
    pub error: Option<String>,
}

pub const IN_PROGRESS_MARKERS: &[&str] =
    &["MERGE_HEAD", "REBASE_HEAD", "rebase-merge", "rebase-apply", "CHERRY_PICK_HEAD", "REVERT_HEAD"];

pub fn inspect(path: &Path) -> Inspection {
    let mut i = Inspection { exists: path.is_dir(), ..Default::default() };
    if !i.exists {
        return i;
    }
    let gd = match git(path, &["rev-parse", "--absolute-git-dir"]) {
        Ok(s) => PathBuf::from(s.trim()),
        Err(e) => {
            i.error = Some(e.to_string());
            return i;
        }
    };
    // The `locked` marker file lives in the gitdir too: it is not "in progress".
    i.in_progress = IN_PROGRESS_MARKERS.iter().filter(|m| gd.join(m).exists()).map(|m| m.to_string()).collect();
    i.index_lock = gd.join("index.lock").exists();
    i.gitdir = Some(gd);
    if let Ok(u) = git(path, &["ls-files", "-u"]) {
        let mut paths: Vec<&str> = u.lines().filter_map(|l| l.split('\t').nth(1)).collect();
        paths.sort();
        paths.dedup();
        i.unmerged = paths.len();
    }
    if let Ok(s) = git(path, &["status", "--porcelain"]) {
        i.dirty = s.lines().count();
    }
    i
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch repo `<tmp>/proj/demo` with one commit on `main`.
    pub fn scratch_repo() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("proj/demo");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.name", "T"]);
        run(&["config", "user.email", "t@example.invalid"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        (d, real(&repo))
    }

    #[test]
    fn naming() {
        let r = Path::new("/x/proj/demo");
        let (p, b) = layout(r, &WorktreeKindSpec::Solo { slug: "fix".into() }).unwrap();
        assert_eq!((p, b.as_str()), (PathBuf::from("/x/proj/demo.worktrees/solo/fix"), "orch/solo/fix"));
        let (p, b) = layout(r, &WorktreeKindSpec::Integration { feature: "checkout".into() }).unwrap();
        assert_eq!(
            (p, b.as_str()),
            (PathBuf::from("/x/proj/demo.worktrees/checkout/integration"), "orch/checkout/integration")
        );
        let (p, b) =
            layout(r, &WorktreeKindSpec::Team { feature: "checkout".into(), role: "backend".into(), n: 2 }).unwrap();
        assert_eq!(
            (p, b.as_str()),
            (PathBuf::from("/x/proj/demo.worktrees/checkout/backend-2"), "orch/checkout/backend-2")
        );
        assert!(layout(r, &WorktreeKindSpec::Solo { slug: "../x".into() }).is_err());
        assert!(layout(r, &WorktreeKindSpec::Integration { feature: "solo".into() }).is_err());
    }

    #[test]
    fn porcelain() {
        let w = parse_porcelain(
            "worktree /r\nHEAD abc\nbranch refs/heads/main\n\nworktree /r.worktrees/solo/x\nHEAD def\nbranch refs/heads/orch/solo/x\nlocked orch\n\nworktree /gone\nHEAD 1\ndetached\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].branch.as_deref(), Some("orch/solo/x"));
        assert!(w[1].locked && !w[0].locked);
        assert!(w[2].prunable);
    }

    #[test]
    fn create_is_idempotent_relative_and_locked() {
        let (_d, repo) = scratch_repo();
        let store = Store::in_memory(Default::default());
        let kind = WorktreeKindSpec::Solo { slug: "fix".into() };
        let row = ensure(&store, &repo, &kind, None, Some("a1")).unwrap();
        assert_eq!(row.state, WorktreeRowState::Ready);
        assert_eq!(row.path, worktree_root(&repo).join("solo/fix"));
        assert_eq!(row.base, "main");
        let gitfile = std::fs::read_to_string(row.path.join(".git")).unwrap();
        assert!(gitfile.contains("gitdir: ../"), "relative link: {gitfile}");
        let wts = list(&repo).unwrap();
        let w = wts.iter().find(|w| real(&w.path) == row.path).unwrap();
        assert!(w.locked);
        assert_eq!(w.branch.as_deref(), Some("orch/solo/fix"));
        // Again: no error, same row.
        assert_eq!(ensure(&store, &repo, &kind, None, Some("a1")).unwrap().path, row.path);
        // A crash after `git worktree add` (row still `creating`, no lock) is finished.
        let k2 = WorktreeKindSpec::Solo { slug: "half".into() };
        let (p2, b2) = layout(&repo, &k2).unwrap();
        git(&repo, &["worktree", "add", "-q", "-b", &b2, &p2.display().to_string(), "main"]).unwrap();
        let half = WorktreeRow {
            path: p2.clone(),
            repo: repo.clone(),
            branch: b2,
            base: "main".into(),
            agent_id: None,
            state: WorktreeRowState::Creating,
            created_at_ms: 0,
        };
        store.write(|s| s.worktrees.insert(p2.clone(), half.clone())).unwrap();
        let done = complete(&store, &half).unwrap();
        assert_eq!(done.state, WorktreeRowState::Ready);
        assert!(list(&repo).unwrap().iter().any(|w| real(&w.path) == p2 && w.locked));
        assert_eq!(others(&repo, &row.path), vec![p2]);
        assert_eq!(common_dir(&row.path).unwrap(), repo.join(".git"));
    }

    #[test]
    fn inspection_finds_merges_and_index_lock() {
        let (_d, repo) = scratch_repo();
        let store = Store::in_memory(Default::default());
        let row = ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: "m".into() }, None, None).unwrap();
        let i = inspect(&row.path);
        assert!(i.exists && i.in_progress.is_empty() && !i.index_lock, "{i:?}");
        let gd = i.gitdir.unwrap();
        std::fs::write(gd.join("MERGE_HEAD"), "0000\n").unwrap();
        std::fs::write(gd.join("index.lock"), "").unwrap();
        std::fs::write(row.path.join("new.txt"), "x").unwrap();
        let i = inspect(&row.path);
        assert_eq!(i.in_progress, ["MERGE_HEAD"]);
        assert!(i.index_lock);
        assert_eq!(i.dirty, 1);
        assert!(!inspect(Path::new("/nonexistent/wt")).exists);
    }
}
