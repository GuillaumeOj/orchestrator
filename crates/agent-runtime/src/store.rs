//! The core's persistent state. A single JSON file written atomically after
//! every change: a stand-in for ADR-005's SQLite that keeps the same rows
//! (agents, worktrees, turns, holds, inbox), so the reconciler logic carries over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
pub enum WorktreeRowState {
    Creating,
    Ready,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorktreeRow {
    pub path: PathBuf,
    pub repo: PathBuf,
    pub branch: String,
    pub base: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub state: WorktreeRowState,
    pub created_at_ms: u64,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: u64,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// gate / alert / permission / budget …
    pub kind: String,
    pub body: String,
    pub pending: bool,
    pub created_at_ms: u64,
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
        keep_last(&mut self.inbox, |i| !i.pending);
    }

    pub fn add_inbox(&mut self, agent_id: Option<&str>, kind: &str, body: String) -> u64 {
        let id = self.inbox.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        self.inbox.push(InboxItem {
            id,
            agent_id: agent_id.map(str::to_string),
            kind: kind.into(),
            body,
            pending: true,
            created_at_ms: orch_protocol::now_ms(),
        });
        id
    }
}

pub struct Store {
    path: Option<PathBuf>,
    data: Mutex<Data>,
}

struct Data {
    state: StateFile,
    /// What is on disk, so a write that changed nothing costs no I/O.
    saved: Vec<u8>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (state, saved) = match std::fs::read(path) {
            Ok(b) => (serde_json::from_slice(&b).with_context(|| format!("reading {}", path.display()))?, b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (StateFile::default(), vec![]),
            Err(e) => return Err(e.into()),
        };
        Ok(Store { path: Some(path.to_path_buf()), data: Mutex::new(Data { state, saved }) })
    }

    /// Not persisted (tests).
    pub fn in_memory(data: StateFile) -> Store {
        Store { path: None, data: Mutex::new(Data { state: data, saved: vec![] }) }
    }

    pub fn read<R>(&self, f: impl FnOnce(&StateFile) -> R) -> R {
        f(&self.data.lock().unwrap_or_else(|e| e.into_inner()).state)
    }

    /// Mutates and saves. A failed save is returned but the change stays in memory.
    pub fn write<R>(&self, f: impl FnOnce(&mut StateFile) -> R) -> Result<R> {
        let mut g = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut g.state);
        g.state.prune();
        if let Some(p) = &self.path {
            let bytes = serde_json::to_vec(&g.state)?;
            if bytes != g.saved {
                crate::compile::write_atomic(p, &bytes)?;
                g.saved = bytes;
            }
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_atomically() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state/state.json");
        let s = Store::open(&p).unwrap();
        s.write(|st| st.add_inbox(Some("a"), "alert", "x".into())).unwrap();
        let s2 = Store::open(&p).unwrap();
        assert_eq!(s2.read(|st| st.inbox.len()), 1);
        assert_eq!(s2.write(|st| st.add_inbox(None, "gate", "y".into())).unwrap(), 2);
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
