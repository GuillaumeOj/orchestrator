#![doc = include_str!("../README.md")]

mod db;
mod entities;
mod events;
mod migrations;
mod model;
mod runtime_rows;
mod teams;
mod transcripts;
mod util;

pub use db::{Pending, Store, Tx};
pub use events::{AuditEntry, Event, Retention};
pub use migrations::SCHEMA_VERSION;
pub use model::*;
pub use runtime_rows::upsert_worktree as upsert_worktree_tx;
pub use runtime_rows::{AgentRow, HoldRowDb, RuntimeDiff, RuntimeRows, TurnRowDb};
pub use teams::{Artifact, Feature, Message, ProjectSettings, RunEvent, Task, TaskBackendConfig, TaskNote, TeamRun};
pub use transcripts::{IndexStats, SearchHit, TranscriptFile, encode_project_dir, extract_text};
pub use util::{tag, untag};

/// Milliseconds since the Unix epoch (the bus protocol's clock).
pub use orch_protocol::now_ms;
