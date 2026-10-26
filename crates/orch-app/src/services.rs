//! The app's view of the core services: projects from the store's registry
//! (orch-git registers them, FR-101), git state from `orch_git::status`
//! (FR-102), and UI settings in the store's `kv` table (§4.12).

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A registered project as the shell shows it (FR-101). Features come later (P2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// The main checkout.
    pub path: PathBuf,
    pub default_branch: String,
    pub last_fetch_at_ms: Option<u64>,
    pub last_fetch_error: Option<String>,
}

impl From<orch_store::Project> for Project {
    fn from(p: orch_store::Project) -> Project {
        Project {
            id: p.id,
            name: p.name,
            path: p.path,
            default_branch: p.default_branch,
            last_fetch_at_ms: p.last_fetch_at_ms,
            last_fetch_error: p.last_fetch_error,
        }
    }
}

/// Git state shown in the sidebar and the status bar (FR-102).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GitSummary {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Changed tracked files plus untracked and conflicted files.
    pub dirty: u32,
    pub staged: u32,
    pub untracked: u32,
    pub conflicted: u32,
    pub stash: u32,
    pub upstream: Option<String>,
    pub last_commit: Option<String>,
    pub last_commit_s: Option<i64>,
}

impl From<orch_git::status::RepoStatus> for GitSummary {
    fn from(s: orch_git::status::RepoStatus) -> GitSummary {
        GitSummary {
            dirty: s.dirty(),
            branch: s.branch,
            ahead: s.ahead.unwrap_or(0),
            behind: s.behind.unwrap_or(0),
            staged: s.staged,
            untracked: s.untracked,
            conflicted: s.conflicted,
            stash: s.stash,
            upstream: s.upstream,
            last_commit_s: s.last_commit.as_ref().map(|c| c.time_s),
            last_commit: s.last_commit.map(|c| c.summary),
        }
    }
}

/// One `git status` (+ last commit); an unreadable folder gives the default.
pub fn git_summary(path: &Path) -> GitSummary {
    orch_git::status::status(path).map(GitSummary::from).unwrap_or_default()
}

/// UI settings (saved layout, theme, the project each session was started
/// from, first-run progress) in the store's `kv` table, under `ui.`.
#[derive(Clone)]
pub struct Settings {
    db: orch_store::Store,
}

const PREFIX: &str = "ui.";

impl Settings {
    pub fn new(db: orch_store::Store) -> Settings {
        Settings { db }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.db.setting(&format!("{PREFIX}{key}")).ok().flatten()
    }

    pub fn set(&self, key: &str, value: Value) -> Result<()> {
        self.db.set_setting(&format!("{PREFIX}{key}"), &value)
    }

    /// Imports the first version's `<home>/ui/settings.json` once: keys the
    /// store already has win; the file is renamed `settings.json.migrated`.
    pub fn migrate_json(&self, path: &Path) -> Result<usize> {
        let Ok(bytes) = std::fs::read(path) else { return Ok(0) };
        let map: serde_json::Map<String, Value> = serde_json::from_slice(&bytes)?;
        let mut n = 0;
        for (k, v) in map {
            if self.get(&k).is_none() {
                self.set(&k, v)?;
                n += 1;
            }
        }
        self.db.flush()?;
        std::fs::rename(path, path.with_extension("json.migrated"))?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_summary_of_a_real_repo() {
        let (d, repo) = orch_git::fixtures::scratch_repo();
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        let g = git_summary(&repo);
        assert_eq!(g.branch.as_deref(), Some("main"));
        assert_eq!((g.dirty, g.untracked), (1, 1));
        assert!(g.last_commit.is_some());
        assert_eq!(git_summary(&d.path().join("missing")), GitSummary::default());
    }

    #[test]
    fn settings_in_the_store_and_the_json_migration() {
        let d = tempfile::tempdir().unwrap();
        let db = orch_store::Store::open(&d.path().join("orch.db")).unwrap();
        let s = Settings::new(db.clone());
        s.set("layout", serde_json::json!({"a": 1})).unwrap();
        let json = d.path().join("ui/settings.json");
        std::fs::create_dir_all(json.parent().unwrap()).unwrap();
        std::fs::write(&json, r#"{"layout": {"a": 0}, "agent-project:a1": "shop"}"#).unwrap();
        assert_eq!(s.migrate_json(&json).unwrap(), 1, "the store's own layout wins");
        assert!(!json.exists() && json.with_extension("json.migrated").exists());
        assert_eq!(s.migrate_json(&json).unwrap(), 0, "once");
        drop(s);
        let s = Settings::new(db);
        assert_eq!(s.get("layout"), Some(serde_json::json!({"a": 1})));
        assert_eq!(s.get("agent-project:a1"), Some(serde_json::json!("shop")));
        assert_eq!(s.get("missing"), None);
    }
}
