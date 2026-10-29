//! Commits on an agent's behalf (SPEC FR-702 known gap, R10, ADR-009 §1):
//! sandboxed Bash can't write the repository's git dir from a linked worktree
//! under Claude Code 2.1.302, so the core runs the commit outside the sandbox,
//! in that agent's worktree only. Push and PRs stay gated (not here).
//!
//! Because this runs outside the sandbox, nothing the agent wrote may execute:
//! - git hooks are off (`core.hooksPath=/dev/null`, `--no-verify`): a hook
//!   script (husky…) is a tracked file the agent can edit;
//! - the identity comes from the agent's generated gitconfig (no signing, no
//!   credential helper); system config is ignored;
//! - pathspecs are literal and must stay inside the worktree.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::git::real;
use crate::worktree;

pub const MAX_MESSAGE: usize = 10_000;
pub const MAX_PATHS: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitRequest {
    pub message: String,
    /// Paths relative to the worktree; `None` commits every change (`git add -A`).
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitResult {
    pub sha: String,
    pub branch: Option<String>,
    pub files: Vec<String>,
    pub summary: String,
}

/// Checks a request before anything runs.
pub fn validate(req: &CommitRequest) -> Result<()> {
    let m = req.message.trim();
    if m.is_empty() {
        bail!("empty commit message");
    }
    if req.message.len() > MAX_MESSAGE || req.message.contains('\0') {
        bail!("commit message too long or contains NUL");
    }
    if let Some(paths) = &req.paths {
        if paths.is_empty() {
            bail!("paths is empty (omit it to commit every change)");
        }
        if paths.len() > MAX_PATHS {
            bail!("too many paths (max {MAX_PATHS})");
        }
        for p in paths {
            check_path(p)?;
        }
    }
    Ok(())
}

fn check_path(p: &str) -> Result<()> {
    let path = Path::new(p);
    if p.is_empty() || p.contains('\0') || path.is_absolute() {
        bail!("path {p:?} must be relative to the worktree");
    }
    for c in path.components() {
        match c {
            Component::Normal(s) if s == ".git" => bail!("path {p:?} is inside .git"),
            Component::Normal(_) | Component::CurDir => {}
            _ => bail!("path {p:?} leaves the worktree"),
        }
    }
    Ok(())
}

fn git_cmd(worktree: &Path, gitconfig: Option<&Path>) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(worktree)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin")
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("LANG", "C.UTF-8")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_CONFIG_GLOBAL", gitconfig.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/dev/null")));
    c
}

fn run(mut c: Command, stdin: Option<&str>) -> Result<String> {
    c.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.spawn()?;
    if let (Some(text), Some(mut si)) = (stdin, child.stdin.take()) {
        si.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `wt` is a linked worktree (never the main checkout) that git isn't busy
/// in; `what` names the refused operation.
fn linked_worktree(wt: &Path, what: &str) -> Result<worktree::Inspection> {
    let insp = worktree::inspect(wt);
    let Some(gitdir) = &insp.gitdir else {
        bail!("{} is not a git worktree: {}", wt.display(), insp.error.clone().unwrap_or_default());
    };
    if real(gitdir) == real(&worktree::common_dir(wt)?) {
        bail!("refusing to {what} in a main checkout");
    }
    if insp.index_lock {
        bail!("git is busy in this worktree (index.lock): try again");
    }
    Ok(insp)
}

/// Stages and commits in `worktree` (a linked worktree, never the main
/// checkout). `gitconfig` is the agent's generated global config.
pub fn commit_in_worktree(worktree_path: &Path, req: &CommitRequest, gitconfig: Option<&Path>) -> Result<CommitResult> {
    validate(req)?;
    let wt = real(worktree_path);
    let insp = linked_worktree(&wt, "commit")?;
    let paths: Vec<String> = req.paths.clone().unwrap_or_default();
    let mut add = git_cmd(&wt, gitconfig);
    add.args(["add", "-A", "--"]);
    if paths.is_empty() {
        add.arg(".");
    } else {
        add.args(&paths);
    }
    run(add, None).context("git add")?;
    let mut staged = git_cmd(&wt, gitconfig);
    staged.args(["diff", "--cached", "--name-only", "-z"]);
    if !paths.is_empty() {
        staged.arg("--").args(&paths);
    }
    let staged = run(staged, None)?;
    if staged.split('\0').all(str::is_empty) && insp.in_progress.is_empty() {
        bail!("nothing to commit");
    }
    let mut commit = git_cmd(&wt, gitconfig);
    commit.args(["commit", "--no-verify", "--quiet", "--file=-"]);
    if !paths.is_empty() {
        commit.arg("--").args(&paths);
    }
    run(commit, Some(&req.message)).context("git commit")?;
    let mut sha = git_cmd(&wt, gitconfig);
    sha.args(["rev-parse", "HEAD"]);
    let sha = run(sha, None)?.trim().to_string();
    let mut files = git_cmd(&wt, gitconfig);
    files.args(["show", "--name-only", "--format=", "-z", "HEAD"]);
    let files = run(files, None)?.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
    let mut br = git_cmd(&wt, gitconfig);
    br.args(["symbolic-ref", "--short", "-q", "HEAD"]);
    let branch = run(br, None).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Ok(CommitResult { sha, branch, files, summary: req.message.lines().next().unwrap_or("").to_string() })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MergeOutcome {
    Merged {
        sha: String,
    },
    UpToDate,
    /// Conflicted paths. With `abort_on_conflict` the merge was aborted;
    /// otherwise it is left in progress for the agent to resolve and commit.
    Conflict {
        files: Vec<String>,
        aborted: bool,
    },
}

/// Merges `branch` into the branch checked out in `worktree` (a linked
/// worktree), outside the sandbox with the same safety as
/// [`commit_in_worktree`]: hooks off, the agent's identity, a cleared
/// environment. The lead's `merge` aborts on a conflict; a specialist's
/// `sync_integration` leaves it for the agent.
pub fn merge_into_worktree(
    worktree_path: &Path,
    branch: &str,
    message: &str,
    gitconfig: Option<&Path>,
    abort_on_conflict: bool,
) -> Result<MergeOutcome> {
    if branch.is_empty() || branch.starts_with('-') || branch.contains("..") || branch.contains('\0') {
        bail!("invalid branch {branch:?}");
    }
    if message.trim().is_empty() || message.len() > MAX_MESSAGE {
        bail!("invalid merge message");
    }
    let wt = real(worktree_path);
    let insp = linked_worktree(&wt, "merge")?;
    if !insp.in_progress.is_empty() {
        bail!("a {} is already in progress in this worktree", insp.in_progress.join("/"));
    }
    if insp.dirty > 0 {
        bail!("the worktree has uncommitted changes: commit them first");
    }
    let g = |args: &[&str]| {
        let mut c = git_cmd(&wt, gitconfig);
        c.args(args);
        run(c, None)
    };
    let target =
        g(&["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("refs/heads/{branch}^{{commit}}")])
            .with_context(|| format!("no branch {branch}"))?
            .trim()
            .to_string();
    let head = g(&["rev-parse", "HEAD"])?.trim().to_string();
    if let Err(e) = g(&["merge", "--no-ff", "--no-verify", "--no-edit", "-m", message, &target]) {
        let files: Vec<String> = g(&["diff", "--name-only", "--diff-filter=U", "-z"])
            .unwrap_or_default()
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if files.is_empty() {
            let _ = g(&["merge", "--abort"]);
            return Err(e.context("git merge"));
        }
        if abort_on_conflict {
            g(&["merge", "--abort"]).context("git merge --abort")?;
        }
        return Ok(MergeOutcome::Conflict { files, aborted: abort_on_conflict });
    }
    let sha = g(&["rev-parse", "HEAD"])?.trim().to_string();
    Ok(if sha == head { MergeOutcome::UpToDate } else { MergeOutcome::Merged { sha } })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebaseOutcome {
    /// The branch now sits on top of `onto`: `from` = its head before.
    Rebased {
        from: String,
        to: String,
    },
    UpToDate,
    /// The rebase stopped on commit `step` of `of`; `files` are conflicted.
    /// Left in progress: fix, then [`rebase_continue`] (or [`rebase_abort`]).
    Conflict {
        files: Vec<String>,
        step: u32,
        of: u32,
    },
    Aborted,
}

/// A rebase is in progress in the worktree `insp` describes.
pub fn rebasing(insp: &worktree::Inspection) -> bool {
    insp.in_progress.iter().any(|m| m == "rebase-merge" || m == "rebase-apply")
}

/// git for a rebase: as [`git_cmd`], with no editor (messages kept, no todo edit).
fn rebase_git(wt: &Path, gitconfig: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut c = git_cmd(wt, gitconfig);
    c.env("GIT_EDITOR", "true").env("GIT_SEQUENCE_EDITOR", "true").args(args);
    run(c, None)
}

fn rebase_file(gitdir: &Path, name: &str) -> Option<String> {
    ["rebase-merge", "rebase-apply"]
        .iter()
        .find_map(|d| std::fs::read_to_string(gitdir.join(d).join(name)).ok())
        .map(|s| s.trim().to_string())
}

/// After `git rebase` (or `--continue`) failed with `err`: a stop on
/// conflicts is left for the agent; anything else is aborted (never a
/// half-rebased worktree on an error) and `err` returned.
fn rebase_stopped(wt: &Path, gitconfig: Option<&Path>, err: anyhow::Error) -> Result<RebaseOutcome> {
    let insp = worktree::inspect(wt);
    if !rebasing(&insp) {
        return Err(err.context("git rebase"));
    }
    let files: Vec<String> = rebase_git(wt, gitconfig, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .unwrap_or_default()
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if files.is_empty() {
        rebase_git(wt, gitconfig, &["rebase", "--abort"]).context("git rebase --abort")?;
        return Err(err.context("git rebase (aborted)"));
    }
    // `--rebase-merges` todo lists hold label/reset lines too: count the commits only.
    let gd = insp.gitdir.unwrap_or_default();
    let commits = |n: &str| {
        rebase_file(&gd, n).map_or(0, |s| {
            s.lines().filter(|l| matches!(l.split_whitespace().next(), Some("pick" | "p" | "merge" | "m"))).count()
                as u32
        })
    };
    let step = commits("done");
    Ok(RebaseOutcome::Conflict { files, step, of: step + commits("git-rebase-todo") })
}

fn valid_rev(rev: &str) -> Result<()> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains("..") || rev.contains('\0') {
        bail!("invalid revision {rev:?}");
    }
    Ok(())
}

/// Rebases the branch checked out in `worktree` (a linked worktree, clean,
/// nothing in progress) onto `onto` (`--rebase-merges`: the task merges are
/// kept), outside the sandbox with the safety of [`commit_in_worktree`].
pub fn rebase_worktree(worktree_path: &Path, onto: &str, gitconfig: Option<&Path>) -> Result<RebaseOutcome> {
    valid_rev(onto)?;
    let wt = real(worktree_path);
    let insp = linked_worktree(&wt, "rebase")?;
    if !insp.in_progress.is_empty() {
        bail!("a {} is already in progress in this worktree", insp.in_progress.join("/"));
    }
    if insp.dirty > 0 {
        bail!("the worktree has uncommitted changes: commit them first");
    }
    let g = |args: &[&str]| rebase_git(&wt, gitconfig, args);
    let target = g(&["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{onto}^{{commit}}")])
        .with_context(|| format!("no {onto}"))?
        .trim()
        .to_string();
    let from = g(&["rev-parse", "HEAD"])?.trim().to_string();
    if g(&["merge-base", "--is-ancestor", &target, "HEAD"]).is_ok() {
        return Ok(RebaseOutcome::UpToDate);
    }
    match g(&["rebase", "--no-autosquash", "--rebase-merges", "--no-verify", "--quiet", &target]) {
        Ok(_) => Ok(RebaseOutcome::Rebased { from, to: g(&["rev-parse", "HEAD"])?.trim().to_string() }),
        Err(e) => rebase_stopped(&wt, gitconfig, e),
    }
}

/// Stages everything and continues the rebase in progress: the next stop or the end.
pub fn rebase_continue(worktree_path: &Path, gitconfig: Option<&Path>) -> Result<RebaseOutcome> {
    let wt = real(worktree_path);
    let insp = linked_worktree(&wt, "continue a rebase")?;
    if !rebasing(&insp) {
        bail!("no rebase in progress in this worktree");
    }
    let gd = insp.gitdir.clone().unwrap_or_default();
    let from = rebase_file(&gd, "orig-head").ok_or_else(|| anyhow::anyhow!("the rebase in progress has no orig-head"))?;
    let g = |args: &[&str]| rebase_git(&wt, gitconfig, args);
    g(&["add", "-A", "--", "."]).context("git add")?;
    match g(&["rebase", "--continue"]) {
        Ok(_) if !rebasing(&worktree::inspect(&wt)) => {
            Ok(RebaseOutcome::Rebased { from, to: g(&["rev-parse", "HEAD"])?.trim().to_string() })
        }
        Ok(_) => rebase_stopped(&wt, gitconfig, anyhow::anyhow!("the rebase stopped")),
        Err(e) => rebase_stopped(&wt, gitconfig, e),
    }
}

/// Aborts the rebase in progress: the branch is back where it was.
pub fn rebase_abort(worktree_path: &Path, gitconfig: Option<&Path>) -> Result<RebaseOutcome> {
    let wt = real(worktree_path);
    if !rebasing(&linked_worktree(&wt, "abort a rebase")?) {
        bail!("no rebase in progress in this worktree");
    }
    rebase_git(&wt, gitconfig, &["rebase", "--abort"]).context("git rebase --abort")?;
    Ok(RebaseOutcome::Aborted)
}

/// Moves the branch checked out in `worktree` (linked, clean, nothing in
/// progress) to `rev` (`reset --hard`): a specialist branch after its
/// feature's integration branch was rebased.
pub fn reset_worktree(worktree_path: &Path, rev: &str, gitconfig: Option<&Path>) -> Result<()> {
    valid_rev(rev)?;
    let wt = real(worktree_path);
    let insp = linked_worktree(&wt, "reset")?;
    if !insp.in_progress.is_empty() || insp.dirty > 0 {
        bail!("the worktree is not clean");
    }
    rebase_git(&wt, gitconfig, &["reset", "--hard", "--quiet", "--end-of-options", rev]).context("git reset")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{MemLedger, git as tgit, scratch_repo};

    #[test]
    fn merges_into_a_worktree_and_reports_conflicts() {
        let (d, repo) = scratch_repo();
        let led = MemLedger::default();
        let integ = worktree::ensure(&led, &repo, &WorktreeKindSpec::Integration { feature: "f".into() }, None, None)
            .unwrap()
            .path;
        let back = worktree::ensure(
            &led,
            &repo,
            &WorktreeKindSpec::Team { feature: "f".into(), role: "back".into(), n: 1 },
            Some("orch/f/integration"),
            None,
        )
        .unwrap()
        .path;
        let gc = d.path().join("gitconfig");
        std::fs::write(&gc, "[user]\n\tname = Agent\n\temail = a@example.invalid\n").unwrap();
        let commit = |wt: &Path, f: &str, body: &str| {
            std::fs::write(wt.join(f), body).unwrap();
            commit_in_worktree(wt, &CommitRequest { message: format!("edit {f}"), paths: None }, Some(&gc)).unwrap();
        };
        commit(&back, "api.rs", "fn api() {}");
        let r = merge_into_worktree(&integ, "orch/f/back-1", "Merge back-1", Some(&gc), true).unwrap();
        assert!(matches!(r, MergeOutcome::Merged { .. }), "{r:?}");
        assert!(integ.join("api.rs").is_file());
        assert_eq!(
            merge_into_worktree(&integ, "orch/f/back-1", "again", Some(&gc), true).unwrap(),
            MergeOutcome::UpToDate
        );
        // Both sides change the same file: the lead's merge aborts…
        commit(&integ, "shared.txt", "lead");
        commit(&back, "shared.txt", "back");
        let r = merge_into_worktree(&integ, "orch/f/back-1", "Merge", Some(&gc), true).unwrap();
        assert_eq!(r, MergeOutcome::Conflict { files: vec!["shared.txt".into()], aborted: true });
        assert!(worktree::inspect(&integ).in_progress.is_empty());
        // …a sync leaves it for the agent, whose commit concludes it.
        let r = merge_into_worktree(&back, "orch/f/integration", "Sync", Some(&gc), false).unwrap();
        assert_eq!(r, MergeOutcome::Conflict { files: vec!["shared.txt".into()], aborted: false });
        assert_eq!(worktree::inspect(&back).in_progress, vec!["MERGE_HEAD"]);
        std::fs::write(back.join("shared.txt"), "both").unwrap();
        commit_in_worktree(&back, &CommitRequest { message: "resolve".into(), paths: None }, Some(&gc)).unwrap();
        assert!(worktree::inspect(&back).in_progress.is_empty());
        assert!(matches!(
            merge_into_worktree(&integ, "orch/f/back-1", "Merge", Some(&gc), true).unwrap(),
            MergeOutcome::Merged { .. }
        ));
        // Refusals.
        assert!(merge_into_worktree(&integ, "--upload-pack=x", "m", Some(&gc), true).is_err());
        assert!(merge_into_worktree(&integ, "orch/f/nope", "m", Some(&gc), true).is_err());
        assert!(merge_into_worktree(&repo, "orch/f/back-1", "m", Some(&gc), true).is_err());
        std::fs::write(integ.join("dirty.txt"), "x").unwrap();
        assert!(
            merge_into_worktree(&integ, "orch/f/back-1", "m", Some(&gc), true)
                .unwrap_err()
                .to_string()
                .contains("uncommitted")
        );
    }
    use orch_protocol::WorktreeKindSpec;

    /// The repo, an integration worktree and a specialist one of feature `f`, and a gitconfig.
    fn feature() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let (d, repo) = scratch_repo();
        let led = MemLedger::default();
        let integ = worktree::ensure(&led, &repo, &WorktreeKindSpec::Integration { feature: "f".into() }, None, None)
            .unwrap()
            .path;
        let back = worktree::ensure(
            &led,
            &repo,
            &WorktreeKindSpec::Team { feature: "f".into(), role: "back".into(), n: 1 },
            Some("orch/f/integration"),
            None,
        )
        .unwrap()
        .path;
        let gc = d.path().join("gitconfig");
        std::fs::write(&gc, "[user]\n\tname = Agent\n\temail = a@example.invalid\n").unwrap();
        (d, repo, integ, back, gc)
    }

    fn edit(wt: &Path, f: &str, body: &str, gc: &Path) {
        std::fs::write(wt.join(f), body).unwrap();
        commit_in_worktree(wt, &CommitRequest { message: format!("edit {f}"), paths: None }, Some(gc)).unwrap();
    }

    fn head(wt: &Path) -> String {
        tgit(wt, &["rev-parse", "HEAD"]).trim().to_string()
    }

    #[test]
    fn rebases_onto_the_base_keeping_the_task_merges() {
        let (_d, repo, integ, back, gc) = feature();
        edit(&integ, "spec.md", "spec", &gc);
        edit(&back, "api.rs", "fn api() {}", &gc);
        merge_into_worktree(&integ, "orch/f/back-1", "Merge back-1", Some(&gc), true).unwrap();
        crate::fixtures::commit_file(&repo, "m.txt", "m", "main moves");
        let before = head(&integ);
        let r = rebase_worktree(&integ, "main", Some(&gc)).unwrap();
        let RebaseOutcome::Rebased { from, to } = r else { panic!("{r:?}") };
        assert_eq!((from, to.clone()), (before, head(&integ)));
        assert!(tgit(&integ, &["merge-base", "--is-ancestor", "main", "HEAD"]).is_empty());
        assert_eq!(tgit(&integ, &["rev-list", "--merges", "--count", "main..HEAD"]).trim(), "1", "the task merge is kept");
        assert!(integ.join("api.rs").is_file() && integ.join("m.txt").is_file());
        assert_eq!(rebase_worktree(&integ, "main", Some(&gc)).unwrap(), RebaseOutcome::UpToDate);
        // The specialist's branch, all merged, moves to the new head.
        reset_worktree(&back, &to, Some(&gc)).unwrap();
        assert_eq!(head(&back), to);
        std::fs::write(back.join("wip.txt"), "x").unwrap();
        assert!(reset_worktree(&back, &to, Some(&gc)).unwrap_err().to_string().contains("not clean"));
        assert!(reset_worktree(&back, "--hard", Some(&gc)).is_err());
    }

    #[test]
    fn a_conflicting_rebase_stops_continues_and_aborts() {
        let (_d, repo, integ, _back, gc) = feature();
        edit(&integ, "shared.txt", "lead 1", &gc);
        edit(&integ, "shared.txt", "lead 2", &gc);
        crate::fixtures::commit_file(&repo, "shared.txt", "main", "main edits shared");
        let before = head(&integ);
        let conflict = |step| RebaseOutcome::Conflict { files: vec!["shared.txt".into()], step, of: 2 };
        assert_eq!(rebase_worktree(&integ, "main", Some(&gc)).unwrap(), conflict(1));
        assert!(rebasing(&worktree::inspect(&integ)));
        assert!(rebase_worktree(&integ, "main", Some(&gc)).unwrap_err().to_string().contains("in progress"));
        std::fs::write(integ.join("shared.txt"), "main + lead 1").unwrap();
        assert_eq!(rebase_continue(&integ, Some(&gc)).unwrap(), conflict(2));
        std::fs::write(integ.join("shared.txt"), "main + lead 2").unwrap();
        let r = rebase_continue(&integ, Some(&gc)).unwrap();
        assert!(matches!(&r, RebaseOutcome::Rebased { from, .. } if *from == before), "{r:?}");
        assert!(worktree::inspect(&integ).in_progress.is_empty(), "a stale REBASE_HEAD is not in progress");
        assert_eq!(std::fs::read_to_string(integ.join("shared.txt")).unwrap(), "main + lead 2");
        assert_eq!(tgit(&integ, &["rev-list", "--count", "main..HEAD"]).trim(), "2");
        assert!(rebase_continue(&integ, Some(&gc)).unwrap_err().to_string().contains("no rebase"));

        // Abort: the branch is back where it was.
        crate::fixtures::commit_file(&repo, "shared.txt", "main again", "main edits shared again");
        let before = head(&integ);
        assert!(matches!(rebase_worktree(&integ, "main", Some(&gc)).unwrap(), RebaseOutcome::Conflict { .. }));
        assert_eq!(rebase_abort(&integ, Some(&gc)).unwrap(), RebaseOutcome::Aborted);
        assert_eq!(head(&integ), before);
        assert!(worktree::inspect(&integ).in_progress.is_empty());
        assert!(rebase_abort(&integ, Some(&gc)).is_err());

        // Refusals.
        assert!(rebase_worktree(&integ, "--exec=x", Some(&gc)).is_err());
        assert!(rebase_worktree(&integ, "nope", Some(&gc)).is_err());
        assert!(rebase_worktree(&repo, "main", Some(&gc)).is_err(), "never the main checkout");
        std::fs::write(integ.join("dirty.txt"), "x").unwrap();
        assert!(rebase_worktree(&integ, "main", Some(&gc)).unwrap_err().to_string().contains("uncommitted"));
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let (d, repo) = scratch_repo();
        let row = worktree::ensure(
            &MemLedger::default(),
            &repo,
            &WorktreeKindSpec::Solo { slug: "a1".into() },
            None,
            Some("a1"),
        )
        .unwrap();
        let gc = d.path().join("gitconfig");
        std::fs::write(&gc, "[user]\n\tname = Agent Test\n\temail = agent@example.invalid\n[credential]\n\thelper =\n")
            .unwrap();
        (d, repo, row.path, gc)
    }

    #[test]
    fn validation() {
        let ok = |m: &str, p: Option<Vec<&str>>| {
            validate(&CommitRequest { message: m.into(), paths: p.map(|v| v.into_iter().map(String::from).collect()) })
        };
        assert!(ok("fix", None).is_ok());
        assert!(ok("  ", None).is_err());
        assert!(ok("fix", Some(vec![])).is_err());
        for bad in ["/etc/passwd", "../x", "a/../../x", ".git/config", "sub/.git/hooks/pre-commit", ""] {
            assert!(ok("fix", Some(vec![bad])).is_err(), "{bad}");
        }
        assert!(ok("fix", Some(vec!["src/a.rs", "./b.txt", ":(top)weird"])).is_ok());
    }

    #[test]
    fn commits_all_or_some_without_hooks_with_the_agent_identity() {
        let (_d, repo, wt, gc) = setup();
        // A hook the agent could have planted must not run outside the sandbox.
        let marker = wt.join("HOOK_RAN");
        let hooks = repo.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        tgit(&repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        // Without a repo-local identity, the agent's gitconfig provides it.
        tgit(&repo, &["config", "--unset", "user.name"]);
        tgit(&repo, &["config", "--unset", "user.email"]);

        std::fs::write(wt.join("a.txt"), "a").unwrap();
        std::fs::write(wt.join("b.txt"), "b").unwrap();
        let r = commit_in_worktree(
            &wt,
            &CommitRequest { message: "add a\n\nbody".into(), paths: Some(vec!["a.txt".into()]) },
            Some(&gc),
        )
        .unwrap();
        assert_eq!(r.files, ["a.txt"]);
        assert_eq!(r.branch.as_deref(), Some("orch/solo/a1"));
        assert_eq!(r.summary, "add a");
        assert!(!marker.exists(), "hooks are disabled");
        let author = tgit(&wt, &["log", "-1", "--format=%an <%ae>"]);
        assert_eq!(author.trim(), "Agent Test <agent@example.invalid>");
        // b.txt is still uncommitted; commit everything.
        let r2 = commit_in_worktree(&wt, &CommitRequest { message: "rest".into(), paths: None }, Some(&gc)).unwrap();
        assert_eq!(r2.files, ["b.txt"]);
        assert!(
            commit_in_worktree(&wt, &CommitRequest { message: "again".into(), paths: None }, Some(&gc))
                .unwrap_err()
                .to_string()
                .contains("nothing to commit")
        );
        // Never in the main checkout; never while git is busy.
        std::fs::write(repo.join("m.txt"), "m").unwrap();
        assert!(commit_in_worktree(&repo, &CommitRequest { message: "x".into(), paths: None }, Some(&gc)).is_err());
        let gd = worktree::inspect(&wt).gitdir.unwrap();
        std::fs::write(wt.join("c.txt"), "c").unwrap();
        std::fs::write(gd.join("index.lock"), "").unwrap();
        assert!(
            commit_in_worktree(&wt, &CommitRequest { message: "x".into(), paths: None }, Some(&gc))
                .unwrap_err()
                .to_string()
                .contains("busy")
        );
    }
}
