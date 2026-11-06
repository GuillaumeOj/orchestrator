#![doc = include_str!("../README.md")]

mod chat;
mod db;
mod ecosystem;
mod entities;
mod events;
mod intake;
mod migrations;
mod model;
mod one_shot;
mod report;
mod reports;
mod runtime_rows;
mod teams;
mod transcripts;
mod util;

pub use chat::{ChatDay, ChatEntry};
pub use db::{Pending, Store, Tx, schema_version};
pub use ecosystem::IntegrationRow;
pub use intake::{IntakeRow, IntakeState, IntakeStatus, IntakeUpsert, TOPIC as INTAKE_TOPIC};
pub use events::{AuditEntry, Event, Retention, RunEvent};
pub use migrations::SCHEMA_VERSION;
pub use model::*;
pub use report::{BASELINE, Baseline, Interventions, Prompts, ReportAgent, ReportQuery, RunReport};
pub use reports::ReportRow;
pub use runtime_rows::upsert_worktree as upsert_worktree_tx;
pub use runtime_rows::{AgentRow, HoldRowDb, RuntimeDiff, RuntimeRows, TurnRowDb};
pub use teams::{
    Artifact, CiJob, DEFAULT_INTAKE_GLOB, GithubIntakeFilter, IntakeInvalid, IntakeSource, CiSummary, DeliveryState, Feature, FileAccess, FileKind, Message, PrPosition, PrStatus,
    ProjectFile, ProjectSettings, SettingsEdit, Task, TaskBackendConfig, TaskFlow, TaskNote, TeamRun, valid_repo,
    pr_number_from_url,
};
pub use transcripts::{IndexStats, SearchHit, TranscriptFile, encode_project_dir, extract_text};
pub use util::{tag, untag};

/// Milliseconds since the Unix epoch (the bus protocol's clock).
pub use orch_protocol::now_ms;
