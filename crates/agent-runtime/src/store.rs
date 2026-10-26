//! The runtime's state (agents, worktrees, turns, holds, inbox): kept in
//! memory for the hot paths (hooks, the 200 ms delivery pump) and written
//! through to SQLite ([`orch_store`], ADR-005) on every change, one
//! transaction per [`Store::write`], so a crash never loses or half-applies a
//! change. Other code must change these rows through the runtime.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
pub use orch_store::{InboxItem, InboxStatus, WorktreeRow, WorktreeRowState};
use orch_store::{AgentRow, HoldRowDb, RuntimeDiff, RuntimeRows, TurnRowDb};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::role::Role;
use crate::state::{self, Tracker};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub role: Role,
    pub session_id: String,
    #[serde(default)]
    pub tracker: Tracker,
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default)]
    pub pgid: Option<i32>,
    pub allocation: String,
    pub tier: governor::Tier,
    pub created_at_ms: u64,
    #[serde(default)]
    pub last_event: Option<String>,
    #[serde(default)]
    pub last_event_at_ms: Option<u64>,
    #[serde(default)]
    pub spawns: u32,
    /// Set by the reconciler when the agent needs the user (merge in progress…).
    #[serde(default)]
    pub flag: Option<String>,
    /// The CLI's trust dialog was seen on the PTY (J0): only the user answers it.
    #[serde(default)]
    pub trust_dialog: bool,
}

impl AgentRecord {
    /// A new `claude` process was spawned for this agent.
    pub fn on_spawned(&mut self, pid: i32, pgid: i32) {
        self.pid = Some(pid);
        self.pgid = Some(pgid);
        self.spawns += 1;
        self.flag = None;
        self.trust_dialog = false;
        self.tracker.state = crate::state::AgentState::Starting;
        self.tracker.stop_intent = crate::state::StopIntent::None;
        self.tracker.background_tasks = 0;
    }

    /// The agent's process is gone; its stop intent decides the state.
    pub fn on_process_gone(&mut self) {
        state::on_exit(&mut self.tracker);
        self.pid = None;
        self.pgid = None;
        self.trust_dialog = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// Waiting for the Governor or for the agent to be idle.
    Queued,
    /// Pasted into the PTY; its marker not seen yet.
    Delivering,
    /// Its marker came back in UserPromptSubmit (or was found in the transcript).
    Delivered,
    Done,
    Cancelled,
}

/// Where a turn stands with the Governor (admission point 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "admission", content = "entry", rename_all = "snake_case")]
pub enum Admission {
    /// Not asked yet: asked when the agent can take it.
    #[default]
    Pending,
    /// In the Governor's queue as this entry, waiting for a release.
    Waiting(u64),
    Admitted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: String,
    pub agent_id: String,
    pub body: String,
    #[serde(default = "message_kind")]
    pub kind: governor::TurnKind,
    pub status: TurnStatus,
    pub created_at_ms: u64,
    #[serde(default)]
    pub delivered_at_ms: Option<u64>,
    #[serde(default)]
    pub admission: Admission,
}

fn message_kind() -> governor::TurnKind {
    governor::TurnKind::Message
}

impl TurnRow {
    /// A new queued turn with a fresh turn id.
    pub fn queued(agent_id: &str, body: &str, kind: governor::TurnKind, admission: Admission) -> TurnRow {
        TurnRow {
            id: crate::delivery::new_turn_id(),
            agent_id: agent_id.into(),
            body: body.into(),
            kind,
            status: TurnStatus::Queued,
            created_at_ms: orch_protocol::now_ms(),
            delivered_at_ms: None,
            admission,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldKind {
    /// Waiting for a user decision.
    Gate,
    /// Held by the Governor.
    Governor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldStatus {
    Held,
    Allowed,
    Denied,
    /// The hook's connection closed (Esc in the pane, or a core restart).
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldRow {
    pub id: String,
    pub agent_id: String,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    pub kind: HoldKind,
    pub status: HoldStatus,
    pub summary: String,
    pub created_at_ms: u64,
    /// The Inbox item a gate hold waits on.
    #[serde(default)]
    pub inbox_id: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentRecord>,
    #[serde(default)]
    pub worktrees: BTreeMap<PathBuf, WorktreeRow>,
    #[serde(default)]
    pub turns: Vec<TurnRow>,
    #[serde(default)]
    pub holds: Vec<HoldRow>,
    #[serde(default)]
    pub inbox: Vec<InboxItem>,
    /// Not persisted as such: rebuilt from the highest id at load.
    #[serde(skip)]
    pub next_inbox_id: u64,
}

/// Finished rows kept for display; older ones are dropped on write.
pub const KEEP_FINISHED: usize = 200;

impl StateFile {
    /// Drops the oldest finished turns, holds and Inbox items beyond
    /// [`KEEP_FINISHED`] each. Open rows are always kept.
    pub fn prune(&mut self) {
        fn keep_last<T>(v: &mut Vec<T>, finished: impl Fn(&T) -> bool) {
            let n = v.iter().filter(|x| finished(x)).count();
            let mut drop = n.saturating_sub(KEEP_FINISHED);
            v.retain(|x| {
                if drop > 0 && finished(x) {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
        keep_last(&mut self.turns, |t| matches!(t.status, TurnStatus::Done | TurnStatus::Cancelled));
        keep_last(&mut self.holds, |h| h.status != HoldStatus::Held);
        keep_last(&mut self.inbox, |i| !i.pending());
    }

    pub fn add_inbox(&mut self, agent_id: Option<&str>, kind: &str, body: String) -> u64 {
        self.add_inbox_item(agent_id, kind, body, Value::Null, 0)
    }

    /// Adds a pending Inbox item; returns its id. Ids are never reused, even
    /// after old items are pruned (`next_inbox_id`).
    pub fn add_inbox_item(&mut self, agent_id: Option<&str>, kind: &str, body: String, payload: Value, priority: i64) -> u64 {
        let id = self.next_inbox_id.max(self.inbox.iter().map(|i| i.id).max().unwrap_or(0) + 1);
        self.next_inbox_id = id + 1;
        let mut item = InboxItem::new(id, kind, agent_id, body);
        item.payload = payload;
        item.priority = priority;
        self.inbox.push(item);
        id
    }
}

pub struct Store {
    db: Option<orch_store::Store>,
    data: Mutex<Data>,
}

struct Data {
    state: StateFile,
    /// What the database holds (the diff base), so a failed write is retried
    /// with the next one.
    saved: StateFile,
}

// ------------------------------------------------------------ row mapping

fn tag<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn agent_row(a: &AgentRecord) -> AgentRow {
    AgentRow {
        id: a.id.clone(),
        repo: a.repo.clone(),
        worktree: a.worktree.clone(),
        session_id: a.session_id.clone(),
        state: tag(&a.tracker.state),
        pid: a.pid,
        tier: Some(tag(&a.tier)),
        allocation: Some(a.allocation.clone()),
        last_active_at_ms: a.last_event_at_ms,
        created_at_ms: a.created_at_ms,
        data: serde_json::to_value(a).unwrap_or(Value::Null),
    }
}

fn turn_row(t: &TurnRow) -> TurnRowDb {
    TurnRowDb {
        id: t.id.clone(),
        agent_id: t.agent_id.clone(),
        kind: tag(&t.kind),
        status: tag(&t.status),
        created_at_ms: t.created_at_ms,
        delivered_at_ms: t.delivered_at_ms,
        data: serde_json::to_value(t).unwrap_or(Value::Null),
    }
}

fn hold_row(h: &HoldRow) -> HoldRowDb {
    HoldRowDb {
        id: h.id.clone(),
        agent_id: h.agent_id.clone(),
        kind: tag(&h.kind),
        status: tag(&h.status),
        created_at_ms: h.created_at_ms,
        data: serde_json::to_value(h).unwrap_or(Value::Null),
    }
}

fn from_rows(rows: RuntimeRows) -> Result<StateFile> {
    let mut st = StateFile::default();
    for a in rows.agents {
        let rec: AgentRecord = serde_json::from_value(a.data).with_context(|| format!("agent row {}", a.id))?;
        st.agents.insert(rec.id.clone(), rec);
    }
    for w in rows.worktrees {
        st.worktrees.insert(w.path.clone(), w);
    }
    for t in rows.turns {
        st.turns.push(serde_json::from_value(t.data).with_context(|| format!("turn row {}", t.id))?);
    }
    for h in rows.holds {
        st.holds.push(serde_json::from_value(h.data).with_context(|| format!("hold row {}", h.id))?);
    }
    st.inbox = rows.inbox;
    st.next_inbox_id = st.inbox.iter().map(|i| i.id).max().unwrap_or(0) + 1;
    Ok(st)
}

/// What changed between two states, as row upserts and deletes.
pub fn diff(before: &StateFile, after: &StateFile) -> RuntimeDiff {
    let mut d = RuntimeDiff::default();
    for (id, a) in &after.agents {
        if before.agents.get(id) != Some(a) {
            d.agents.push(agent_row(a));
        }
    }
    d.agents_deleted = before.agents.keys().filter(|k| !after.agents.contains_key(*k)).cloned().collect();
    for (p, w) in &after.worktrees {
        if before.worktrees.get(p) != Some(w) {
            d.worktrees.push(w.clone());
        }
    }
    d.worktrees_deleted = before.worktrees.keys().filter(|k| !after.worktrees.contains_key(*k)).cloned().collect();
    fn by_id<T, K: Ord>(v: &[T], k: impl Fn(&T) -> K) -> BTreeMap<K, &T> {
        v.iter().map(|x| (k(x), x)).collect()
    }
    let bt = by_id(&before.turns, |t| t.id.clone());
    let at = by_id(&after.turns, |t| t.id.clone());
    d.turns = after.turns.iter().filter(|t| bt.get(&t.id) != Some(t)).map(turn_row).collect();
    d.turns_deleted = bt.keys().filter(|k| !at.contains_key(*k)).cloned().collect();
    let bh = by_id(&before.holds, |h| h.id.clone());
    let ah = by_id(&after.holds, |h| h.id.clone());
    d.holds = after.holds.iter().filter(|h| bh.get(&h.id) != Some(h)).map(hold_row).collect();
    d.holds_deleted = bh.keys().filter(|k| !ah.contains_key(*k)).cloned().collect();
    let bi = by_id(&before.inbox, |i| i.id);
    let ai = by_id(&after.inbox, |i| i.id);
    d.inbox = after.inbox.iter().filter(|i| bi.get(&i.id) != Some(i)).cloned().collect();
    d.inbox_deleted = bi.keys().filter(|k| !ai.contains_key(*k)).copied().collect();
    d
}

/// The JSON stand-in's Inbox rows (before SQLite): `pending` instead of a status.
#[derive(Deserialize)]
struct LegacyInbox {
    id: u64,
    #[serde(default)]
    agent_id: Option<String>,
    kind: String,
    body: String,
    pending: bool,
    created_at_ms: u64,
}

#[derive(Deserialize)]
struct LegacyState {
    #[serde(default)]
    agents: BTreeMap<String, AgentRecord>,
    #[serde(default)]
    worktrees: BTreeMap<PathBuf, WorktreeRow>,
    #[serde(default)]
    turns: Vec<TurnRow>,
    #[serde(default)]
    holds: Vec<HoldRow>,
    #[serde(default)]
    inbox: Vec<LegacyInbox>,
}

/// Reads the pre-SQLite `state/state.json` (one-time import).
pub fn read_legacy_json(path: &Path) -> Result<StateFile> {
    let l: LegacyState = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("reading {}", path.display()))?;
    let mut st = StateFile { agents: l.agents, worktrees: l.worktrees, turns: l.turns, holds: l.holds, ..Default::default() };
    for i in l.inbox {
        let mut item = InboxItem::new(i.id, &i.kind, i.agent_id.as_deref(), i.body);
        item.created_at_ms = i.created_at_ms;
        if !i.pending {
            item.status = InboxStatus::Dismissed;
        }
        st.inbox.push(item);
    }
    st.next_inbox_id = st.inbox.iter().map(|i| i.id).max().unwrap_or(0) + 1;
    Ok(st)
}

impl Store {
    /// Loads the runtime's rows from the database. On the first start with an
    /// empty database, a `state.json` left by the JSON stand-in is imported
    /// once and renamed `state.json.imported`.
    pub fn open(db: orch_store::Store, legacy_json: Option<&Path>) -> Result<Store> {
        if let Some(j) = legacy_json.filter(|j| j.is_file()) {
            if db.runtime_is_empty()? {
                let st = read_legacy_json(j)?;
                db.sync_runtime(diff(&StateFile::default(), &st))?;
            }
            std::fs::rename(j, j.with_extension("json.imported"))?;
        }
        let state = from_rows(db.load_runtime()?)?;
        Ok(Store { db: Some(db), data: Mutex::new(Data { saved: state.clone(), state }) })
    }

    /// Not persisted (tests).
    pub fn in_memory(mut data: StateFile) -> Store {
        data.next_inbox_id = data.inbox.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        Store { db: None, data: Mutex::new(Data { state: data, saved: StateFile::default() }) }
    }

    /// The database behind this store, if any.
    pub fn db(&self) -> Option<&orch_store::Store> {
        self.db.as_ref()
    }

    pub fn read<R>(&self, f: impl FnOnce(&StateFile) -> R) -> R {
        f(&self.data.lock().unwrap_or_else(|e| e.into_inner()).state)
    }

    /// Mutates, then writes the changed rows in one transaction. A failed
    /// write is returned and the change stays in memory; it is written with
    /// the next successful one (the diff is against what the database holds).
    pub fn write<R>(&self, f: impl FnOnce(&mut StateFile) -> R) -> Result<R> {
        let mut g = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut g.state);
        g.state.prune();
        if let Some(db) = &self.db {
            let d = diff(&g.saved, &g.state);
            if !d.is_empty() {
                db.sync_runtime(d).context("saving the runtime state")?;
                g.saved = g.state.clone();
            }
        }
        Ok(r)
    }
}

impl orch_git::worktree::WorktreeLedger for Store {
    fn get_row(&self, path: &Path) -> Option<WorktreeRow> {
        self.read(|s| s.worktrees.get(path).cloned())
    }

    fn put_row(&self, row: WorktreeRow) -> Result<()> {
        self.write(|s| {
            s.worktrees.insert(row.path.clone(), row);
        })
    }

    fn rows_of(&self, repo: &Path) -> Vec<WorktreeRow> {
        self.read(|s| s.worktrees.values().filter(|w| w.repo == repo).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(d: &tempfile::TempDir) -> orch_store::Store {
        orch_store::Store::open(&d.path().join("state/orch.db")).unwrap()
    }

    #[test]
    fn persists_through_sqlite() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = Store::open(db(&d), None).unwrap();
            s.write(|st| st.add_inbox(Some("a"), "alert", "x".into())).unwrap();
            s.write(|st| {
                st.turns.push(TurnRow::queued("a", "hi", governor::TurnKind::Message, Admission::Waiting(3)));
            })
            .unwrap();
        }
        let s2 = Store::open(db(&d), None).unwrap();
        assert_eq!(s2.read(|st| st.inbox.len()), 1);
        assert_eq!(s2.read(|st| st.turns[0].admission), Admission::Waiting(3));
        assert_eq!(s2.write(|st| st.add_inbox(None, "gate", "y".into())).unwrap(), 2);
        // Removing a row deletes it.
        s2.write(|st| st.turns.clear()).unwrap();
        assert!(Store::open(db(&d), None).unwrap().read(|st| st.turns.is_empty()));
    }

    #[test]
    fn imports_the_json_stand_in_once() {
        let d = tempfile::tempdir().unwrap();
        let json = d.path().join("state/state.json");
        std::fs::create_dir_all(json.parent().unwrap()).unwrap();
        std::fs::write(
            &json,
            r#"{"agents":{},"worktrees":{},"turns":[],"holds":[],
                "inbox":[{"id":4,"kind":"alert","body":"old","pending":false,"created_at_ms":1},
                         {"id":5,"kind":"gate","agent_id":"a1","body":"open","pending":true,"created_at_ms":2}]}"#,
        )
        .unwrap();
        let s = Store::open(db(&d), Some(&json)).unwrap();
        let inbox = s.read(|st| st.inbox.clone());
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0].status, InboxStatus::Dismissed);
        assert!(inbox[1].pending());
        assert!(!json.exists() && json.with_extension("json.imported").exists());
        assert_eq!(s.write(|st| st.add_inbox(None, "alert", "n".into())).unwrap(), 6);
    }

    #[test]
    fn prunes_old_finished_rows_only() {
        let s = Store::in_memory(StateFile::default());
        s.write(|st| {
            for i in 0..KEEP_FINISHED + 50 {
                let status = if i == 0 { TurnStatus::Queued } else { TurnStatus::Done };
                let mut t = TurnRow::queued("a", "", governor::TurnKind::Message, Admission::Pending);
                t.id = format!("{i}");
                t.status = status;
                st.turns.push(t);
            }
        })
        .unwrap();
        let turns = s.read(|st| st.turns.clone());
        assert_eq!(turns.len(), KEEP_FINISHED + 1);
        assert_eq!(turns[0].status, TurnStatus::Queued, "open rows are kept");
        assert_eq!(turns.last().unwrap().id, format!("{}", KEEP_FINISHED + 49), "newest kept");
    }
}
