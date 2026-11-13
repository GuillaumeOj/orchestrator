//! Improve intake B1: one tracker per project (`projects.tracker`, migration
//! 18) replacing the task backend (`projects.task_backend`) and the intake
//! source (`projects.intake_source`), which stay in the schema unread after
//! the migration (dropped by I1). Shapes: implement-improve-intake
//! `api-contract.md` §1.
//!
//! The old [`TaskBackendConfig`] and [`IntakeSource`] survive as *views* of a
//! [`TrackerSettings`] (`task_backend()`, `intake_source()`) and as the input
//! of the adapters of the old commands (`with_backend`, `with_intake`), so
//! the readers of the old settings keep their types until B2 lands the tree
//! readers.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use orch_team::TaskStatus;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db::Store;
use crate::teams::{GithubIntakeFilter, IntakeInvalid, IntakeSource, TaskBackendConfig, valid_repo};
use crate::util::js;

/// The refresh intervals the settings offer, in seconds.
pub const REFRESH_SECS: [u64; 4] = [300, 600, 1800, 3600];

/// Linear workflow state types a Ready filter can name.
pub const LINEAR_STATE_TYPES: [&str; 5] = ["backlog", "unstarted", "started", "completed", "canceled"];

/// A GitHub Projects v2 board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectBoard {
    pub owner: String,
    pub number: u64,
}

/// Where a project's work comes from (contract §1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tracker {
    Local {
        #[serde(default)]
        backlog_glob: Option<String>,
    },
    Github {
        /// `None` = the project's remote.
        #[serde(default)]
        repo: Option<String>,
        #[serde(default)]
        project_board: Option<ProjectBoard>,
    },
    Linear {
        #[serde(default)]
        teams: Vec<String>,
        /// `None` = every project of the teams.
        #[serde(default)]
        projects: Option<Vec<String>>,
    },
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker::Local { backlog_glob: None }
    }
}

impl Tracker {
    pub fn kind(&self) -> &'static str {
        match self {
            Tracker::Local { .. } => "local",
            Tracker::Github { .. } => "github",
            Tracker::Linear { .. } => "linear",
        }
    }
}

/// Which items are "Ready to start"; `kind` must equal the tracker's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadyFilter {
    Github {
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        milestone: Option<String>,
        #[serde(default)]
        board_status: Option<String>,
        #[serde(default)]
        issue_type: Option<String>,
    },
    Linear {
        #[serde(default)]
        state_types: Vec<String>,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        project: Option<String>,
        /// `current` | `next`.
        #[serde(default)]
        cycle: Option<String>,
    },
    Local,
}

impl ReadyFilter {
    pub fn kind(&self) -> &'static str {
        match self {
            ReadyFilter::Github { .. } => "github",
            ReadyFilter::Linear { .. } => "linear",
            ReadyFilter::Local => "local",
        }
    }

    /// A filter that matches by nothing (no Ready lane), of `kind`.
    pub fn empty(kind: &str) -> ReadyFilter {
        match kind {
            "github" => ReadyFilter::Github { labels: vec![], milestone: None, board_status: None, issue_type: None },
            "linear" => ReadyFilter::Linear { state_types: vec![], label: None, project: None, cycle: None },
            _ => ReadyFilter::Local,
        }
    }

    /// The contract's defaults (§1, migration): GitHub label `ready`,
    /// Linear unstarted issues of the current cycle.
    pub fn defaults(kind: &str) -> ReadyFilter {
        match kind {
            "github" => ReadyFilter::Github {
                labels: vec!["ready".into()],
                milestone: None,
                board_status: None,
                issue_type: None,
            },
            "linear" => ReadyFilter::Linear {
                state_types: vec!["unstarted".into()],
                label: None,
                project: None,
                cycle: Some("current".into()),
            },
            _ => ReadyFilter::Local,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteBack {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub assign_me: bool,
    #[serde(default = "running_label")]
    pub running_label: String,
    #[serde(default)]
    pub board_status: Option<String>,
    #[serde(default)]
    pub linear_state: Option<String>,
}

fn yes() -> bool {
    true
}

fn running_label() -> String {
    "orch:running".into()
}

impl Default for WriteBack {
    fn default() -> Self {
        WriteBack {
            enabled: true,
            assign_me: false,
            running_label: running_label(),
            board_status: None,
            linear_state: None,
        }
    }
}

fn default_refresh() -> u64 {
    600
}

/// A project's tracker settings (contract §1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerSettings {
    pub tracker: Tracker,
    pub ready: ReadyFilter,
    #[serde(default)]
    pub write_back: WriteBack,
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    #[serde(default = "yes")]
    pub group_by_milestone: bool,
    /// Linear only: a workflow state id → a task column.
    #[serde(default)]
    pub task_states: Option<BTreeMap<String, TaskStatus>>,
}

impl Default for TrackerSettings {
    fn default() -> Self {
        TrackerSettings::new(Tracker::default(), ReadyFilter::Local)
    }
}

/// The notice a migrated project shows once (contract §1, notes §10.6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MigrationNotice {
    /// `merged` | `different_repo` | `mismatch` | `backlog`.
    pub kind: String,
    pub from: Value,
    pub kept: Tracker,
    #[serde(default)]
    pub alternative: Option<Tracker>,
    #[serde(default)]
    pub dismissed: bool,
}

fn opt_trim(o: &mut Option<String>) {
    *o = o.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
}

fn clean_list(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    *v = v.drain(..).map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && seen.insert(s.clone())).collect();
}

impl TrackerSettings {
    pub fn new(tracker: Tracker, ready: ReadyFilter) -> TrackerSettings {
        TrackerSettings {
            tracker,
            ready,
            write_back: WriteBack::default(),
            refresh_secs: default_refresh(),
            group_by_milestone: true,
            task_states: None,
        }
    }

    /// Settings of the task backend `b` alone: no Ready filter.
    pub fn from_backend(b: &TaskBackendConfig) -> TrackerSettings {
        TrackerSettings::default().with_backend(b)
    }

    /// Names trimmed, empties dropped, `task_states` only for Linear.
    pub fn normalized(mut self) -> TrackerSettings {
        match &mut self.tracker {
            Tracker::Local { backlog_glob } => opt_trim(backlog_glob),
            Tracker::Github { repo, project_board } => {
                opt_trim(repo);
                if let Some(b) = project_board {
                    b.owner = b.owner.trim().to_string();
                }
            }
            Tracker::Linear { teams, projects } => {
                clean_list(teams);
                if let Some(p) = projects {
                    clean_list(p);
                    if p.is_empty() {
                        *projects = None;
                    }
                }
            }
        }
        match &mut self.ready {
            ReadyFilter::Github { labels, milestone, board_status, issue_type } => {
                clean_list(labels);
                opt_trim(milestone);
                opt_trim(board_status);
                opt_trim(issue_type);
            }
            ReadyFilter::Linear { state_types, label, project, cycle } => {
                clean_list(state_types);
                opt_trim(label);
                opt_trim(project);
                opt_trim(cycle);
            }
            ReadyFilter::Local => {}
        }
        self.write_back.running_label = self.write_back.running_label.trim().to_string();
        opt_trim(&mut self.write_back.board_status);
        opt_trim(&mut self.write_back.linear_state);
        if self.tracker.kind() != "linear" || self.task_states.as_ref().is_some_and(BTreeMap::is_empty) {
            self.task_states = None;
        }
        self
    }

    /// Contract §1 validation. The glob's syntax is checked where it is
    /// compiled (`orch_tasks::intake`).
    pub fn validate(&self) -> std::result::Result<(), IntakeInvalid> {
        let bad = |field, message: &str| Err(IntakeInvalid { field, message: message.into() });
        match &self.tracker {
            Tracker::Local { backlog_glob } => {
                if let Some(g) = backlog_glob {
                    if g.starts_with('/') {
                        return bad("backlog_glob", "the glob is relative to the repository (no leading /)");
                    }
                    if g.split(['/', '\\']).any(|c| c == "..") {
                        return bad("backlog_glob", "the glob cannot leave the repository (..)");
                    }
                }
            }
            Tracker::Github { repo, project_board } => {
                if let Some(r) = repo
                    && !valid_repo(r)
                {
                    return bad("repo", "the repository must be owner/name");
                }
                if let Some(b) = project_board {
                    if b.number == 0 {
                        return bad("project_board", "the project number is 1 or more");
                    }
                    if !valid_repo(&format!("{}/x", b.owner)) {
                        return bad("project_board", "the project owner is a GitHub user or organization login");
                    }
                }
            }
            Tracker::Linear { teams, .. } if teams.is_empty() => return bad("teams", "Pick at least one team."),
            Tracker::Linear { .. } => {}
        }
        if self.ready.kind() != self.tracker.kind() {
            return bad("ready", "Ready filter does not match the tracker");
        }
        match &self.ready {
            ReadyFilter::Github { labels, .. } if labels.iter().any(|n| n.contains(',')) => {
                return bad("labels", "a label name cannot contain a comma");
            }
            ReadyFilter::Github { board_status: Some(_), .. }
                if !matches!(&self.tracker, Tracker::Github { project_board: Some(_), .. }) =>
            {
                return bad("board_status", "pick a Projects board to filter by its Status");
            }
            ReadyFilter::Linear { state_types, .. }
                if state_types.iter().any(|t| !LINEAR_STATE_TYPES.contains(&t.as_str())) =>
            {
                return bad("state_types", "a Linear state type is backlog, unstarted, started, completed or canceled");
            }
            ReadyFilter::Linear { cycle: Some(c), .. } if c != "current" && c != "next" => {
                return bad("cycle", "the cycle is current or next");
            }
            _ => {}
        }
        if !REFRESH_SECS.contains(&self.refresh_secs) {
            return bad("refresh_secs", "refresh every 5, 10, 30 or 60 minutes");
        }
        if self.write_back.running_label.is_empty() {
            return bad("running_label", "name the label");
        }
        Ok(())
    }

    // -------------------------------------------------- views of the old types

    /// The task backend the tracker stands for. Linear: the first team and
    /// first project only until B2 lands the tree readers (which read all).
    pub fn task_backend(&self) -> TaskBackendConfig {
        match &self.tracker {
            Tracker::Local { .. } => TaskBackendConfig::Local,
            Tracker::Github { repo, .. } => TaskBackendConfig::Github { repo: repo.clone() },
            Tracker::Linear { teams, projects } => TaskBackendConfig::Linear {
                team: teams.first().cloned().unwrap_or_default(),
                project: projects.as_ref().and_then(|p| p.first().cloned()),
                states: self.task_states.clone().unwrap_or_default(),
            },
        }
    }

    /// The Ready lane's source as today's intake reader takes it (`None`
    /// when the Ready filter matches by nothing).
    pub fn intake_source(&self) -> IntakeSource {
        match (&self.tracker, &self.ready) {
            (Tracker::Local { backlog_glob: Some(g) }, _) => IntakeSource::RepoDocs { glob: g.clone(), branch: None },
            (Tracker::Github { repo, project_board }, ReadyFilter::Github { labels, milestone, board_status, .. }) => {
                let filter = match (project_board, board_status, milestone) {
                    (Some(b), Some(s), _) => GithubIntakeFilter::Project {
                        owner: b.owner.clone(),
                        number: b.number,
                        status: Some(s.clone()),
                    },
                    (_, _, Some(m)) => GithubIntakeFilter::Milestone { title: m.clone() },
                    _ if !labels.is_empty() => GithubIntakeFilter::Label { names: labels.clone() },
                    _ => return IntakeSource::None,
                };
                IntakeSource::Github { repo: repo.clone(), filter }
            }
            (Tracker::Linear { teams, .. }, ReadyFilter::Linear { state_types, label, project, .. }) => {
                if state_types.is_empty() && label.is_none() && project.is_none() {
                    return IntakeSource::None;
                }
                IntakeSource::Linear {
                    team: teams.first().cloned().unwrap_or_default(),
                    project: project.clone(),
                    label: label.clone(),
                    include_backlog: state_types.iter().any(|t| t == "backlog"),
                }
            }
            _ => IntakeSource::None,
        }
    }

    /// Adapter of the old task-backend settings: `b` replaces the tracker's
    /// repo / team (keeping the rest when the kind is unchanged; a kind
    /// change resets the Ready filter to none).
    pub fn with_backend(mut self, b: &TaskBackendConfig) -> TrackerSettings {
        match b {
            TaskBackendConfig::Local => {
                if self.tracker.kind() != "local" {
                    self.tracker = Tracker::Local { backlog_glob: None };
                    self.ready = ReadyFilter::Local;
                }
                self.task_states = None;
            }
            TaskBackendConfig::Github { repo } => {
                match &mut self.tracker {
                    Tracker::Github { repo: r, .. } => *r = repo.clone(),
                    _ => {
                        self.tracker = Tracker::Github { repo: repo.clone(), project_board: None };
                        self.ready = ReadyFilter::empty("github");
                    }
                }
                self.task_states = None;
            }
            TaskBackendConfig::Linear { team, project, states } => {
                match &mut self.tracker {
                    Tracker::Linear { teams, projects } => {
                        if teams.first() != Some(team) {
                            *teams = vec![team.clone()];
                        }
                        if projects.as_ref().and_then(|p| p.first()) != project.as_ref() {
                            *projects = project.clone().map(|p| vec![p]);
                        }
                    }
                    _ => {
                        self.tracker =
                            Tracker::Linear { teams: vec![team.clone()], projects: project.clone().map(|p| vec![p]) };
                        self.ready = ReadyFilter::empty("linear");
                    }
                }
                self.task_states = (!states.is_empty()).then(|| states.clone());
            }
        }
        self
    }

    /// Adapter of the old intake source: the tracker follows the source's
    /// kind, repo and team (one tracker, so the lane's source is also where
    /// the tasks go); repo docs over a remote tracker switch it to Local.
    pub fn with_intake(mut self, s: &IntakeSource) -> TrackerSettings {
        match s {
            IntakeSource::None => {
                self.ready = ReadyFilter::empty(self.tracker.kind());
                if let Tracker::Local { backlog_glob } = &mut self.tracker {
                    *backlog_glob = None;
                }
            }
            IntakeSource::Github { repo, filter } => {
                match &mut self.tracker {
                    Tracker::Github { repo: r, .. } => *r = repo.clone(),
                    _ => {
                        self.tracker = Tracker::Github { repo: repo.clone(), project_board: None };
                        self.task_states = None;
                    }
                }
                let mut ready = ReadyFilter::empty("github");
                if let ReadyFilter::Github { labels, milestone, board_status, .. } = &mut ready {
                    match filter {
                        GithubIntakeFilter::Label { names } => *labels = names.clone(),
                        GithubIntakeFilter::Milestone { title } => *milestone = Some(title.clone()),
                        GithubIntakeFilter::Project { owner, number, status } => {
                            *board_status = status.clone();
                            if let Tracker::Github { project_board, .. } = &mut self.tracker {
                                *project_board = Some(ProjectBoard { owner: owner.clone(), number: *number });
                            }
                        }
                    }
                }
                self.ready = ready;
            }
            IntakeSource::Linear { team, project, label, include_backlog } => {
                match &mut self.tracker {
                    Tracker::Linear { teams, .. } => {
                        if teams.first() != Some(team) {
                            *teams = vec![team.clone()];
                        }
                    }
                    _ => self.tracker = Tracker::Linear { teams: vec![team.clone()], projects: None },
                }
                let mut state_types = vec!["unstarted".to_string()];
                if *include_backlog {
                    state_types.push("backlog".into());
                }
                self.ready =
                    ReadyFilter::Linear { state_types, label: label.clone(), project: project.clone(), cycle: None };
            }
            IntakeSource::RepoDocs { glob, .. } => {
                self.tracker = Tracker::Local { backlog_glob: Some(glob.clone()) };
                self.ready = ReadyFilter::Local;
                self.task_states = None;
            }
        }
        self
    }
}

// ---------------------------------------------------------------- migration

/// What the migration makes of one project's old settings.
#[derive(Debug, PartialEq)]
pub(crate) struct Migrated {
    pub settings: TrackerSettings,
    pub notice: Option<MigrationNotice>,
    /// GitHub, no intake filter before: the `ready` label default stands
    /// until the repository was asked whether it has that label.
    pub ready_pending: bool,
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Notes §10.6. Pure.
pub(crate) fn migrate_settings(backend: &TaskBackendConfig, intake: &IntakeSource) -> Migrated {
    let base = TrackerSettings::from_backend(backend);
    let from = json!({"task_backend": backend, "intake_source": intake});
    let notice = |kind: &str, kept: &TrackerSettings, alt: Option<Tracker>| MigrationNotice {
        kind: kind.into(),
        from: from.clone(),
        kept: kept.tracker.clone(),
        alternative: alt,
        dismissed: false,
    };
    let with_defaults = |mut s: TrackerSettings| {
        s.ready = ReadyFilter::defaults(s.tracker.kind());
        s
    };
    let done = |settings: TrackerSettings, notice: Option<MigrationNotice>, pending: bool| Migrated {
        ready_pending: pending && settings.tracker.kind() == "github",
        settings,
        notice,
    };
    match (backend, intake) {
        (_, IntakeSource::None) => {
            // No intake filter: the Ready defaults, no notice.
            let s = with_defaults(base);
            done(s, None, true)
        }
        // The intake's repo docs folder is the Local tracker's backlog.
        (TaskBackendConfig::Local, IntakeSource::RepoDocs { .. }) => {
            let s = base.with_intake(intake);
            let n = notice("backlog", &s, None);
            done(s, Some(n), false)
        }
        // Repo docs over a remote tracker: the user picks.
        (_, IntakeSource::RepoDocs { .. }) => {
            let alt = TrackerSettings::default().with_intake(intake).tracker;
            let s = with_defaults(base);
            let n = notice("mismatch", &s, Some(alt));
            done(s, Some(n), true)
        }
        // Local tasks, a remote intake: the intake's tracker is the tracker.
        (TaskBackendConfig::Local, _) => {
            let s = base.with_intake(intake);
            let n = notice("merged", &s, None);
            done(s, Some(n), false)
        }
        (TaskBackendConfig::Github { repo: a }, IntakeSource::Github { repo: b, .. }) => {
            // An unset repo is the project's remote: same as any repo the store cannot resolve.
            let same = a.is_none() || b.is_none() || a == b;
            let mut s = base.with_intake(intake);
            if let Tracker::Github { repo, .. } = &mut s.tracker {
                *repo = a.clone();
            }
            let alt = (!same).then(|| Tracker::Github { repo: b.clone(), project_board: None });
            let n = notice(if same { "merged" } else { "different_repo" }, &s, alt);
            done(s, Some(n), false)
        }
        (TaskBackendConfig::Linear { team: a, project: pa, .. }, IntakeSource::Linear { team: b, .. }) => {
            let same = eq_ci(a, b);
            let mut s = base.with_intake(intake);
            // The backend's team and project are kept; the intake's project is a Ready filter.
            if let Tracker::Linear { teams, projects } = &mut s.tracker {
                *teams = vec![a.clone()];
                *projects = pa.clone().map(|p| vec![p]);
            }
            let alt = (!same).then(|| Tracker::Linear { teams: vec![b.clone()], projects: None });
            let n = notice(if same { "merged" } else { "different_repo" }, &s, alt);
            done(s, Some(n), false)
        }
        // GitHub vs Linear, either way: the task backend's until the user picks.
        _ => {
            let alt = TrackerSettings::default().with_intake(intake).tracker;
            let s = with_defaults(base);
            let n = notice("mismatch", &s, Some(alt));
            done(s, Some(n), true)
        }
    }
}

fn ready_key(project: &str) -> String {
    format!("tracker.ready_pending:{project}")
}

/// Migration 18's data step: every project gets its tracker (and notice)
/// from its old columns.
pub(crate) fn migrate_projects(tx: &rusqlite::Transaction) -> Result<()> {
    let rows: Vec<(String, String, Option<String>)> = {
        let mut st = tx.prepare("SELECT id, task_backend, intake_source FROM projects")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
    };
    // The global refresh interval (seconds, F16) is now per project, on the nearest step.
    let global: Option<u64> = tx
        .query_row("SELECT value FROM kv WHERE key='intake.refresh_secs'", [], |r| r.get::<_, String>(0))
        .optional()?
        .and_then(|v| v.trim().parse().ok());
    let refresh = global.map(|g| REFRESH_SECS.into_iter().find(|s| g <= *s).unwrap_or(3600));
    for (id, backend, intake) in rows {
        let backend: TaskBackendConfig = serde_json::from_str(&backend).unwrap_or_default();
        let intake: IntakeSource = intake.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut m = migrate_settings(&backend, &intake);
        if let Some(r) = refresh {
            m.settings.refresh_secs = r;
        }
        tx.execute(
            "UPDATE projects SET tracker=?2, tracker_notice=?3 WHERE id=?1",
            params![id, js(&m.settings), m.notice.as_ref().map(js)],
        )?;
        if m.ready_pending {
            tx.execute(
                "INSERT INTO kv(key, value) VALUES (?1,'true') ON CONFLICT(key) DO UPDATE SET value='true'",
                [ready_key(&id)],
            )?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- store

/// A project's stored tracker; an unmigrated row (inserted since) is read
/// from its `task_backend` column. `None` = no such project.
pub(crate) fn read_tracker(tx: &rusqlite::Connection, id: &str) -> rusqlite::Result<Option<TrackerSettings>> {
    let row: Option<(Option<String>, String)> = tx
        .query_row("SELECT tracker, task_backend FROM projects WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(row.map(|(t, b)| {
        t.and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| TrackerSettings::from_backend(&serde_json::from_str(&b).unwrap_or_default()))
    }))
}

/// Writes `new` as the project's tracker (the caller validated it). A
/// changed task backend drops the task-sync cursor; a changed Ready source
/// drops the cached items but the started ones and the refresh status; a
/// `mismatch` notice is resolved by any save. True when the tracker changed.
pub(crate) fn apply_tracker(tx: &rusqlite::Transaction, id: &str, new: &TrackerSettings) -> Result<bool> {
    let Some(old) = read_tracker(tx, id)? else { bail!("no project {id}") };
    if old == *new {
        return Ok(false);
    }
    tx.execute("UPDATE projects SET tracker=?2 WHERE id=?1", params![id, js(new)])?;
    if old.task_backend() != new.task_backend() {
        tx.execute("DELETE FROM kv WHERE key=?1", [format!("task_sync_cursor:{id}")])?;
    }
    if old.intake_source() != new.intake_source() {
        tx.execute("DELETE FROM intake_items WHERE project_id=?1 AND state!='started'", [id])?;
        tx.execute("DELETE FROM kv WHERE key=?1", [crate::intake::status_key(id)])?;
    }
    Ok(true)
}

impl Store {
    /// The project's tracker settings (an error when the project is gone).
    pub fn tracker(&self, project: &str) -> Result<TrackerSettings> {
        let id = project.to_string();
        self.read(move |c| match read_tracker(c, &id)? {
            Some(t) => Ok(t),
            None => bail!("no project {id}"),
        })
    }

    /// Stores `settings` (validated by the caller). True when something
    /// changed; refreshes its topics.
    pub fn set_tracker(&self, project: &str, settings: &TrackerSettings) -> Result<bool> {
        let (id, s) = (project.to_string(), settings.clone());
        // The user's save settles a `mismatch` notice and the unchecked default.
        let changed = self.write(move |tx| {
            let resolved = tx.execute(
                "UPDATE projects SET tracker_notice=NULL
                 WHERE id=?1 AND json_extract(tracker_notice,'$.kind')='mismatch'",
                [&id],
            )? > 0;
            tx.execute("DELETE FROM kv WHERE key=?1", [ready_key(&id)])?;
            Ok(apply_tracker(tx, &id, &s)? || resolved)
        })?;
        if changed {
            self.touch([
                crate::intake::TOPIC.to_string(),
                orch_protocol::Topic::Project(project.to_string()).to_string(),
            ]);
        }
        Ok(changed)
    }

    /// The migration notice to show: none when absent or dismissed.
    pub fn tracker_notice(&self, project: &str) -> Result<Option<MigrationNotice>> {
        let id = project.to_string();
        self.read(move |c| {
            let s: Option<Option<String>> =
                c.query_row("SELECT tracker_notice FROM projects WHERE id=?1", [&id], |r| r.get(0)).optional()?;
            let Some(s) = s else { bail!("no project {id}") };
            let n: Option<MigrationNotice> = s.and_then(|s| serde_json::from_str(&s).ok());
            Ok(n.filter(|n| !n.dismissed))
        })
    }

    /// Stores (or clears) the project's migration notice.
    pub fn set_tracker_notice(&self, project: &str, notice: Option<&MigrationNotice>) -> Result<()> {
        let (id, n) = (project.to_string(), notice.map(js));
        self.write(move |tx| {
            tx.execute("UPDATE projects SET tracker_notice=?2 WHERE id=?1", params![id, n])?;
            Ok(())
        })?;
        self.note([orch_protocol::Topic::Project(project.to_string()).to_string()]);
        Ok(())
    }

    pub fn dismiss_tracker_notice(&self, project: &str) -> Result<()> {
        let id = project.to_string();
        self.write(move |tx| {
            tx.execute(
                "UPDATE projects SET tracker_notice=json_set(tracker_notice,'$.dismissed',json('true'))
                 WHERE id=?1 AND tracker_notice IS NOT NULL",
                [&id],
            )?;
            Ok(())
        })?;
        self.note([orch_protocol::Topic::Project(project.to_string()).to_string()]);
        Ok(())
    }

    /// The migrated GitHub default (label `ready`) was not checked against
    /// the repository yet.
    pub fn tracker_ready_pending(&self, project: &str) -> Result<bool> {
        Ok(self.setting(&ready_key(project))?.is_some())
    }

    /// The repository answered: `has_ready_label` false empties the default
    /// labels (when still the migrated `["ready"]`). Done once.
    pub fn resolve_tracker_ready(&self, project: &str, has_ready_label: bool) -> Result<()> {
        let id = project.to_string();
        let changed = self.write(move |tx| {
            let pending =
                tx.query_row("SELECT 1 FROM kv WHERE key=?1", [ready_key(&id)], |_| Ok(())).optional()?.is_some();
            if !pending {
                return Ok(false);
            }
            tx.execute("DELETE FROM kv WHERE key=?1", [ready_key(&id)])?;
            let Some(mut t) = read_tracker(tx, &id)? else { return Ok(false) };
            if !has_ready_label
                && let ReadyFilter::Github { labels, .. } = &mut t.ready
                && labels == &["ready".to_string()]
            {
                labels.clear();
                tx.execute("UPDATE projects SET tracker=?2 WHERE id=?1", params![id, js(&t)])?;
                tx.execute("DELETE FROM intake_items WHERE project_id=?1 AND state!='started'", [&id])?;
                tx.execute("DELETE FROM kv WHERE key=?1", [crate::intake::status_key(&id)])?;
                return Ok(true);
            }
            Ok(false)
        })?;
        if changed {
            self.touch([crate::intake::TOPIC.to_string()]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::tests::project;
    use serde_json::json;

    fn gh(repo: &str) -> TaskBackendConfig {
        TaskBackendConfig::Github { repo: Some(repo.into()) }
    }

    fn label_intake(repo: Option<&str>, names: &[&str]) -> IntakeSource {
        IntakeSource::Github {
            repo: repo.map(str::to_string),
            filter: GithubIntakeFilter::Label { names: names.iter().map(|s| s.to_string()).collect() },
        }
    }

    fn linear(team: &str) -> TaskBackendConfig {
        TaskBackendConfig::Linear { team: team.into(), project: None, states: BTreeMap::new() }
    }

    fn linear_intake(team: &str) -> IntakeSource {
        IntakeSource::Linear { team: team.into(), project: None, label: Some("bug".into()), include_backlog: false }
    }

    fn docs() -> IntakeSource {
        IntakeSource::RepoDocs { glob: "docs/backlog/*.md".into(), branch: None }
    }

    fn kind(m: &Migrated) -> Option<&str> {
        m.notice.as_ref().map(|n| n.kind.as_str())
    }

    #[test]
    fn migration_same_repo_merges_and_keeps_the_ready_filter() {
        let m = migrate_settings(&gh("me/shop"), &label_intake(Some("me/shop"), &["go"]));
        assert_eq!(kind(&m), Some("merged"));
        assert_eq!(m.settings.tracker, Tracker::Github { repo: Some("me/shop".into()), project_board: None });
        assert_eq!(
            m.settings.ready,
            ReadyFilter::Github { labels: vec!["go".into()], milestone: None, board_status: None, issue_type: None }
        );
        assert_eq!(m.notice.as_ref().unwrap().alternative, None);
        assert!(!m.ready_pending);
        // The unset repo (the project's remote) is the same repo.
        assert_eq!(kind(&migrate_settings(&gh("me/shop"), &label_intake(None, &["go"]))), Some("merged"));
        // Same Linear team, whatever the case.
        let m = migrate_settings(&linear("ENG"), &linear_intake("eng"));
        assert_eq!(kind(&m), Some("merged"));
        assert_eq!(m.settings.tracker, Tracker::Linear { teams: vec!["ENG".into()], projects: None });
        assert_eq!(
            m.settings.ready,
            ReadyFilter::Linear {
                state_types: vec!["unstarted".into()],
                label: Some("bug".into()),
                project: None,
                cycle: None
            }
        );
    }

    #[test]
    fn migration_different_repo_keeps_the_backends_and_offers_the_intakes() {
        let m = migrate_settings(&gh("me/shop"), &label_intake(Some("me/web"), &["go"]));
        assert_eq!(kind(&m), Some("different_repo"));
        assert_eq!(m.settings.tracker, Tracker::Github { repo: Some("me/shop".into()), project_board: None });
        assert_eq!(
            m.notice.as_ref().unwrap().alternative,
            Some(Tracker::Github { repo: Some("me/web".into()), project_board: None })
        );
        // The intake filter is the Ready filter.
        assert_eq!(m.settings.intake_source(), label_intake(Some("me/shop"), &["go"]));
        let m = migrate_settings(&linear("ENG"), &linear_intake("OPS"));
        assert_eq!(kind(&m), Some("different_repo"));
        assert_eq!(m.notice.unwrap().alternative, Some(Tracker::Linear { teams: vec!["OPS".into()], projects: None }));
    }

    #[test]
    fn migration_github_against_linear_is_a_mismatch_keeping_the_task_backend() {
        let m = migrate_settings(&linear("ENG"), &label_intake(Some("me/shop"), &["go"]));
        assert_eq!(kind(&m), Some("mismatch"));
        assert_eq!(m.settings.tracker.kind(), "linear");
        assert_eq!(m.settings.ready, ReadyFilter::defaults("linear"));
        assert_eq!(
            m.notice.as_ref().unwrap().alternative,
            Some(Tracker::Github { repo: Some("me/shop".into()), project_board: None })
        );
        let m = migrate_settings(&gh("me/shop"), &linear_intake("ENG"));
        assert_eq!(kind(&m), Some("mismatch"));
        assert_eq!(m.settings.tracker.kind(), "github");
        assert!(m.ready_pending, "the GitHub default awaits its label check");
        assert_eq!(m.notice.unwrap().alternative, Some(Tracker::Linear { teams: vec!["ENG".into()], projects: None }));
    }

    #[test]
    fn migration_repo_docs_are_the_local_backlog_or_a_mismatch_over_a_remote() {
        let m = migrate_settings(&TaskBackendConfig::Local, &docs());
        assert_eq!(kind(&m), Some("backlog"));
        assert_eq!(m.settings.tracker, Tracker::Local { backlog_glob: Some("docs/backlog/*.md".into()) });
        assert_eq!(m.settings.ready, ReadyFilter::Local);
        for b in [gh("me/shop"), linear("ENG")] {
            let m = migrate_settings(&b, &docs());
            assert_eq!(kind(&m), Some("mismatch"), "{b:?}");
            assert_eq!(m.settings.tracker.kind(), b.kind());
            assert_eq!(
                m.notice.unwrap().alternative,
                Some(Tracker::Local { backlog_glob: Some("docs/backlog/*.md".into()) })
            );
        }
    }

    #[test]
    fn migration_defaults_the_ready_filter_without_an_intake_filter() {
        let m = migrate_settings(&gh("me/shop"), &IntakeSource::None);
        assert_eq!(m.settings.ready, ReadyFilter::defaults("github"));
        assert!(m.notice.is_none() && m.ready_pending);
        let m = migrate_settings(&linear("ENG"), &IntakeSource::None);
        assert_eq!(
            m.settings.ready,
            ReadyFilter::Linear {
                state_types: vec!["unstarted".into()],
                label: None,
                project: None,
                cycle: Some("current".into())
            }
        );
        assert!(m.notice.is_none() && !m.ready_pending);
        let m = migrate_settings(&TaskBackendConfig::Local, &IntakeSource::None);
        assert_eq!(m.settings, TrackerSettings::default());
        assert!(m.notice.is_none() && !m.ready_pending);
    }

    #[test]
    fn migration_moves_the_linear_task_states_and_keeps_the_notice_source() {
        let states = BTreeMap::from([("st1".to_string(), TaskStatus::Review)]);
        let b = TaskBackendConfig::Linear { team: "ENG".into(), project: Some("pr1".into()), states: states.clone() };
        let m = migrate_settings(&b, &linear_intake("ENG"));
        assert_eq!(m.settings.task_states, Some(states));
        assert_eq!(
            m.settings.tracker,
            Tracker::Linear { teams: vec!["ENG".into()], projects: Some(vec!["pr1".into()]) }
        );
        let from = &m.notice.unwrap().from;
        assert_eq!(from["task_backend"]["type"], "linear");
        assert_eq!(from["intake_source"]["type"], "linear");
        // Without a team filter: no task states.
        assert_eq!(migrate_settings(&gh("me/shop"), &IntakeSource::None).settings.task_states, None);
    }

    #[test]
    fn the_sql_migration_fills_every_project_and_keeps_the_intake_marks() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let mut p = project("p1", "/tmp/p1");
        p.task_backend = json!({"type": "github", "repo": "me/shop"});
        s.upsert_project(&p).unwrap();
        s.upsert_project(&project("p2", "/tmp/p2")).unwrap();
        let item =
            crate::IntakeUpsert { reference: "#1".into(), title: "t".into(), rev: "r".into(), ..Default::default() };
        s.intake_apply("p1", "github", vec![item], 1).unwrap();
        let id = s.intake_items(Some("p1")).unwrap()[0].id;
        s.intake_hide(id, 5).unwrap();
        drop(s);
        // Back to the schema before the tracker, with the old columns set.
        let c = rusqlite::Connection::open(d.path().join("orch.db")).unwrap();
        c.execute_batch("ALTER TABLE projects DROP COLUMN tracker; ALTER TABLE projects DROP COLUMN tracker_notice;")
            .unwrap();
        c.execute(
            "UPDATE projects SET intake_source=?1, task_backend=?2 WHERE id='p1'",
            [
                r#"{"type":"github","repo":"me/web","filter":{"kind":"label","names":["go"]}}"#,
                r#"{"type":"github","repo":"me/shop"}"#,
            ],
        )
        .unwrap();
        c.pragma_update(None, "user_version", 17).unwrap();
        drop(c);
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let t = s.tracker("p1").unwrap();
        assert_eq!(t.tracker, Tracker::Github { repo: Some("me/shop".into()), project_board: None });
        assert_eq!(s.tracker_notice("p1").unwrap().unwrap().kind, "different_repo");
        assert_eq!(s.tracker("p2").unwrap(), TrackerSettings::default());
        assert!(s.tracker_notice("p2").unwrap().is_none());
        let rows = s.intake_items(Some("p1")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, crate::IntakeState::Hidden, "the hidden mark survives");
        // Readers see the tracker.
        assert_eq!(s.project("p1").unwrap().unwrap().backend(), gh("me/shop"));
        assert_eq!(s.intake_source("p1").unwrap(), label_intake(Some("me/shop"), &["go"]));
    }

    fn valid() -> TrackerSettings {
        TrackerSettings::new(
            Tracker::Github { repo: Some("me/shop".into()), project_board: None },
            ReadyFilter::defaults("github"),
        )
    }

    fn field(s: TrackerSettings) -> Option<&'static str> {
        s.normalized().validate().err().map(|e| e.field)
    }

    #[test]
    fn validation_names_the_field_at_fault() {
        assert_eq!(field(valid()), None);
        let with = |t: Tracker| TrackerSettings { tracker: t, ..valid() };
        let gh = |repo: &str| Tracker::Github { repo: Some(repo.into()), project_board: None };
        assert_eq!(field(with(gh("bad"))), Some("repo"));
        assert_eq!(field(with(gh("-a/b"))), Some("repo"));
        let board = |owner: &str, number| Tracker::Github {
            repo: None,
            project_board: Some(ProjectBoard { owner: owner.into(), number }),
        };
        assert_eq!(field(with(board("acme", 0))), Some("project_board"));
        assert_eq!(field(with(board("not an owner", 4))), Some("project_board"));
        assert_eq!(field(with(board("acme", 4))), None);
        let lin =
            TrackerSettings::new(Tracker::Linear { teams: vec![], projects: None }, ReadyFilter::defaults("linear"));
        assert_eq!(field(lin.clone()), Some("teams"));
        assert_eq!(
            field(TrackerSettings { tracker: Tracker::Linear { teams: vec![" ".into()], projects: None }, ..lin }),
            Some("teams")
        );
        let local = |g: &str| TrackerSettings::new(Tracker::Local { backlog_glob: Some(g.into()) }, ReadyFilter::Local);
        assert_eq!(field(local("/abs/*.md")), Some("backlog_glob"));
        assert_eq!(field(local("a/../b/*.md")), Some("backlog_glob"));
        assert_eq!(field(local("   ")), None, "blank is no backlog");
        assert_eq!(field(local(".orchestrator/backlog/*.md")), None);
        // Ready.
        assert_eq!(field(TrackerSettings { ready: ReadyFilter::Local, ..valid() }), Some("ready"));
        assert_eq!(field(TrackerSettings { ready: ReadyFilter::defaults("linear"), ..valid() }), Some("ready"));
        let labels = |l: &[&str]| ReadyFilter::Github {
            labels: l.iter().map(|s| s.to_string()).collect(),
            milestone: None,
            board_status: None,
            issue_type: None,
        };
        assert_eq!(field(TrackerSettings { ready: labels(&["a,b"]), ..valid() }), Some("labels"));
        assert_eq!(field(TrackerSettings { ready: labels(&[]), ..valid() }), None);
        let lready = |types: &[&str], cycle: Option<&str>| ReadyFilter::Linear {
            state_types: types.iter().map(|s| s.to_string()).collect(),
            label: None,
            project: None,
            cycle: cycle.map(str::to_string),
        };
        let l = TrackerSettings::new(Tracker::Linear { teams: vec!["ENG".into()], projects: None }, lready(&[], None));
        assert_eq!(field(TrackerSettings { ready: lready(&["doing"], None), ..l.clone() }), Some("state_types"));
        assert_eq!(field(TrackerSettings { ready: lready(&["started"], Some("last")), ..l.clone() }), Some("cycle"));
        assert_eq!(field(TrackerSettings { ready: lready(&["started"], Some("next")), ..l }), None);
        // The rest.
        assert_eq!(field(TrackerSettings { refresh_secs: 42, ..valid() }), Some("refresh_secs"));
        let mut w = valid();
        w.write_back.running_label = "  ".into();
        assert_eq!(field(w), Some("running_label"));
    }

    #[test]
    fn the_contract_json_round_trips() {
        let t: TrackerSettings = serde_json::from_value(json!({
            "tracker": {"kind": "github", "repo": null, "project_board": {"owner": "acme", "number": 4}},
            "ready": {"kind": "github", "labels": ["ready"], "milestone": "v2", "board_status": "Todo", "issue_type": null},
            "write_back": {"enabled": true, "assign_me": false, "running_label": "orch:running",
                           "board_status": "In progress", "linear_state": null},
            "refresh_secs": 600, "group_by_milestone": true, "task_states": null
        }))
        .unwrap();
        assert_eq!(t.tracker.kind(), "github");
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["tracker"]["project_board"]["number"], 4);
        assert_eq!(v["task_states"], Value::Null);
        assert_eq!(serde_json::from_value::<TrackerSettings>(v).unwrap(), t);
        // A kind-only Local filter and the defaults of a sparse payload.
        let l: TrackerSettings =
            serde_json::from_value(json!({"tracker": {"kind": "local"}, "ready": {"kind": "local"}})).unwrap();
        assert_eq!((l.refresh_secs, l.group_by_milestone, l.write_back.enabled), (600, true, true));
    }

    #[test]
    fn normalizing_trims_and_drops_what_does_not_apply() {
        let mut t = TrackerSettings::new(
            Tracker::Linear { teams: vec![" ENG ".into(), "ENG".into(), "".into()], projects: Some(vec![" ".into()]) },
            ReadyFilter::defaults("linear"),
        );
        t.task_states = Some(BTreeMap::new());
        let t = t.normalized();
        assert_eq!(t.tracker, Tracker::Linear { teams: vec!["ENG".into()], projects: None });
        assert_eq!(t.task_states, None);
        let mut g = valid();
        g.task_states = Some(BTreeMap::from([("s".to_string(), TaskStatus::Done)]));
        assert_eq!(g.normalized().task_states, None, "task states are Linear's");
    }

    #[test]
    fn the_old_views_round_trip_through_the_tracker() {
        let b = TaskBackendConfig::Linear {
            team: "ENG".into(),
            project: Some("p".into()),
            states: BTreeMap::from([("s".to_string(), TaskStatus::Ready)]),
        };
        for b in [TaskBackendConfig::Local, gh("me/shop"), b] {
            assert_eq!(TrackerSettings::from_backend(&b).task_backend(), b);
        }
        for i in [
            IntakeSource::None,
            label_intake(Some("me/shop"), &["a", "b"]),
            IntakeSource::Github { repo: None, filter: GithubIntakeFilter::Milestone { title: "v2".into() } },
            IntakeSource::Github {
                repo: None,
                filter: GithubIntakeFilter::Project { owner: "acme".into(), number: 4, status: Some("Todo".into()) },
            },
            IntakeSource::Linear { team: "ENG".into(), project: Some("p".into()), label: None, include_backlog: true },
            docs(),
        ] {
            let t = TrackerSettings::default().with_intake(&i);
            assert_eq!(t.intake_source(), i, "{i:?}");
            assert!(t.validate().is_ok(), "{i:?}");
        }
        // The backend adapter keeps the Ready filter of an unchanged kind.
        let t = TrackerSettings::default().with_intake(&label_intake(None, &["go"])).with_backend(&gh("me/web"));
        assert_eq!(t.intake_source(), label_intake(Some("me/web"), &["go"]));
        // A kind change empties it.
        assert_eq!(t.with_backend(&TaskBackendConfig::Local).intake_source(), IntakeSource::None);
    }

    #[test]
    fn saving_resets_what_depends_on_the_tracker_and_resolves_a_mismatch() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let mut p = project("p1", "/tmp/p1");
        p.task_backend = json!({"type": "github", "repo": "me/shop"});
        s.upsert_project(&p).unwrap();
        s.set_task_sync_cursor("p1", "github:me/shop", "c").unwrap();
        let mut t = s.tracker("p1").unwrap();
        // A Ready change keeps the sync cursor and drops cached items.
        let item =
            crate::IntakeUpsert { reference: "#1".into(), title: "t".into(), rev: "r".into(), ..Default::default() };
        s.intake_apply("p1", "github", vec![item], 1).unwrap();
        t.ready = ReadyFilter::defaults("github");
        assert!(s.set_tracker("p1", &t).unwrap());
        assert!(s.task_sync_cursor("p1").unwrap().is_some());
        assert!(s.intake_items(Some("p1")).unwrap().is_empty());
        assert!(!s.set_tracker("p1", &t).unwrap(), "unchanged");
        // A repo change drops the cursor.
        t.tracker = Tracker::Github { repo: Some("me/web".into()), project_board: None };
        assert!(s.set_tracker("p1", &t).unwrap());
        assert!(s.task_sync_cursor("p1").unwrap().is_none());
        // A mismatch notice goes with any save; another one stays until dismissed.
        let notice = |kind: &str| MigrationNotice {
            kind: kind.into(),
            from: json!({}),
            kept: t.tracker.clone(),
            alternative: None,
            dismissed: false,
        };
        let c = |n: &MigrationNotice| {
            s.write({
                let (n, id) = (js(n), "p1");
                move |tx| Ok(tx.execute("UPDATE projects SET tracker_notice=?2 WHERE id=?1", params![id, n])?)
            })
            .unwrap()
        };
        c(&notice("mismatch"));
        assert!(s.tracker_notice("p1").unwrap().is_some());
        s.set_tracker("p1", &t).unwrap();
        assert!(s.tracker_notice("p1").unwrap().is_none(), "resolved by the save");
        c(&notice("merged"));
        s.set_tracker("p1", &t).unwrap();
        assert_eq!(s.tracker_notice("p1").unwrap().unwrap().kind, "merged");
        s.dismiss_tracker_notice("p1").unwrap();
        assert!(s.tracker_notice("p1").unwrap().is_none());
        assert!(s.tracker("nope").is_err());
    }

    #[test]
    fn the_unchecked_ready_default_is_resolved_once_and_only_by_an_answer() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        s.upsert_project(&project("p1", "/tmp/p1")).unwrap();
        let mut t = TrackerSettings::from_backend(&gh("me/shop"));
        t.ready = ReadyFilter::defaults("github");
        s.set_tracker("p1", &t).unwrap();
        let mark = |on: bool| {
            s.write(move |tx| {
                let sql = if on {
                    "INSERT OR REPLACE INTO kv(key, value) VALUES ('tracker.ready_pending:p1','true')"
                } else {
                    "DELETE FROM kv WHERE key='tracker.ready_pending:p1'"
                };
                Ok(tx.execute(sql, [])?)
            })
            .unwrap()
        };
        // The repo has the label: kept, and not pending any more.
        mark(true);
        assert!(s.tracker_ready_pending("p1").unwrap());
        s.resolve_tracker_ready("p1", true).unwrap();
        assert!(!s.tracker_ready_pending("p1").unwrap());
        assert_eq!(s.tracker("p1").unwrap().ready, ReadyFilter::defaults("github"));
        // No such label: emptied.
        mark(true);
        s.resolve_tracker_ready("p1", false).unwrap();
        assert_eq!(s.tracker("p1").unwrap().ready, ReadyFilter::empty("github"));
        // Not pending: an answer changes nothing.
        s.resolve_tracker_ready("p1", true).unwrap();
        s.set_tracker("p1", &t).unwrap();
        mark(true);
        s.set_tracker("p1", &t).unwrap();
        assert!(!s.tracker_ready_pending("p1").unwrap(), "the user's save settles it");
        s.resolve_tracker_ready("p1", false).unwrap();
        assert_eq!(s.tracker("p1").unwrap().ready, ReadyFilter::defaults("github"));
    }
}
