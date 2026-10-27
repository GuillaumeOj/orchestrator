//! Team runs (SPEC §4.4, J3, ADR-003/004): launching a team on a feature,
//! dispatching tasks to nodes, routing messages, the run's state machine,
//! team controls and the team tools of the orchestrator MCP server.
//!
//! - **Launch**: the team is snapshotted (`team_versions`, FR-308), a Feature
//!   and a TeamRun are stored, the integration worktree is created and the
//!   lead is created on it with the brief as its first turn. Like every
//!   autonomous turn it waits for the Governor; the lead's process starts
//!   when it is admitted ("Not started yet" until then).
//! - **Dispatch** (FR-404): a Ready task whose dependencies are done goes to a
//!   free agent of its node, or to a new one (`orch/<feature>/<node>-<n>`,
//!   branched from the integration branch, up to the node's concurrency).
//! - **Messages** (FR-403): a busy agent gets them at its next tool call
//!   (PostToolUse `additionalContext`); an idle or suspended one as a turn
//!   through the Governor.
//! - **Merges** run in the core (like commits): sandboxed git can't write the
//!   repository's git dir from a linked worktree (FR-702 known gap).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use governor::{AgentId, TurnKind};
use orch_git::commit::MergeOutcome;
use orch_protocol::{WorktreeKindSpec, now_ms};
use orch_store::{Artifact, Feature, Message, Project, Task, TeamRun};
use orch_tasks::TaskService;
use orch_team::{FeatureCtx, RoleDef, RunState, TaskStatus, Team, rules};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::role::Role;
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::{Admission, AgentRecord, TeamSeat, TurnRow, TurnStatus};

/// What to launch (J3 step 2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub project_id: String,
    pub team_id: String,
    /// Feature slug: new, or an existing feature of the project.
    pub slug: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub brief: String,
    /// Base of the integration branch (default: the project's base branch).
    #[serde(default)]
    pub base: Option<String>,
}

/// Everything about a run, loaded together.
#[derive(Clone, Debug)]
pub struct RunCtx {
    pub run: TeamRun,
    pub feature: Feature,
    pub team: Team,
    pub roles: Vec<RoleDef>,
    pub project: Project,
}

impl RunCtx {
    fn fctx(&self) -> FeatureCtx {
        FeatureCtx {
            slug: self.feature.slug.clone(),
            title: self.feature.title.clone(),
            base: self.feature.base.clone(),
        }
    }
}

/// Builds the `gh` the GitHub task backend uses (tests pass a fake).
#[derive(Clone)]
pub struct GhFactory(pub std::sync::Arc<dyn Fn() -> Box<dyn orch_tasks::github::Gh> + Send + Sync>);

impl std::fmt::Debug for GhFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GhFactory")
    }
}

/// Who sends a message: a node (or [`rules::USER`]) and, for an agent, its id.
#[derive(Clone, Copy, Debug)]
pub struct Sender<'a>(pub &'a str, pub Option<&'a str>);

/// FR-407: a working agent silent this long gets an Inbox alert.
pub const NO_PROGRESS_MS: u64 = 15 * 60 * 1000;

/// What the watchdog found about one agent's running turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// Past its role's ceiling (minutes).
    Ceiling(u32),
    /// No hook for [`NO_PROGRESS_MS`].
    Stalled,
}

/// FR-407 for one agent at `now`: what is wrong with its running turn, with
/// the turn's start (the key that makes each alert once per turn).
pub fn watchdog_check(a: &AgentRecord, now: u64) -> Option<(Watch, u64)> {
    let start = a.tracker.turn_started_ms?;
    if a.tracker.state != AgentState::Working {
        return None;
    }
    if let Some(m) = a.role.turn_ceiling_minutes
        && now.saturating_sub(start) > u64::from(m) * 60_000
    {
        return Some((Watch::Ceiling(m), start));
    }
    let last = a.last_event_at_ms.unwrap_or(start).max(start);
    (a.tracker.background_tasks == 0 && now.saturating_sub(last) > NO_PROGRESS_MS).then_some((Watch::Stalled, start))
}

/// Longest message body kept (the rest is cut).
const MAX_BODY: usize = 8_000;
/// Longest text injected at a tool call.
const MAX_INJECT: usize = 1_500;

pub(crate) fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

pub fn agent_id_for(slug: &str, node: &str, n: u32) -> String {
    format!("{slug}-{node}-{n}")
}

/// The state a run is in or, paused or deferred, the one it resumes into.
pub fn effective(run: &TeamRun) -> RunState {
    if run.status.held() { run.resume_status.unwrap_or(run.status) } else { run.status }
}

impl Runtime {
    // ------------------------------------------------------------ loading

    pub fn run_ctx(&self, run_id: &str) -> Result<RunCtx> {
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let feature = self.db.feature(&run.feature_id)?.ok_or_else(|| anyhow!("no feature {}", run.feature_id))?;
        let (team, roles) = self
            .db
            .team_version(&run.team_scope, &run.team_id, run.team_version)?
            .ok_or_else(|| anyhow!("team {} v{} is gone", run.team_id, run.team_version))?;
        let project =
            self.db.project(&feature.project_id)?.ok_or_else(|| anyhow!("no project {}", feature.project_id))?;
        Ok(RunCtx { run, feature, team, roles, project })
    }

    /// The agents of a run (archived and killed ones left out), by id.
    pub fn run_agents(&self, run_id: &str) -> Vec<AgentRecord> {
        self.store.read(|s| {
            s.agents
                .values()
                .filter(|a| a.team.as_ref().is_some_and(|t| t.run_id == run_id))
                .filter(|a| !matches!(a.tracker.state, AgentState::Archived | AgentState::Killed))
                .cloned()
                .collect()
        })
    }

    pub(crate) fn lead_of(&self, cx: &RunCtx) -> Option<AgentRecord> {
        let lead = cx.team.lead()?.id.clone();
        self.run_agents(&cx.run.id).into_iter().find(|a| a.team.as_ref().is_some_and(|t| t.node == lead))
    }

    pub(crate) fn run_event(&self, run_id: &str, agent: Option<&str>, kind: &str, detail: Value) {
        self.db.add_run_event(now_ms(), run_id, agent, kind, detail);
    }

    fn tasks_of(&self, cx: &RunCtx) -> TaskService<'_> {
        self.tasks_for(&cx.project)
    }

    /// The tasks of `project` over its backend, kept across calls.
    pub fn tasks_for(&self, project: &orch_store::Project) -> TaskService<'_> {
        self.task_backends.service(&self.db, project, || self.gh_override())
    }

    /// Tests replace `gh` (orch-tasks' `Gh`); `None` = the real one.
    fn gh_override(&self) -> Option<Box<dyn orch_tasks::github::Gh>> {
        self.cfg.gh.as_ref().map(|f| (f.0)())
    }

    // ------------------------------------------------------------ run state

    /// Moves a run to `to` (checked against §6.2) and logs it on its timeline.
    pub fn set_run_state(&self, run_id: &str, to: RunState, why: &str) -> Result<TeamRun> {
        let mut run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        if run.status == to {
            return Ok(run);
        }
        if !run.status.can_go(to) {
            bail!("a {} run can't go to {}", run.status, to);
        }
        let from = run.status;
        if to.held() && !from.held() {
            run.resume_status = Some(from);
        }
        if from.held() && !to.held() {
            run.resume_status = None;
        }
        run.status = to;
        if to.finished() {
            run.ended_at_ms = Some(now_ms());
        }
        self.db.upsert_run(&run)?;
        self.run_event(run_id, None, "run.state", json!({"from": from, "to": to, "why": why}));
        Ok(run)
    }

    /// A move the run makes by itself (a gate decided, a PR asked or
    /// opened). On a paused or deferred run it becomes the state the run
    /// resumes into, so the pause holds (its agents stay paused).
    pub(crate) fn advance_run(&self, run_id: &str, to: RunState, why: &str) -> Result<TeamRun> {
        let mut run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        if !run.status.held() || to.held() || to == RunState::Failed {
            return self.set_run_state(run_id, to, why);
        }
        let from = effective(&run);
        if from == to {
            return Ok(run);
        }
        if !from.can_go(to) {
            bail!("a {from} run can't go to {to}");
        }
        run.resume_status = Some(to);
        self.db.upsert_run(&run)?;
        self.run_event(run_id, None, "run.state", json!({"from": from, "to": to, "why": why, "while": run.status}));
        Ok(run)
    }

    // ------------------------------------------------------------ launch

    /// J3 steps 2–3. Returns the run; the lead waits for the Governor.
    pub fn launch(&self, spec: &LaunchSpec) -> Result<TeamRun> {
        let spec = &LaunchSpec {
            slug: spec.slug.trim().to_string(),
            base: spec.base.clone().filter(|b| !b.trim().is_empty()),
            ..spec.clone()
        };
        let project = self.db.project(&spec.project_id)?.ok_or_else(|| anyhow!("no project {}", spec.project_id))?;
        let team =
            self.db.team_in(Some(&project.id), &spec.team_id)?.ok_or_else(|| anyhow!("no team {}", spec.team_id))?;
        let roles = self.db.roles(Some(&project.id))?;
        let issues = orch_team::validate(&team, Some(&roles));
        if !issues.is_empty() {
            bail!("the team is not valid: {}", issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "));
        }
        if !orch_team::valid_slug(&spec.slug) || spec.slug == "solo" {
            bail!("invalid feature slug {:?} (a-z, 0-9, - and _)", spec.slug);
        }
        let lead = team.lead().ok_or_else(|| anyhow!("no lead"))?.clone();
        if !orch_git::valid_id(&agent_id_for(&spec.slug, &lead.id, 1))
            || team.nodes.iter().any(|n| !orch_git::valid_id(&agent_id_for(&spec.slug, &n.id, 99)))
        {
            bail!("feature slug {:?} is too long", spec.slug);
        }
        let now = now_ms();
        let feature = match self.db.feature_by_slug(&project.id, &spec.slug)? {
            Some(f) => {
                if !self.db.runs(Some(&f.id), true)?.is_empty() {
                    bail!("feature {} already has a running team", f.slug);
                }
                // A deleted feature launched again comes back.
                Feature { archived: false, ..f }
            }
            None => {
                let base = spec
                    .base
                    .clone()
                    .or_else(|| project.typed_settings().base_branch)
                    .unwrap_or_else(|| project.default_branch.clone());
                Feature {
                    id: format!("{}-{}", project.id, spec.slug),
                    project_id: project.id.clone(),
                    slug: spec.slug.clone(),
                    title: if spec.title.trim().is_empty() { spec.slug.clone() } else { spec.title.trim().to_string() },
                    brief: spec.brief.trim().to_string(),
                    base,
                    integration_worktree: None,
                    allocation: None,
                    created_at_ms: now,
                    archived: false,
                }
            }
        };
        // The snapshot this run uses (FR-308). A snapshot of this version
        // taken earlier with other roles (a role edited since, or created
        // after a draft was saved) gets a new version instead of being kept.
        let mut team = team;
        let used = team.used_roles(&roles);
        while let Some((t, r)) = self.db.team_version(&team.scope, &team.id, team.version)? {
            if t == team && r == used {
                break;
            }
            team.version += 1;
        }
        self.db.save_team(&team, &roles, now)?;
        let row = self.create_worktree(
            &project.path,
            &WorktreeKindSpec::Integration { feature: feature.slug.clone() },
            Some(&feature.base),
        )?;
        let mut feature = feature;
        feature.integration_worktree = Some(row.path.clone());
        if !spec.brief.trim().is_empty() {
            feature.brief = spec.brief.trim().to_string();
        }
        self.db.upsert_feature(&feature)?;
        let run = TeamRun {
            id: format!("run-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]),
            team_id: team.id.clone(),
            team_scope: team.scope.clone(),
            team_version: team.version,
            feature_id: feature.id.clone(),
            status: RunState::Draft,
            resume_status: None,
            pr_url: None,
            started_at_ms: now,
            ended_at_ms: None,
        };
        self.db.upsert_run(&run)?;
        self.run_event(
            &run.id,
            None,
            "run.launched",
            json!({"team": team.id, "version": team.version, "feature": feature.slug}),
        );
        let cx = RunCtx { run: run.clone(), feature: feature.clone(), team, roles, project };
        // A launch that fails from here ends its run: a draft left behind
        // would count as the feature's running team forever.
        match self.start_lead(&cx, &lead.id, &row) {
            Ok(()) => self.set_run_state(&run.id, RunState::Spec, "launched"),
            Err(e) => {
                let _ = self.set_run_state(&run.id, RunState::Failed, &format!("launch failed: {e:#}"));
                Err(e)
            }
        }
    }

    /// The lead of a new run, on the integration worktree, with the brief as
    /// its first turn. A feature launched again keeps its earlier agents: the
    /// new lead takes the next free number.
    fn start_lead(&self, cx: &RunCtx, lead: &str, row: &orch_store::WorktreeRow) -> Result<()> {
        let feature = &cx.feature;
        let rec = self.add_team_agent(cx, lead, self.free_agent_number(&feature.slug, lead)?, row)?;
        let brief = format!(
            "ORCH: you lead the team on feature {} ({}). Brief from the user:\n\n{}\n\nFollow your protocol: write the docs in {}, commit them, then call mcp__orch__request_spec_approval.",
            feature.title,
            feature.slug,
            if feature.brief.is_empty() {
                "(no brief: read the code and ask the user with mcp__orch__ask_user)"
            } else {
                &feature.brief
            },
            cx.fctx().docs_dir()
        );
        self.deliver_kind(&rec.id, &brief, TurnKind::Task)?;
        Ok(())
    }

    /// The first `n` with no agent `<slug>-<node>-<n>` yet (archived ones
    /// keep their number).
    fn free_agent_number(&self, slug: &str, node: &str) -> Result<u32> {
        (1..=99)
            .find(|k| !self.store.read(|s| s.agents.contains_key(&agent_id_for(slug, node, *k))))
            .ok_or_else(|| anyhow!("no free agent number for {node}"))
    }

    /// The branch an agent works on, as its worktree row records it.
    fn branch_of(&self, rec: &AgentRecord) -> String {
        self.store.read(|s| s.worktrees.get(&rec.worktree).map(|w| w.branch.clone())).unwrap_or_default()
    }

    /// The agent's own git identity file, when it has one.
    pub(crate) fn agent_gitconfig(&self, id: &str) -> Option<std::path::PathBuf> {
        Some(self.cfg.paths.agent_dir(id).join("gitconfig")).filter(|p| p.is_file())
    }

    /// Sets a task's status and what follows for its agent: back in the
    /// queue (Ready, Backlog) it loses its agent, whose seat is freed; in
    /// review or done the agent stays on record and its seat is freed.
    pub(crate) fn apply_status(&self, t: &mut Task, s: TaskStatus) {
        t.status = s;
        match s {
            TaskStatus::Ready | TaskStatus::Backlog => {
                if let Some(a) = t.agent_id.take() {
                    self.free_seat(&a, t.id);
                }
            }
            TaskStatus::Review | TaskStatus::Done => {
                if let Some(a) = &t.agent_id {
                    self.free_seat(a, t.id);
                }
            }
            TaskStatus::InProgress => {}
        }
    }

    /// Creates an agent for `node` in `worktree` (not started: its first
    /// turn starts it once the Governor admits it).
    fn add_team_agent(&self, cx: &RunCtx, node: &str, n: u32, row: &orch_store::WorktreeRow) -> Result<AgentRecord> {
        let id = agent_id_for(&cx.feature.slug, node, n);
        let cfg = orch_team::compile_node(&cx.team, node, &cx.roles, &cx.fctx()).map_err(|e| anyhow!(e))?;
        let role = Role::from_node(&cfg).map_err(|e| anyhow!(e))?;
        if self.store.read(|s| s.agents.get(&id).is_some_and(|a| a.tracker.state != AgentState::Archived)) {
            bail!("agent {id} exists");
        }
        let rec = AgentRecord {
            id: id.clone(),
            repo: row.repo.clone(),
            worktree: row.path.clone(),
            role,
            session_id: uuid::Uuid::new_v4().to_string(),
            tracker: Default::default(),
            pid: None,
            pgid: None,
            allocation: cx.feature.allocation.clone().unwrap_or_else(|| cx.project.name.clone()),
            tier: cfg.tier,
            created_at_ms: now_ms(),
            last_event: None,
            last_event_at_ms: None,
            spawns: 0,
            flag: None,
            trust_dialog: false,
            team: Some(TeamSeat {
                run_id: cx.run.id.clone(),
                node: node.into(),
                feature_id: cx.feature.id.clone(),
                project_id: cx.project.id.clone(),
                task: None,
            }),
        };
        self.store.write(|s| s.agents.insert(id.clone(), rec.clone()))?;
        // Bind the worktree row to its agent.
        self.store.write(|s| {
            if let Some(w) = s.worktrees.get_mut(&row.path) {
                w.agent_id = Some(id.clone());
            }
        })?;
        if cx.run.status.held() {
            self.governor_act(|g, now| (g.pause_agent(&AgentId(id.clone()), now), Vec::new()));
        }
        self.run_event(&cx.run.id, Some(&id), "agent.added", json!({"node": node, "branch": row.branch}));
        Ok(rec)
    }

    /// Queues a turn carrying team messages: they are read when it ends.
    pub(crate) fn deliver_messages_turn(&self, id: &str, text: &str, messages: Vec<u64>) -> Result<String> {
        let tid = self.deliver_kind(id, text, TurnKind::Message)?;
        self.store.write(|s| {
            if let Some(t) = s.turns.iter_mut().find(|t| t.id == tid) {
                t.messages = messages;
            }
        })?;
        Ok(tid)
    }

    /// Queues a turn of `kind` (delivered when the Governor admits it).
    pub(crate) fn deliver_kind(&self, id: &str, text: &str, kind: TurnKind) -> Result<String> {
        match self.store.read(|s| s.agents.get(id).map(|a| a.tracker.state)) {
            None => bail!("no agent {id}"),
            Some(AgentState::Archived) => bail!("agent {id} is deleted"),
            Some(_) => {}
        }
        let turn = TurnRow::queued(id, text, kind, Admission::Pending);
        let tid = turn.id.clone();
        self.store.write(|s| s.turns.push(turn))?;
        Ok(tid)
    }

    // ------------------------------------------------------------ the periodic pass

    /// Dispatch, message delivery and run progression for every active run
    /// (call every few seconds, off the delivery loop: a GitHub backend makes
    /// network calls).
    pub fn team_tick(&self) {
        let runs = self.db.runs(None, true).unwrap_or_default();
        for run in runs {
            let cx = match self.run_ctx(&run.id) {
                Ok(cx) => cx,
                Err(e) => {
                    self.event("-", "team_error", json!({"run": run.id, "error": e.to_string()}));
                    continue;
                }
            };
            self.promote_lead(&cx);
            if let Err(e) = self.dispatch(&cx) {
                self.run_event(&run.id, None, "dispatch_error", json!(format!("{e:#}")));
            }
            self.supervise(&cx, now_ms());
            self.deliver_messages(&cx);
            if let Err(e) = self.progress(&cx) {
                self.run_event(&run.id, None, "team_error", json!(format!("{e:#}")));
            }
        }
    }

    /// A lead started before leads ran at the critical tier gets it now,
    /// and the lead tools added since (seen from its next start).
    fn promote_lead(&self, cx: &RunCtx) {
        let tools = orch_team::compile::tools_for(true);
        let _ = self.store.write(|s| {
            for a in s.agents.values_mut() {
                if !a.team.as_ref().is_some_and(|t| t.run_id == cx.run.id && cx.team.is_lead(&t.node)) {
                    continue;
                }
                a.tier = governor::Tier::Critical;
                for t in &tools {
                    if !a.role.orch_tools.contains(t) {
                        a.role.orch_tools.push(t.clone());
                    }
                }
            }
        });
    }

    fn task_turn(&self, cx: &RunCtx, t: &Task, rec: &AgentRecord, branch: &str) -> String {
        let up = rules::reports_to(&cx.team, &rec.team.as_ref().map(|s| s.node.clone()).unwrap_or_default())
            .unwrap_or_else(|| "the lead".into());
        let deps = if t.depends_on.is_empty() {
            String::new()
        } else {
            format!("\nIt builds on: {}.", t.depends_on.iter().map(|d| format!("T-{d}")).collect::<Vec<_>>().join(", "))
        };
        format!(
            "ORCH: task {} (id {}): {}\n\n{}{deps}\n\nWork in your worktree on {}. When it is done, tested and committed, call mcp__orch__report_done for `{up}`.",
            t.handle(),
            t.id,
            t.title,
            if t.body.trim().is_empty() {
                "(no details: ask the lead with mcp__orch__send_message if unclear)"
            } else {
                t.body.trim()
            },
            branch
        )
    }

    fn dispatch(&self, cx: &RunCtx) -> Result<()> {
        if !matches!(cx.run.status, RunState::Planning | RunState::Executing | RunState::Integrating) {
            return Ok(());
        }
        let tasks = self.db.tasks(None, None, Some(&cx.run.id))?;
        let done = |id: u64| tasks.iter().any(|t| t.id == id && t.status == TaskStatus::Done);
        let mut started = 0;
        for t in tasks.iter().filter(|t| t.status == TaskStatus::Ready && t.agent_id.is_none()) {
            let Some(node) = t.node.as_deref() else { continue };
            let Some(n) = cx.team.node(node) else { continue };
            if n.is_lead || !t.depends_on.iter().all(|d| done(*d)) {
                continue;
            }
            let agents: Vec<AgentRecord> = self
                .run_agents(&cx.run.id)
                .into_iter()
                .filter(|a| a.team.as_ref().is_some_and(|s| s.node == node))
                .collect();
            let free = agents.iter().find(|a| a.team.as_ref().is_some_and(|s| s.task.is_none()) && a.flag.is_none());
            let rec = match free {
                Some(a) => a.clone(),
                None if (agents.len() as u32) < n.max_concurrency.max(1) => {
                    let k = self.free_agent_number(&cx.feature.slug, node)?;
                    let row = self.create_worktree(
                        &cx.project.path,
                        &WorktreeKindSpec::Team { feature: cx.feature.slug.clone(), role: node.into(), n: k },
                        Some(&cx.feature.integration_branch()),
                    )?;
                    self.add_team_agent(cx, node, k, &row)?
                }
                None => continue,
            };
            self.store.write(|s| {
                if let Some(a) = s.agents.get_mut(&rec.id)
                    && let Some(seat) = a.team.as_mut()
                {
                    seat.task = Some(t.id);
                }
            })?;
            let mut task = t.clone();
            task.agent_id = Some(rec.id.clone());
            task.status = TaskStatus::InProgress;
            let branch = self.branch_of(&rec);
            task.branch = Some(branch.clone()).filter(|b| !b.is_empty());
            let out = self.tasks_of(cx).update(task.clone(), now_ms())?;
            if let Some(e) = out.remote_error {
                self.run_event(&cx.run.id, None, "task.remote_error", json!({"task": t.id, "error": e}));
            }
            self.deliver_kind(&rec.id, &self.task_turn(cx, &task, &rec, &branch), TurnKind::Task)?;
            self.run_event(
                &cx.run.id,
                Some(&rec.id),
                "task.assigned",
                json!({"task": t.id, "title": t.title, "node": node}),
            );
            started += 1;
        }
        if started > 0 && cx.run.status == RunState::Planning {
            self.set_run_state(&cx.run.id, RunState::Executing, "first task started")?;
        }
        Ok(())
    }

    /// Executing → Integrating once every task is done; the lead is told.
    fn progress(&self, cx: &RunCtx) -> Result<()> {
        if cx.run.status != RunState::Executing {
            return Ok(());
        }
        let tasks = self.db.tasks(None, None, Some(&cx.run.id))?;
        if tasks.is_empty() || tasks.iter().any(|t| t.status != TaskStatus::Done) {
            return Ok(());
        }
        self.set_run_state(&cx.run.id, RunState::Integrating, "every task is done")?;
        if let Some(lead) = self.lead_of(cx) {
            self.deliver_kind(
                &lead.id,
                &format!(
                    "ORCH: every task of {} is done and merged. Check {} (tests pass), then call mcp__orch__request_pr.",
                    cx.feature.slug,
                    cx.feature.integration_branch()
                ),
                TurnKind::Continue,
            )?;
        }
        Ok(())
    }

    /// FR-407 watchdog, for every live agent: a turn past its role's ceiling
    /// is interrupted (Esc) and the user told; a working agent with no hook
    /// for `NO_PROGRESS` gets an Inbox alert. Once per turn each.
    pub fn watchdog(&self, now: u64) {
        // Only the hits leave the store lock: (agent, run, what, turn start).
        let hits: Vec<(String, Option<String>, Watch, u64)> = self.store.read(|s| {
            s.agents
                .values()
                .filter_map(|a| {
                    let (what, start) = watchdog_check(a, now)?;
                    Some((a.id.clone(), a.team.as_ref().map(|t| t.run_id.clone()), what, start))
                })
                .collect()
        });
        let keys: std::collections::BTreeSet<String> =
            hits.iter().map(|(id, _, what, start)| format!("{id}:{start}:{what:?}")).collect();
        for (id, run, what, start) in hits {
            if self.live(&id).is_none() || !self.watchdog_once(format!("{id}:{start}:{what:?}")) {
                continue;
            }
            let (text, detail) = match what {
                Watch::Ceiling(m) => {
                    let _ = self.interrupt(&id);
                    (
                        format!("{id}: the turn ran past its {m} min ceiling and was interrupted (FR-407)"),
                        json!({"kind": "ceiling", "minutes": m}),
                    )
                }
                Watch::Stalled => (
                    format!("{id}: no progress for {} min (no tool call or answer)", NO_PROGRESS_MS / 60_000),
                    json!({"kind": "stalled"}),
                ),
            };
            let mut payload = detail.clone();
            payload["action"] = json!("watchdog");
            let _ = self.add_item(Some(&id), "alert", text, payload, crate::inbox::PRIORITY_ALERT);
            if let Some(run) = run {
                self.run_event(&run, Some(&id), "watchdog", detail);
            }
        }
        // A turn that ended (or moved on) needs no key any more.
        self.watchdog_keep(&keys);
    }

    /// FR-503: syncs the remote task backend of every project with an active
    /// run. A conflict keeps the remote's version and tells the user.
    pub fn sync_task_backends(&self) {
        let mut seen = std::collections::BTreeSet::new();
        for run in self.db.runs(None, true).unwrap_or_default() {
            let Ok(Some(feature)) = self.db.feature(&run.feature_id) else { continue };
            if !seen.insert(feature.project_id.clone()) {
                continue;
            }
            let Ok(Some(project)) = self.db.project(&feature.project_id) else { continue };
            if project.backend().kind() == "local" {
                continue;
            }
            match self.tasks_for(&project).sync(now_ms()) {
                Ok(rep) => {
                    for id in &rep.conflicts {
                        let handle = self.db.task(*id).ok().flatten().map(|t| t.handle()).unwrap_or_default();
                        let _ = self.add_item(
                            None,
                            "info",
                            format!(
                                "{}: task {handle} changed here and on GitHub; GitHub's version was kept",
                                project.name
                            ),
                            json!({"action": "task_conflict", "task": id, "project": project.id}),
                            0,
                        );
                    }
                    if rep != orch_tasks::SyncReport::default() {
                        let mut detail = rep.summary();
                        detail["project"] = json!(project.id);
                        self.event("-", "tasks_synced", detail);
                    }
                }
                Err(e) => {
                    self.event("-", "tasks_sync_error", json!({"project": project.id, "error": format!("{e:#}")}))
                }
            }
        }
    }

    // ------------------------------------------------------------ messages

    /// Stores a message from `from` (a node, or [`rules::USER`]) to a node
    /// or one agent, after checking the team rules. A message to a node with
    /// several agents is copied to each.
    pub fn post_message(
        &self,
        cx: &RunCtx,
        Sender(from_node, from_agent): Sender,
        to: &str,
        kind: &str,
        body: &str,
        r#ref: Value,
    ) -> Result<Vec<u64>> {
        let agents = self.run_agents(&cx.run.id);
        // `to` is a node id or an agent id of this run.
        let (to_node, to_agent) = match cx.team.node(to) {
            Some(n) => (n.id.clone(), None),
            None => {
                let a = agents.iter().find(|a| a.id == to).ok_or_else(|| {
                    anyhow!(
                        "no node or agent {to:?} in this team (nodes: {})",
                        cx.team.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>().join(", ")
                    )
                })?;
                (a.team.as_ref().map(|s| s.node.clone()).unwrap_or_default(), Some(a.id.clone()))
            }
        };
        if !rules::can_message(&cx.team, from_node, &to_node) {
            bail!("the team rules don't let {from_node} message {to_node}");
        }
        let body = cut(body.trim(), MAX_BODY);
        if body.is_empty() {
            bail!("empty message");
        }
        let recipients: Vec<Option<String>> = match to_agent {
            Some(a) => vec![Some(a)],
            None => {
                let of_node: Vec<Option<String>> = agents
                    .iter()
                    .filter(|a| a.team.as_ref().is_some_and(|s| s.node == to_node))
                    .filter(|a| Some(a.id.as_str()) != from_agent)
                    .map(|a| Some(a.id.clone()))
                    .collect();
                if of_node.is_empty() { vec![None] } else { of_node }
            }
        };
        let mut ids = vec![];
        for r in recipients {
            ids.push(self.db.add_message(&Message {
                id: 0,
                team_run_id: cx.run.id.clone(),
                from_node: from_node.into(),
                from_agent: from_agent.map(str::to_string),
                to_node: to_node.clone(),
                to_agent: r,
                kind: kind.into(),
                body: body.clone(),
                r#ref: r#ref.clone(),
                created_at_ms: now_ms(),
                delivered_at_ms: None,
                read_at_ms: None,
            })?);
        }
        self.run_event(
            &cx.run.id,
            from_agent,
            "message",
            json!({"from": from_node, "to": to, "kind": kind, "body": cut(&body, 300)}),
        );
        Ok(ids)
    }

    fn render_messages(msgs: &[Message], max: usize) -> String {
        let mut s = String::new();
        for m in msgs {
            let who = m.from_agent.clone().unwrap_or_else(|| m.from_node.clone());
            let line = match m.kind.as_str() {
                "message" => format!("\n- from {who}: {}", m.body),
                k => format!("\n- {} from {who}: {}", k.replace('_', " "), m.body),
            };
            if s.chars().count() + line.chars().count() > max {
                s.push_str("\n(more: call mcp__orch__inbox)");
                break;
            }
            s.push_str(&line);
        }
        s
    }

    fn undelivered(&self, rec: &AgentRecord) -> Vec<Message> {
        let Some(seat) = &rec.team else { return vec![] };
        let unread = self.db.messages(&seat.run_id, Some((&seat.node, Some(&rec.id))), true).unwrap_or_default();
        unread.into_iter().filter(|m| m.delivered_at_ms.is_none()).collect()
    }

    /// The agent's unread messages among `ids` (to remind it at its Stop).
    pub(crate) fn unread_of(&self, rec: &AgentRecord, ids: &[u64]) -> Vec<Message> {
        let Some(seat) = &rec.team else { return vec![] };
        let unread = self.db.messages(&seat.run_id, Some((&seat.node, Some(&rec.id))), true).unwrap_or_default();
        unread.into_iter().filter(|m| ids.contains(&m.id)).collect()
    }

    /// FR-403 for a busy agent: its new messages, as PostToolUse context.
    pub(crate) fn inject_messages(&self, rec: &AgentRecord) -> Option<String> {
        let msgs = self.undelivered(rec);
        if msgs.is_empty() {
            return None;
        }
        let ids: Vec<u64> = msgs.iter().map(|m| m.id).collect();
        let _ = self.db.deliver_messages(&ids, &rec.id, now_ms());
        // Read when this turn ends; its Stop reminds the agent of them first.
        self.note_injected(&rec.id, &ids);
        Some(format!("ORCH: {} new message(s):{}", msgs.len(), Self::render_messages(&msgs, MAX_INJECT)))
    }

    /// FR-403 for idle and suspended agents: their new messages as one turn
    /// (the Governor admits it; a suspended agent is resumed for it).
    fn deliver_messages(&self, cx: &RunCtx) {
        for rec in self.run_agents(&cx.run.id) {
            let busy = matches!(
                rec.tracker.state,
                AgentState::Working | AgentState::WaitingForYou | AgentState::Queued | AgentState::Starting
            );
            // Busy: at its next tool call. Not started yet: with its first turn.
            if busy || rec.never_started() && self.has_queued_turn(&rec.id) {
                continue;
            }
            let msgs = self.undelivered(&rec);
            if msgs.is_empty() {
                continue;
            }
            let text = format!(
                "ORCH: {} new message(s) for you:{}\n\nAnswer with mcp__orch__send_message when needed.",
                msgs.len(),
                Self::render_messages(&msgs, MAX_BODY)
            );
            let ids: Vec<u64> = msgs.iter().map(|m| m.id).collect();
            if self.deliver_messages_turn(&rec.id, &text, ids.clone()).is_ok() {
                let _ = self.db.deliver_messages(&ids, &rec.id, now_ms());
            }
        }
    }

    fn has_queued_turn(&self, id: &str) -> bool {
        self.store.read(|s| s.turns.iter().any(|t| t.agent_id == id && t.status == TurnStatus::Queued))
    }

    // ------------------------------------------------------------ controls (FR-406)

    pub fn run_pause(&self, run_id: &str) -> Result<TeamRun> {
        let run = self.set_run_state(run_id, RunState::Paused, "paused by the user")?;
        for a in self.run_agents(run_id) {
            self.governor_act(|g, now| (g.pause_agent(&AgentId(a.id.clone()), now), Vec::new()));
        }
        Ok(run)
    }

    pub fn run_resume(&self, run_id: &str) -> Result<TeamRun> {
        let cur = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let to = cur.resume_status.unwrap_or(RunState::Executing);
        let run = self.set_run_state(run_id, to, "resumed by the user")?;
        for a in self.run_agents(run_id) {
            self.resume_agent(&a.id);
        }
        Ok(run)
    }

    /// Stops the team: every agent is suspended (sessions kept) and the run
    /// ends as failed ("stopped by the user"). Worktrees stay.
    pub fn run_stop(&self, run_id: &str) -> Result<TeamRun> {
        let run = self.set_run_state(run_id, RunState::Failed, "stopped by the user")?;
        for a in self.run_agents(run_id) {
            self.drop_queued_turns(&a.id);
            if self.live(&a.id).is_some() {
                let _ = self.suspend(&a.id);
            }
        }
        Ok(run)
    }

    /// Deletes a feature: its runs end as failed ("deleted by the user"), every
    /// agent of the feature is deleted (killed, checked, archived), then the
    /// feature is archived. With `remove_worktrees`, the agents' worktrees and
    /// the integration worktree go too (unmerged work needs `discard`). Stops
    /// at the first agent whose kill can't be verified: the feature stays.
    pub fn delete_feature(&self, feature_id: &str, remove_worktrees: bool, discard: bool) -> Result<Value> {
        let mut f = self.db.feature(feature_id)?.ok_or_else(|| anyhow!("no feature {feature_id}"))?;
        // First the runs, so the team tick starts no new seat meanwhile.
        for run in self.db.runs(Some(feature_id), true)? {
            self.set_run_state(&run.id, RunState::Failed, "deleted by the user")?;
        }
        let recs: Vec<(String, std::path::PathBuf)> = self.store.read(|s| {
            s.agents
                .values()
                .filter(|a| a.team.as_ref().is_some_and(|t| t.feature_id == feature_id))
                .filter(|a| a.tracker.state != AgentState::Archived)
                .map(|a| (a.id.clone(), crate::paths::real(&a.worktree)))
                .collect()
        });
        let mut agents = vec![];
        for (id, _) in &recs {
            agents.push(self.delete_agent(id, false, false)?);
        }
        // Then each worktree once, now that none of the feature's agents
        // holds it (the lead shares the integration worktree).
        let mut worktrees = vec![];
        if remove_worktrees {
            let mut paths: Vec<std::path::PathBuf> = recs.into_iter().map(|(_, wt)| wt).collect();
            paths.extend(f.integration_worktree.as_deref().map(crate::paths::real));
            paths.sort();
            paths.dedup();
            for wt in paths {
                let mut r = self.remove_worktree_report(&f.project_id, &wt, discard);
                r["path"] = json!(wt);
                worktrees.push(r);
            }
        }
        f.archived = true;
        self.db.upsert_feature(&f)?;
        let out = json!({"deleted": f.slug, "agents": agents, "worktrees": worktrees});
        self.db.audit("user", None, "feature_deleted", Some(&f.slug), "ok", out.clone())?;
        Ok(out)
    }

    /// Gives a task to another node (FR-406). Its current agent is freed.
    pub fn reassign_task(&self, task_id: u64, node: &str) -> Result<Task> {
        let mut t = self.db.task(task_id)?.ok_or_else(|| anyhow!("no task {task_id}"))?;
        let run_id = t.team_run_id.clone().ok_or_else(|| anyhow!("task {task_id} is not part of a team run"))?;
        let cx = self.run_ctx(&run_id)?;
        if cx.team.node(node).is_none() {
            bail!("no node {node} in the team");
        }
        if let Some(a) = t.agent_id.take() {
            self.free_seat(&a, task_id);
        }
        t.node = Some(node.into());
        if t.status == TaskStatus::InProgress {
            t.status = TaskStatus::Ready;
        }
        let out = self.tasks_of(&cx).update(t, now_ms())?;
        self.run_event(&run_id, None, "task.reassigned", json!({"task": task_id, "node": node}));
        Ok(out.value)
    }

    pub(crate) fn free_seat(&self, agent: &str, task: u64) {
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(agent)
                && let Some(seat) = a.team.as_mut()
                && seat.task == Some(task)
            {
                seat.task = None;
            }
        });
    }

    /// Adds a node to a running team (FR-406): the team gets a new version,
    /// which the run then uses.
    pub fn add_node(&self, run_id: &str, team: &Team) -> Result<TeamRun> {
        let mut run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        if team.id != run.team_id || team.scope != run.team_scope {
            bail!("run {run_id} uses team {}", run.team_id);
        }
        let old = self.run_ctx(run_id)?;
        for n in &old.team.nodes {
            let Some(m) = team.node(&n.id) else { bail!("node {} can't be removed from a running team", n.id) };
            if m.is_lead != n.is_lead {
                bail!("the lead can't change in a running team");
            }
        }
        let roles = self.db.roles(Some(&old.project.id))?;
        let issues = orch_team::validate(team, Some(&roles));
        if !issues.is_empty() {
            bail!("the team is not valid: {}", issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "));
        }
        let mut team = team.clone();
        // Past every version snapshotted so far: an existing one is kept as
        // it was, so reusing its number would drop the new node.
        let latest = self.db.team(&team.scope, &team.id)?.map_or(0, |t| t.version);
        team.version = team.version.max(old.team.version).max(latest) + 1;
        while self.db.team_version(&team.scope, &team.id, team.version)?.is_some() {
            team.version += 1;
        }
        self.db.save_team(&team, &roles, now_ms())?;
        run.team_version = team.version;
        self.db.upsert_run(&run)?;
        self.run_event(run_id, None, "run.team_changed", json!({"version": team.version}));
        Ok(run)
    }

    // ------------------------------------------------------------ team tools

    /// The P2 tools of the orchestrator MCP server. Returns the model's text
    /// and structured data.
    pub(crate) fn team_tool(&self, rec: &AgentRecord, tool: &str, args: &Value) -> Result<(String, Value)> {
        let seat = rec.team.clone().ok_or_else(|| anyhow!("{tool} is for team agents; this is a solo session"))?;
        let cx = self.run_ctx(&seat.run_id)?;
        if cx.run.status.finished() {
            bail!("the team run is {}: nothing more to do", cx.run.status.label());
        }
        let is_lead = cx.team.is_lead(&seat.node);
        let me = seat.node.as_str();
        match tool {
            "team_status" | "nudge" | "resume_agent" => {
                if !is_lead {
                    bail!("only the lead manages the team");
                }
                self.lead_tool(&cx, tool, args)
            }
            "send_message" => {
                let to = str_arg(args, "to").ok_or_else(|| anyhow!("`to` is required"))?;
                let body = str_arg(args, "body").ok_or_else(|| anyhow!("`body` is required"))?;
                let ids = self.post_message(&cx, Sender(me, Some(&rec.id)), to, "message", body, Value::Null)?;
                Ok((format!("Sent to {to}."), json!({"ids": ids})))
            }
            "inbox" => {
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let msgs = self.db.messages(&cx.run.id, Some((me, Some(&rec.id))), !all)?;
                let unread: Vec<u64> = msgs.iter().filter(|m| m.read_at_ms.is_none()).map(|m| m.id).collect();
                self.db.mark_messages(&unread, Some(&rec.id), now_ms())?;
                if msgs.is_empty() {
                    return Ok(("No new messages.".into(), json!([])));
                }
                Ok((format!("{} message(s):{}", msgs.len(), Self::render_messages(&msgs, MAX_BODY * 2)), json!(msgs)))
            }
            "list_tasks" => {
                let mine = args.get("mine").and_then(Value::as_bool).unwrap_or(false);
                let status =
                    str_arg(args, "status").map(|s| s.parse::<TaskStatus>().map_err(|e| anyhow!(e))).transpose()?;
                let tasks: Vec<Task> = self
                    .db
                    .tasks(None, None, Some(&cx.run.id))?
                    .into_iter()
                    .filter(|t| !mine || t.node.as_deref() == Some(me))
                    .filter(|t| status.is_none_or(|s| t.status == s))
                    .collect();
                if tasks.is_empty() {
                    return Ok(("No tasks.".into(), json!([])));
                }
                let lines: Vec<String> = tasks
                    .iter()
                    .map(|t| {
                        format!(
                            "- id {} {} [{}] {} → {}{}{}",
                            t.id,
                            t.handle(),
                            t.status.label(),
                            t.title,
                            t.node.as_deref().unwrap_or("unassigned"),
                            if t.depends_on.is_empty() {
                                String::new()
                            } else {
                                format!(" (after {:?})", t.depends_on)
                            },
                            t.branch.as_deref().map(|b| format!(" · {b}")).unwrap_or_default()
                        )
                    })
                    .collect();
                Ok((lines.join("\n"), json!(tasks)))
            }
            "handoff" => {
                let artifact = str_arg(args, "artifact").ok_or_else(|| anyhow!("`artifact` is required"))?;
                let to = str_arg(args, "to").ok_or_else(|| anyhow!("`to` is required"))?;
                if !rules::can_handoff(&cx.team, me, to) {
                    bail!("the team rules don't let {me} hand off to {to}");
                }
                let kind = str_arg(args, "kind").unwrap_or("doc");
                if !matches!(kind, "doc" | "design" | "build") {
                    bail!("kind is doc, design or build");
                }
                let note = str_arg(args, "note").unwrap_or("");
                let id = self.db.add_artifact(&Artifact {
                    id: 0,
                    feature_id: cx.feature.id.clone(),
                    team_run_id: Some(cx.run.id.clone()),
                    kind: kind.into(),
                    uri: cut(artifact, 2_000),
                    from_node: me.into(),
                    to_node: Some(to.into()),
                    note: cut(note, 2_000),
                    created_at_ms: now_ms(),
                })?;
                let body =
                    format!("{kind} {artifact}{}", if note.is_empty() { String::new() } else { format!(": {note}") });
                self.post_message(&cx, Sender(me, Some(&rec.id)), to, "handoff", &body, json!({"artifact": id}))?;
                Ok((format!("Handed {artifact} to {to}."), json!({"artifact": id})))
            }
            "request_review" => {
                let to = str_arg(args, "to").ok_or_else(|| anyhow!("`to` is required"))?;
                if !rules::can_request_review(&cx.team, me, to) {
                    bail!("the team rules don't let {me} ask {to} for a review");
                }
                let branch = match str_arg(args, "branch") {
                    Some(b) => b.to_string(),
                    None => self.branch_of(rec),
                };
                let note = str_arg(args, "note").unwrap_or("");
                let body = format!(
                    "please review {branch}{}. Read it with `git log`/`git diff {}..{branch}` in your worktree, then reply with mcp__orch__send_message.",
                    if note.is_empty() { String::new() } else { format!(" ({note})") },
                    cx.feature.integration_branch()
                );
                self.post_message(
                    &cx,
                    Sender(me, Some(&rec.id)),
                    to,
                    "review_request",
                    &body,
                    json!({"branch": branch}),
                )?;
                Ok((format!("Review of {branch} asked from {to}."), Value::Null))
            }
            "ask_user" => {
                let q = str_arg(args, "question").ok_or_else(|| anyhow!("`question` is required"))?;
                let options: Vec<String> = args
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|s| cut(s, 200))
                    .take(8)
                    .collect();
                let id = self.add_item(
                    Some(&rec.id),
                    "question",
                    format!("{}: {}", rec.id, cut(q, 300)),
                    json!({"action": "ask_user", "question": cut(q, 4_000), "options": options, "run": cx.run.id, "notify": str_arg(args, "kind") != Some("fyi")}),
                    crate::inbox::PRIORITY_GATE,
                )?;
                self.mark_asked(&rec.id);
                self.run_event(&cx.run.id, Some(&rec.id), "asked_user", json!({"item": id, "question": cut(q, 300)}));
                Ok((
                    "Asked the user; the answer comes as a new turn. Continue with other work or end your turn.".into(),
                    json!({"item": id}),
                ))
            }
            "create_task" => {
                if !is_lead {
                    bail!("only the lead creates tasks");
                }
                if !effective(&cx.run).tasks_allowed() {
                    bail!(
                        "tasks can be created once the user approves the spec (call mcp__orch__request_spec_approval first)"
                    );
                }
                let title = str_arg(args, "title").ok_or_else(|| anyhow!("`title` is required"))?;
                let node = str_arg(args, "node").ok_or_else(|| anyhow!("`node` is required"))?;
                let n = cx.team.node(node).ok_or_else(|| anyhow!("no node {node} in the team"))?;
                if n.is_lead {
                    bail!("assign tasks to the specialists; the lead's own work needs no task");
                }
                let deps: Vec<u64> = args
                    .get("depends_on")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .collect();
                let run_tasks = self.db.tasks(None, None, Some(&cx.run.id))?;
                for d in &deps {
                    if !run_tasks.iter().any(|t| t.id == *d) {
                        bail!("depends_on: no task {d} in this feature");
                    }
                }
                let status = match str_arg(args, "status") {
                    None => TaskStatus::Ready,
                    Some(s) => match s.parse::<TaskStatus>().map_err(|e| anyhow!(e))? {
                        s @ (TaskStatus::Ready | TaskStatus::Backlog) => s,
                        _ => bail!("a new task is ready or backlog"),
                    },
                };
                let mut t = Task::new(&cx.project.id, "", title, now_ms());
                t.title = cut(title, 200);
                t.body = cut(str_arg(args, "body").unwrap_or(""), MAX_BODY);
                t.node = Some(node.into());
                t.depends_on = deps;
                t.status = status;
                t.feature_id = Some(cx.feature.id.clone());
                t.team_run_id = Some(cx.run.id.clone());
                let out = self.tasks_of(&cx).create(t)?;
                let t = out.value;
                self.run_event(
                    &cx.run.id,
                    Some(&rec.id),
                    "task.created",
                    json!({"task": t.id, "title": t.title, "node": node}),
                );
                let warn = out
                    .remote_error
                    .map(|e| format!(" (the task backend failed: {e}; it retries)"))
                    .unwrap_or_default();
                Ok((format!("Created task id {} ({}) for {node}{warn}.", t.id, t.handle()), json!(t)))
            }
            "update_task" => {
                let id = args.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow!("`id` is required"))?;
                let mut t = self.db.task(id)?.ok_or_else(|| anyhow!("no task {id}"))?;
                if t.team_run_id.as_deref() != Some(&cx.run.id) {
                    bail!("task {id} is not part of this feature");
                }
                if !is_lead && t.node.as_deref() != Some(me) {
                    bail!("you can only update your own tasks");
                }
                if let Some(node) = str_arg(args, "node") {
                    if !is_lead {
                        bail!("only the lead reassigns tasks");
                    }
                    t = self.reassign_task(id, node)?;
                }
                if let Some(s) = str_arg(args, "status") {
                    let s: TaskStatus = s.parse().map_err(|e: String| anyhow!(e))?;
                    if s == TaskStatus::Done && !is_lead {
                        bail!("the lead marks tasks done when it merges them: call mcp__orch__report_done");
                    }
                    self.apply_status(&mut t, s);
                }
                let svc = self.tasks_of(&cx);
                let mut t = svc.update(t, now_ms())?.value;
                if let Some(n) = str_arg(args, "notes") {
                    t = svc.note(id, me, &cut(n, 4_000), now_ms())?.value;
                }
                self.run_event(&cx.run.id, Some(&rec.id), "task.updated", json!({"task": id, "status": t.status}));
                Ok((format!("Task {} is {}.", t.handle(), t.status.label()), json!(t)))
            }
            "report_done" => {
                if is_lead {
                    bail!("the lead finishes with mcp__orch__request_pr");
                }
                let summary = str_arg(args, "summary").ok_or_else(|| anyhow!("`summary` is required"))?;
                let id = args
                    .get("task")
                    .and_then(Value::as_u64)
                    .or(seat.task)
                    .ok_or_else(|| anyhow!("you have no current task"))?;
                let mut t = self.db.task(id)?.ok_or_else(|| anyhow!("no task {id}"))?;
                if t.node.as_deref() != Some(me) || t.team_run_id.as_deref() != Some(&cx.run.id) {
                    bail!("task {id} is not yours");
                }
                let st = orch_git::git(&rec.worktree, &["status", "--porcelain"])?;
                if !st.trim().is_empty() {
                    bail!("you have uncommitted changes: commit them with mcp__orch__commit first");
                }
                self.apply_status(&mut t, TaskStatus::Review);
                let svc = self.tasks_of(&cx);
                svc.update(t.clone(), now_ms())?;
                svc.note(id, me, &cut(summary, 4_000), now_ms())?;
                let up = rules::reports_to(&cx.team, me).unwrap_or_else(|| "lead".into());
                let branch = t.branch.clone().unwrap_or_default();
                self.post_message(
                    &cx,
                    Sender(me, Some(&rec.id)),
                    &up,
                    "report",
                    &format!("task id {id} ({}) is done on {branch}: {}", t.title, cut(summary, 2_000)),
                    json!({"task": id, "branch": branch}),
                )?;
                self.run_event(&cx.run.id, Some(&rec.id), "task.reported", json!({"task": id}));
                Ok((format!("Reported task {id} to {up}. End your turn; the next task comes by itself."), Value::Null))
            }
            "sync_integration" => {
                if is_lead {
                    bail!("you are on the integration branch");
                }
                let integ = cx.feature.integration_branch();
                let r = orch_git::commit::merge_into_worktree(
                    &rec.worktree,
                    &integ,
                    &format!("Merge {integ} into {}", rec.id),
                    self.agent_gitconfig(&rec.id).as_deref(),
                    false,
                )?;
                self.run_event(&cx.run.id, Some(&rec.id), "git.sync", json!(r));
                let _ =
                    self.db.audit(&format!("agent:{}", rec.id), Some(&rec.id), "merge", Some(&integ), "ok", json!(r));
                Ok((
                    match &r {
                        MergeOutcome::Merged { sha } => format!("Merged {integ} ({}).", &sha[..sha.len().min(12)]),
                        MergeOutcome::UpToDate => "Already up to date.".into(),
                        MergeOutcome::Conflict { files, .. } => format!(
                            "Conflicts in {}. Fix them (remove the markers), then call mcp__orch__commit to conclude the merge.",
                            files.join(", ")
                        ),
                    },
                    json!(r),
                ))
            }
            "merge" => {
                if !is_lead {
                    bail!("only the lead merges into the integration branch");
                }
                let branch = str_arg(args, "branch").ok_or_else(|| anyhow!("`branch` is required"))?;
                let prefix = format!("orch/{}/", cx.feature.slug);
                if !branch.starts_with(&prefix) || branch == cx.feature.integration_branch() {
                    bail!("only branches of this feature ({prefix}<node>-<n>) can be merged here");
                }
                let r = orch_git::commit::merge_into_worktree(
                    &rec.worktree,
                    branch,
                    &format!("Merge {branch}"),
                    self.agent_gitconfig(&rec.id).as_deref(),
                    true,
                );
                let (decision, detail) = match &r {
                    Ok(o) => ("ok", json!(o)),
                    Err(e) => ("failed", json!({"error": format!("{e:#}")})),
                };
                let _ = self.db.audit(
                    &format!("agent:{}", rec.id),
                    Some(&rec.id),
                    "merge",
                    Some(branch),
                    decision,
                    detail.clone(),
                );
                self.run_event(&cx.run.id, Some(&rec.id), "git.merge", json!({"branch": branch, "result": detail}));
                let r = r?;
                let task = args.get("task").and_then(Value::as_u64).map(|id| self.db.task(id)).transpose()?.flatten();
                if let Some(t) = &task
                    && t.team_run_id.as_deref() != Some(&cx.run.id)
                {
                    bail!("task {} is not part of this feature", t.id);
                }
                match (&r, task) {
                    (MergeOutcome::Conflict { files, .. }, task) => {
                        // Back to the owner (FR-405).
                        let owner = self.run_agents(&cx.run.id).into_iter().find(|a| self.branch_of(a) == branch);
                        if let Some(o) = &owner {
                            self.post_message(
                                &cx,
                                Sender(me, Some(&rec.id)),
                                &o.id,
                                "message",
                                &format!(
                                    "merging {branch} into {} conflicts in {}. Call mcp__orch__sync_integration, fix the conflicts, commit, then report again.",
                                    cx.feature.integration_branch(),
                                    files.join(", ")
                                ),
                                json!({"branch": branch, "files": files}),
                            )?;
                        }
                        if let Some(mut t) = task {
                            t.status = TaskStatus::InProgress;
                            // The owner works on it again: no new task meanwhile.
                            if let (Some(o), Some(a)) = (&owner, &t.agent_id)
                                && o.id == *a
                            {
                                let tid = t.id;
                                self.store.write(|s| {
                                    if let Some(seat) = s.agents.get_mut(a).and_then(|x| x.team.as_mut())
                                        && seat.task.is_none()
                                    {
                                        seat.task = Some(tid);
                                    }
                                })?;
                            }
                            self.tasks_of(&cx).update(t, now_ms())?;
                        }
                        Ok((
                            format!(
                                "Conflicts in {}: nothing was merged. {} was told to sync and fix.",
                                files.join(", "),
                                owner.map(|o| o.id).unwrap_or_else(|| "The owner (not found)".into())
                            ),
                            json!(r),
                        ))
                    }
                    (MergeOutcome::Merged { .. } | MergeOutcome::UpToDate, Some(mut t)) => {
                        self.apply_status(&mut t, TaskStatus::Done);
                        self.tasks_of(&cx).update(t.clone(), now_ms())?;
                        self.run_event(&cx.run.id, Some(&rec.id), "task.done", json!({"task": t.id}));
                        Ok((format!("Merged {branch}; task {} is done.", t.handle()), json!(r)))
                    }
                    (MergeOutcome::Merged { .. }, None) => Ok((format!("Merged {branch}."), json!(r))),
                    (MergeOutcome::UpToDate, None) => Ok((format!("{branch} is already merged."), json!(r))),
                }
            }
            "request_spec_approval" => {
                if !is_lead {
                    bail!("only the lead asks for the spec gate");
                }
                if !matches!(effective(&cx.run), RunState::Spec | RunState::Draft) {
                    bail!("the spec gate is for a run in spec (this one is {})", effective(&cx.run).label());
                }
                let summary = str_arg(args, "summary").ok_or_else(|| anyhow!("`summary` is required"))?;
                let st = orch_git::git(&rec.worktree, &["status", "--porcelain"])?;
                if !st.trim().is_empty() {
                    bail!("commit the docs with mcp__orch__commit first");
                }
                let docs = feature_docs(&rec.worktree, &cx.feature.slug);
                if docs.is_empty() {
                    bail!("no docs in {}: write spec.md first", FeatureCtx::docs_dir(&cx.fctx()));
                }
                let id = self.add_item(
                    Some(&rec.id),
                    "gate",
                    format!("Spec gate · {}: {}", cx.feature.slug, cut(summary, 200)),
                    json!({"action": "spec_approval", "run": cx.run.id, "feature": cx.feature.slug,
                           "summary": cut(summary, 4_000), "docs": docs, "worktree": rec.worktree,
                           "branch": cx.feature.integration_branch()}),
                    crate::inbox::PRIORITY_GATE,
                )?;
                self.advance_run(&cx.run.id, RunState::SpecGate, "spec gate asked")?;
                let _ = self.db.audit(
                    &format!("agent:{}", rec.id),
                    Some(&rec.id),
                    "spec_gate",
                    Some(&cx.feature.slug),
                    "pending",
                    json!({"item": id}),
                );
                Ok((
                    "The spec gate is in the user's Inbox. End your turn now; you will be told the decision.".into(),
                    json!({"item": id}),
                ))
            }
            other => Err(anyhow!("unknown tool {other:?}")),
        }
    }

    // ------------------------------------------------------------ gate follow-ups

    /// The user decided the spec gate (FR-704).
    pub(crate) fn spec_decided(&self, payload: &Value, agent: Option<&str>, approved: bool, message: Option<&str>) {
        let Some(run_id) = payload.get("run").and_then(Value::as_str) else { return };
        let r = if approved {
            self.advance_run(run_id, RunState::Planning, "spec approved").map(|_| {
                "ORCH: approved: proceed. The user approved the spec. Create the tasks with mcp__orch__create_task (each to one node, with depends_on); they start by themselves.".to_string()
            })
        } else {
            self.advance_run(run_id, RunState::Spec, "spec sent back").map(|_| {
                format!(
                    "ORCH: the user did not approve the spec.{} Revise the docs, commit, and call mcp__orch__request_spec_approval again.",
                    message.map(|m| format!(" They said: {m}")).unwrap_or_default()
                )
            })
        };
        match (r, agent) {
            (Ok(text), Some(a)) => {
                let _ = self.deliver_user(a, &text);
            }
            (Err(e), _) => self.event("-", "team_error", json!({"run": run_id, "error": e.to_string()})),
            _ => {}
        }
    }

    /// A team lead's PR gate was approved and the PR opened (or failed).
    pub(crate) fn pr_opened(&self, agent: &str, url: Option<&str>) {
        let Some(seat) = self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.clone())) else { return };
        let Ok(Some(mut run)) = self.db.run(&seat.run_id) else { return };
        match url {
            Some(u) => {
                run.pr_url = Some(u.to_string());
                let _ = self.db.upsert_run(&run);
                let _ = self.advance_run(&run.id, RunState::Done, "pull request opened");
            }
            None => {
                let _ = self.advance_run(&run.id, RunState::Integrating, "pull request failed");
            }
        }
    }

    /// A team agent's `request_pr` is refused before the run can reach its PR
    /// gate (a PR opened from the spec step would leave the run behind).
    pub(crate) fn pr_allowed(&self, rec: &AgentRecord) -> Result<()> {
        let Some(seat) = &rec.team else { return Ok(()) };
        let run = self.db.run(&seat.run_id)?.ok_or_else(|| anyhow!("no team run {}", seat.run_id))?;
        let eff = effective(&run);
        if eff != RunState::PrGate && !eff.can_go(RunState::PrGate) {
            bail!("the pull request comes once the tasks are merged (this run is {})", eff.label());
        }
        Ok(())
    }

    /// A team lead asked for the PR: the run is at its PR gate.
    pub(crate) fn pr_requested(&self, agent: &str) {
        if let Some(seat) = self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.clone())) {
            let _ = self.advance_run(&seat.run_id, RunState::PrGate, "pull request asked");
        }
    }

    /// A team lead's PR gate was declined.
    pub(crate) fn pr_declined(&self, agent: &str) {
        if let Some(seat) = self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.clone())) {
            let _ = self.advance_run(&seat.run_id, RunState::Integrating, "pull request declined");
        }
    }

    /// Run summaries for the UI and orchd.
    pub fn runs_json(&self) -> Value {
        let runs = self.db.runs(None, false).unwrap_or_default();
        let mut by_feature: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for r in runs.iter().take(50) {
            let agents: Vec<Value> = self
                .run_agents(&r.id)
                .into_iter()
                .map(|a| json!({"id": a.id, "node": a.team.as_ref().map(|s| s.node.clone()), "state": a.tracker.state, "task": a.team.as_ref().and_then(|s| s.task)}))
                .collect();
            by_feature.entry(r.feature_id.clone()).or_default().push(json!({"run": r, "agents": agents}));
        }
        json!(by_feature)
    }
}

/// The markdown files in the feature's docs folder of a worktree.
pub fn feature_docs(worktree: &Path, slug: &str) -> Vec<String> {
    let dir = worktree.join(".orchestrator").join("features").join(slug);
    let mut v: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort_by_key(|n| (n != "spec.md", n.clone()));
    v
}

/// Reads one doc of a feature from a worktree (for the gate preview and the
/// Feature pane), refusing anything outside the docs folder.
pub fn read_feature_doc(worktree: &Path, slug: &str, name: &str) -> Result<String> {
    if name.contains('/') || name.contains("..") || !name.ends_with(".md") {
        bail!("invalid doc name {name:?}");
    }
    let p = worktree.join(".orchestrator").join("features").join(slug).join(name);
    let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    Ok(cut(&text, 200_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use orch_git::fixtures::scratch_repo;
    use orch_protocol::{InboxDecision, McpMsg};
    use std::sync::Arc;

    fn call(rt: &Runtime, agent: &str, tool: &str, args: Value) -> (bool, String) {
        let (err, text, _) = rt
            .on_mcp(&McpMsg { agent_id: agent.into(), tool: tool.into(), arguments: args, deadline_ms: 10_000 }, None);
        (err, text)
    }

    fn ok(rt: &Runtime, agent: &str, tool: &str, args: Value) -> String {
        let (err, text) = call(rt, agent, tool, args);
        assert!(!err, "{agent} {tool}: {text}");
        text
    }

    fn queued(rt: &Runtime, agent: &str) -> Vec<String> {
        rt.store.read(|s| {
            s.turns
                .iter()
                .filter(|t| t.agent_id == agent && t.status == TurnStatus::Queued)
                .map(|t| t.body.clone())
                .collect()
        })
    }

    fn state(rt: &Runtime, run: &str) -> RunState {
        rt.db.run(run).unwrap().unwrap().status
    }

    fn setup() -> (tempfile::TempDir, Arc<Runtime>, tempfile::TempDir, std::path::PathBuf, String) {
        let (d, rt) = test_runtime(false);
        let (rd, repo) = scratch_repo();
        let p = orch_git::project::register(&rt.db, &repo).unwrap();
        let (roles, team) = orch_team::presets::bugfix_squad();
        for r in &roles {
            rt.db.upsert_role(r, 1).unwrap();
        }
        rt.db.save_team(&team, &roles, 1).unwrap();
        (d, rt, rd, repo, p.id)
    }

    fn write_commit(rt: &Runtime, agent: &str, file: &str, body: &str) {
        let wt = rt.store.read(|s| s.agents[agent].worktree.clone());
        let p = wt.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        ok(rt, agent, "commit", json!({"message": format!("add {file}")}));
    }

    #[test]
    fn a_feature_goes_from_launch_through_both_gates() {
        let (_d, rt, _rd, _repo, pid) = setup();
        let run = rt
            .launch(&LaunchSpec {
                project_id: pid.clone(),
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                title: "Fix X".into(),
                brief: "Fix the X bug.".into(),
                base: None,
            })
            .unwrap();
        assert_eq!(run.status, RunState::Spec);
        let lead = "fx-lead-1";
        let rec = rt.store.read(|s| s.agents[lead].clone());
        assert_eq!(rec.tracker.state, AgentState::Created, "not started until the Governor admits it");
        assert_eq!(
            orch_git::git(&rec.worktree, &["symbolic-ref", "--short", "HEAD"]).unwrap().trim(),
            "orch/fx/integration"
        );
        assert!(queued(&rt, lead)[0].contains("Fix the X bug."));
        assert!(
            rt.launch(&LaunchSpec {
                project_id: pid.clone(),
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                ..Default::default()
            })
            .is_err(),
            "one running team per feature"
        );

        // FR-401: no tasks before the spec gate; no gate without docs.
        assert!(call(&rt, lead, "create_task", json!({"title": "t", "node": "back"})).1.contains("approves the spec"));
        assert!(call(&rt, lead, "request_spec_approval", json!({"summary": "s"})).1.contains("no docs"));
        write_commit(&rt, lead, ".orchestrator/features/fx/spec.md", "# Fix X\n");
        ok(&rt, lead, "request_spec_approval", json!({"summary": "two tasks"}));
        assert_eq!(state(&rt, &run.id), RunState::SpecGate);
        let gate = rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
        assert_eq!(gate.payload["action"], "spec_approval");
        assert_eq!(gate.payload["docs"], json!(["spec.md"]));
        // Sent back once, then approved.
        rt.decide(gate.id, InboxDecision::Reply, None, Some("name the files")).unwrap();
        assert_eq!(state(&rt, &run.id), RunState::Spec);
        assert!(queued(&rt, lead).iter().any(|b| b.contains("name the files")));
        ok(&rt, lead, "request_spec_approval", json!({"summary": "two tasks, files named"}));
        let gate = rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
        rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(state(&rt, &run.id), RunState::Planning);
        assert!(queued(&rt, lead).iter().any(|b| b.contains("approved: proceed")));

        // Tasks with a dependency (FR-404).
        let t1: Task = serde_json::from_value(
            rt.on_mcp(
                &McpMsg {
                    agent_id: lead.into(),
                    tool: "create_task".into(),
                    arguments: json!({"title": "API", "node": "back", "body": "add api.rs"}),
                    deadline_ms: 1,
                },
                None,
            )
            .2,
        )
        .unwrap();
        let (err, _) = call(&rt, lead, "create_task", json!({"title": "UI", "node": "front", "depends_on": [t1.id]}));
        assert!(!err);
        assert!(call(&rt, lead, "create_task", json!({"title": "x", "node": "ghost"})).1.contains("no node"));
        rt.team_tick();
        assert_eq!(state(&rt, &run.id), RunState::Executing);
        let back = "fx-back-1";
        let brec = rt.store.read(|s| s.agents[back].clone());
        assert_eq!(
            orch_git::git(&brec.worktree, &["symbolic-ref", "--short", "HEAD"]).unwrap().trim(),
            "orch/fx/back-1"
        );
        assert!(brec.worktree.join(".orchestrator/features/fx/spec.md").is_file(), "branched from integration");
        assert_eq!(brec.team.as_ref().unwrap().task, Some(t1.id));
        assert!(queued(&rt, back)[0].contains("task T-"));
        assert!(rt.store.read(|s| !s.agents.contains_key("fx-front-1")), "front waits for its dependency");
        assert_eq!(rt.db.task(t1.id).unwrap().unwrap().status, TaskStatus::InProgress);

        // Tools by role, recipients by the rules.
        assert!(call(&rt, back, "merge", json!({"branch": "orch/fx/back-1"})).1.contains("not one of your tools"));
        assert!(call(&rt, back, "send_message", json!({"to": "nobody", "body": "hi"})).1.contains("no node or agent"));
        // Messages: a busy agent gets them at its next tool call.
        ok(&rt, back, "send_message", json!({"to": "lead", "body": "where is the config?"}));
        let lrec = rt.store.read(|s| s.agents[lead].clone());
        let injected = rt.inject_messages(&lrec).unwrap();
        assert!(injected.starts_with("ORCH: 1 new message") && injected.contains("where is the config?"));
        assert!(rt.inject_messages(&lrec).is_none(), "delivered once");
        ok(&rt, lead, "send_message", json!({"to": "back", "body": "in config/"}));
        assert!(ok(&rt, back, "inbox", json!({})).contains("in config/"));

        // Report, merge, next task.
        std::fs::write(brec.worktree.join("api.rs"), "fn api() {}").unwrap();
        assert!(call(&rt, back, "report_done", json!({"summary": "s"})).1.contains("uncommitted"));
        ok(&rt, back, "commit", json!({"message": "add api.rs"}));
        ok(&rt, back, "report_done", json!({"summary": "api.rs added, tested"}));
        assert_eq!(rt.db.task(t1.id).unwrap().unwrap().status, TaskStatus::Review);
        assert!(rt.inject_messages(&lrec).unwrap().contains("report from fx-back-1"));
        assert!(call(&rt, lead, "merge", json!({"branch": "main"})).1.contains("only branches of this feature"));
        assert!(ok(&rt, lead, "merge", json!({"branch": "orch/fx/back-1", "task": t1.id})).contains("is done"));
        assert!(lrec.worktree.join("api.rs").is_file());
        assert_eq!(rt.db.task(t1.id).unwrap().unwrap().status, TaskStatus::Done);
        let audit = rt.db.audit_log(Some(lead), 10).unwrap();
        assert!(audit.iter().any(|a| a.action == "merge" && a.decision == "ok"));
        rt.team_tick();
        let front = "fx-front-1";
        assert!(queued(&rt, front)[0].contains("UI"));
        ok(&rt, front, "sync_integration", json!({}));
        write_commit(&rt, front, "ui.rs", "fn ui() {}");
        ok(&rt, front, "report_done", json!({"summary": "ui done"}));
        let t2 = rt.db.tasks(None, None, Some(&run.id)).unwrap().into_iter().find(|t| t.title == "UI").unwrap();
        ok(&rt, lead, "merge", json!({"branch": "orch/fx/front-1", "task": t2.id}));
        rt.team_tick();
        assert_eq!(state(&rt, &run.id), RunState::Integrating);
        assert!(queued(&rt, lead).iter().any(|b| b.contains("request_pr")));

        // Pause and resume the team (FR-406).
        rt.run_pause(&run.id).unwrap();
        assert_eq!(state(&rt, &run.id), RunState::Paused);
        assert!(rt.with_governor(|g| g.carry().paused_agents.iter().any(|a| a.0 == back)));
        rt.run_resume(&run.id).unwrap();
        assert_eq!(state(&rt, &run.id), RunState::Integrating);

        // PR gate: declined goes back to integrating.
        ok(&rt, lead, "request_pr", json!({"title": "Fix X"}));
        assert_eq!(state(&rt, &run.id), RunState::PrGate);
        let gate = rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap());
        assert_eq!(gate.payload["branch"], "orch/fx/integration");
        rt.decide(gate.id, InboxDecision::Reject, None, Some("not yet")).unwrap();
        assert_eq!(state(&rt, &run.id), RunState::Integrating);
        let ev = rt.db.run_events(&run.id, 100).unwrap();
        for k in ["run.launched", "task.created", "task.assigned", "message", "git.merge", "task.done", "run.state"] {
            assert!(ev.iter().any(|e| e.kind == k), "timeline has {k}");
        }
    }

    #[test]
    fn deleting_a_feature_ends_its_run_and_hides_it_and_its_agents() {
        let (_d, rt, _rd, _repo, pid) = setup();
        let run = rt
            .launch(&LaunchSpec {
                project_id: pid.clone(),
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                title: "Fix X".into(),
                brief: "Fix the X bug.".into(),
                base: None,
            })
            .unwrap();
        let lead = "fx-lead-1";
        let wt = rt.store.read(|s| s.agents[lead].worktree.clone());
        let out = rt.delete_feature(&run.feature_id, true, true).unwrap();
        assert_eq!(out["agents"].as_array().unwrap().len(), 1, "{out}");
        assert_eq!(state(&rt, &run.id), RunState::Failed);
        assert_eq!(rt.store.read(|s| s.agents[lead].tracker.state), AgentState::Archived);
        assert!(queued(&rt, lead).is_empty());
        assert!(rt.db.features(Some(&pid)).unwrap().is_empty());
        assert!(!wt.exists(), "integration worktree removed");
        // The lead's worktree is the integration one: removed once, reported once.
        let wts = out["worktrees"].as_array().unwrap();
        assert_eq!(wts.len(), 1, "{out}");
        assert_eq!(wts[0]["worktree_removed"], true, "{out}");
        // The tick starts nothing for a deleted feature.
        let before = rt.store.read(|s| s.agents.len());
        rt.team_tick();
        assert_eq!(rt.store.read(|s| s.agents.len()), before);
        // The slug can be launched again, and the feature comes back.
        let again = rt
            .launch(&LaunchSpec {
                project_id: pid.clone(),
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                title: "Again".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(again.feature_id, run.feature_id);
        assert_eq!(rt.db.features(Some(&pid)).unwrap().len(), 1);
    }

    #[test]
    fn the_watchdog_flags_long_and_silent_turns() {
        let (_d, rt) = test_runtime(false);
        let mut a = rt.store.read(|s| s.agents["a1"].clone());
        let min = 60_000;
        assert_eq!(watchdog_check(&a, 100 * min), None, "no turn running");
        a.tracker.state = AgentState::Working;
        a.tracker.turn_started_ms = Some(0);
        a.last_event_at_ms = Some(10 * min);
        assert_eq!(watchdog_check(&a, 20 * min), None);
        assert_eq!(watchdog_check(&a, 26 * min), Some((Watch::Stalled, 0)));
        a.tracker.background_tasks = 1;
        assert_eq!(watchdog_check(&a, 26 * min), None, "background work is progress elsewhere");
        a.role.turn_ceiling_minutes = Some(20);
        assert_eq!(watchdog_check(&a, 21 * min), Some((Watch::Ceiling(20), 0)));
        a.tracker.state = AgentState::Idle;
        assert_eq!(watchdog_check(&a, 60 * min), None);
        assert!(rt.watchdog_once("a1:0:Stalled".into()) && !rt.watchdog_once("a1:0:Stalled".into()));
    }

    #[test]
    fn solo_sessions_keep_their_two_tools() {
        let (_d, rt) = test_runtime(false);
        assert!(call(&rt, "a1", "send_message", json!({"to": "x", "body": "y"})).1.contains("not one of your tools"));
    }

    #[test]
    fn docs_are_read_only_from_the_feature_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".orchestrator/features/f");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spec.md"), "# S").unwrap();
        std::fs::write(dir.join("api-contract.md"), "# A").unwrap();
        assert_eq!(feature_docs(d.path(), "f"), vec!["spec.md", "api-contract.md"]);
        assert_eq!(read_feature_doc(d.path(), "f", "spec.md").unwrap(), "# S");
        for bad in ["../x.md", "a/b.md", "spec.txt"] {
            assert!(read_feature_doc(d.path(), "f", bad).is_err());
        }
    }
}
