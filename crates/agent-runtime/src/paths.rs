//! Where the core keeps its files (app data).

use std::path::PathBuf;

/// The app-data layout:
/// - `core.sock`: the bus (0600, < 104 bytes);
/// - `bin/`: helper binaries at a stable path (FR-1201);
/// - `agents/<id>/`: per-agent settings, MCP config, gitconfig, spool, messages, PTY log;
/// - `state/orch.db`: the core's SQLite database (WAL), `state/orch.blobs/` for large payloads.
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

    /// The JSON stand-in used before SQLite (imported once, then renamed).
    pub fn legacy_state_file(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }

    /// The core's SQLite database (ADR-005).
    pub fn db_file(&self) -> PathBuf {
        self.state_dir().join("orch.db")
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub use orch_git::{real, valid_id};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_fits() {
        let p = AppPaths::new(AppPaths::default_root());
        assert!(orch_protocol::check_socket_path(&p.socket()).is_ok(), "{}", p.socket().display());
    }
}
