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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{MemLedger, git as tgit, scratch_repo};
    use orch_protocol::WorktreeKindSpec;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let (d, repo) = scratch_repo();
        let row = worktree::ensure(&MemLedger::default(), &repo, &WorktreeKindSpec::Solo { slug: "a1".into() }, None, Some("a1"))
            .unwrap();
        let gc = d.path().join("gitconfig");
        std::fs::write(&gc, "[user]\n\tname = Agent Test\n\temail = agent@example.invalid\n[credential]\n\thelper =\n").unwrap();
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
        let r = commit_in_worktree(&wt, &CommitRequest { message: "add a\n\nbody".into(), paths: Some(vec!["a.txt".into()]) }, Some(&gc))
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
        assert!(commit_in_worktree(&wt, &CommitRequest { message: "again".into(), paths: None }, Some(&gc))
            .unwrap_err()
            .to_string()
            .contains("nothing to commit"));
        // Never in the main checkout; never while git is busy.
        std::fs::write(repo.join("m.txt"), "m").unwrap();
        assert!(commit_in_worktree(&repo, &CommitRequest { message: "x".into(), paths: None }, Some(&gc)).is_err());
        let gd = worktree::inspect(&wt).gitdir.unwrap();
        std::fs::write(wt.join("c.txt"), "c").unwrap();
        std::fs::write(gd.join("index.lock"), "").unwrap();
        assert!(commit_in_worktree(&wt, &CommitRequest { message: "x".into(), paths: None }, Some(&gc))
            .unwrap_err()
            .to_string()
            .contains("busy"));
    }
}
