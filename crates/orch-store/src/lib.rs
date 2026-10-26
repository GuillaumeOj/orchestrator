#![doc = include_str!("../README.md")]

mod db;
mod entities;
mod events;
mod migrations;
mod model;
mod runtime_rows;
mod transcripts;

pub use db::{Store, Tx};
pub use entities::tag;
pub use events::{AuditEntry, Event, Retention};
pub use migrations::SCHEMA_VERSION;
pub use model::*;
pub use runtime_rows::upsert_worktree as upsert_worktree_tx;
pub use runtime_rows::{AgentRow, HoldRowDb, RuntimeDiff, RuntimeRows, TurnRowDb};
pub use transcripts::{IndexStats, SearchHit, TranscriptFile, encode_project_dir, extract_text};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
