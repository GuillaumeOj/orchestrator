//! P2 teams (SPEC §7): roles, teams and their versions, features, team runs,
//! tasks, messages and artifacts; per-project settings (FR-109).

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use orch_team::{RoleDef, RunState, Scope, TaskStatus, Team};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Store;
use crate::model::Project;
use crate::util::{from_js, js, opt_u64, scope_key, scope_of};

/// A feature of a project (SPEC §7 Feature): its docs live on
/// `orch/<slug>/integration`, in the lead's worktree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub brief: String,
    /// The branch the integration branch was cut from (usually `main`).
    pub base: String,
    #[serde(default)]
    pub integration_worktree: Option<PathBuf>,
    /// Governor allocation (FR-802); `None` = the project's.
    #[serde(default)]
    pub allocation: Option<String>,
    pub created_at_ms: u64,
    #[serde(default)]
    pub archived: bool,
}

impl Feature {
    pub fn integration_branch(&self) -> String {
        format!("orch/{}/integration", self.slug)
    }
}

/// A team launched on a feature (SPEC §7 TeamRun). The team graph and roles
/// it uses are the snapshot `team_versions(team_id, team_version)` (FR-308).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamRun {
    pub id: String,
    pub team_id: String,
    /// Where `team_id` lives: global or the feature's project.
    #[serde(default)]
    pub team_scope: Scope,
    pub team_version: u32,
    pub feature_id: String,
    pub status: RunState,
    /// Where a paused or deferred run goes back to.
    #[serde(default)]
    pub resume_status: Option<RunState>,
    #[serde(default)]
    pub pr_url: Option<String>,
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    /// The open (or last) PR of the run, parsed from `pr_url`.
    #[serde(default)]
    pub pr_number: Option<u64>,
    /// The PR's last status (the watcher's, §3.3); `None` before the first check.
    #[serde(default)]
    pub pr: Option<PrStatus>,
}

/// The PR number at the end of a PR URL (`…/pull/12`).
pub fn pr_number_from_url(url: &str) -> Option<u64> {
    let (_, n) = url.trim().trim_end_matches('/').rsplit_once("/pull/")?;
    n.parse().ok()
}

/// A run's PR as the watcher last saw it (state, CI, position against its base).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PrStatus {
    /// `open` / `merged` / `closed`.
    pub state: String,
    pub head_sha: String,
    pub base: String,
    pub ci: CiSummary,
    pub position: PrPosition,
    /// The PR head the app itself last pushed (the force-with-lease lease).
    #[serde(default)]
    pub pushed_sha: Option<String>,
    /// Local integration commits not in `head_sha`.
    pub unpushed: u32,
    /// The local branch does not contain `pushed_sha` (rebased).
    pub rewritten: bool,
    pub checked_at_ms: u64,
    /// The last check failed: the rest is the last good status.
    #[serde(default)]
    pub error: Option<String>,
}

/// CI of a PR: one job per CheckRun / StatusContext.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CiSummary {
    /// `ok` / `error` / `running` / `waiting` / `none`.
    pub overall: String,
    /// Finished jobs, whatever the outcome.
    pub done: u32,
    pub total: u32,
    pub jobs: Vec<CiJob>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CiJob {
    pub name: String,
    #[serde(default)]
    pub workflow: Option<String>,
    /// The GitHub state lower-cased (`success`, `failure`, `in_progress`…).
    pub state: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// The PR head against `origin/<base>`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PrPosition {
    /// `up_to_date` / `behind` / `conflicts` / `unknown`.
    pub status: String,
    pub behind: u32,
    pub ahead: u32,
    /// The `origin/<base>` head they were counted against (behind is told once per base head).
    #[serde(default)]
    pub base_sha: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskNote {
    pub ts_ms: u64,
    /// `node`, `agent:<id>`, `user` or `remote`.
    pub by: String,
    pub text: String,
}

/// A task in the board's one model (ADR-004), mirrored here whatever the
/// backend. `id` is the local handle agents use; `external_id` the backend's
/// (a GitHub issue number).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub project_id: String,
    /// `local`, `github` or `linear` (a task left local when the project's
    /// backend changed keeps `local`, D8).
    pub backend: String,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub feature_id: Option<String>,
    #[serde(default)]
    pub team_run_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub status: TaskStatus,
    /// Assignee node (tasks go to nodes, not sessions: ADR-004).
    #[serde(default)]
    pub node: Option<String>,
    /// The agent working on it, once dispatched.
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<u64>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub notes: Vec<TaskNote>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// The backend's `updatedAt` at the last sync (conflict detection, FR-503).
    #[serde(default)]
    pub remote_updated_at: Option<String>,
    /// The core's workflow state (never synced to a backend).
    #[serde(default)]
    pub flow: TaskFlow,
}

/// Where a task stands in the team's own workflow: a plan approved by the
/// lead before any edit, and a review before the merge.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskFlow {
    /// The lead asked for a plan first: edits are refused until it approves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_first: bool,
    /// The plan the owner submitted, waiting for the lead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_approved: bool,
    /// The review is under way (self-review commands, or a reviewer).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reviewing: bool,
    /// The review's outcome: `{fixed: [...], unfixed: [{finding, reason}]}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Value>,
    /// On a reviewer's own task: the task it reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviews: Option<u64>,
}

impl Task {
    pub fn new(project_id: &str, backend: &str, title: &str, now_ms: u64) -> Task {
        Task {
            id: 0,
            project_id: project_id.into(),
            backend: backend.into(),
            external_id: None,
            feature_id: None,
            team_run_id: None,
            title: title.into(),
            body: String::new(),
            status: TaskStatus::Ready,
            node: None,
            agent_id: None,
            depends_on: vec![],
            branch: None,
            pr_url: None,
            notes: vec![],
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            remote_updated_at: None,
            flow: TaskFlow::default(),
        }
    }

    /// `#12` for a GitHub issue, `T-3` for a local task.
    pub fn handle(&self) -> String {
        match &self.external_id {
            Some(n) if self.backend == "github" => format!("#{n}"),
            _ => format!("T-{}", self.id),
        }
    }
}

/// A message between nodes of a run (SPEC §7 Message). `to_agent` is set
/// when the message is for one agent of the node (or once it is delivered
/// to one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: u64,
    pub team_run_id: String,
    pub from_node: String,
    #[serde(default)]
    pub from_agent: Option<String>,
    pub to_node: String,
    #[serde(default)]
    pub to_agent: Option<String>,
    /// `message`, `report`, `handoff`, `review_request`, `review`, `task`, `system`.
    pub kind: String,
    pub body: String,
    /// Kind-specific reference (task id, branch, artifact id…).
    #[serde(default)]
    pub r#ref: Value,
    pub created_at_ms: u64,
    #[serde(default)]
    pub delivered_at_ms: Option<u64>,
    #[serde(default)]
    pub read_at_ms: Option<u64>,
}

/// A design, doc or build artifact handed between nodes (SPEC §7 Artifact).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub feature_id: String,
    #[serde(default)]
    pub team_run_id: Option<String>,
    /// `doc`, `design`, `build`.
    pub kind: String,
    pub uri: String,
    pub from_node: String,
    #[serde(default)]
    pub to_node: Option<String>,
    #[serde(default)]
    pub note: String,
    pub created_at_ms: u64,
    /// What `uri` points at (FR-602); `None` for rows from before P3 or an
    /// unrecognised URI.
    #[serde(default)]
    pub meta: Option<orch_team::artifact::ArtifactMeta>,
}

/// Typed view of `projects.settings` (FR-109). Unknown keys are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Base branch for new sessions and features (default: the repo's default branch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_team: Option<String>,
    /// Run in each new worktree (outside the sandbox, by the core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_script: Option<String>,
    /// Copy `.env*` into worktrees (FR-110: opt-in, agents will read them).
    #[serde(default)]
    pub copy_env: bool,
    /// FR-505: the Notion page (URL or id) feature pages are published under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notion_parent_page: Option<String>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, Value>,
}

/// Typed view of `projects.task_backend` (ADR-004).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TaskBackendConfig {
    #[default]
    Local,
    /// GitHub Issues of `repo` (`owner/name`), or of the `origin` remote.
    Github {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
    },
    /// Linear issues of team `team` (its key, e.g. `ENG`), optionally of one
    /// project (id); `states` maps a workflow state id to a column, over the
    /// default by state type (FR-501).
    Linear {
        team: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        states: std::collections::BTreeMap<String, orch_team::TaskStatus>,
    },
}

impl TaskBackendConfig {
    pub fn kind(&self) -> &'static str {
        match self {
            TaskBackendConfig::Local => "local",
            TaskBackendConfig::Github { .. } => "github",
            TaskBackendConfig::Linear { .. } => "linear",
        }
    }
}

impl Project {
    pub fn typed_settings(&self) -> ProjectSettings {
        serde_json::from_value(self.settings.clone()).unwrap_or_default()
    }

    pub fn backend(&self) -> TaskBackendConfig {
        serde_json::from_value(self.task_backend.clone()).unwrap_or_default()
    }
}

/// Global rows plus `project`'s own, a project row shadowing the global one
/// of the same id. `rows` is (scope, id, data) ordered by name, id.
fn shadowed<T: serde::de::DeserializeOwned>(rows: Vec<(String, String, String)>) -> Result<Vec<T>> {
    let local: std::collections::HashSet<String> =
        rows.iter().filter(|(s, ..)| !s.is_empty()).map(|(_, id, _)| id.clone()).collect();
    rows.into_iter()
        .filter(|(s, id, _)| !s.is_empty() || !local.contains(id))
        .map(|(_, _, d)| Ok(serde_json::from_str(&d)?))
        .collect()
}

const FEATURE_COLS: &str =
    "id, project_id, slug, title, brief, base, integration_worktree, allocation, created_at_ms, archived";

fn row_feature(r: &rusqlite::Row) -> rusqlite::Result<Feature> {
    Ok(Feature {
        id: r.get(0)?,
        project_id: r.get(1)?,
        slug: r.get(2)?,
        title: r.get(3)?,
        brief: r.get(4)?,
        base: r.get(5)?,
        integration_worktree: r.get::<_, Option<String>>(6)?.map(PathBuf::from),
        allocation: r.get(7)?,
        created_at_ms: r.get::<_, i64>(8)? as u64,
        archived: r.get(9)?,
    })
}

const RUN_COLS: &str = "id, team_id, team_version, feature_id, status, resume_status, pr_url, started_at_ms, ended_at_ms, \
                        team_scope, pr_number, pr_status";

fn bad(s: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, s.into())
}

fn row_run(r: &rusqlite::Row) -> rusqlite::Result<TeamRun> {
    let status: String = r.get(4)?;
    let resume: Option<String> = r.get(5)?;
    Ok(TeamRun {
        id: r.get(0)?,
        team_id: r.get(1)?,
        team_version: r.get::<_, i64>(2)? as u32,
        feature_id: r.get(3)?,
        status: status.parse().map_err(bad)?,
        resume_status: resume.map(|s| s.parse().map_err(bad)).transpose()?,
        pr_url: r.get(6)?,
        started_at_ms: r.get::<_, i64>(7)? as u64,
        ended_at_ms: opt_u64(r, 8)?,
        team_scope: scope_of(r.get(9)?),
        // Runs done before the PR number was stored: read from the URL.
        pr_number: opt_u64(r, 10)?
            .or_else(|| r.get::<_, Option<String>>(6).ok().flatten().and_then(|u| pr_number_from_url(&u))),
        pr: r.get::<_, Option<String>>(11)?.map(|s| from_js(&s)).transpose()?,
    })
}

const TASK_COLS: &str = "id, project_id, backend, external_id, feature_id, team_run_id, title, body, status, node, agent_id, depends_on, branch, pr_url, notes, created_at_ms, updated_at_ms, remote_updated_at, flow";

fn row_task(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    let status: String = r.get(8)?;
    Ok(Task {
        id: r.get::<_, i64>(0)? as u64,
        project_id: r.get(1)?,
        backend: r.get(2)?,
        external_id: r.get(3)?,
        feature_id: r.get(4)?,
        team_run_id: r.get(5)?,
        title: r.get(6)?,
        body: r.get(7)?,
        status: status.parse().map_err(bad)?,
        node: r.get(9)?,
        agent_id: r.get(10)?,
        depends_on: from_js(&r.get::<_, String>(11)?)?,
        branch: r.get(12)?,
        pr_url: r.get(13)?,
        notes: from_js(&r.get::<_, String>(14)?)?,
        created_at_ms: r.get::<_, i64>(15)? as u64,
        updated_at_ms: r.get::<_, i64>(16)? as u64,
        remote_updated_at: r.get(17)?,
        flow: from_js(&r.get::<_, String>(18)?)?,
    })
}

const MSG_COLS: &str = "id, team_run_id, from_node, from_agent, to_node, to_agent, kind, body, ref, created_at_ms, delivered_at_ms, read_at_ms";

fn row_message(r: &rusqlite::Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get::<_, i64>(0)? as u64,
        team_run_id: r.get(1)?,
        from_node: r.get(2)?,
        from_agent: r.get(3)?,
        to_node: r.get(4)?,
        to_agent: r.get(5)?,
        kind: r.get(6)?,
        body: r.get(7)?,
        r#ref: from_js(&r.get::<_, String>(8)?)?,
        created_at_ms: r.get::<_, i64>(9)? as u64,
        delivered_at_ms: opt_u64(r, 10)?,
        read_at_ms: opt_u64(r, 11)?,
    })
}

const ARTIFACT_COLS: &str = "id, feature_id, team_run_id, kind, uri, from_node, to_node, note, created_at_ms, meta";

fn row_artifact(r: &rusqlite::Row) -> rusqlite::Result<Artifact> {
    Ok(Artifact {
        id: r.get::<_, i64>(0)? as u64,
        feature_id: r.get(1)?,
        team_run_id: r.get(2)?,
        kind: r.get(3)?,
        uri: r.get(4)?,
        from_node: r.get(5)?,
        to_node: r.get(6)?,
        note: r.get(7)?,
        created_at_ms: r.get::<_, i64>(8)? as u64,
        meta: r.get::<_, Option<String>>(9)?.and_then(|s| serde_json::from_str(&s).ok()),
    })
}

impl Store {
    // ------------------------------------------------------------ roles

    /// Inserts or replaces a role. The caller bumps `version` on a change.
    pub fn upsert_role(&self, role: &RoleDef, at_ms: u64) -> Result<()> {
        let role = role.clone();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO roles(id, scope, name, version, data, updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(scope, id) DO UPDATE SET name=excluded.name,
                   version=excluded.version, data=excluded.data, updated_at_ms=excluded.updated_at_ms, archived=0",
                params![role.id, scope_key(&role.scope), role.name, role.version, js(&role), at_ms as i64],
            )?;
            Ok(())
        })
    }

    /// The role `id` in exactly `scope`.
    pub fn role(&self, scope: &Scope, id: &str) -> Result<Option<RoleDef>> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.read(move |c| {
            let s: Option<String> =
                c.query_row("SELECT data FROM roles WHERE scope=?1 AND id=?2", [scope, id], |r| r.get(0)).optional()?;
            Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
        })
    }

    /// Global roles, plus `project`'s own when given (a project role
    /// shadows the global one of the same id). Archived ones are left out.
    pub fn roles(&self, project: Option<&str>) -> Result<Vec<RoleDef>> {
        let project = project.unwrap_or_default().to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT scope, id, data FROM roles WHERE archived=0 AND scope IN ('', ?1) ORDER BY name, id",
            )?;
            let rows = st.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            shadowed(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn archive_role(&self, scope: &Scope, id: &str) -> Result<()> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.write(move |tx| {
            tx.execute("UPDATE roles SET archived=1 WHERE scope=?1 AND id=?2", [scope, id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ teams

    /// Saves a team and snapshots it with the roles it uses under its
    /// version (`team_versions`, FR-308). A version already snapshotted is
    /// left as it was: the caller bumps `version` when the graph changes.
    pub fn save_team(&self, team: &Team, roles: &[RoleDef], at_ms: u64) -> Result<()> {
        let team = team.clone();
        let used = team.used_roles(roles);
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO teams(id, scope, name, version, data, updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(scope, id) DO UPDATE SET name=excluded.name,
                   version=excluded.version, data=excluded.data, updated_at_ms=excluded.updated_at_ms, archived=0",
                params![team.id, scope_key(&team.scope), team.name, team.version, js(&team), at_ms as i64],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO team_versions(team_scope, team_id, version, team, roles, created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![scope_key(&team.scope), team.id, team.version, js(&team), js(&used), at_ms as i64],
            )?;
            Ok(())
        })
    }

    /// The team `id` in exactly `scope` (archived ones too).
    pub fn team(&self, scope: &Scope, id: &str) -> Result<Option<Team>> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.read(move |c| {
            let s: Option<String> =
                c.query_row("SELECT data FROM teams WHERE scope=?1 AND id=?2", [scope, id], |r| r.get(0)).optional()?;
            Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
        })
    }

    /// The team `id` as seen from `project`: its own, else the global one.
    pub fn team_in(&self, project: Option<&str>, id: &str) -> Result<Option<Team>> {
        if let Some(p) = project
            && let Some(t) = self.team(&Scope::Project(p.to_string()), id)?
        {
            return Ok(Some(t));
        }
        self.team(&Scope::Global, id)
    }

    /// Global teams, plus `project`'s own (shadowing a global one of the
    /// same id). Archived ones are left out.
    pub fn teams(&self, project: Option<&str>) -> Result<Vec<Team>> {
        let project = project.unwrap_or_default().to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT scope, id, data FROM teams WHERE archived=0 AND scope IN ('', ?1) ORDER BY name, id",
            )?;
            let rows = st.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            shadowed(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The snapshot a run uses: the team graph and its roles at `version`.
    pub fn team_version(&self, scope: &Scope, id: &str, version: u32) -> Result<Option<(Team, Vec<RoleDef>)>> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.read(move |c| {
            let row: Option<(String, String)> = c
                .query_row(
                    "SELECT team, roles FROM team_versions WHERE team_scope=?1 AND team_id=?2 AND version=?3",
                    params![scope, id, version],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            row.map(|(t, r)| Ok::<_, anyhow::Error>((serde_json::from_str(&t)?, serde_json::from_str(&r)?))).transpose()
        })
    }

    pub fn archive_team(&self, scope: &Scope, id: &str) -> Result<()> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.write(move |tx| {
            tx.execute("UPDATE teams SET archived=1 WHERE scope=?1 AND id=?2", [scope, id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ features

    pub fn upsert_feature(&self, f: &Feature) -> Result<()> {
        let f = f.clone();
        let topics = vec![
            orch_protocol::Topic::Features,
            orch_protocol::Topic::Feature(f.id.clone()),
            orch_protocol::Topic::Project(f.project_id.clone()),
        ];
        self.write_touching(topics, move |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO features({FEATURE_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                     ON CONFLICT(id) DO UPDATE SET title=excluded.title, brief=excluded.brief, base=excluded.base,
                       integration_worktree=excluded.integration_worktree, allocation=excluded.allocation,
                       archived=excluded.archived"
                ),
                params![
                    f.id,
                    f.project_id,
                    f.slug,
                    f.title,
                    f.brief,
                    f.base,
                    f.integration_worktree.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    f.allocation,
                    f.created_at_ms as i64,
                    f.archived
                ],
            )?;
            Ok(())
        })
    }

    pub fn feature(&self, id: &str) -> Result<Option<Feature>> {
        let id = id.to_string();
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {FEATURE_COLS} FROM features WHERE id=?1"), [id], row_feature)
                .optional()?)
        })
    }

    pub fn feature_by_slug(&self, project_id: &str, slug: &str) -> Result<Option<Feature>> {
        let (p, s) = (project_id.to_string(), slug.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                &format!("SELECT {FEATURE_COLS} FROM features WHERE project_id=?1 AND slug=?2"),
                [p, s],
                row_feature,
            )
            .optional()?)
        })
    }

    /// A project's features (all projects when `None`), archived left out.
    pub fn features(&self, project_id: Option<&str>) -> Result<Vec<Feature>> {
        let p = project_id.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {FEATURE_COLS} FROM features WHERE archived=0 AND (?1 IS NULL OR project_id=?1)
                 ORDER BY created_at_ms DESC, id"
            ))?;
            Ok(st.query_map([p], row_feature)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ team runs

    pub fn upsert_run(&self, run: &TeamRun) -> Result<()> {
        let run = run.clone();
        let topics = vec![
            orch_protocol::Topic::Features,
            orch_protocol::Topic::Teams,
            orch_protocol::Topic::Feature(run.feature_id.clone()),
        ];
        self.write_touching(topics, move |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO team_runs({RUN_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                     ON CONFLICT(id) DO UPDATE SET team_version=excluded.team_version, status=excluded.status,
                       resume_status=excluded.resume_status, pr_url=excluded.pr_url, ended_at_ms=excluded.ended_at_ms,
                       pr_number=excluded.pr_number, pr_status=excluded.pr_status"
                ),
                params![
                    run.id,
                    run.team_id,
                    run.team_version,
                    run.feature_id,
                    run.status.as_str(),
                    run.resume_status.map(|s| s.as_str()),
                    run.pr_url,
                    run.started_at_ms as i64,
                    run.ended_at_ms.map(|v| v as i64),
                    scope_key(&run.team_scope),
                    run.pr_number.map(|v| v as i64),
                    run.pr.as_ref().map(js)
                ],
            )?;
            Ok(())
        })
    }

    pub fn run(&self, id: &str) -> Result<Option<TeamRun>> {
        let id = id.to_string();
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {RUN_COLS} FROM team_runs WHERE id=?1"), [id], row_run).optional()?)
        })
    }

    /// Runs, newest first; only unfinished ones when `active_only`.
    pub fn runs(&self, feature_id: Option<&str>, active_only: bool) -> Result<Vec<TeamRun>> {
        let f = feature_id.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM team_runs WHERE (?1 IS NULL OR feature_id=?1)
                   AND (NOT ?2 OR status NOT IN ('done','failed'))
                 ORDER BY started_at_ms DESC, id"
            ))?;
            Ok(st.query_map(params![f, active_only], row_run)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ tasks

    /// Inserts a task (`id == 0`) or updates it; returns its id.
    pub fn upsert_task(&self, t: &Task) -> Result<u64> {
        let t = t.clone();
        let mut topics = vec![orch_protocol::Topic::Board(t.project_id.clone())];
        topics.extend(t.feature_id.iter().map(|f| orch_protocol::Topic::Feature(f.clone())));
        self.write_touching(topics, move |tx| {
            let id = if t.id == 0 { None } else { Some(t.id as i64) };
            tx.execute(
                &format!(
                    "INSERT INTO tasks({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
                     ON CONFLICT(id) DO UPDATE SET backend=excluded.backend, external_id=excluded.external_id, feature_id=excluded.feature_id,
                       team_run_id=excluded.team_run_id, title=excluded.title, body=excluded.body,
                       status=excluded.status, node=excluded.node, agent_id=excluded.agent_id,
                       depends_on=excluded.depends_on, branch=excluded.branch, pr_url=excluded.pr_url,
                       notes=excluded.notes, updated_at_ms=excluded.updated_at_ms,
                       remote_updated_at=excluded.remote_updated_at, flow=excluded.flow"
                ),
                params![
                    id,
                    t.project_id,
                    t.backend,
                    t.external_id,
                    t.feature_id,
                    t.team_run_id,
                    t.title,
                    t.body,
                    t.status.as_str(),
                    t.node,
                    t.agent_id,
                    js(&t.depends_on),
                    t.branch,
                    t.pr_url,
                    js(&t.notes),
                    t.created_at_ms as i64,
                    t.updated_at_ms as i64,
                    t.remote_updated_at,
                    js(&t.flow)
                ],
            )?;
            Ok(match id {
                Some(i) => i as u64,
                None => tx.last_insert_rowid() as u64,
            })
        })
    }

    pub fn task(&self, id: u64) -> Result<Option<Task>> {
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {TASK_COLS} FROM tasks WHERE id=?1"), [id as i64], row_task).optional()?)
        })
    }

    pub fn task_by_external(&self, project_id: &str, backend: &str, external_id: &str) -> Result<Option<Task>> {
        let (p, b, e) = (project_id.to_string(), backend.to_string(), external_id.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                &format!("SELECT {TASK_COLS} FROM tasks WHERE project_id=?1 AND backend=?2 AND external_id=?3"),
                [p, b, e],
                row_task,
            )
            .optional()?)
        })
    }

    /// Tasks of a project, optionally of one feature or run, oldest first.
    pub fn tasks(&self, project_id: Option<&str>, feature_id: Option<&str>, run_id: Option<&str>) -> Result<Vec<Task>> {
        let (p, f, r) = (project_id.map(str::to_string), feature_id.map(str::to_string), run_id.map(str::to_string));
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {TASK_COLS} FROM tasks WHERE (?1 IS NULL OR project_id=?1) AND (?2 IS NULL OR feature_id=?2)
                   AND (?3 IS NULL OR team_run_id=?3) ORDER BY id"
            ))?;
            Ok(st.query_map(params![p, f, r], row_task)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// (done, total) tasks per team run, for progress counts.
    pub fn task_counts_by_run(&self) -> Result<std::collections::HashMap<String, (usize, usize)>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT team_run_id, SUM(status='done'), COUNT(*) FROM tasks
                 WHERE team_run_id IS NOT NULL GROUP BY team_run_id",
            )?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)? as usize, r.get::<_, i64>(2)? as usize)))
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ messages

    pub fn add_message(&self, m: &Message) -> Result<u64> {
        let m = m.clone();
        let (id, feature) = self.write(move |tx| {
            tx.execute(
                "INSERT INTO messages(team_run_id, from_node, from_agent, to_node, to_agent, kind, body, ref,
                   created_at_ms, delivered_at_ms, read_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    m.team_run_id,
                    m.from_node,
                    m.from_agent,
                    m.to_node,
                    m.to_agent,
                    m.kind,
                    m.body,
                    js(&m.r#ref),
                    m.created_at_ms as i64,
                    m.delivered_at_ms.map(|v| v as i64),
                    m.read_at_ms.map(|v| v as i64)
                ],
            )?;
            let id = tx.last_insert_rowid() as u64;
            // The feature pane shows the run's messages.
            let feature: Option<String> = tx
                .query_row("SELECT feature_id FROM team_runs WHERE id=?1", [&m.team_run_id], |r| r.get(0))
                .optional()?;
            Ok((id, feature))
        })?;
        self.touch(std::iter::once(orch_protocol::Topic::Teams).chain(feature.map(orch_protocol::Topic::Feature)));
        Ok(id)
    }

    /// Messages of a run, oldest first: for one node (and its agent: rows for
    /// the node as a whole or for that agent), unread only when `unread`.
    pub fn messages(&self, run_id: &str, to: Option<(&str, Option<&str>)>, unread: bool) -> Result<Vec<Message>> {
        let run = run_id.to_string();
        let node = to.map(|(n, _)| n.to_string());
        let agent = to.and_then(|(_, a)| a.map(str::to_string));
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {MSG_COLS} FROM messages WHERE team_run_id=?1
                   AND (?2 IS NULL OR (to_node=?2 AND (to_agent IS NULL OR ?3 IS NULL OR to_agent=?3)))
                   AND (NOT ?4 OR read_at_ms IS NULL)
                 ORDER BY id"
            ))?;
            Ok(st.query_map(params![run, node, agent, unread], row_message)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// A run's last `n` messages, oldest first.
    pub fn recent_messages(&self, run_id: &str, n: usize) -> Result<Vec<Message>> {
        let run = run_id.to_string();
        self.read(move |c| {
            let mut st =
                c.prepare(&format!("SELECT {MSG_COLS} FROM messages WHERE team_run_id=?1 ORDER BY id DESC LIMIT ?2"))?;
            let mut v: Vec<Message> =
                st.query_map(params![run, n as i64], row_message)?.collect::<rusqlite::Result<_>>()?;
            v.reverse();
            Ok(v)
        })
    }

    /// Runs `sql` once per message id (`?1`), the same `args` after it.
    fn each_message(&self, ids: &[u64], sql: &'static str, args: Vec<rusqlite::types::Value>) -> Result<()> {
        let ids = ids.to_vec();
        let topics = vec![orch_protocol::Topic::Teams];
        self.write_touching(topics, move |tx| {
            let mut st = tx.prepare_cached(sql)?;
            for id in ids {
                let all = std::iter::once(rusqlite::types::Value::Integer(id as i64)).chain(args.iter().cloned());
                st.execute(rusqlite::params_from_iter(all))?;
            }
            Ok(())
        })
    }

    /// Marks messages delivered (handed to the agent, not yet acted on) and
    /// binds them to the agent of the node that got them. They stay unread
    /// until [`Store::ack_messages`].
    pub fn deliver_messages(&self, ids: &[u64], agent_id: &str, at_ms: u64) -> Result<()> {
        self.each_message(
            ids,
            "UPDATE messages SET to_agent=coalesce(to_agent, ?2), delivered_at_ms=coalesce(delivered_at_ms, ?3) WHERE id=?1",
            vec![agent_id.to_string().into(), (at_ms as i64).into()],
        )
    }

    /// The turn that carried these messages ended: they are read.
    pub fn ack_messages(&self, ids: &[u64], at_ms: u64) -> Result<()> {
        self.each_message(
            ids,
            "UPDATE messages SET read_at_ms=coalesce(read_at_ms, ?2) WHERE id=?1",
            vec![(at_ms as i64).into()],
        )
    }

    /// The turn that carried these messages never ran (cancelled, dropped):
    /// unread ones are delivered again.
    pub fn undeliver_messages(&self, ids: &[u64]) -> Result<()> {
        self.each_message(ids, "UPDATE messages SET delivered_at_ms=NULL WHERE id=?1 AND read_at_ms IS NULL", vec![])
    }

    /// Marks messages delivered and read at once (shown in full where they
    /// are listed: the `inbox` tool), and to which agent of the node.
    pub fn mark_messages(&self, ids: &[u64], agent_id: Option<&str>, at_ms: u64) -> Result<()> {
        let ids = ids.to_vec();
        let agent = agent_id.map(str::to_string);
        let topics = vec![orch_protocol::Topic::Teams];
        self.write_touching(topics, move |tx| {
            for id in ids {
                tx.execute(
                    "UPDATE messages SET to_agent=coalesce(to_agent, ?2),
                       delivered_at_ms=coalesce(delivered_at_ms, ?3),
                       read_at_ms=coalesce(read_at_ms, ?3)
                     WHERE id=?1",
                    params![id as i64, agent, at_ms as i64],
                )?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ artifacts

    pub fn add_artifact(&self, a: &Artifact) -> Result<u64> {
        let a = a.clone();
        let topics = vec![orch_protocol::Topic::Feature(a.feature_id.clone())];
        self.write_touching(topics, move |tx| {
            tx.execute(
                "INSERT INTO artifacts(feature_id, team_run_id, kind, uri, from_node, to_node, note, created_at_ms, meta)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    a.feature_id,
                    a.team_run_id,
                    a.kind,
                    a.uri,
                    a.from_node,
                    a.to_node,
                    a.note,
                    a.created_at_ms as i64,
                    a.meta.as_ref().map(js)
                ],
            )?;
            Ok(tx.last_insert_rowid() as u64)
        })
    }

    pub fn artifacts(&self, feature_id: &str) -> Result<Vec<Artifact>> {
        let f = feature_id.to_string();
        self.read(move |c| {
            let mut st =
                c.prepare(&format!("SELECT {ARTIFACT_COLS} FROM artifacts WHERE feature_id=?1 ORDER BY id"))?;
            Ok(st.query_map([f], row_artifact)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ project settings

    /// FR-109: typed settings, task backend and whether `.orchestrator/` is tracked.
    pub fn set_project_settings(
        &self,
        id: &str,
        settings: &ProjectSettings,
        backend: &TaskBackendConfig,
        track_orchestrator_dir: bool,
    ) -> Result<()> {
        if let TaskBackendConfig::Github { repo: Some(r) } = backend
            && (r.split('/').count() != 2 || r.starts_with('-') || r.contains(char::is_whitespace))
        {
            bail!("GitHub repository must be owner/name, got {r:?}");
        }
        let topics = [orch_protocol::Topic::Project(id.to_string()), orch_protocol::Topic::Board(id.to_string())];
        let (id, s, b) = (id.to_string(), js(settings), js(backend));
        let n = self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE projects SET settings=?2, task_backend=?3, track_orchestrator_dir=?4 WHERE id=?1",
                params![id, s, b, track_orchestrator_dir],
            )?)
        })?;
        if n == 0 {
            return Err(anyhow!("no such project"));
        }
        self.note(topics);
        Ok(())
    }

    /// J11 "delete" for the team side of a project: features, runs, tasks,
    /// messages, artifacts, project-scoped roles and teams. Events stay.
    pub(crate) fn delete_project_teams(tx: &rusqlite::Transaction, id: &str) -> rusqlite::Result<()> {
        tx.execute(
            "DELETE FROM messages WHERE team_run_id IN
               (SELECT r.id FROM team_runs r JOIN features f ON f.id=r.feature_id WHERE f.project_id=?1)",
            [id],
        )?;
        tx.execute("DELETE FROM artifacts WHERE feature_id IN (SELECT id FROM features WHERE project_id=?1)", [id])?;
        tx.execute("DELETE FROM team_runs WHERE feature_id IN (SELECT id FROM features WHERE project_id=?1)", [id])?;
        tx.execute("DELETE FROM tasks WHERE project_id=?1", [id])?;
        tx.execute("DELETE FROM features WHERE project_id=?1", [id])?;
        tx.execute("DELETE FROM team_versions WHERE team_scope=?1", [id])?;
        tx.execute("DELETE FROM teams WHERE scope=?1", [id])?;
        tx.execute("DELETE FROM roles WHERE scope=?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_team::presets;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("o.db")).unwrap();
        (d, s)
    }

    #[test]
    fn roles_and_team_versions() {
        let (_d, s) = store();
        let (roles, mut team) = presets::full_stack();
        for r in &roles {
            s.upsert_role(r, 1).unwrap();
        }
        let mut proj = roles[1].clone();
        proj.id = "backend-shop".into();
        proj.scope = Scope::Project("p1".into());
        s.upsert_role(&proj, 1).unwrap();
        assert_eq!(s.roles(None).unwrap().len(), roles.len());
        assert_eq!(s.roles(Some("p1")).unwrap().len(), roles.len() + 1);
        s.save_team(&team, &roles, 1).unwrap();
        team.version = 2;
        team.nodes.retain(|n| n.id != "ux");
        team.edges.retain(|e| e.from != "ux" && e.to != "ux");
        s.save_team(&team, &roles, 2).unwrap();
        let g = Scope::Global;
        assert_eq!(s.team(&g, "full-stack").unwrap().unwrap().version, 2);
        let (v1, r1) = s.team_version(&g, "full-stack", 1).unwrap().unwrap();
        assert_eq!(v1.nodes.len(), 5);
        assert_eq!(r1.len(), 5);
        let (v2, r2) = s.team_version(&g, "full-stack", 2).unwrap().unwrap();
        assert_eq!(v2.nodes.len(), 4);
        assert!(r2.iter().all(|r| r.id != "ux"));
        s.archive_team(&g, "full-stack").unwrap();
        assert!(s.teams(None).unwrap().is_empty());
        assert!(s.team_version(&g, "full-stack", 1).unwrap().is_some(), "snapshots outlive the team");
    }

    #[test]
    fn a_project_role_or_team_shadows_the_global_one() {
        let (_d, s) = store();
        let (roles, team) = presets::full_stack();
        for r in &roles {
            s.upsert_role(r, 1).unwrap();
        }
        s.save_team(&team, &roles, 1).unwrap();
        let p1 = Scope::Project("p1".into());
        let mut backend = roles.iter().find(|r| r.id == "backend").unwrap().clone();
        backend.scope = p1.clone();
        backend.prompt = "p1's own backend".into();
        s.upsert_role(&backend, 2).unwrap();
        let mut local = team.clone();
        local.scope = p1.clone();
        local.name = "p1's team".into();
        local.nodes.retain(|n| n.id != "ux");
        local.edges.retain(|e| e.from != "ux" && e.to != "ux");
        let p1_roles = s.roles(Some("p1")).unwrap();
        s.save_team(&local, &p1_roles, 2).unwrap();

        // The global rows are untouched.
        let g = Scope::Global;
        assert_ne!(s.role(&g, "backend").unwrap().unwrap().prompt, "p1's own backend");
        assert_eq!(s.team(&g, "full-stack").unwrap().unwrap().nodes.len(), 5);
        assert_eq!(s.roles(None).unwrap().len(), roles.len());
        // From p1, its own shadow the global ones; from p2, only globals.
        assert_eq!(p1_roles.len(), roles.len());
        assert_eq!(p1_roles.iter().find(|r| r.id == "backend").unwrap().prompt, "p1's own backend");
        assert_eq!(s.teams(Some("p1")).unwrap().len(), 1);
        assert_eq!(s.team_in(Some("p1"), "full-stack").unwrap().unwrap().name, "p1's team");
        assert_eq!(s.team_in(Some("p2"), "full-stack").unwrap().unwrap().nodes.len(), 5);
        let (snap, snap_roles) = s.team_version(&p1, "full-stack", 1).unwrap().unwrap();
        assert_eq!(snap.nodes.len(), 4);
        assert_eq!(snap_roles.iter().find(|r| r.id == "backend").unwrap().prompt, "p1's own backend");
        assert_eq!(s.team_version(&g, "full-stack", 1).unwrap().unwrap().0.nodes.len(), 5);
    }

    #[test]
    fn features_runs_tasks_messages_artifacts() {
        let (_d, s) = store();
        let f = Feature {
            id: "f1".into(),
            project_id: "p1".into(),
            slug: "checkout-v2".into(),
            title: "Checkout v2".into(),
            brief: "b".into(),
            base: "main".into(),
            integration_worktree: None,
            allocation: None,
            created_at_ms: 1,
            archived: false,
        };
        s.upsert_feature(&f).unwrap();
        assert_eq!(s.feature_by_slug("p1", "checkout-v2").unwrap().unwrap().id, "f1");
        assert_eq!(f.integration_branch(), "orch/checkout-v2/integration");
        let mut run = TeamRun {
            id: "r1".into(),
            team_id: "full-stack".into(),
            team_scope: Scope::Project("p1".into()),
            team_version: 1,
            feature_id: "f1".into(),
            status: RunState::Spec,
            resume_status: None,
            pr_url: None,
            started_at_ms: 1,
            ended_at_ms: None,
            pr_number: None,
            pr: None,
        };
        s.upsert_run(&run).unwrap();
        assert_eq!(s.run("r1").unwrap().unwrap(), run, "no PR yet: None");
        run.pr_number = Some(12);
        run.pr = Some(PrStatus {
            state: "open".into(),
            head_sha: "abc".into(),
            base: "main".into(),
            ci: CiSummary {
                overall: "error".into(),
                done: 1,
                total: 2,
                jobs: vec![CiJob {
                    name: "test".into(),
                    workflow: Some("ci".into()),
                    state: "failure".into(),
                    url: None,
                }],
            },
            position: PrPosition { status: "behind".into(), behind: 2, ahead: 3, base_sha: Some("def".into()) },
            pushed_sha: Some("abc".into()),
            unpushed: 1,
            rewritten: false,
            checked_at_ms: 9,
            error: None,
        });
        s.upsert_run(&run).unwrap();
        assert_eq!(s.run("r1").unwrap().unwrap(), run, "the PR status round-trips");
        let mut legacy = run.clone();
        (legacy.id, legacy.status, legacy.pr_number, legacy.pr) = ("r0".into(), RunState::Done, None, None);
        legacy.pr_url = Some("https://github.com/o/r/pull/7".into());
        s.upsert_run(&legacy).unwrap();
        assert_eq!(s.run("r0").unwrap().unwrap().pr_number, Some(7), "a run from before: the number from its URL");
        run.status = RunState::Paused;
        run.resume_status = Some(RunState::Spec);
        s.upsert_run(&run).unwrap();
        assert_eq!(s.run("r1").unwrap().unwrap(), run);
        assert_eq!(s.runs(Some("f1"), true).unwrap().len(), 1);
        run.status = RunState::Done;
        s.upsert_run(&run).unwrap();
        assert!(s.runs(None, true).unwrap().is_empty());

        let mut t = Task::new("p1", "local", "Session endpoint", 5);
        t.team_run_id = Some("r1".into());
        t.node = Some("back".into());
        let id = s.upsert_task(&t).unwrap();
        t.id = id;
        let mut t2 = Task::new("p1", "github", "Form", 6);
        t2.external_id = Some("15".into());
        t2.depends_on = vec![id];
        t2.id = s.upsert_task(&t2).unwrap();
        assert_eq!(t2.handle(), "#15");
        assert_eq!(t.handle(), format!("T-{id}"));
        t.status = TaskStatus::Done;
        t.notes.push(TaskNote { ts_ms: 7, by: "back".into(), text: "merged".into() });
        s.upsert_task(&t).unwrap();
        assert_eq!(s.task(id).unwrap().unwrap(), t);
        assert_eq!(s.task_by_external("p1", "github", "15").unwrap().unwrap().depends_on, vec![id]);
        assert_eq!(s.tasks(Some("p1"), None, Some("r1")).unwrap().len(), 1);

        let m = Message {
            id: 0,
            team_run_id: "r1".into(),
            from_node: "lead".into(),
            from_agent: Some("lead-1".into()),
            to_node: "back".into(),
            to_agent: None,
            kind: "message".into(),
            body: "hi".into(),
            r#ref: Value::Null,
            created_at_ms: 1,
            delivered_at_ms: None,
            read_at_ms: None,
        };
        let m1 = s.add_message(&m).unwrap();
        let m2 = s.add_message(&Message { to_agent: Some("back-2".into()), ..m.clone() }).unwrap();
        assert_eq!(s.messages("r1", Some(("back", Some("back-1"))), true).unwrap().len(), 1);
        assert_eq!(s.messages("r1", Some(("back", Some("back-2"))), true).unwrap().len(), 2);
        s.mark_messages(&[m1], Some("back-1"), 9).unwrap();
        let got = s.messages("r1", None, false).unwrap().into_iter().find(|x| x.id == m1).unwrap();
        assert_eq!((got.delivered_at_ms, got.read_at_ms, got.to_agent.as_deref()), (Some(9), Some(9), Some("back-1")));
        s.mark_messages(&[m1, m2], None, 10).unwrap();
        assert!(s.messages("r1", Some(("back", None)), true).unwrap().is_empty());
        assert_eq!(s.messages("r1", None, false).unwrap().len(), 2);

        s.add_artifact(&Artifact {
            id: 0,
            feature_id: "f1".into(),
            team_run_id: Some("r1".into()),
            kind: "design".into(),
            uri: "penpot://x".into(),
            from_node: "ux".into(),
            to_node: Some("ui".into()),
            note: String::new(),
            created_at_ms: 3,
            meta: None,
        })
        .unwrap();
        let meta = orch_team::artifact::parse("design/a.png");
        s.add_artifact(&Artifact {
            id: 0,
            feature_id: "f1".into(),
            team_run_id: Some("r1".into()),
            kind: "design".into(),
            uri: "design/a.png".into(),
            from_node: "ui".into(),
            to_node: Some("front".into()),
            note: String::new(),
            created_at_ms: 4,
            meta: meta.clone(),
        })
        .unwrap();
        let arts = s.artifacts("f1").unwrap();
        assert_eq!((arts.len(), &arts[0].meta, &arts[1].meta), (2, &None, &meta));

        s.add_run_event(5, "r1", Some("lead-1"), "run.state", serde_json::json!({"to": "spec_gate"}));
        s.add_event(6, Some("lead-1"), "hook.Stop", Value::Null);
        s.flush().unwrap();
        let ev = s.run_events("r1", 10).unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "run.state");
    }

    #[test]
    fn project_settings_and_delete_cascade() {
        let (_d, s) = store();
        let pr = crate::model::Project {
            id: "p1".into(),
            name: "shop".into(),
            path: "/tmp/shop".into(),
            default_branch: "main".into(),
            remotes: vec![],
            worktree_root: "/tmp/shop.worktrees".into(),
            task_backend: serde_json::json!({"type": "local"}),
            track_orchestrator_dir: true,
            settings: serde_json::json!({"keep": 1}),
            created_at_ms: 1,
            last_active_at_ms: 1,
            last_fetch_at_ms: None,
            last_fetch_error: None,
            archived: false,
        };
        s.upsert_project(&pr).unwrap();
        let mut st = s.project("p1").unwrap().unwrap().typed_settings();
        assert_eq!(st.other.get("keep"), Some(&serde_json::json!(1)));
        st.default_team = Some("full-stack".into());
        let gh = TaskBackendConfig::Github { repo: Some("me/shop".into()) };
        s.set_project_settings("p1", &st, &gh, false).unwrap();
        let p = s.project("p1").unwrap().unwrap();
        assert_eq!(p.backend(), gh);
        assert_eq!(p.typed_settings().default_team.as_deref(), Some("full-stack"));
        assert_eq!(p.typed_settings().other.get("keep"), Some(&serde_json::json!(1)));
        assert!(!p.track_orchestrator_dir);
        assert!(
            s.set_project_settings("p1", &st, &TaskBackendConfig::Github { repo: Some("bad".into()) }, true).is_err()
        );
        assert!(s.set_project_settings("nope", &st, &gh, true).is_err());

        let mut t = Task::new("p1", "local", "x", 1);
        t.feature_id = Some("f1".into());
        s.upsert_task(&t).unwrap();
        s.upsert_feature(&Feature {
            id: "f1".into(),
            project_id: "p1".into(),
            slug: "x".into(),
            title: "x".into(),
            brief: String::new(),
            base: "main".into(),
            integration_worktree: None,
            allocation: None,
            created_at_ms: 1,
            archived: false,
        })
        .unwrap();
        s.delete_project("p1").unwrap();
        assert!(s.tasks(Some("p1"), None, None).unwrap().is_empty());
        assert!(s.features(Some("p1")).unwrap().is_empty());
    }
}
