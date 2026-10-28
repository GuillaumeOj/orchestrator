//! Append-only events (hook, statusline, governor, runtime) with retention,
//! and the audit log of gate decisions and permission changes (FR-709), which
//! is kept.

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Store;

/// `SELECT id, ts_ms, agent_id, kind, detail` → [`Event`].
fn row_event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get::<_, i64>(0)? as u64,
        ts_ms: r.get::<_, i64>(1)? as u64,
        agent_id: r.get(2)?,
        kind: r.get(3)?,
        detail: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
    })
}

/// An event's detail as stored text, read from its blob file when it was
/// stored outside the row (the reader of what [`Store::add_event`] writes).
pub(crate) fn detail(text: &str) -> Value {
    let d: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    match d.get("blob").and_then(Value::as_str) {
        Some(p) => std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        None => d,
    }
}

/// Details larger than this go to a file in [`Store::blob_dir`] (NFR-8).
pub const BLOB_THRESHOLD: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub ts_ms: u64,
    pub agent_id: Option<String>,
    pub kind: String,
    /// `{"blob": "<path>"}` when the detail was stored outside the row.
    pub detail: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: u64,
    pub ts_ms: u64,
    /// Who decided (FR-709): `user` (Inbox card or app action), `pane` (Claude
    /// Code's own dialog, or Esc in the pane), `rule:<id>`, `expired` (hold
    /// deadline reached: parked), `core` (closed by the reconciler after a
    /// restart), `agent:<id>`, `governor`. `overwatch` is reserved (P4.4).
    pub actor: String,
    pub agent_id: Option<String>,
    /// `permission`, `commit`, `gate`, `rule_added`, `rule_revoked`, `push`, `pr`…
    pub action: String,
    pub target: Option<String>,
    /// `allow`, `deny`, `ok`, `failed`…
    pub decision: String,
    pub detail: Value,
}

/// How long rows are kept, in days (NFR-8: raw events 30 days by default).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Retention {
    pub events_days: u32,
    pub forecasts_days: u32,
    pub usage_days: u32,
    /// Decided / expired Inbox items.
    pub inbox_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Retention { events_days: 30, forecasts_days: 30, usage_days: 90, inbox_days: 90 }
    }
}

const DAY_MS: u64 = 24 * 3600 * 1000;

impl Store {
    /// Appends an event without waiting.
    pub fn add_event(&self, ts_ms: u64, agent_id: Option<&str>, kind: &str, detail: Value) {
        self.add_event_in(None, ts_ms, agent_id, kind, detail)
    }

    /// An event on a team run's timeline too (FR-408): one row, which the
    /// agent's and the global event lists show as well.
    pub fn add_run_event(&self, ts_ms: u64, run_id: &str, agent_id: Option<&str>, kind: &str, detail: Value) {
        self.add_event_in(Some(run_id), ts_ms, agent_id, kind, detail)
    }

    fn add_event_in(&self, run_id: Option<&str>, ts_ms: u64, agent_id: Option<&str>, kind: &str, detail: Value) {
        let run_id = run_id.map(str::to_string);
        let agent_id = agent_id.map(str::to_string);
        let kind = kind.to_string();
        let blob_dir = self.blob_dir();
        self.write_detached(move |tx| {
            let mut text = serde_json::to_string(&detail)?;
            if text.len() > BLOB_THRESHOLD {
                std::fs::create_dir_all(&blob_dir)?;
                let path = blob_dir.join(format!("{ts_ms}-{}.json", uuid_like()));
                std::fs::write(&path, &text)?;
                text = serde_json::to_string(&serde_json::json!({"blob": path, "bytes": text.len()}))?;
            }
            tx.execute(
                "INSERT INTO events(ts_ms, agent_id, kind, detail, team_run_id) VALUES (?1,?2,?3,?4,?5)",
                params![ts_ms as i64, agent_id, kind, text, run_id],
            )?;
            Ok(())
        });
    }

    /// Events with `ts >= since_ms`, newest first, optionally for one agent or kind prefix.
    pub fn events(
        &self,
        since_ms: u64,
        agent_id: Option<&str>,
        kind_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Event>> {
        let agent_id = agent_id.map(str::to_string);
        let like = kind_prefix.map(|k| format!("{}%", k.replace('%', "")));
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, ts_ms, agent_id, kind, detail FROM events
                 WHERE ts_ms >= ?1 AND (?2 IS NULL OR agent_id = ?2) AND (?3 IS NULL OR kind LIKE ?3)
                 ORDER BY ts_ms DESC, id DESC LIMIT ?4",
            )?;
            Ok(st
                .query_map(params![since_ms as i64, agent_id, like, limit as i64], row_event)?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    /// A team run's timeline (FR-408), newest first.
    pub fn run_events(&self, run_id: &str, limit: usize) -> Result<Vec<Event>> {
        let run = run_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, ts_ms, agent_id, kind, detail FROM events WHERE team_run_id=?1
                 ORDER BY ts_ms DESC, id DESC LIMIT ?2",
            )?;
            Ok(st.query_map(params![run, limit as i64], row_event)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Appends to the audit log and waits for the commit (a decision is not
    /// acted on before it is recorded).
    pub fn audit(
        &self,
        actor: &str,
        agent_id: Option<&str>,
        action: &str,
        target: Option<&str>,
        decision: &str,
        detail: Value,
    ) -> Result<u64> {
        let (actor, agent_id, action, target, decision) = (
            actor.to_string(),
            agent_id.map(str::to_string),
            action.to_string(),
            target.map(str::to_string),
            decision.to_string(),
        );
        let now = crate::now_ms();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO audit_log(ts_ms, actor, agent_id, action, target, decision, detail) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![now as i64, actor, agent_id, action, target, decision, serde_json::to_string(&detail)?],
            )?;
            Ok(tx.last_insert_rowid() as u64)
        })
    }

    /// The audit log, newest first.
    pub fn audit_log(&self, agent_id: Option<&str>, limit: usize) -> Result<Vec<AuditEntry>> {
        let agent_id = agent_id.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, ts_ms, actor, agent_id, action, target, decision, detail FROM audit_log
                 WHERE ?1 IS NULL OR agent_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            Ok(st
                .query_map(params![agent_id, limit as i64], |r| {
                    Ok(AuditEntry {
                        id: r.get::<_, i64>(0)? as u64,
                        ts_ms: r.get::<_, i64>(1)? as u64,
                        actor: r.get(2)?,
                        agent_id: r.get(3)?,
                        action: r.get(4)?,
                        target: r.get(5)?,
                        decision: r.get(6)?,
                        detail: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(Value::Null),
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Deletes rows past their retention (and the blobs of deleted events).
    /// Returns how many rows went. The audit log is never purged.
    pub fn purge(&self, now_ms: u64, r: Retention) -> Result<usize> {
        let cut = move |days: u32| now_ms.saturating_sub(days as u64 * DAY_MS) as i64;
        self.write(move |tx| {
            let mut blobs: Vec<PathBuf> = vec![];
            {
                let mut st = tx.prepare("SELECT detail FROM events WHERE ts_ms < ?1 AND detail LIKE '{\"blob\":%'")?;
                let rows = st.query_map([cut(r.events_days)], |row| row.get::<_, String>(0))?;
                for d in rows.flatten() {
                    if let Some(p) =
                        serde_json::from_str::<Value>(&d).ok().and_then(|v| v["blob"].as_str().map(PathBuf::from))
                    {
                        blobs.push(p);
                    }
                }
            }
            let mut n = tx.execute("DELETE FROM events WHERE ts_ms < ?1", [cut(r.events_days)])?;
            n += tx.execute("DELETE FROM forecast_snapshots WHERE ts_ms < ?1", [cut(r.forecasts_days)])?;
            n += tx.execute("DELETE FROM usage_samples WHERE ts_ms < ?1", [cut(r.usage_days)])?;
            n += tx.execute(
                "DELETE FROM inbox_items WHERE status <> 'pending' AND created_at_ms < ?1",
                [cut(r.inbox_days)],
            )?;
            for b in blobs {
                let _ = std::fs::remove_file(b);
            }
            Ok(n)
        })
    }
}

/// A short unique-enough suffix for blob file names (no extra dependency).
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_blobs_retention_and_audit() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let now = 100 * DAY_MS;
        s.add_event(now - 40 * DAY_MS, Some("a1"), "hook.Stop", json!({"x": 1}));
        s.add_event(now - 40 * DAY_MS, Some("a1"), "big", json!({"s": "x".repeat(BLOB_THRESHOLD + 10)}));
        s.add_event(now - DAY_MS, Some("a1"), "hook.PreToolUse", json!({"tool": "Bash"}));
        s.add_event(now, None, "reconciled", json!({}));
        s.flush().unwrap();
        let all = s.events(0, None, None, 10).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].kind, "reconciled", "newest first");
        let big = all.iter().find(|e| e.kind == "big").unwrap();
        let blob = PathBuf::from(big.detail["blob"].as_str().unwrap());
        assert!(blob.exists());
        assert_eq!(s.events(0, Some("a1"), Some("hook."), 10).unwrap().len(), 2);

        s.audit("user", Some("a1"), "permission", Some("Bash(ls)"), "allow", json!({"inbox": 3})).unwrap();
        let purged = s.purge(now, Retention::default()).unwrap();
        assert_eq!(purged, 2);
        assert!(!blob.exists(), "blob of a purged event is removed");
        assert_eq!(s.events(0, None, None, 10).unwrap().len(), 2);
        let log = s.audit_log(Some("a1"), 10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].decision, "allow");
    }
}
