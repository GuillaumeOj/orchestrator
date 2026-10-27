//! Versioned migrations: `PRAGMA user_version` is the number of migrations
//! applied. Each one runs in its own transaction; a database newer than this
//! build is refused (never downgraded).

use anyhow::{Result, bail};
use rusqlite::Connection;

/// The schema version this build writes.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Append only: never edit a migration that has shipped.
const MIGRATIONS: &[&str] = &[
    // 1: P1 core services.
    r#"
CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    default_branch TEXT NOT NULL,
    remotes TEXT NOT NULL DEFAULT '[]',
    worktree_root TEXT NOT NULL,
    task_backend TEXT NOT NULL DEFAULT '{"type":"local"}',
    track_orchestrator_dir INTEGER NOT NULL DEFAULT 1,
    settings TEXT NOT NULL DEFAULT '{}',
    created_at_ms INTEGER NOT NULL,
    last_active_at_ms INTEGER NOT NULL,
    last_fetch_at_ms INTEGER,
    last_fetch_error TEXT,
    archived INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE worktrees (
    path TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    branch TEXT NOT NULL,
    base TEXT NOT NULL,
    agent_id TEXT,
    state TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX worktrees_repo ON worktrees(repo);

CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    worktree TEXT NOT NULL,
    session_id TEXT NOT NULL,
    state TEXT NOT NULL,
    pid INTEGER,
    tier TEXT,
    allocation TEXT,
    last_active_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX agents_session ON agents(session_id);

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER,
    data TEXT NOT NULL
);
CREATE INDEX turns_agent ON turns(agent_id, created_at_ms);

CREATE TABLE holds (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE inbox_items (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    body TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT 'null',
    priority INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    decided_at_ms INTEGER,
    decision TEXT
);
CREATE INDEX inbox_status ON inbox_items(status, priority, created_at_ms);

CREATE TABLE permission_rules (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    pattern TEXT NOT NULL,
    effect TEXT NOT NULL,
    source_inbox_item_id INTEGER,
    created_at_ms INTEGER NOT NULL,
    revoked_at_ms INTEGER
);

CREATE TABLE usage_samples (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    agent_id TEXT,
    turn_id TEXT,
    model TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    cache_read INTEGER,
    cache_write INTEGER,
    cost_estimate REAL,
    cost_delta REAL,
    five_hour_pct REAL,
    five_hour_resets_at INTEGER,
    weekly_pct REAL,
    weekly_resets_at INTEGER,
    model_weekly_pct REAL,
    provenance TEXT NOT NULL,
    source TEXT NOT NULL,
    class TEXT,
    allocation TEXT
);
CREATE INDEX usage_samples_ts ON usage_samples(ts_ms);
CREATE INDEX usage_samples_agent ON usage_samples(agent_id, ts_ms);

CREATE TABLE weekly_plans (
    id TEXT PRIMARY KEY,
    week_start_ms INTEGER NOT NULL,
    reset_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL,
    shadow_mode INTEGER NOT NULL,
    reserve_pct REAL NOT NULL,
    data TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE budget_allocations (
    weekly_plan_id TEXT NOT NULL REFERENCES weekly_plans(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    target TEXT NOT NULL,
    weight REAL NOT NULL,
    tier TEXT NOT NULL,
    hard_cap_pct REAL,
    PRIMARY KEY (weekly_plan_id, id)
);

CREATE TABLE pacing_policies (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE queue_entries (
    id INTEGER NOT NULL,
    boot_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    agent_id TEXT,
    allocation_id TEXT,
    tier TEXT,
    effective_tier TEXT,
    enqueued_at_ms INTEGER,
    reason TEXT,
    estimated_start_ms INTEGER,
    data TEXT NOT NULL,
    PRIMARY KEY (boot_ms, id)
);

CREATE TABLE governor_actions (
    rowid_ INTEGER PRIMARY KEY,
    boot_ms INTEGER NOT NULL,
    gov_id INTEGER NOT NULL,
    plan_id TEXT,
    ts_ms INTEGER NOT NULL,
    state TEXT,
    action TEXT NOT NULL,
    reason TEXT,
    mode TEXT,
    outcome TEXT,
    undone_at_ms INTEGER,
    data TEXT NOT NULL,
    UNIQUE (boot_ms, gov_id)
);
CREATE INDEX governor_actions_ts ON governor_actions(ts_ms);

CREATE TABLE calibrations (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    cost_per_weekly_pct REAL NOT NULL,
    confidence REAL NOT NULL,
    unattributed_rate REAL NOT NULL,
    data TEXT NOT NULL
);

CREATE TABLE forecast_snapshots (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    used REAL,
    ideal REAL,
    governed REAL,
    error REAL,
    projected REAL,
    eta_ms INTEGER,
    state TEXT,
    data TEXT NOT NULL
);
CREATE INDEX forecast_snapshots_ts ON forecast_snapshots(ts_ms);

CREATE TABLE terminals (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    agent_id TEXT,
    cwd TEXT NOT NULL,
    shell TEXT NOT NULL,
    layout TEXT NOT NULL DEFAULT '{}',
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    agent_id TEXT,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT 'null'
);
CREATE INDEX events_ts ON events(ts_ms);
CREATE INDEX events_agent ON events(agent_id, ts_ms);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    ts_ms INTEGER NOT NULL,
    actor TEXT NOT NULL,
    agent_id TEXT,
    action TEXT NOT NULL,
    target TEXT,
    decision TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT 'null'
);
CREATE INDEX audit_ts ON audit_log(ts_ms);

CREATE TABLE transcript_files (
    path TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    project_id TEXT,
    offset INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime_ms INTEGER NOT NULL,
    indexed_at_ms INTEGER NOT NULL
);
CREATE INDEX transcript_files_session ON transcript_files(session_id);

CREATE VIRTUAL TABLE transcript_fts USING fts5(
    body,
    session_id UNINDEXED,
    project_id UNINDEXED,
    role UNINDEXED,
    ts UNINDEXED,
    uuid UNINDEXED,
    path UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
"#,
    // 2: P2 teams (roles, teams, features, team runs, tasks, messages, artifacts).
    r#"
CREATE TABLE roles (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    data TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    data TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE team_versions (
    team_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    team TEXT NOT NULL,
    roles TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (team_id, version)
);

CREATE TABLE features (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    brief TEXT NOT NULL DEFAULT '',
    base TEXT NOT NULL,
    integration_worktree TEXT,
    allocation TEXT,
    created_at_ms INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    UNIQUE (project_id, slug)
);

CREATE TABLE team_runs (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    team_version INTEGER NOT NULL,
    feature_id TEXT NOT NULL,
    status TEXT NOT NULL,
    resume_status TEXT,
    pr_url TEXT,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER
);
CREATE INDEX team_runs_feature ON team_runs(feature_id);

CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    project_id TEXT NOT NULL,
    backend TEXT NOT NULL,
    external_id TEXT,
    feature_id TEXT,
    team_run_id TEXT,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    node TEXT,
    agent_id TEXT,
    depends_on TEXT NOT NULL DEFAULT '[]',
    branch TEXT,
    pr_url TEXT,
    notes TEXT NOT NULL DEFAULT '[]',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    remote_updated_at TEXT
);
CREATE INDEX tasks_project ON tasks(project_id);
CREATE INDEX tasks_run ON tasks(team_run_id);
CREATE UNIQUE INDEX tasks_external ON tasks(project_id, backend, external_id) WHERE external_id IS NOT NULL;

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    team_run_id TEXT NOT NULL,
    from_node TEXT NOT NULL,
    from_agent TEXT,
    to_node TEXT NOT NULL,
    to_agent TEXT,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    ref TEXT NOT NULL DEFAULT 'null',
    created_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER,
    read_at_ms INTEGER
);
CREATE INDEX messages_to ON messages(team_run_id, to_node, read_at_ms);

CREATE TABLE artifacts (
    id INTEGER PRIMARY KEY,
    feature_id TEXT NOT NULL,
    team_run_id TEXT,
    kind TEXT NOT NULL,
    uri TEXT NOT NULL,
    from_node TEXT NOT NULL,
    to_node TEXT,
    note TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX artifacts_feature ON artifacts(feature_id);

ALTER TABLE events ADD COLUMN team_run_id TEXT;
CREATE INDEX events_run ON events(team_run_id, ts_ms);
"#,
];

pub(crate) fn migrate(c: &mut Connection) -> Result<()> {
    let have: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if have > SCHEMA_VERSION {
        bail!("database schema {have} is newer than this build ({SCHEMA_VERSION}): refusing to downgrade");
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(have as usize) {
        let tx = c.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_once_and_refuses_newer() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("m.db");
        let mut c = Connection::open(&p).unwrap();
        migrate(&mut c).unwrap();
        migrate(&mut c).unwrap(); // idempotent
        let v: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        // FTS5 is compiled in (bundled SQLite).
        c.execute("INSERT INTO transcript_fts(body, session_id) VALUES ('hello world', 's')", []).unwrap();
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
        assert!(migrate(&mut c).unwrap_err().to_string().contains("newer"));
    }
}
