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

/// Stages and commits in `worktree` (a linked worktree, never the main
/// checkout). `gitconfig` is the agent's generated global config.
pub fn commit_in_worktree(worktree_path: &Path, req: &CommitRequest, gitconfig: Option<&Path>) -> Result<CommitResult> {
    validate(req)?;
    let wt = real(worktree_path);
    let insp = worktree::inspect(&wt);
    let Some(gitdir) = &insp.gitdir else {
        bail!("{} is not a git worktree: {}", wt.display(), insp.error.unwrap_or_default());
    };
    let common = worktree::common_dir(&wt)?;
    if real(gitdir) == real(&common) {
        bail!("refusing to commit in a main checkout");
    }
    if insp.index_lock {
        bail!("git is busy in this worktree (index.lock): try again");
    }
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
    let insp = worktree::inspect(&wt);
    let Some(gitdir) = &insp.gitdir else {
        bail!("{} is not a git worktree: {}", wt.display(), insp.error.unwrap_or_default());
    };
    if real(gitdir) == real(&worktree::common_dir(&wt)?) {
        bail!("refusing to merge in a main checkout");
    }
    if insp.index_lock {
        bail!("git is busy in this worktree (index.lock): try again");
    }
    if !insp.in_progress.is_empty() {
        bail!("a {} is already in progress in this worktree", insp.in_progress.join("/"));
    }
    if insp.dirty > 0 {
        bail!("the worktree has uncommitted changes: commit them first");
    }
    let mut verify = git_cmd(&wt, gitconfig);
    verify.args(["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("refs/heads/{branch}^{{commit}}")]);
    let target = run(verify, None).with_context(|| format!("no branch {branch}"))?.trim().to_string();
    let mut before = git_cmd(&wt, gitconfig);
    before.args(["rev-parse", "HEAD"]);
    let head = run(before, None)?.trim().to_string();
    let mut merge = git_cmd(&wt, gitconfig);
    merge.args(["merge", "--no-ff", "--no-verify", "--no-edit", "-m", message, &target]);
    if let Err(e) = run(merge, None) {
        let mut u = git_cmd(&wt, gitconfig);
        u.args(["diff", "--name-only", "--diff-filter=U", "-z"]);
        let files: Vec<String> =
            run(u, None).unwrap_or_default().split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
        if files.is_empty() {
            let mut abort = git_cmd(&wt, gitconfig);
            abort.args(["merge", "--abort"]);
            let _ = run(abort, None);
            return Err(e.context("git merge"));
        }
        if abort_on_conflict {
            let mut abort = git_cmd(&wt, gitconfig);
            abort.args(["merge", "--abort"]);
            run(abort, None).context("git merge --abort")?;
        }
        return Ok(MergeOutcome::Conflict { files, aborted: abort_on_conflict });
    }
    let mut after = git_cmd(&wt, gitconfig);
    after.args(["rev-parse", "HEAD"]);
    let sha = run(after, None)?.trim().to_string();
    Ok(if sha == head { MergeOutcome::UpToDate } else { MergeOutcome::Merged { sha } })
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
