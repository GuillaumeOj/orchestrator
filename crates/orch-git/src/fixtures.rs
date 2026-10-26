//! Test fixtures shared with other crates (feature `fixtures`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use anyhow::Result;

use crate::git::real;
use crate::worktree::{WorktreeLedger, WorktreeRow};

/// git with no user/system config, panicking on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A scratch repo `<tmp>/proj/demo` with one commit on `main`.
pub fn scratch_repo() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("proj/demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "T"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    (d, real(&repo))
}

/// Commits one file in `dir` (test identity, no hooks).
pub fn commit_file(dir: &Path, name: &str, body: &str, msg: &str) {
    std::fs::write(dir.join(name), body).unwrap();
    git(dir, &["add", name]);
    git(dir, &["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-qm", msg]);
}

/// An in-memory worktree ledger.
#[derive(Default)]
pub struct MemLedger(pub Mutex<BTreeMap<PathBuf, WorktreeRow>>);

impl WorktreeLedger for MemLedger {
    fn get_row(&self, path: &Path) -> Option<WorktreeRow> {
        self.0.lock().unwrap().get(path).cloned()
    }

    fn put_row(&self, row: WorktreeRow) -> Result<()> {
        self.0.lock().unwrap().insert(row.path.clone(), row);
        Ok(())
    }

    fn rows_of(&self, repo: &Path) -> Vec<WorktreeRow> {
        self.0.lock().unwrap().values().filter(|r| r.repo == repo).cloned().collect()
    }
}
