//! Worktree service (FR-105, ADR-003): moved to [`orch_git::worktree`];
//! re-exported here so the runtime's callers keep their paths.

pub use orch_git::worktree::*;
