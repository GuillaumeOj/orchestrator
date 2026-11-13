//! P4.5 intake (FR-1312): a project's intake source (`projects.intake_source`)
//! and the cache of its items (`work_nodes`, migration 17). A refresh
//! replaces the project's items with what the source lists now: an item gone
//! upstream is deleted unless it was started (kept for the report); a hidden
//! one comes back when it changes upstream.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Store;
use crate::teams::IntakeSource;
use crate::util::{js, opt_u64};

/// The `now` topic intake writes touch.
pub const TOPIC: &str = "intake";

/// The `source` of a started planned brief's row (its `ref` is the
/// decision id): what gives a run or session its brief origin.
pub const BRIEF_SOURCE: &str = "brief";

pub(crate) fn status_key(project: &str) -> String {
    format!("intake.status:{project}")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeState {
    #[default]
    Ready,
    Started,
    Hidden,
}

impl IntakeState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntakeState::Ready => "ready",
            IntakeState::Started => "started",
            IntakeState::Hidden => "hidden",
        }
    }

    pub fn parse(s: &str) -> IntakeState {
        match s {
            "started" => IntakeState::Started,
            "hidden" => IntakeState::Hidden,
            _ => IntakeState::Ready,
        }
    }
}

/// One cached item.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IntakeRow {
    pub id: u64,
    pub project_id: String,
    /// `github` | `linear` | `repo_docs`.
    pub source: String,
    /// The display ref: `#123`, `ENG-42`, a repo path.
    pub reference: String,
    pub title: String,
    pub url: Option<String>,
    /// Markdown, at most 4 000 chars (`meta.truncated` when cut).
    pub body: String,
    /// Labels, size, mode, team, `updated_at_ms`… (the source's).
    pub meta: Value,
    /// The revision upstream: `updated_at`, or a doc's blob id.
    pub rev: String,
    pub fetched_at_ms: u64,
    pub position: i64,
    pub state: IntakeState,
    pub started_kind: Option<String>,
    pub started_ref: Option<String>,
    pub hidden_at_ms: Option<u64>,
    pub hidden_rev: Option<String>,
    /// `gh:acme/shop#12`, `lin:ENG-42`, `local:<path>`… (contract §2).
    pub node_id: String,
    /// `issue` | `milestone` | `linear_project` | `linear_milestone` | `initiative` | `local` | `brief`.
    pub kind: String,
    pub parent_id: Option<String>,
    /// 1 for a root (the project row is 0).
    pub depth: u32,
    /// Gone upstream, kept because a run or session started from it.
    pub deleted: bool,
}

/// A node's place in its tree, without its body and facts (the breadcrumbs
/// of the lane read every node of every project).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkIndex {
    pub project_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub reference: String,
    pub title: String,
}

/// What a refresh writes for one item, in the source's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntakeUpsert {
    /// Empty: `<source>:<reference>`.
    pub node_id: String,
    /// Empty: `issue`.
    pub kind: String,
    pub parent_id: Option<String>,
    pub reference: String,
    pub title: String,
    pub url: Option<String>,
    pub body: String,
    pub meta: Value,
    pub rev: String,
}

/// A project's last refresh (kv `intake.status:<project>`): kept so the
/// lane shows "updated 12 min ago" and the last error at start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IntakeStatus {
    /// The last successful refresh.
    #[serde(default)]
    pub refreshed_at_ms: Option<u64>,
    /// `{code, message, fix}` of the last refresh when it failed.
    #[serde(default)]
    pub error: Option<Value>,
    /// Items the source listed past the cap.
    #[serde(default)]
    pub more: usize,
    /// The last refresh stopped at the read bound: `more` counts only what
    /// was read.
    #[serde(default)]
    pub partial: bool,
    /// Repo docs: the branch tip the cached items were read at.
    #[serde(default)]
    pub tip: Option<String>,
    /// The source's own view when the source said where it is (a Projects
    /// v2 board, a Linear team or project): kept across a failed refresh.
    #[serde(default)]
    pub url: Option<String>,
    /// `gh_sub_issues`: the tree has no hierarchy (F26).
    #[serde(default)]
    pub warning: Option<String>,
    /// The source could not be reached: the cache is shown (F15).
    #[serde(default)]
    pub offline: bool,
}

const COLS: &str = "id, project_id, source, ref, title, url, body, meta, updated_at, fetched_at_ms, position, state, \
                    started_kind, started_ref, hidden_at_ms, hidden_rev, node_id, kind, parent_id, depth, deleted";

/// [`COLS`] without the body (empty): for reads of the whole tree.
const COLS_LIGHT: &str = "id, project_id, source, ref, title, url, '' AS body, meta, updated_at, fetched_at_ms, position, state, \
                    started_kind, started_ref, hidden_at_ms, hidden_rev, node_id, kind, parent_id, depth, deleted";

/// One entry of a node's history in the app.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkEvent {
    pub at_ms: u64,
    pub what: String,
    pub feature: Option<String>,
    pub program: Option<Value>,
    pub detail: Option<String>,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<IntakeRow> {
    Ok(IntakeRow {
        id: r.get::<_, i64>(0)? as u64,
        project_id: r.get(1)?,
        source: r.get(2)?,
        reference: r.get(3)?,
        title: r.get(4)?,
        url: r.get(5)?,
        body: r.get(6)?,
        meta: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(Value::Null),
        rev: r.get(8)?,
        fetched_at_ms: r.get::<_, i64>(9)? as u64,
        position: r.get(10)?,
        state: IntakeState::parse(&r.get::<_, String>(11)?),
        started_kind: r.get(12)?,
        started_ref: r.get(13)?,
        hidden_at_ms: opt_u64(r, 14)?,
        hidden_rev: r.get(15)?,
        node_id: r.get(16)?,
        kind: r.get(17)?,
        parent_id: r.get(18)?,
        depth: r.get::<_, i64>(19)? as u32,
        deleted: r.get::<_, i64>(20)? != 0,
    })
}

impl Store {
    /// The project's intake source, as the tracker's Ready filter states it
    /// (`None` when the filter matches by nothing).
    pub fn intake_source(&self, project: &str) -> Result<IntakeSource> {
        Ok(self.tracker(project)?.intake_source())
    }

    /// Adapter of the old command: stores `source` into the tracker (one
    /// tracker, see `TrackerSettings::with_intake`). A changed source drops
    /// the items of the old one (but the started ones) and its status. True
    /// when it changed.
    pub fn set_intake_source(&self, project: &str, source: &IntakeSource) -> Result<bool> {
        let new = self.tracker(project)?.with_intake(source);
        self.set_tracker(project, &new)
    }

    /// Cached items, of one project or all, in their lane order.
    pub fn work_nodes(&self, project: Option<&str>) -> Result<Vec<IntakeRow>> {
        let p = project.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS} FROM work_nodes WHERE ?1 IS NULL OR project_id=?1 ORDER BY project_id, position, id"
            ))?;
            Ok(st.query_map([p], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The nodes of one project (the brief rows apart), without their bodies.
    pub fn work_nodes_light(&self, project: &str) -> Result<Vec<IntakeRow>> {
        let p = project.to_string();
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS_LIGHT} FROM work_nodes WHERE project_id=?1 AND source!=?2 ORDER BY position, id"
            ))?;
            Ok(st.query_map(params![p, BRIEF_SOURCE], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The nodes of every project whose `node_id` is `node`.
    pub fn work_node_rows(&self, node: &str) -> Result<Vec<IntakeRow>> {
        let n = node.to_string();
        self.read(move |c| {
            let mut st = c.prepare(&format!("SELECT {COLS} FROM work_nodes WHERE node_id=?1 AND source!=?2"))?;
            Ok(st.query_map(params![n, BRIEF_SOURCE], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The node a run was started from.
    pub fn work_node_of_run(&self, run: &str) -> Result<Option<IntakeRow>> {
        let r = run.to_string();
        self.read(move |c| {
            Ok(c.query_row(
                &format!("SELECT {COLS} FROM work_nodes WHERE state='started' AND started_kind='run' AND started_ref=?1 AND source!=?2"),
                params![r, BRIEF_SOURCE],
                row,
            )
            .optional()?)
        })
    }

    /// A node that ran before (its run merged) starts again: the new run is its mark.
    pub fn intake_restart(&self, id: u64, kind: &str, reference: &str) -> Result<bool> {
        let (kind, reference) = (kind.to_string(), reference.to_string());
        let n = self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE work_nodes SET started_kind=?2, started_ref=?3 WHERE id=?1 AND state='started'",
                params![id as i64, kind, reference],
            )?)
        })?;
        if n > 0 {
            self.note([TOPIC]);
        }
        Ok(n > 0)
    }

    /// Adds an entry to a node's history.
    pub fn work_note(&self, project: &str, node: &str, e: &WorkEvent) -> Result<()> {
        let (p, n, e) = (project.to_string(), node.to_string(), e.clone());
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO work_history(project_id, node_id, at_ms, what, feature, program, detail)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![p, n, e.at_ms as i64, e.what, e.feature, e.program.as_ref().map(js), e.detail],
            )?;
            Ok(())
        })
    }

    /// A node's history, oldest first.
    pub fn work_history(&self, project: &str, node: &str) -> Result<Vec<WorkEvent>> {
        let (p, n) = (project.to_string(), node.to_string());
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT at_ms, what, feature, program, detail FROM work_history
                 WHERE project_id=?1 AND node_id=?2 ORDER BY at_ms, id",
            )?;
            let rows = st.query_map(params![p, n], |r| {
                Ok(WorkEvent {
                    at_ms: r.get::<_, i64>(0)? as u64,
                    what: r.get(1)?,
                    feature: r.get(2)?,
                    program: r.get::<_, Option<String>>(3)?.and_then(|s| serde_json::from_str(&s).ok()),
                    detail: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Every node's place in its tree (no body, no facts), of every project.
    pub fn work_index(&self) -> Result<Vec<WorkIndex>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT project_id, node_id, parent_id, ref, title FROM work_nodes WHERE source!=?1 AND deleted=0",
            )?;
            let rows = st.query_map([BRIEF_SOURCE], |r| {
                Ok(WorkIndex {
                    project_id: r.get(0)?,
                    node_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    reference: r.get(3)?,
                    title: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn intake_item(&self, id: u64) -> Result<Option<IntakeRow>> {
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM work_nodes WHERE id=?1"), [id as i64], row).optional()?)
        })
    }

    /// Writes what `source` (the tracker kind) lists now, in that order, as
    /// `project`'s nodes. `full`: the listing is the whole tree, so a node
    /// not in it is gone upstream (deleted, unless started: kept, flagged
    /// `deleted`); otherwise (an incremental read) nothing is removed. A
    /// legacy row (`intake_items` before B2) is adopted by its source and ref.
    /// A hidden node whose revision moved is ready again. True when something
    /// changed (a fetch time alone is not a change).
    pub fn intake_apply(
        &self,
        project: &str,
        source: &str,
        items: Vec<IntakeUpsert>,
        full: bool,
        now_ms: u64,
    ) -> Result<bool> {
        let (id, source) = (project.to_string(), source.to_string());
        let changed = self.write(move |tx| {
            let mut changed = false;
            let mut keep: Vec<i64> = Vec::with_capacity(items.len());
            for (pos, it) in items.iter().enumerate() {
                let node_id =
                    if it.node_id.is_empty() { format!("{source}:{}", it.reference) } else { it.node_id.clone() };
                let kind = if it.kind.is_empty() { "issue" } else { it.kind.as_str() };
                let have = tx
                    .query_row(
                        &format!("SELECT {COLS} FROM work_nodes WHERE project_id=?1 AND node_id=?2"),
                        params![id, node_id],
                        row,
                    )
                    .optional()?;
                // A row of before the tree: same source and ref, adopted.
                let have = match have {
                    Some(h) => Some(h),
                    None => tx
                        .query_row(
                            &format!(
                                "SELECT {COLS} FROM work_nodes WHERE project_id=?1 AND source=?2 AND ref=?3
                                   AND node_id LIKE 'legacy:%'"
                            ),
                            params![id, source, it.reference],
                            row,
                        )
                        .optional()?,
                };
                let meta = js(&it.meta);
                match have {
                    None => {
                        // An incremental read does not know the place in the whole order: last.
                        let at = if full {
                            pos as i64
                        } else {
                            tx.query_row(
                                "SELECT coalesce(max(position) + 1, 0) FROM work_nodes WHERE project_id=?1",
                                [&id],
                                |r| r.get(0),
                            )?
                        };
                        tx.execute(
                            "INSERT INTO work_nodes(project_id, node_id, source, kind, ref, title, url, body, meta,
                               updated_at, fetched_at_ms, position, state, parent_id)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'ready',?13)",
                            params![
                                id,
                                node_id,
                                source,
                                kind,
                                it.reference,
                                it.title,
                                it.url,
                                it.body,
                                meta,
                                it.rev,
                                now_ms as i64,
                                at,
                                it.parent_id
                            ],
                        )?;
                        keep.push(tx.last_insert_rowid());
                        changed = true;
                    }
                    Some(h) => {
                        // Hidden until it changes upstream (spec §3.3 "Not now").
                        let unhide = h.state == IntakeState::Hidden && h.hidden_rev.as_deref() != Some(&it.rev);
                        let same = h.node_id == node_id
                            && h.kind == kind
                            && h.title == it.title
                            && h.url == it.url
                            && h.body == it.body
                            && h.meta == it.meta
                            && h.rev == it.rev
                            && h.parent_id == it.parent_id
                            && !h.deleted
                            && (h.position == pos as i64 || !full);
                        changed |= unhide || !same;
                        let state = if unhide { IntakeState::Ready } else { h.state };
                        // An incremental read does not know the node's place in the whole order.
                        let position = if full { pos as i64 } else { h.position };
                        tx.execute(
                            "UPDATE work_nodes SET node_id=?11, kind=?12, title=?2, url=?3, body=?4, meta=?5,
                               updated_at=?6, fetched_at_ms=?7, position=?8, state=?9, parent_id=?13, deleted=0,
                               hidden_at_ms=CASE WHEN ?10 THEN NULL ELSE hidden_at_ms END,
                               hidden_rev=CASE WHEN ?10 THEN NULL ELSE hidden_rev END
                             WHERE id=?1",
                            params![
                                h.id as i64,
                                it.title,
                                it.url,
                                it.body,
                                meta,
                                it.rev,
                                now_ms as i64,
                                position,
                                state.as_str(),
                                unhide,
                                node_id,
                                kind,
                                it.parent_id
                            ],
                        )?;
                        keep.push(h.id as i64);
                    }
                }
            }
            let mut gone = 0;
            if full {
                // Gone upstream (or of another source): deleted; a started one is kept, flagged.
                let keep = js(&keep);
                let not_kept = "project_id=?1 AND source!=?3 AND id NOT IN (SELECT value FROM json_each(?2))";
                gone += tx.execute(
                    &format!("UPDATE work_nodes SET deleted=1 WHERE {not_kept} AND state='started' AND deleted=0"),
                    params![id, keep, BRIEF_SOURCE],
                )?;
                gone += tx.execute(
                    &format!("DELETE FROM work_nodes WHERE {not_kept} AND state!='started'"),
                    params![id, keep, BRIEF_SOURCE],
                )?;
            }
            // Depth from the parent chain (roots are 1; the project row is 0).
            tx.execute(
                "WITH RECURSIVE d(id, depth) AS (
                   SELECT node_id, 1 FROM work_nodes WHERE project_id=?1 AND (parent_id IS NULL
                     OR parent_id NOT IN (SELECT node_id FROM work_nodes WHERE project_id=?1))
                   UNION ALL
                   SELECT w.node_id, d.depth + 1 FROM work_nodes w JOIN d ON w.parent_id = d.id
                   WHERE w.project_id=?1 AND d.depth < 50)
                 UPDATE work_nodes SET depth = (SELECT min(depth) FROM d WHERE d.id = work_nodes.node_id)
                 WHERE project_id=?1 AND source!=?2 AND node_id IN (SELECT id FROM d)",
                params![id, BRIEF_SOURCE],
            )?;
            Ok(changed || gone > 0)
        })?;
        if changed {
            self.note([TOPIC]);
        }
        Ok(changed)
    }

    /// "Not now": a ready item leaves the lane until it changes upstream.
    pub fn intake_hide(&self, id: u64, now_ms: u64) -> Result<bool> {
        let n = self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE work_nodes SET state='hidden', hidden_at_ms=?2, hidden_rev=updated_at
                   WHERE id=?1 AND state='ready'",
                params![id as i64, now_ms as i64],
            )?)
        })?;
        if n > 0 {
            self.note([TOPIC]);
        }
        Ok(n > 0)
    }

    pub fn intake_unhide(&self, id: u64) -> Result<bool> {
        let n = self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE work_nodes SET state='ready', hidden_at_ms=NULL, hidden_rev=NULL
                   WHERE id=?1 AND state='hidden'",
                [id as i64],
            )?)
        })?;
        if n > 0 {
            self.note([TOPIC]);
        }
        Ok(n > 0)
    }

    /// A ready item was started (spec §3.3): `kind` is `run` or `agent`,
    /// `reference` the run or agent id. False when it was not ready anymore.
    pub fn intake_mark_started(&self, id: u64, kind: &str, reference: &str) -> Result<bool> {
        let (kind, reference) = (kind.to_string(), reference.to_string());
        let n = self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE work_nodes SET state='started', started_kind=?2, started_ref=?3,
                   hidden_at_ms=NULL, hidden_rev=NULL WHERE id=?1 AND state IN ('ready', 'hidden')",
                params![id as i64, kind, reference],
            )?)
        })?;
        if n > 0 {
            self.note([TOPIC]);
        }
        Ok(n > 0)
    }

    /// A planned brief was started: kept as a started row of source
    /// [`BRIEF_SOURCE`] (ref = the decision id), so the run or session
    /// shows its origin. Never in the lane (not the project's source).
    pub fn intake_add_brief_start(
        &self,
        project: &str,
        decision: &str,
        title: &str,
        kind: &str,
        reference: &str,
        now_ms: u64,
    ) -> Result<u64> {
        let args = [project, decision, title, kind, reference].map(str::to_string);
        self.write(move |tx| {
            let [project, decision, title, kind, reference] = &args;
            tx.execute(
                "INSERT INTO work_nodes(project_id, node_id, source, kind, ref, title, updated_at, fetched_at_ms, state,
                   started_kind, started_ref) VALUES (?1, 'brief:' || ?3, ?2, 'brief', ?3, ?4, '', ?5, 'started', ?6, ?7)
                 ON CONFLICT(project_id, node_id) DO UPDATE SET title=?4, state='started',
                   started_kind=?6, started_ref=?7",
                params![project, BRIEF_SOURCE, decision, title, now_ms as i64, kind, reference],
            )?;
            Ok(tx.query_row(
                "SELECT id FROM work_nodes WHERE project_id=?1 AND source=?2 AND ref=?3",
                params![project, BRIEF_SOURCE, decision],
                |r| r.get::<_, i64>(0),
            )? as u64)
        })
        .inspect(|_| self.note([TOPIC]))
    }

    /// A run or session (`kind`, `reference`) was started from an item or brief.
    pub fn intake_started_by(&self, kind: &str, reference: &str) -> Result<bool> {
        let (kind, reference) = (kind.to_string(), reference.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT 1 FROM work_nodes WHERE state='started' AND started_kind=?1 AND started_ref=?2 LIMIT 1",
                params![kind, reference],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })
    }

    /// The items the lane lists (ready and hidden): the nodes the project's
    /// Ready filter matches, of one project or all, in their tree order.
    pub fn intake_listed(&self, project: Option<&str>) -> Result<Vec<IntakeRow>> {
        let p = project.map(str::to_string);
        let rows: Vec<IntakeRow> = self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS} FROM work_nodes WHERE state!='started' AND deleted=0 AND source!=?2
                   AND (?1 IS NULL OR project_id=?1) ORDER BY project_id, position, id"
            ))?;
            Ok(st.query_map(params![p, BRIEF_SOURCE], row)?.collect::<rusqlite::Result<_>>()?)
        })?;
        let mut ready: std::collections::HashMap<String, Option<crate::TrackerSettings>> = Default::default();
        Ok(rows
            .into_iter()
            .filter(|r| {
                let t = ready.entry(r.project_id.clone()).or_insert_with(|| self.tracker(&r.project_id).ok());
                t.as_ref().is_some_and(|t| t.is_ready(&r.kind, &r.meta))
            })
            .collect())
    }

    /// The started rows (items and briefs), oldest first.
    pub fn intake_started(&self) -> Result<Vec<IntakeRow>> {
        self.read(move |c| {
            let mut st = c.prepare(&format!("SELECT {COLS} FROM work_nodes WHERE state='started' ORDER BY id"))?;
            Ok(st.query_map([], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The run or session `kind`/`reference` went before any outcome: its
    /// items are ready again (a brief's row is dropped: the card stays
    /// closed). The items returned.
    pub fn intake_return(&self, kind: &str, reference: &str) -> Result<Vec<IntakeRow>> {
        let (kind, reference) = (kind.to_string(), reference.to_string());
        let back = self.write(move |tx| {
            let mut st = tx.prepare(&format!(
                "SELECT {COLS} FROM work_nodes WHERE state='started' AND started_kind=?1 AND started_ref=?2"
            ))?;
            let rows: Vec<IntakeRow> = st.query_map(params![kind, reference], row)?.collect::<rusqlite::Result<_>>()?;
            drop(st);
            for r in &rows {
                if r.source == BRIEF_SOURCE {
                    tx.execute("DELETE FROM work_nodes WHERE id=?1", [r.id as i64])?;
                } else {
                    tx.execute(
                        "UPDATE work_nodes SET state='ready', started_kind=NULL, started_ref=NULL WHERE id=?1",
                        [r.id as i64],
                    )?;
                }
            }
            Ok(rows.into_iter().filter(|r| r.source != BRIEF_SOURCE).collect::<Vec<_>>())
        })?;
        if !back.is_empty() {
            self.note([TOPIC]);
        }
        Ok(back)
    }

    pub fn intake_status(&self, project: &str) -> Result<IntakeStatus> {
        Ok(self.setting(&status_key(project))?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }

    /// Stores the project's refresh status; touches the topic when it changed.
    pub fn set_intake_status(&self, project: &str, status: &IntakeStatus) -> Result<()> {
        if self.intake_status(project)? == *status {
            return Ok(());
        }
        self.set_setting(&status_key(project), &serde_json::to_value(status)?)?;
        self.note([TOPIC]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::GithubIntakeFilter;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("o.db")).unwrap();
        s.upsert_project(&crate::entities::tests::project("p1", "/x/p1")).unwrap();
        (d, s)
    }

    fn item(r: &str, rev: &str) -> IntakeUpsert {
        IntakeUpsert {
            reference: r.into(),
            title: format!("T {r}"),
            url: None,
            body: "b".into(),
            meta: json!({"labels": [], "state": "open"}),
            rev: rev.into(),
            ..Default::default()
        }
    }

    #[test]
    fn source_round_trips_through_the_tracker_and_a_ready_change_keeps_the_tree() {
        let (_d, s) = store();
        assert_eq!(s.intake_source("p1").unwrap(), IntakeSource::None);
        let gh = IntakeSource::Github {
            repo: Some("acme/shop".into()),
            filter: GithubIntakeFilter::Label { names: vec!["ready".into()] },
        };
        assert!(s.set_intake_source("p1", &gh).unwrap());
        assert!(!s.set_intake_source("p1", &gh).unwrap());
        assert_eq!(s.intake_source("p1").unwrap(), gh);
        s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a")], true, 5).unwrap();
        let started = s.work_nodes(Some("p1")).unwrap()[0].id;
        s.write(move |tx| {
            tx.execute("UPDATE work_nodes SET state='started' WHERE id=?1", [started as i64])?;
            Ok(())
        })
        .unwrap();
        s.set_intake_status("p1", &IntakeStatus { refreshed_at_ms: Some(5), ..Default::default() }).unwrap();
        assert!(s.set_intake_source("p1", &IntakeSource::None).unwrap());
        // The Ready filter is gone, the tracker (and its cached tree) is not.
        assert_eq!(s.work_nodes(Some("p1")).unwrap().len(), 2);
        assert!(s.intake_listed(Some("p1")).unwrap().is_empty());
        assert_eq!(s.intake_status("p1").unwrap().refreshed_at_ms, Some(5));
        // Another repository: its tree is dropped, but the started node.
        assert!(
            s.set_intake_source(
                "p1",
                &IntakeSource::Github {
                    repo: Some("acme/web".into()),
                    filter: GithubIntakeFilter::Label { names: vec!["ready".into()] },
                }
            )
            .unwrap()
        );
        let left = s.work_nodes(Some("p1")).unwrap();
        assert_eq!(left.iter().map(|r| r.reference.as_str()).collect::<Vec<_>>(), ["#1"]);
        assert_eq!(s.intake_status("p1").unwrap(), IntakeStatus::default());
        assert!(s.intake_source("nope").is_err());
    }

    #[test]
    fn apply_orders_deletes_gone_keeps_started_and_unhides_on_change() {
        let (_d, s) = store();
        assert!(
            s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a"), item("#3", "a")], true, 1).unwrap()
        );
        // Same answer again: no change, only the fetch time moves.
        assert!(
            !s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a"), item("#3", "a")], true, 2).unwrap()
        );
        let rows = s.work_nodes(Some("p1")).unwrap();
        assert_eq!(rows[0].fetched_at_ms, 2);
        let (one, two, three) = (rows[0].id, rows[1].id, rows[2].id);
        assert!(s.intake_hide(two, 3).unwrap());
        assert!(!s.intake_hide(two, 3).unwrap(), "only a ready item hides");
        s.write(move |tx| {
            tx.execute("UPDATE work_nodes SET state='started' WHERE id=?1", [three as i64])?;
            Ok(())
        })
        .unwrap();
        // #3 is gone upstream but started: kept. #2 unchanged: still hidden.
        assert!(s.intake_apply("p1", "github", vec![item("#2", "a"), item("#1", "a")], true, 4).unwrap());
        let rows = s.work_nodes(Some("p1")).unwrap();
        let by = |id| rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(by(two).state, IntakeState::Hidden);
        assert_eq!((by(two).position, by(one).position), (0, 1), "the source's order");
        assert_eq!(by(three).state, IntakeState::Started);
        // #2 edited upstream: back in the lane, same id.
        s.intake_apply("p1", "github", vec![item("#2", "b"), item("#1", "a")], true, 5).unwrap();
        let r = s.intake_item(two).unwrap().unwrap();
        assert_eq!((r.state, r.hidden_rev, r.rev.as_str()), (IntakeState::Ready, None, "b"));
        // Unhide by hand.
        s.intake_hide(one, 6).unwrap();
        assert!(s.intake_unhide(one).unwrap());
        assert_eq!(s.intake_item(one).unwrap().unwrap().state, IntakeState::Ready);
        // Gone and not started: deleted.
        s.intake_apply("p1", "github", vec![], true, 7).unwrap();
        let left: Vec<u64> = s.work_nodes(Some("p1")).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(left, [three]);
        // A project's delete takes its cache.
        s.delete_project("p1").unwrap();
        assert!(s.work_nodes(None).unwrap().is_empty());
    }

    #[test]
    fn started_items_and_briefs_return_or_drop() {
        let (_d, s) = store();
        s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a")], true, 1).unwrap();
        let rows = s.work_nodes(Some("p1")).unwrap();
        let (one, two) = (rows[0].id, rows[1].id);
        assert!(s.intake_mark_started(one, "run", "run-1").unwrap());
        assert!(!s.intake_mark_started(one, "agent", "x").unwrap(), "only a ready item starts");
        s.intake_hide(two, 2).unwrap();
        assert!(!s.intake_mark_started(two, "agent", "x").unwrap(), "a hidden item does not start");
        let b = s.intake_add_brief_start("p1", "brief-4", "Plan", "agent", "solo-1", 3).unwrap();
        let started: Vec<(String, Option<String>)> =
            s.intake_started().unwrap().into_iter().map(|r| (r.source, r.started_ref)).collect();
        assert_eq!(started, [("github".into(), Some("run-1".into())), ("brief".into(), Some("solo-1".into()))]);
        // A refresh keeps both (started), and the brief row never counts as the source's.
        s.intake_apply("p1", "github", vec![item("#2", "a")], true, 4).unwrap();
        assert_eq!(s.intake_item(one).unwrap().unwrap().state, IntakeState::Started);
        assert!(s.intake_item(b).unwrap().is_some());
        // The run went before any outcome: #1 is ready again.
        let back = s.intake_return("run", "run-1").unwrap();
        assert_eq!(back.iter().map(|r| r.id).collect::<Vec<_>>(), [one]);
        let r = s.intake_item(one).unwrap().unwrap();
        assert_eq!((r.state, r.started_kind), (IntakeState::Ready, None));
        // A brief's row is dropped, not listed as returned.
        assert!(s.intake_return("agent", "solo-1").unwrap().is_empty());
        assert!(s.intake_item(b).unwrap().is_none());
    }

    #[test]
    fn status_is_kept_and_touches_on_change_only() {
        let (_d, s) = store();
        s.take_topics();
        let st = IntakeStatus { refreshed_at_ms: Some(9), more: 3, ..Default::default() };
        s.set_intake_status("p1", &st).unwrap();
        assert!(s.take_topics().contains(TOPIC));
        s.set_intake_status("p1", &st).unwrap();
        assert!(!s.take_topics().contains(TOPIC));
        assert_eq!(s.intake_status("p1").unwrap(), st);
    }

    fn node(id: &str, kind: &str, parent: Option<&str>, reference: &str, rev: &str, meta: Value) -> IntakeUpsert {
        IntakeUpsert {
            node_id: id.into(),
            kind: kind.into(),
            parent_id: parent.map(str::to_string),
            reference: reference.into(),
            title: format!("T {reference}"),
            rev: rev.into(),
            meta,
            ..Default::default()
        }
    }

    fn tree() -> Vec<IntakeUpsert> {
        let open = json!({"state": "open", "labels": ["ready"]});
        vec![
            node("gh-milestone:a/b/3", "milestone", None, "v2", "m", json!({"state": "open"})),
            node("gh:a/b#1", "issue", Some("gh-milestone:a/b/3"), "#1", "a", open.clone()),
            node("gh:a/b#2", "issue", Some("gh:a/b#1"), "#2", "a", open),
        ]
    }

    #[test]
    fn a_tree_gets_depths_and_adopts_the_marks_of_the_rows_before_it() {
        let (_d, s) = store();
        // A row of before the tree (`intake_items` migrated to a legacy node id), hidden.
        s.write(|tx| {
            tx.execute(
                "INSERT INTO work_nodes(project_id, node_id, source, kind, ref, title, updated_at, fetched_at_ms,
                   state, hidden_at_ms, hidden_rev) VALUES ('p1', 'legacy:github:#1', 'github', 'issue', '#1', 'old',
                   'a', 1, 'hidden', 5, 'a')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let old_id = s.work_nodes(Some("p1")).unwrap()[0].id;
        assert!(s.intake_apply("p1", "github", tree(), true, 10).unwrap());
        let rows = s.work_nodes(Some("p1")).unwrap();
        let by = |r: &str| rows.iter().find(|x| x.reference == r).unwrap();
        assert_eq!((by("#1").id, by("#1").state, by("#1").node_id.as_str()), (old_id, IntakeState::Hidden, "gh:a/b#1"));
        assert_eq!((by("v2").depth, by("#1").depth, by("#2").depth), (1, 2, 3), "roots are 1");
        assert_eq!(by("#2").parent_id.as_deref(), Some("gh:a/b#1"));
        assert_eq!(rows.len(), 3, "the legacy row was adopted, not duplicated");
        // Unchanged: nothing to report. A changed revision shows the hidden one again.
        assert!(!s.intake_apply("p1", "github", tree(), true, 11).unwrap());
        let mut t = tree();
        t[1].rev = "b".into();
        assert!(s.intake_apply("p1", "github", t, true, 12).unwrap());
        assert_eq!(
            s.work_nodes(Some("p1")).unwrap().iter().find(|x| x.reference == "#1").unwrap().state,
            IntakeState::Ready
        );
    }

    #[test]
    fn an_incremental_read_removes_nothing_and_a_full_one_flags_the_started_node_it_lost() {
        let (_d, s) = store();
        s.intake_apply("p1", "github", tree(), true, 1).unwrap();
        let id2 = s.work_nodes(Some("p1")).unwrap().iter().find(|x| x.reference == "#2").unwrap().id;
        assert!(s.intake_mark_started(id2, "run", "r1").unwrap());
        // Incremental: only #1 changed; the others stay, in their place.
        let mut one = tree().remove(1);
        one.title = "renamed".into();
        one.rev = "z".into();
        assert!(s.intake_apply("p1", "github", vec![one], false, 2).unwrap());
        let rows = s.work_nodes(Some("p1")).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|r| r.position).collect::<Vec<_>>(), [0, 1, 2], "the order is the full read's");
        // A full read without #1 and #2 and the milestone: #2 (started) is kept, flagged; the rest is gone.
        s.intake_apply("p1", "github", vec![], true, 3).unwrap();
        let rows = s.work_nodes(Some("p1")).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].deleted && rows[0].reference == "#2" && rows[0].state == IntakeState::Started);
        // It is not in the lane; a brief's row is never touched by a read.
        s.intake_add_brief_start("p1", "d1", "A brief", "run", "r2", 4).unwrap();
        s.intake_apply("p1", "github", vec![], true, 5).unwrap();
        assert_eq!(s.work_nodes(Some("p1")).unwrap().len(), 2);
        assert!(s.intake_listed(Some("p1")).unwrap().is_empty());
    }

    #[test]
    fn the_lane_is_the_ready_filter_over_the_tree() {
        let (_d, s) = store();
        let mut t = s.tracker("p1").unwrap();
        t.tracker = crate::Tracker::Github { repo: Some("a/b".into()), project_board: None };
        t.ready = crate::ReadyFilter::defaults("github");
        s.set_tracker("p1", &t).unwrap();
        let mut nodes = tree();
        nodes.push(node("gh:a/b#3", "issue", None, "#3", "a", json!({"state": "open", "labels": ["bug"]})));
        nodes.push(node("gh:a/b#4", "issue", None, "#4", "a", json!({"state": "closed", "labels": ["ready"]})));
        s.intake_apply("p1", "github", nodes, true, 1).unwrap();
        let lane =
            |s: &Store| s.intake_listed(Some("p1")).unwrap().iter().map(|r| r.reference.clone()).collect::<Vec<_>>();
        assert_eq!(lane(&s), ["#1", "#2"], "open issues with the label, parents too; no milestone, no closed");
        // The filter is read when the lane is: no refetch to change it, and the cache is kept.
        t.ready = crate::ReadyFilter::Github {
            labels: vec!["bug".into()],
            milestone: None,
            board_status: None,
            issue_type: None,
        };
        s.set_tracker("p1", &t).unwrap();
        assert_eq!(lane(&s), ["#3"]);
        assert_eq!(s.work_nodes(Some("p1")).unwrap().len(), 5);
        // Another tracker: the old tree is dropped (a started node stays).
        t.tracker = crate::Tracker::Github { repo: Some("a/other".into()), project_board: None };
        s.set_tracker("p1", &t).unwrap();
        assert!(s.work_nodes(Some("p1")).unwrap().is_empty());
    }
}
