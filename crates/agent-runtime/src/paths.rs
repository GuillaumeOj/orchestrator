//! Where the core keeps its files (app data).

use std::path::{Path, PathBuf};

/// The app-data layout:
/// - `core.sock`: the bus (0600, < 104 bytes);
/// - `bin/`: helper binaries at a stable path (FR-1201);
/// - `agents/<id>/`: per-agent settings, MCP config, gitconfig, spool, messages, PTY log;
/// - `state/state.json`: the core's state (stand-in for SQLite, see README).
#[derive(Clone, Debug, PartialEq)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        AppPaths { root: root.into() }
    }

    /// `~/Library/Application Support/Orchestrator`.
    pub fn default_root() -> PathBuf {
        home_dir().join("Library/Application Support/Orchestrator")
    }

    pub fn socket(&self) -> PathBuf {
        self.root.join("core.sock")
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.agents_dir().join(agent_id)
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.root.join("core.log")
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Agent ids and slugs end up in paths, branch names and env values.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !s.starts_with('-')
}

/// `canonicalize` when the path exists, otherwise the path as given.
pub fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("solo-fix_1"));
        for bad in ["", "-x", "a/b", "a b", "../x", &"x".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn default_socket_fits() {
        let p = AppPaths::new(AppPaths::default_root());
        assert!(orch_protocol::check_socket_path(&p.socket()).is_ok(), "{}", p.socket().display());
    }
}
