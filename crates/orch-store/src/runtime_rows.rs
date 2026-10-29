//! Tables the agent runtime owns (agents, worktrees, turns, holds, inbox).
//! The runtime keeps them in memory for its hot paths and writes every change
//! through [`Store::sync_runtime`], one transaction per change, so a crash
//! never leaves half an update. Other writers must go through the runtime.

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{Store, Tx};
use crate::model::{InboxItem, InboxStatus, WorktreeRow, WorktreeRowState};
use crate::util::{js, opt_u64, p};

/// An agent: key columns for queries plus the runtime's full record in `data`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub session_id: String,
    pub state: String,
    pub pid: Option<i32>,
    pub tier: Option<String>,
    pub allocation: Option<String>,
    pub last_active_at_ms: Option<u64>,
    pub created_at_ms: u64,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRowDb {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub status: String,
    pub created_at_ms: u64,
    pub delivered_at_ms: Option<u64>,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldRowDb {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub status: String,
    pub created_at_ms: u64,
    pub data: Value,
}

/// Everything the runtime loads at start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeRows {
    pub agents: Vec<AgentRow>,
    pub worktrees: Vec<WorktreeRow>,
    pub turns: Vec<TurnRowDb>,
    pub holds: Vec<HoldRowDb>,
    pub inbox: Vec<InboxItem>,
}

/// One change of the runtime's state: upserts and deletes, applied atomically.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeDiff {
    pub agents: Vec<AgentRow>,
    pub agents_deleted: Vec<String>,
    pub worktrees: Vec<WorktreeRow>,
    pub worktrees_deleted: Vec<PathBuf>,
    pub turns: Vec<TurnRowDb>,
    pub turns_deleted: Vec<String>,
    pub holds: Vec<HoldRowDb>,
    pub holds_deleted: Vec<String>,
    pub inbox: Vec<InboxItem>,
    pub inbox_deleted: Vec<u64>,
}

impl RuntimeDiff {
    pub fn is_empty(&self) -> bool {
        self == &RuntimeDiff::default()
    }
}

/// Upserts one worktree row inside a write transaction.
pub fn upsert_worktree(tx: &Tx, w: &WorktreeRow) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO worktrees(path, repo, branch, base, agent_id, state, created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(path) DO UPDATE SET repo=excluded.repo, branch=excluded.branch, base=excluded.base,
           agent_id=excluded.agent_id, state=excluded.state, created_at_ms=excluded.created_at_ms",
        params![p(&w.path), p(&w.repo), w.branch, w.base, w.agent_id, w.state.as_str(), w.created_at_ms as i64],
    )
}

pub(crate) fn upsert_inbox(tx: &Tx, i: &InboxItem) -> rusqlite::Result<usize> {
    tx.execute(
        "INSERT INTO inbox_items(id, kind, agent_id, project_id, body, payload, priority, status, stale, created_at_ms, decided_at_ms, decision, decided_by,
           subject, thread, sent_at_ms, effect, effect_detail, effect_at_ms, error, stale_why)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
         ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, agent_id=excluded.agent_id, project_id=excluded.project_id,
           body=excluded.body, payload=excluded.payload, priority=excluded.priority, status=excluded.status,
           stale=excluded.stale, created_at_ms=excluded.created_at_ms, decided_at_ms=excluded.decided_at_ms,
           decision=excluded.decision, decided_by=excluded.decided_by, subject=excluded.subject, thread=excluded.thread,
           sent_at_ms=excluded.sent_at_ms, effect=excluded.effect, effect_detail=excluded.effect_detail,
           effect_at_ms=excluded.effect_at_ms, error=excluded.error, stale_why=excluded.stale_why",
        params![
            i.id as i64,
            i.kind,
            i.agent_id,
            i.project_id,
            i.body,
            js(&i.payload),
            i.priority,
            i.status.as_str(),
            i.stale,
            i.created_at_ms as i64,
            i.decided_at_ms.map(|v| v as i64),
            i.decision,
            i.decided_by,
            (!i.subject.is_null()).then(|| js(&i.subject)),
            i.thread,
            i.sent_at_ms.map(|v| v as i64),
            i.effect,
            i.effect_detail,
            i.effect_at_ms.map(|v| v as i64),
            i.error,
            i.stale_why
        ],
    )
}

pub(crate) fn row_worktree(r: &rusqlite::Row) -> rusqlite::Result<WorktreeRow> {
    Ok(WorktreeRow {
        path: PathBuf::from(r.get::<_, String>(0)?),
        repo: PathBuf::from(r.get::<_, String>(1)?),
        branch: r.get(2)?,
        base: r.get(3)?,
        agent_id: r.get(4)?,
        state: WorktreeRowState::parse(&r.get::<_, String>(5)?),
        created_at_ms: r.get::<_, i64>(6)? as u64,
    })
}

pub(crate) fn row_inbox(r: &rusqlite::Row) -> rusqlite::Result<InboxItem> {
    Ok(InboxItem {
        id: r.get::<_, i64>(0)? as u64,
        kind: r.get(1)?,
        agent_id: r.get(2)?,
        project_id: r.get(3)?,
        body: r.get(4)?,
        payload: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
        priority: r.get(6)?,
        status: InboxStatus::parse(&r.get::<_, String>(7)?),
        stale: r.get(8)?,
        created_at_ms: r.get::<_, i64>(9)? as u64,
        decided_at_ms: opt_u64(r, 10)?,
        decision: r.get(11)?,
        decided_by: r.get(12)?,
        subject: r.get::<_, Option<String>>(13)?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        thread: r.get(14)?,
        sent_at_ms: opt_u64(r, 15)?,
        effect: r.get(16)?,
        effect_detail: r.get(17)?,
        effect_at_ms: opt_u64(r, 18)?,
        error: r.get(19)?,
        stale_why: r.get(20)?,
    })
}

pub(crate) const WORKTREE_COLS: &str = "path, repo, branch, base, agent_id, state, created_at_ms";
pub(crate) const INBOX_COLS: &str = "id, kind, agent_id, project_id, body, payload, priority, status, stale, created_at_ms, decided_at_ms, decision, decided_by, subject, thread, sent_at_ms, effect, effect_detail, effect_at_ms, error, stale_why";

fn load(c: &Connection) -> Result<RuntimeRows> {
    let parse = |s: String| serde_json::from_str::<Value>(&s).unwrap_or(Value::Null);
    let mut rows = RuntimeRows::default();
    let mut st = c.prepare(
        "SELECT id, repo, worktree, session_id, state, pid, tier, allocation, last_active_at_ms, created_at_ms, data
         FROM agents ORDER BY id",
    )?;
    rows.agents = st
        .query_map([], |r| {
            Ok(AgentRow {
                id: r.get(0)?,
                repo: PathBuf::from(r.get::<_, String>(1)?),
                worktree: PathBuf::from(r.get::<_, String>(2)?),
                session_id: r.get(3)?,
                state: r.get(4)?,
                pid: r.get(5)?,
                tier: r.get(6)?,
                allocation: r.get(7)?,
                last_active_at_ms: opt_u64(r, 8)?,
                created_at_ms: r.get::<_, i64>(9)? as u64,
                data: parse(r.get(10)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut st = c.prepare(&format!("SELECT {WORKTREE_COLS} FROM worktrees ORDER BY path"))?;
    rows.worktrees = st.query_map([], row_worktree)?.collect::<rusqlite::Result<_>>()?;
    let mut st = c.prepare(
        "SELECT id, agent_id, kind, status, created_at_ms, delivered_at_ms, data FROM turns ORDER BY created_at_ms, rowid",
    )?;
    rows.turns = st
        .query_map([], |r| {
            Ok(TurnRowDb {
                id: r.get(0)?,
                agent_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                created_at_ms: r.get::<_, i64>(4)? as u64,
                delivered_at_ms: opt_u64(r, 5)?,
                data: parse(r.get(6)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut st =
        c.prepare("SELECT id, agent_id, kind, status, created_at_ms, data FROM holds ORDER BY created_at_ms, rowid")?;
    rows.holds = st
        .query_map([], |r| {
            Ok(HoldRowDb {
                id: r.get(0)?,
                agent_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                created_at_ms: r.get::<_, i64>(4)? as u64,
                data: parse(r.get(5)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut st = c.prepare(&format!("SELECT {INBOX_COLS} FROM inbox_items ORDER BY id"))?;
    rows.inbox = st.query_map([], row_inbox)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Runs a statement through the connection's statement cache (the same
/// upserts run on every runtime change).
fn exec(tx: &Tx, sql: &str, params: impl rusqlite::Params) -> rusqlite::Result<usize> {
    tx.prepare_cached(sql)?.execute(params)
}

fn apply(tx: &Tx, d: &RuntimeDiff) -> Result<()> {
    for a in &d.agents {
        exec(tx,
            "INSERT INTO agents(id, repo, worktree, session_id, state, pid, tier, allocation, last_active_at_ms, created_at_ms, data)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET repo=excluded.repo, worktree=excluded.worktree, session_id=excluded.session_id,
               state=excluded.state, pid=excluded.pid, tier=excluded.tier, allocation=excluded.allocation,
               last_active_at_ms=excluded.last_active_at_ms, created_at_ms=excluded.created_at_ms, data=excluded.data",
            params![
                a.id,
                p(&a.repo),
                p(&a.worktree),
                a.session_id,
                a.state,
                a.pid,
                a.tier,
                a.allocation,
                a.last_active_at_ms.map(|v| v as i64),
                a.created_at_ms as i64,
                js(&a.data)
            ],
        )?;
    }
    for id in &d.agents_deleted {
        exec(tx, "DELETE FROM agents WHERE id=?1", [id])?;
    }
    for w in &d.worktrees {
        upsert_worktree(tx, w)?;
    }
    for path in &d.worktrees_deleted {
        exec(tx, "DELETE FROM worktrees WHERE path=?1", [p(path)])?;
    }
    for t in &d.turns {
        exec(tx,
            "INSERT INTO turns(id, agent_id, kind, status, created_at_ms, delivered_at_ms, data) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET agent_id=excluded.agent_id, kind=excluded.kind, status=excluded.status,
               created_at_ms=excluded.created_at_ms, delivered_at_ms=excluded.delivered_at_ms, data=excluded.data",
            params![
                t.id,
                t.agent_id,
                t.kind,
                t.status,
                t.created_at_ms as i64,
                t.delivered_at_ms.map(|v| v as i64),
                js(&t.data)
            ],
        )?;
    }
    for id in &d.turns_deleted {
        exec(tx, "DELETE FROM turns WHERE id=?1", [id])?;
    }
    for h in &d.holds {
        exec(
            tx,
            "INSERT INTO holds(id, agent_id, kind, status, created_at_ms, data) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET agent_id=excluded.agent_id, kind=excluded.kind, status=excluded.status,
               created_at_ms=excluded.created_at_ms, data=excluded.data",
            params![h.id, h.agent_id, h.kind, h.status, h.created_at_ms as i64, js(&h.data)],
        )?;
    }
    for id in &d.holds_deleted {
        exec(tx, "DELETE FROM holds WHERE id=?1", [id])?;
    }
    for i in &d.inbox {
        upsert_inbox(tx, i)?;
    }
    for id in &d.inbox_deleted {
        exec(tx, "DELETE FROM inbox_items WHERE id=?1", [*id as i64])?;
    }
    Ok(())
}

impl Store {
    /// Loads the runtime's tables.
    pub fn load_runtime(&self) -> Result<RuntimeRows> {
        self.read(load)
    }

    /// Applies one runtime change in a single transaction and waits for the commit.
    pub fn sync_runtime(&self, diff: RuntimeDiff) -> Result<()> {
        self.sync_runtime_begin(diff)?.wait()
    }

    /// Queues one runtime change (in order) and returns the wait for its commit.
    pub fn sync_runtime_begin(&self, diff: RuntimeDiff) -> Result<crate::Pending<()>> {
        self.write_begin(move |tx| if diff.is_empty() { Ok(()) } else { apply(tx, &diff) })
    }

    /// True when the runtime tables hold nothing (first start, or a JSON state to import).
    pub fn runtime_is_empty(&self) -> Result<bool> {
        self.read(|c| {
            let n: Option<i64> = c
                .query_row(
                    "SELECT 1 FROM agents UNION ALL SELECT 1 FROM worktrees UNION ALL SELECT 1 FROM turns LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(n.is_none())
        })
    }

    pub fn worktree(&self, path: &std::path::Path) -> Result<Option<WorktreeRow>> {
        let key = p(path);
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {WORKTREE_COLS} FROM worktrees WHERE path=?1"), [key], row_worktree)
                .optional()?)
        })
    }

    /// Worktree rows of one project (by main checkout path).
    pub fn worktrees_of(&self, repo: &std::path::Path) -> Result<Vec<WorktreeRow>> {
        let key = p(repo);
        self.read(move |c| {
            let mut st = c.prepare(&format!("SELECT {WORKTREE_COLS} FROM worktrees WHERE repo=?1 ORDER BY path"))?;
            Ok(st.query_map([key], row_worktree)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn inbox_item(&self, id: u64) -> Result<Option<InboxItem>> {
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {INBOX_COLS} FROM inbox_items WHERE id=?1"), [id as i64], row_inbox)
                .optional()?)
        })
    }

    /// Open (pending or parked) items in attention-ladder order (priority, then oldest first).
    pub fn inbox_pending(&self) -> Result<Vec<InboxItem>> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {INBOX_COLS} FROM inbox_items WHERE status IN ('pending', 'parked') ORDER BY priority DESC, created_at_ms, id"
            ))?;
            Ok(st.query_map([], row_inbox)?.collect::<rusqlite::Result<_>>()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str, state: &str) -> AgentRow {
        AgentRow {
            id: id.into(),
            repo: "/r".into(),
            worktree: "/r.worktrees/solo/x".into(),
            session_id: "s".into(),
            state: state.into(),
            pid: Some(42),
            tier: Some("normal".into()),
            allocation: Some("default".into()),
            last_active_at_ms: Some(5),
            created_at_ms: 1,
            data: json!({"tracker": {"state": state}}),
        }
    }

    #[test]
    fn sync_load_round_trip_and_survives_reopen() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("orch.db");
        {
            let s = Store::open(&path).unwrap();
            assert!(s.runtime_is_empty().unwrap());
            let mut inbox = InboxItem::new(7, "permission", Some("a1"), "Bash ls".into());
            inbox.payload = json!({"tool": "Bash"});
            s.sync_runtime(RuntimeDiff {
                agents: vec![agent("a1", "working"), agent("a2", "idle")],
                worktrees: vec![WorktreeRow {
                    path: "/r.worktrees/solo/x".into(),
                    repo: "/r".into(),
                    branch: "orch/solo/x".into(),
                    base: "main".into(),
                    agent_id: Some("a1".into()),
                    state: WorktreeRowState::Creating,
                    created_at_ms: 3,
                }],
                turns: vec![TurnRowDb {
                    id: "t1".into(),
                    agent_id: "a1".into(),
                    kind: "message".into(),
                    status: "queued".into(),
                    created_at_ms: 9,
                    delivered_at_ms: None,
                    data: json!({"body": "hi"}),
                }],
                holds: vec![HoldRowDb {
                    id: "h1".into(),
                    agent_id: "a1".into(),
                    kind: "gate".into(),
                    status: "held".into(),
                    created_at_ms: 9,
                    data: json!({}),
                }],
                inbox: vec![inbox.clone()],
                ..Default::default()
            })
            .unwrap();
            s.sync_runtime(RuntimeDiff { agents_deleted: vec!["a2".into()], ..Default::default() }).unwrap();
            assert_eq!(s.inbox_pending().unwrap(), vec![inbox]);
        }
        // A new process reads the same rows.
        let s = Store::open(&path).unwrap();
        let rows = s.load_runtime().unwrap();
        assert_eq!(rows.agents.len(), 1);
        assert_eq!(rows.agents[0], agent("a1", "working"));
        assert_eq!(rows.worktrees[0].state, WorktreeRowState::Creating);
        assert_eq!(rows.turns[0].data["body"], "hi");
        assert_eq!(rows.holds[0].status, "held");
        assert_eq!(rows.inbox[0].payload["tool"], "Bash");
        assert!(!s.runtime_is_empty().unwrap());
        assert_eq!(s.worktrees_of(std::path::Path::new("/r")).unwrap().len(), 1);
    }
}
