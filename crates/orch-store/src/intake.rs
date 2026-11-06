//! P4.5 intake (FR-1312): a project's intake source (`projects.intake_source`)
//! and the cache of its items (`intake_items`, migration 17). A refresh
//! replaces the project's items with what the source lists now: an item gone
//! upstream is deleted unless it was started (kept for the report); a hidden
//! one comes back when it changes upstream.

use anyhow::{Result, bail};
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
}

/// What a refresh writes for one item, in the source's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntakeUpsert {
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
    /// Repo docs: the branch tip the cached items were read at.
    #[serde(default)]
    pub tip: Option<String>,
    /// The source's own view when the source said where it is (a Projects
    /// v2 board, a Linear team or project): kept across a failed refresh.
    #[serde(default)]
    pub url: Option<String>,
}

const COLS: &str = "id, project_id, source, ref, title, url, body, meta, updated_at, fetched_at_ms, position, state, \
                    started_kind, started_ref, hidden_at_ms, hidden_rev";

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
    })
}

impl Store {
    /// The project's intake source (`None` when unset or unreadable).
    pub fn intake_source(&self, project: &str) -> Result<IntakeSource> {
        let id = project.to_string();
        self.read(move |c| {
            let s: Option<Option<String>> =
                c.query_row("SELECT intake_source FROM projects WHERE id=?1", [&id], |r| r.get(0)).optional()?;
            let Some(s) = s else { bail!("no project {id}") };
            Ok(s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
        })
    }

    /// Stores `source` (validated by the caller). A changed source drops the
    /// items of the old one (but the started ones) and its status. True when
    /// it changed.
    pub fn set_intake_source(&self, project: &str, source: &IntakeSource) -> Result<bool> {
        let (id, text) = (project.to_string(), (!source.is_none()).then(|| js(source)));
        let new = source.clone();
        let changed = self.write(move |tx| {
            let old: Option<Option<String>> =
                tx.query_row("SELECT intake_source FROM projects WHERE id=?1", [&id], |r| r.get(0)).optional()?;
            let Some(old) = old else { bail!("no project {id}") };
            let old: IntakeSource = old.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            if old == new {
                return Ok(false);
            }
            tx.execute("UPDATE projects SET intake_source=?2 WHERE id=?1", params![id, text])?;
            tx.execute("DELETE FROM intake_items WHERE project_id=?1 AND state!='started'", [&id])?;
            tx.execute("DELETE FROM kv WHERE key=?1", [status_key(&id)])?;
            Ok(true)
        })?;
        if changed {
            self.touch([TOPIC.to_string(), orch_protocol::Topic::Project(project.to_string()).to_string()]);
        }
        Ok(changed)
    }

    /// Cached items, of one project or all, in their lane order.
    pub fn intake_items(&self, project: Option<&str>) -> Result<Vec<IntakeRow>> {
        let p = project.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS} FROM intake_items WHERE ?1 IS NULL OR project_id=?1 ORDER BY project_id, position, id"
            ))?;
            Ok(st.query_map([p], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn intake_item(&self, id: u64) -> Result<Option<IntakeRow>> {
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM intake_items WHERE id=?1"), [id as i64], row).optional()?)
        })
    }

    /// Replaces `project`'s items with what `source` (its kind) lists now,
    /// in that order. Gone items are deleted unless started; a hidden item
    /// whose revision moved is ready again. True when something the lane
    /// shows changed (a fetch time alone is not a change).
    pub fn intake_apply(&self, project: &str, source: &str, items: Vec<IntakeUpsert>, now_ms: u64) -> Result<bool> {
        let (id, source) = (project.to_string(), source.to_string());
        let changed = self.write(move |tx| {
            let mut changed = false;
            let mut keep: Vec<i64> = Vec::with_capacity(items.len());
            for (pos, it) in items.iter().enumerate() {
                let have = tx
                    .query_row(
                        &format!("SELECT {COLS} FROM intake_items WHERE project_id=?1 AND source=?2 AND ref=?3"),
                        params![id, source, it.reference],
                        row,
                    )
                    .optional()?;
                let meta = js(&it.meta);
                match have {
                    None => {
                        tx.execute(
                            "INSERT INTO intake_items(project_id, source, ref, title, url, body, meta, updated_at,
                               fetched_at_ms, position, state) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'ready')",
                            params![
                                id,
                                source,
                                it.reference,
                                it.title,
                                it.url,
                                it.body,
                                meta,
                                it.rev,
                                now_ms as i64,
                                pos as i64
                            ],
                        )?;
                        keep.push(tx.last_insert_rowid());
                        changed = true;
                    }
                    Some(h) => {
                        // Hidden until it changes upstream (spec §3.3 "Not now").
                        let unhide = h.state == IntakeState::Hidden && h.hidden_rev.as_deref() != Some(&it.rev);
                        let same = h.title == it.title
                            && h.url == it.url
                            && h.body == it.body
                            && h.meta == it.meta
                            && h.rev == it.rev
                            && h.position == pos as i64;
                        changed |= unhide || !same;
                        let state = if unhide { IntakeState::Ready } else { h.state };
                        tx.execute(
                            "UPDATE intake_items SET title=?2, url=?3, body=?4, meta=?5, updated_at=?6,
                               fetched_at_ms=?7, position=?8, state=?9,
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
                                pos as i64,
                                state.as_str(),
                                unhide
                            ],
                        )?;
                        keep.push(h.id as i64);
                    }
                }
            }
            // Gone upstream (or of another source): deleted, unless started.
            let keep = js(&keep);
            let gone = tx.execute(
                "DELETE FROM intake_items WHERE project_id=?1 AND state!='started'
                   AND id NOT IN (SELECT value FROM json_each(?2))",
                params![id, keep],
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
                "UPDATE intake_items SET state='hidden', hidden_at_ms=?2, hidden_rev=updated_at
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
                "UPDATE intake_items SET state='ready', hidden_at_ms=NULL, hidden_rev=NULL
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
                "UPDATE intake_items SET state='started', started_kind=?2, started_ref=?3,
                   hidden_at_ms=NULL, hidden_rev=NULL WHERE id=?1 AND state='ready'",
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
                "INSERT INTO intake_items(project_id, source, ref, title, updated_at, fetched_at_ms, state,
                   started_kind, started_ref) VALUES (?1, ?2, ?3, ?4, '', ?5, 'started', ?6, ?7)
                 ON CONFLICT(project_id, source, ref) DO UPDATE SET title=?4, state='started',
                   started_kind=?6, started_ref=?7",
                params![project, BRIEF_SOURCE, decision, title, now_ms as i64, kind, reference],
            )?;
            Ok(tx.query_row(
                "SELECT id FROM intake_items WHERE project_id=?1 AND source=?2 AND ref=?3",
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
                "SELECT 1 FROM intake_items WHERE state='started' AND started_kind=?1 AND started_ref=?2 LIMIT 1",
                params![kind, reference],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })
    }

    /// The items the lane lists (ready and hidden), of one project or all.
    pub fn intake_listed(&self, project: Option<&str>) -> Result<Vec<IntakeRow>> {
        let p = project.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS} FROM intake_items WHERE state!='started' AND (?1 IS NULL OR project_id=?1)
                 ORDER BY project_id, position, id"
            ))?;
            Ok(st.query_map([p], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The started rows (items and briefs), oldest first.
    pub fn intake_started(&self) -> Result<Vec<IntakeRow>> {
        self.read(move |c| {
            let mut st = c.prepare(&format!("SELECT {COLS} FROM intake_items WHERE state='started' ORDER BY id"))?;
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
                "SELECT {COLS} FROM intake_items WHERE state='started' AND started_kind=?1 AND started_ref=?2"
            ))?;
            let rows: Vec<IntakeRow> = st.query_map(params![kind, reference], row)?.collect::<rusqlite::Result<_>>()?;
            drop(st);
            for r in &rows {
                if r.source == BRIEF_SOURCE {
                    tx.execute("DELETE FROM intake_items WHERE id=?1", [r.id as i64])?;
                } else {
                    tx.execute(
                        "UPDATE intake_items SET state='ready', started_kind=NULL, started_ref=NULL WHERE id=?1",
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
            meta: json!({"labels": []}),
            rev: rev.into(),
        }
    }

    #[test]
    fn source_round_trips_and_a_change_drops_the_old_items_but_started() {
        let (_d, s) = store();
        assert_eq!(s.intake_source("p1").unwrap(), IntakeSource::None);
        let gh = IntakeSource::Github {
            repo: Some("acme/shop".into()),
            filter: GithubIntakeFilter::Label { names: vec!["ready".into()] },
        };
        assert!(s.set_intake_source("p1", &gh).unwrap());
        assert!(!s.set_intake_source("p1", &gh).unwrap());
        assert_eq!(s.intake_source("p1").unwrap(), gh);
        s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a")], 5).unwrap();
        let started = s.intake_items(Some("p1")).unwrap()[0].id;
        s.write(move |tx| {
            tx.execute("UPDATE intake_items SET state='started' WHERE id=?1", [started as i64])?;
            Ok(())
        })
        .unwrap();
        s.set_intake_status("p1", &IntakeStatus { refreshed_at_ms: Some(5), ..Default::default() }).unwrap();
        assert!(s.set_intake_source("p1", &IntakeSource::None).unwrap());
        let left = s.intake_items(Some("p1")).unwrap();
        assert_eq!(left.iter().map(|r| r.reference.as_str()).collect::<Vec<_>>(), ["#1"]);
        assert_eq!(s.intake_status("p1").unwrap(), IntakeStatus::default());
        assert!(s.intake_source("nope").is_err());
    }

    #[test]
    fn apply_orders_deletes_gone_keeps_started_and_unhides_on_change() {
        let (_d, s) = store();
        assert!(s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a"), item("#3", "a")], 1).unwrap());
        // Same answer again: no change, only the fetch time moves.
        assert!(!s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a"), item("#3", "a")], 2).unwrap());
        let rows = s.intake_items(Some("p1")).unwrap();
        assert_eq!(rows[0].fetched_at_ms, 2);
        let (one, two, three) = (rows[0].id, rows[1].id, rows[2].id);
        assert!(s.intake_hide(two, 3).unwrap());
        assert!(!s.intake_hide(two, 3).unwrap(), "only a ready item hides");
        s.write(move |tx| {
            tx.execute("UPDATE intake_items SET state='started' WHERE id=?1", [three as i64])?;
            Ok(())
        })
        .unwrap();
        // #3 is gone upstream but started: kept. #2 unchanged: still hidden.
        assert!(s.intake_apply("p1", "github", vec![item("#2", "a"), item("#1", "a")], 4).unwrap());
        let rows = s.intake_items(Some("p1")).unwrap();
        let by = |id| rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(by(two).state, IntakeState::Hidden);
        assert_eq!((by(two).position, by(one).position), (0, 1), "the source's order");
        assert_eq!(by(three).state, IntakeState::Started);
        // #2 edited upstream: back in the lane, same id.
        s.intake_apply("p1", "github", vec![item("#2", "b"), item("#1", "a")], 5).unwrap();
        let r = s.intake_item(two).unwrap().unwrap();
        assert_eq!((r.state, r.hidden_rev, r.rev.as_str()), (IntakeState::Ready, None, "b"));
        // Unhide by hand.
        s.intake_hide(one, 6).unwrap();
        assert!(s.intake_unhide(one).unwrap());
        assert_eq!(s.intake_item(one).unwrap().unwrap().state, IntakeState::Ready);
        // Gone and not started: deleted.
        s.intake_apply("p1", "github", vec![], 7).unwrap();
        let left: Vec<u64> = s.intake_items(Some("p1")).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(left, [three]);
        // A project's delete takes its cache.
        s.delete_project("p1").unwrap();
        assert!(s.intake_items(None).unwrap().is_empty());
    }

    #[test]
    fn started_items_and_briefs_return_or_drop() {
        let (_d, s) = store();
        s.intake_apply("p1", "github", vec![item("#1", "a"), item("#2", "a")], 1).unwrap();
        let rows = s.intake_items(Some("p1")).unwrap();
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
        s.intake_apply("p1", "github", vec![item("#2", "a")], 4).unwrap();
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
}
