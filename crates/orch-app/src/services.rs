//! Backend services the app needs but that are built elsewhere (the SQLite
//! store, the project and git registry, the OTLP receiver). The app codes
//! against these small traits; the stubs here keep it usable until the real
//! crates land, and are replaced by implementing the same traits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A registered project (FR-101). Features come later (P2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

/// Git state shown in the sidebar and the status bar (FR-102).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GitSummary {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Changed tracked files plus untracked files.
    pub dirty: u32,
}

/// The project and git registry (§4.1). Real implementation: the parallel
/// `projects`/git crate with its SQLite rows and periodic fetches.
pub trait ProjectRegistry: Send + Sync {
    fn list(&self) -> Vec<Project>;
    fn add(&self, path: &Path) -> Result<Project>;
    fn remove(&self, id: &str) -> Result<()>;
    fn git_summary(&self, path: &Path) -> GitSummary;
}

/// Small key-value settings store: saved layouts per project (§9.1 principle 4),
/// UI preferences. Real implementation: a table in the SQLite store.
pub trait SettingsStore: Send + Sync {
    fn get(&self, key: &str) -> Option<Value>;
    fn set(&self, key: &str, value: Value) -> Result<()>;
}

/// Stub registry: projects live in memory, seeded at start; git state comes
/// from one `git status --porcelain=v2 --branch` per call (no fetch, no PRs).
#[derive(Default)]
pub struct InMemoryProjects {
    projects: Mutex<Vec<Project>>,
}

impl InMemoryProjects {
    pub fn seeded(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let me = Self::default();
        for p in paths {
            let _ = me.add(&p);
        }
        me
    }
}

fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    s.trim_matches('-').to_string()
}

impl ProjectRegistry for InMemoryProjects {
    fn list(&self) -> Vec<Project> {
        self.projects.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn add(&self, path: &Path) -> Result<Project> {
        let path = path.canonicalize().with_context(|| format!("{} does not exist", path.display()))?;
        if !path.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
        let mut list = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = list.iter().find(|p| p.path == path) {
            return Ok(p.clone());
        }
        let mut id = slug(&name);
        if id.is_empty() {
            id = "project".into();
        }
        let base = id.clone();
        let mut n = 2;
        while list.iter().any(|p| p.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let p = Project { id, name, path };
        list.push(p.clone());
        Ok(p)
    }

    fn remove(&self, id: &str) -> Result<()> {
        let mut list = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        let before = list.len();
        list.retain(|p| p.id != id);
        if list.len() == before {
            bail!("no project {id}");
        }
        Ok(())
    }

    fn git_summary(&self, path: &Path) -> GitSummary {
        let out = Command::new("git")
            .args(["status", "--porcelain=v2", "--branch", "--untracked-files=normal"])
            .current_dir(path)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output();
        match out {
            Ok(o) if o.status.success() => parse_porcelain_v2(&String::from_utf8_lossy(&o.stdout)),
            _ => GitSummary::default(),
        }
    }
}

/// Parses `git status --porcelain=v2 --branch`.
pub fn parse_porcelain_v2(text: &str) -> GitSummary {
    let mut g = GitSummary::default();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            if head != "(detached)" {
                g.branch = Some(head.to_string());
            }
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    g.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    g.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with(['1', '2', 'u', '?']) {
            g.dirty += 1;
        }
    }
    g
}

/// Stub settings store: a JSON file, written whole on every change.
pub struct JsonFileSettings {
    path: PathBuf,
    data: Mutex<BTreeMap<String, Value>>,
}

impl JsonFileSettings {
    pub fn open(path: PathBuf) -> Self {
        let data = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        JsonFileSettings { path, data: Mutex::new(data) }
    }
}

impl SettingsStore for JsonFileSettings {
    fn get(&self, key: &str) -> Option<Value> {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }

    fn set(&self, key: &str, value: Value) -> Result<()> {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        d.insert(key.to_string(), value);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&*d)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_v2() {
        let g = parse_porcelain_v2(
            "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n\
             1 .M N... 100644 100644 100644 a b src/x.rs\n? new.txt\n",
        );
        assert_eq!(g, GitSummary { branch: Some("main".into()), ahead: 2, behind: 1, dirty: 2 });
        assert_eq!(parse_porcelain_v2("# branch.head (detached)\n").branch, None);
    }

    #[test]
    fn projects_dedupe_and_unique_ids() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("Shop API");
        let b = d.path().join("x").join("shop-api");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let reg = InMemoryProjects::default();
        let pa = reg.add(&a).unwrap();
        assert_eq!(pa.id, "shop-api");
        assert_eq!(reg.add(&a).unwrap(), pa, "same folder twice");
        assert_eq!(reg.add(&b).unwrap().id, "shop-api-2");
        assert!(reg.add(&d.path().join("missing")).is_err());
        reg.remove("shop-api").unwrap();
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn settings_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ui/settings.json");
        let s = JsonFileSettings::open(p.clone());
        s.set("layout:shop", serde_json::json!({"a": 1})).unwrap();
        assert_eq!(JsonFileSettings::open(p).get("layout:shop"), Some(serde_json::json!({"a": 1})));
    }
}
