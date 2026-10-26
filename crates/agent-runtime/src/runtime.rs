//! The headless core: owns the agents' PTYs, applies hook events to the state
//! machine, feeds the usage collector and the Governor, holds PreToolUse
//! calls the Governor does not admit, and delivers queued turns.
//!
//! I/O-free entry points (`on_hook`, `on_status`, `pump`, `tick`) so a server
//! (orchd now, the Tauri core later) only moves bytes.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use governor::jiff::{Timestamp, ToSpan, tz::TimeZone};
use governor::{
    AgentId, BudgetAllocation, Decision, Governor, Release, Tier, ToolCallRequest, TurnKind, TurnRequest,
    Verdict, WeeklyPlan, WorkingCalendar,
};
use orch_protocol::ctl::{AdmissionDecision, AdmissionPoint, AdmissionRequest, AdmissionVerdict};
use orch_protocol::{
    AgentSpec, HookEvent, HookMsg, HookOutput, HookPayload, StatusMsg, StatusSample, ToolDecision, now_ms,
};
use serde_json::{Value, json};

use crate::compile::{CompileCtx, HelperPaths, SessionMode, compile};
use crate::delivery::{self, GateInputs};
use crate::env::EnvInputs;
use crate::paths::{AppPaths, real, valid_id};
use crate::preflight::{self, VersionCheck, VersionRange};
use crate::procs::{self, AgentMatch, ProcessTable, SystemProcs};
use crate::resources::{self, LiveAgent, MemoryProbe, PressureLevel, SystemMemory};
use crate::pty::PtyProcess;
use crate::reconcile::{self, ReconcileEnv, Report};
use crate::role::{PermissionMode, Role};
use crate::state::{self, AgentState, Effect, StopIntent, TurnOrigin};
use crate::store::{Admission, AgentRecord, HoldKind, HoldRow, HoldStatus, Store, TurnRow, TurnStatus};
use crate::usage::UsageCollector;
use crate::worktree;

pub const DEFAULT_ALLOCATION: &str = "default";
/// The CLI shows its "continuing automatically at …" banner just after the
/// StopFailure hook returns; Esc sent before it would cancel nothing.
const ESC_AFTER_RATE_LIMIT: Duration = Duration::from_secs(1);
const CONTINUE_MESSAGE: &str = "ORCH: the orchestrator admitted your work again. Continue the task you were doing.";

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub paths: AppPaths,
    pub claude: PathBuf,
    pub helpers: HelperPaths,
    pub version_range: VersionRange,
    /// `~/.claude/projects` (read only).
    pub projects_root: PathBuf,
    /// Local OTLP receiver; `None` = telemetry off.
    pub otel_endpoint: Option<String>,
    /// FR-812: log governor decisions without enforcing them.
    pub shadow: bool,
    pub base_cap: u32,
    /// How long a PreToolUse waits for the Governor before denying (ADR-007: "a few seconds").
    pub hold: Duration,
    pub rows: u16,
    pub cols: u16,
    /// FR-705: how long a PermissionRequest waits for an Inbox decision
    /// before the CLI's own dialog takes over in the pane. Zero = straight to the pane.
    pub permission_hold: Duration,
    /// FR-204: suspend an idle agent after this long (`None` = never).
    pub idle_suspend: Option<Duration>,
    /// FR-203: macOS memory pressure lowers the live-agent cap.
    pub memory: Arc<dyn MemoryProbe>,
    /// NFR-8 retention of raw events, snapshots and samples.
    pub retention: orch_store::Retention,
    /// FR-103 background fetch.
    pub fetch: orch_git::fetch::FetchPolicy,
    /// Power and network state for the fetch throttle.
    pub conditions: Arc<dyn orch_git::fetch::Conditions>,
}

impl RuntimeConfig {
    pub fn new(paths: AppPaths) -> RuntimeConfig {
        let home = crate::paths::home_dir();
        RuntimeConfig {
            helpers: HelperPaths::in_dir(&paths.bin_dir()),
            paths,
            claude: home.join(".local/bin/claude"),
            version_range: preflight::SUPPORTED_CLAUDE,
            projects_root: crate::transcript::projects_root(&home),
            otel_endpoint: None,
            shadow: true,
            base_cap: 3,
            hold: Duration::from_secs(5),
            rows: 50,
            cols: 160,
            permission_hold: Duration::from_secs(300),
            idle_suspend: Some(Duration::from_secs(10 * 60)),
            memory: Arc::new(SystemMemory),
            retention: orch_store::Retention::default(),
            fetch: orch_git::fetch::FetchPolicy::default(),
            conditions: Arc::new(orch_git::fetch::SystemConditions),
        }
    }
}

/// A live agent process and its delivery clocks.
pub struct Live {
    pub pty: PtyProcess,
    /// 0 = input not allowed yet (before SessionStart + 1.5 s, or during a turn).
    pub ready_at_ms: AtomicU64,
    pub last_user_input_ms: AtomicU64,
}

impl Live {
    /// The user's keystrokes: delivery waits 2 s after them (FR-206).
    pub fn user_input(&self, bytes: &[u8]) -> Result<()> {
        self.last_user_input_ms.store(now_ms(), Ordering::SeqCst);
        self.pty.write(bytes)
    }
}

pub struct Runtime {
    pub cfg: RuntimeConfig,
    pub store: Store,
    /// The SQLite database (shared with the git service and the UI's queries).
    pub db: orch_store::Store,
    gov: Mutex<Governor>,
    usage: Mutex<UsageCollector>,
    live: Mutex<BTreeMap<String, Arc<Live>>>,
    decisions: Mutex<VecDeque<AdmissionDecision>>,
    login_path: String,
    user_statusline: Option<String>,
    git_identity: (String, String),
    pub warnings: Vec<String>,
    pub(crate) me: Weak<Runtime>,
    /// Core start time: Governor action ids restart at each boot.
    boot_ms: u64,
    house: Mutex<House>,
    /// The local OTLP receiver once started ([`Runtime::start_otlp`]).
    otlp: Mutex<Option<crate::otlp::OtlpReceiver>>,
}

/// Clocks and flags of the periodic work (`tick`, `housekeeping`).
#[derive(Default)]
struct House {
    last_forecast_ms: u64,
    last_calibration_ms: u64,
    last_purge_ms: u64,
    last_index_ms: u64,
    pressure: PressureLevel,
    /// Agents whose transcript changed since the last index pass.
    index_dirty: std::collections::BTreeSet<String>,
    index_all: bool,
    /// Agents told they wait for a live slot (logged once per wait).
    cap_waiting: std::collections::BTreeSet<String>,
}

pub const FORECAST_EVERY_MS: u64 = 5 * 60 * 1000;
pub const CALIBRATION_EVERY_MS: u64 = 60 * 60 * 1000;
pub const PURGE_EVERY_MS: u64 = 24 * 3600 * 1000;
pub const INDEX_EVERY_MS: u64 = 60 * 1000;
pub const POLICY_ID: &str = "default";

/// The plan week: from last Monday 09:00 local to next Monday 09:00, weekdays
/// 09:00–19:00, one allocation. The Week planner (FR-802) replaces this later.
pub fn default_plan(now: Timestamp, shadow: bool) -> WeeklyPlan {
    let tz = TimeZone::system();
    let zdt = now.to_zoned(tz.clone());
    let date = zdt.date();
    let monday = date.checked_sub((date.weekday().to_monday_zero_offset() as i64).days()).unwrap_or(date);
    let mut reset = monday.at(9, 0, 0, 0).to_zoned(tz.clone()).map(|z| z.timestamp()).unwrap_or(now);
    if reset <= now {
        reset = reset.checked_add(168.hours()).unwrap_or(reset);
    }
    let week_start = reset.checked_sub(168.hours()).unwrap_or(reset);
    let tz_name = tz.iana_name().unwrap_or("UTC").to_string();
    WeeklyPlan {
        id: format!("week-{}", week_start.as_second()),
        week_start,
        reset_at: reset,
        calendar: WorkingCalendar::weekdays(
            &tz_name,
            governor::jiff::civil::time(9, 0, 0, 0),
            governor::jiff::civil::time(19, 0, 0, 0),
        ),
        reserve_pct: 10.0,
        off_hours_allowance_pct: 0.0,
        shadow_mode: shadow,
        allocations: vec![BudgetAllocation {
            id: DEFAULT_ALLOCATION.into(),
            target: "all projects".into(),
            weight: 100.0,
            tier: Tier::Normal,
            hard_cap_pct: None,
        }],
    }
}

fn verdict_kind(v: &Verdict) -> AdmissionVerdict {
    match v {
        Verdict::Admit => AdmissionVerdict::Admit,
        Verdict::Queue { .. } => AdmissionVerdict::Queue,
        Verdict::Deny { .. } => AdmissionVerdict::Deny,
    }
}

fn verdict_reason(v: &Verdict) -> Option<String> {
    match v {
        Verdict::Admit => None,
        Verdict::Queue { reason, .. } => Some(reason.to_string()),
        Verdict::Deny { reason } => Some(reason.to_string()),
    }
}

fn parse_tier(s: Option<&str>) -> Result<Tier> {
    Ok(match s.unwrap_or("normal") {
        "low" => Tier::Low,
        "normal" => Tier::Normal,
        "high" => Tier::High,
        "critical" => Tier::Critical,
        other => bail!("unknown tier {other:?}"),
    })
}

/// The Role an `AgentSpec` describes (solo sessions; team roles come later).
pub fn role_from_spec(spec: &AgentSpec) -> Result<Role> {
    let mut role = Role::solo();
    role.model = spec.model.clone();
    role.effort = spec.effort.clone();
    role.prompt = spec.prompt.clone();
    role.strict_mcp = spec.strict_mcp.unwrap_or(false);
    if let Some(m) = &spec.permission_mode {
        role.permission_mode = PermissionMode::parse(m).map_err(|e| anyhow!(e))?;
    }
    role.network_allow.extend(spec.network_allow.iter().cloned());
    role.allow_rules.extend(spec.allow_rules.iter().cloned());
    Ok(role)
}

impl Runtime {
    pub fn new(cfg: RuntimeConfig) -> Result<Arc<Runtime>> {
        std::fs::create_dir_all(cfg.paths.agents_dir())?;
        std::fs::create_dir_all(cfg.paths.state_dir())?;
        orch_protocol::check_socket_path(&cfg.paths.socket())?;
        let db = orch_store::Store::open(&cfg.paths.db_file())?;
        let store = Store::open(db.clone(), Some(&cfg.paths.legacy_state_file()))?;
        let now = Timestamp::now();
        let gov = restore_governor(&db, &store, &cfg, now)?;
        let home = crate::paths::home_dir();
        let shell = std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| "/bin/zsh".into());
        // Independent subprocesses (the login shell can take seconds): run them together.
        let (login_path, version, git, git_identity) = std::thread::scope(|sc| {
            let path = sc.spawn(|| crate::env::login_path(&shell, &home));
            let version = sc.spawn(|| preflight::claude_version(&cfg.claude).and_then(|v| cfg.version_range.check(v)));
            let git = sc.spawn(preflight::check_git);
            let identity = crate::env::user_git_identity();
            let join = "startup probe panicked";
            (path.join().expect(join), version.join().expect(join), git.join().expect(join), identity)
        });
        let mut warnings = crate::env::parent_env_warnings();
        match version {
            Ok(VersionCheck::Untested(v)) => {
                warnings.push(format!("claude {v} is newer than the last tested version: run the contract tests (R2)"))
            }
            Ok(VersionCheck::Supported(_)) => {}
            Err(e) => warnings.push(format!("claude: {e}")),
        }
        if let Err(e) = git {
            warnings.push(e.to_string());
        }
        Ok(Arc::new_cyclic(|me| Runtime {
            user_statusline: crate::env::user_statusline_command(&home),
            git_identity,
            cfg,
            store,
            db,
            gov: Mutex::new(gov),
            usage: Mutex::new(UsageCollector::new()),
            live: Mutex::new(BTreeMap::new()),
            decisions: Mutex::new(VecDeque::new()),
            login_path,
            warnings,
            me: me.clone(),
            boot_ms: now_ms(),
            house: Mutex::new(House { index_all: true, ..Default::default() }),
            otlp: Mutex::new(None),
        }))
    }

    fn gov(&self) -> std::sync::MutexGuard<'_, Governor> {
        self.gov.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn live_get(&self, id: &str) -> Option<Arc<Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// The live process of an agent, for a UI that drives its PTY directly
    /// (pull transport, keystrokes) instead of `attach`.
    pub fn live(&self, id: &str) -> Option<Arc<Live>> {
        self.live_get(id)
    }

    /// Read access to the Governor (snapshot, curve, queue, log) for a UI.
    pub fn with_governor<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        f(&self.gov())
    }

    /// A user control on the Governor (FR-809/811: run now, undo, asks, sprint).
    /// Releases it returns are handled like the loop's.
    pub fn governor_act<R>(&self, f: impl FnOnce(&mut Governor, Timestamp) -> (R, Vec<Release>)) -> R {
        let (r, releases) = f(&mut self.gov(), Timestamp::now());
        self.handle_releases(releases);
        r
    }

    /// A permission prompt was answered outside the pane (the Inbox): the
    /// tool now runs or is refused, so the agent is working again until its
    /// next hook says otherwise.
    pub fn permission_answered(&self, id: &str) -> Result<()> {
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id).filter(|a| a.tracker.state == AgentState::WaitingForYou) {
                a.tracker.state = AgentState::Working;
            }
        })
    }

    pub fn live_ids(&self) -> Vec<String> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// Appends to the event table (never blocks on disk). `agent` "-" = core-wide.
    pub(crate) fn event(&self, agent: &str, kind: &str, detail: Value) {
        self.db.add_event(now_ms(), Some(agent).filter(|a| *a != "-"), kind, detail);
    }

    fn record_decision(&self, req: AdmissionRequest, d: &Decision) {
        let est = match &d.effective {
            Verdict::Queue { estimated_start: Some(t), .. } => Some(t.as_millisecond() as u64),
            _ => None,
        };
        let a = AdmissionDecision {
            request: req,
            ts_ms: now_ms(),
            verdict: verdict_kind(&d.effective),
            proposed: verdict_kind(&d.proposed),
            reason: verdict_reason(&d.proposed),
            estimated_start_ms: est,
        };
        self.event(&a.request.agent_id.clone(), "admission", serde_json::to_value(&a).unwrap_or(Value::Null));
        let mut q = self.decisions.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(a);
        while q.len() > 200 {
            q.pop_front();
        }
    }

    /// Admission point 1: asks the Governor and logs the decision.
    fn decide_turn(&self, rec: &AgentRecord, kind: TurnKind, origin: TurnOrigin) -> Decision {
        let req = TurnRequest {
            agent: AgentId(rec.id.clone()),
            allocation: rec.allocation.clone().into(),
            tier: rec.tier,
            interactive: origin == TurnOrigin::User,
            kind,
            self_started: origin == TurnOrigin::SelfStarted,
            ceiling: None,
        };
        let ar = AdmissionRequest {
            agent_id: rec.id.clone(),
            point: AdmissionPoint::TurnStart,
            interactive: req.interactive,
            self_started: req.self_started,
        };
        let d = self.gov().request_turn(req, Timestamp::now());
        self.record_decision(ar, &d);
        d
    }

    // ------------------------------------------------------------ recovery

    /// J10: reconcile, then resume the agents that were working.
    pub fn recover(&self, apply: bool) -> Result<Report> {
        let me = [std::process::id() as i32];
        let kill = |g: &[i32], p: &[i32]| procs::kill_tree(g, p, Duration::from_secs(5));
        let report = reconcile::reconcile(&ReconcileEnv {
            store: &self.store,
            paths: &self.cfg.paths,
            projects_root: &self.cfg.projects_root,
            procs: &SystemProcs,
            kill: &kill,
            exclude_pids: &me,
            apply,
        })?;
        if apply {
            self.event("-", "reconciled", serde_json::to_value(&report).unwrap_or(Value::Null));
            for id in report.to_resume() {
                if let Err(e) = self.spawn(&id, None) {
                    self.event(&id, "resume_failed", json!(e.to_string()));
                }
            }
        }
        Ok(report)
    }

    // ------------------------------------------------------------ lifecycle

    pub fn create_worktree(
        &self,
        repo: &Path,
        kind: &orch_protocol::WorktreeKindSpec,
        base: Option<&str>,
    ) -> Result<crate::store::WorktreeRow> {
        preflight::check_git()?;
        let repo = worktree::toplevel(repo).with_context(|| format!("{} is not a git repository", repo.display()))?;
        worktree::ensure(&self.store, &repo, kind, base, None)
    }

    pub fn start(&self, spec: &AgentSpec) -> Result<AgentRecord> {
        if !valid_id(&spec.agent_id) {
            bail!("invalid agent id {:?}", spec.agent_id);
        }
        if self.store.read(|s| s.agents.get(&spec.agent_id).is_some_and(|a| a.tracker.state != AgentState::Archived)) {
            bail!("agent {} exists: resume it instead", spec.agent_id);
        }
        preflight::check_git()?;
        let repo = worktree::toplevel(Path::new(&spec.repo))
            .with_context(|| format!("{} is not a git repository", spec.repo))?;
        let slug = spec.slug.clone().unwrap_or_else(|| spec.agent_id.clone());
        let row = worktree::ensure(
            &self.store,
            &repo,
            &orch_protocol::WorktreeKindSpec::Solo { slug },
            spec.base.as_deref(),
            Some(&spec.agent_id),
        )?;
        let rec = AgentRecord {
            id: spec.agent_id.clone(),
            // The main checkout (FR-105): its entries are denied one by one, and
            // the project's own folder, if a linked worktree, is an "other" one.
            repo: row.repo.clone(),
            worktree: row.path.clone(),
            role: role_from_spec(spec)?,
            session_id: uuid::Uuid::new_v4().to_string(),
            tracker: Default::default(),
            pid: None,
            pgid: None,
            allocation: spec.allocation.clone().unwrap_or_else(|| DEFAULT_ALLOCATION.into()),
            tier: parse_tier(spec.tier.as_deref())?,
            created_at_ms: now_ms(),
            last_event: None,
            last_event_at_ms: None,
            spawns: 0,
            flag: None,
            trust_dialog: false,
        };
        self.store.write(|s| s.agents.insert(rec.id.clone(), rec.clone()))?;
        // Every agent belongs to a registered project (J1); starting one marks it active (FR-103).
        if let Ok(p) = orch_git::project::register(&self.db, &repo) {
            let _ = self.db.touch_project(&p.id, now_ms());
        }
        self.spawn(&rec.id, Some(SessionMode::New(rec.session_id.clone())))?;
        if let Some(m) = &spec.first_message {
            self.deliver(&rec.id, m)?;
        }
        Ok(self.store.read(|s| s.agents[&rec.id].clone()))
    }

    /// Spawns `claude` for an existing agent: `--session-id` on first start,
    /// `--resume` afterwards (the default when `mode` is `None`).
    pub fn spawn(&self, id: &str, mode: Option<SessionMode>) -> Result<()> {
        if self.live_get(id).is_some() {
            bail!("agent {id} is already running");
        }
        let rec = self.store.read(|s| s.agents.get(id).cloned()).ok_or_else(|| anyhow!("no agent {id}"))?;
        let mode = mode.unwrap_or_else(|| SessionMode::Resume(rec.session_id.clone()));
        // Checked at every spawn (NFR-7, FR-1201).
        let v = preflight::claude_version(&self.cfg.claude)?;
        self.cfg.version_range.check(v)?;
        preflight::check_helpers(&self.cfg.helpers)?;
        let wt = real(&rec.worktree);
        if !wt.is_dir() {
            bail!("worktree {} is missing", wt.display());
        }
        let agent_dir = self.cfg.paths.agent_dir(id);
        let mut env = EnvInputs::from_process(self.login_path.clone());
        env.socket = self.cfg.paths.socket();
        env.user_statusline = self.user_statusline.clone();
        env.otel_endpoint = self.cfg.otel_endpoint.clone().or_else(|| self.otlp_endpoint());
        let git_common_dir = worktree::common_dir(&wt)?;
        let main = real(&rec.repo);
        let ctx = CompileCtx {
            agent_id: id.into(),
            name: id.into(),
            agent_dir: agent_dir.clone(),
            app_root: self.cfg.paths.root.clone(),
            main_read_denies: crate::compile::main_read_denies(&main, &git_common_dir),
            git_common_dir,
            other_worktrees: worktree::others(&rec.repo, &wt),
            main_checkout: main,
            worktree: wt,
            helpers: self.cfg.helpers.clone(),
            claude: self.cfg.claude.clone(),
            session: mode,
            env,
            git_identity: self.git_identity.clone(),
        };
        let compiled = compile(&rec.role, &ctx);
        compiled.write()?;
        let weak = self.me.clone();
        let id_owned = id.to_string();
        // The pid is known only after the spawn; the exit callback needs it so a
        // late exit of an old process never touches a newer one.
        let pid_cell = Arc::new(std::sync::OnceLock::<i32>::new());
        let pid_for_exit = pid_cell.clone();
        let pty = PtyProcess::spawn(
            &compiled,
            self.cfg.rows,
            self.cfg.cols,
            None,
            Box::new(move |status| {
                if let (Some(rt), Some(pid)) = (weak.upgrade(), pid_for_exit.get()) {
                    rt.on_exit(&id_owned, *pid, status);
                }
            }),
        )?;
        let (pid, pgid) = (pty.pid, pty.pgid);
        let _ = pid_cell.set(pid);
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id.into(),
            Arc::new(Live { pty, ready_at_ms: AtomicU64::new(0), last_user_input_ms: AtomicU64::new(0) }),
        );
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.on_spawned(pid, pgid);
            }
        })?;
        self.event(id, "spawned", json!({"pid": pid, "args": compiled.args}));
        Ok(())
    }

    fn on_exit(&self, id: &str, pid: i32, status: Option<u32>) {
        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            if live.get(id).is_some_and(|l| l.pty.pid == pid) {
                live.remove(id);
            }
        }
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id).filter(|a| a.pid == Some(pid)) {
                a.on_process_gone();
            }
        });
        let releases = self.gov().turn_ended(&AgentId(id.into()), Timestamp::now());
        self.handle_releases(releases);
        self.event(id, "exited", json!({"status": status}));
    }

    /// Stops the process and keeps the session (FR-204).
    pub fn suspend(&self, id: &str) -> Result<Vec<String>> {
        self.stop(id, StopIntent::Suspend)
    }

    /// FR-207: SIGTERM to `claude`'s group and to every tty-less process whose
    /// cwd is inside the worktree, SIGKILL after 5 s.
    pub fn kill(&self, id: &str) -> Result<Vec<String>> {
        self.stop(id, StopIntent::Kill)
    }

    fn stop(&self, id: &str, intent: StopIntent) -> Result<Vec<String>> {
        let rec = self.store.read(|s| s.agents.get(id).cloned()).ok_or_else(|| anyhow!("no agent {id}"))?;
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker.stop_intent = intent;
            }
        })?;
        let live = self.live_get(id);
        let (pgid, pid) = match &live {
            Some(l) => (Some(l.pty.pgid), Some(l.pty.pid)),
            None => (rec.pgid, rec.pid),
        };
        let log = match intent {
            StopIntent::Suspend => procs::kill_tree(
                &pgid.into_iter().collect::<Vec<_>>(),
                &pid.into_iter().collect::<Vec<_>>(),
                Duration::from_secs(5),
            ),
            _ => {
                let agent_dir = self.cfg.paths.agent_dir(id);
                let wt = real(&rec.worktree);
                let me = [std::process::id() as i32];
                let found = procs::find_agent_procs(
                    &SystemProcs.snapshot(),
                    &AgentMatch { pgid, agent_dir: &agent_dir, worktree: &wt, exclude: &me },
                );
                let mut pgids: Vec<i32> = found.iter().map(|f| f.proc.pgid).chain(pgid).filter(|g| *g > 1).collect();
                pgids.sort();
                pgids.dedup();
                let pids: Vec<i32> = found.iter().map(|f| f.proc.pid).chain(pid).collect();
                procs::kill_tree(&pgids, &pids, Duration::from_secs(5))
            }
        };
        if live.is_none() {
            // Not ours to wait for: settle the state now.
            self.store.write(|s| {
                if let Some(a) = s.agents.get_mut(id) {
                    a.on_process_gone();
                }
            })?;
        } else if let Some(l) = live {
            // Wait (bounded) until the exit callback has run, so callers see the
            // new state and a resume does not race it.
            for _ in 0..100 {
                if self.live_get(id).is_none_or(|cur| cur.pty.pid != l.pty.pid) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        self.event(id, "stopped", json!({"intent": intent, "log": log}));
        Ok(log)
    }

    pub fn resume(&self, id: &str, message: Option<&str>) -> Result<()> {
        self.spawn(id, None)?;
        if let Some(m) = message {
            self.deliver(id, m)?;
        }
        Ok(())
    }

    /// Esc to the PTY.
    pub fn interrupt(&self, id: &str) -> Result<()> {
        let l = self.live_get(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        l.pty.write(b"\x1b")
    }

    /// Queues an app-delivered turn. The Governor admits it when the agent is
    /// idle; then it is pasted with its turn marker.
    pub fn deliver(&self, id: &str, text: &str) -> Result<String> {
        if !self.store.read(|s| s.agents.contains_key(id)) {
            bail!("no agent {id}");
        }
        let turn = TurnRow::queued(id, text, TurnKind::Message, Admission::Pending);
        let tid = turn.id.clone();
        self.store.write(|s| s.turns.push(turn))?;
        Ok(tid)
    }

    /// FR-811 Pause all. Returns the Governor action id.
    pub fn pause_all(&self) -> u64 {
        self.gov().pause_all(Timestamp::now()).0
    }

    /// Lifts Pause all; returns how many queued turns were released.
    pub fn resume_all(&self) -> usize {
        let releases = self.gov().resume_all(Timestamp::now());
        let n = releases.len();
        self.handle_releases(releases);
        n
    }

    /// Graceful stop of the core: suspend every live agent.
    pub fn shutdown(&self) {
        for id in self.live_ids() {
            let _ = self.suspend(&id);
        }
        self.otlp.lock().unwrap_or_else(|e| e.into_inner()).take();
        let _ = self.db.flush();
    }

    /// FR-801: starts the local OTLP receiver (127.0.0.1, `port` 0 = any) and
    /// points agents spawned from now on at it. Returns the endpoint.
    pub fn start_otlp(&self, port: u16) -> Result<String> {
        let weak = self.me.clone();
        let rx = crate::otlp::OtlpReceiver::start(
            port,
            Arc::new(move |reqs| {
                if let Some(rt) = weak.upgrade() {
                    rt.on_api_requests(reqs);
                }
            }),
        )?;
        let ep = rx.endpoint();
        *self.otlp.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
        self.event("-", "otlp_started", json!({"endpoint": ep}));
        Ok(ep)
    }

    pub fn otlp_endpoint(&self) -> Option<String> {
        self.otlp.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.endpoint())
    }

    /// Per-request usage from OTLP (tokens, cost), stored as `otel` samples.
    /// An unknown agent id is kept as unattributed.
    pub fn on_api_requests(&self, reqs: Vec<crate::otlp::ApiRequest>) {
        for r in reqs {
            let agent = r.agent_id.clone().filter(|a| self.store.read(|s| s.agents.contains_key(a)));
            self.db.add_usage(orch_store::UsageRecord {
                ts_ms: r.ts_ms,
                agent_id: agent,
                model: r.model,
                tokens_in: Some(r.input_tokens),
                tokens_out: Some(r.output_tokens),
                cache_read: Some(r.cache_read_tokens),
                cache_write: Some(r.cache_creation_tokens),
                cost_estimate: Some(r.cost_usd),
                provenance: "observed".into(),
                source: Some(orch_store::UsageSource::Otel),
                ..Default::default()
            });
        }
    }

    // ------------------------------------------------------------ hooks

    /// One hook from orch-hook. `conn` is watched for EOF while a PreToolUse
    /// is held (Esc in the pane kills orch-hook and no hook event reports it).
    pub fn on_hook(&self, m: &HookMsg, conn: Option<&UnixStream>) -> Option<HookOutput> {
        let id = m.agent_id.as_str();
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else {
            self.event(id, "hook_unknown_agent", json!(m.event));
            // Fail closed: an agent the core doesn't know is not governed.
            return HookOutput::fail_closed(&m.event);
        };
        let p = HookPayload::parse(&m.payload);
        self.event(
            id,
            m.event.as_str(),
            json!({"tool": p.tool_name, "prompt_id": p.prompt_id, "source": p.source, "error": p.error}),
        );
        // Applied under the store lock: concurrent hooks (parallel tool calls)
        // must not overwrite each other's transitions.
        let (tracker, effects) = self
            .store
            .write(|s| {
                let a = s.agents.get_mut(id)?;
                let effects = state::apply(&mut a.tracker, &m.event, &p);
                a.last_event = Some(m.event.to_string());
                a.last_event_at_ms = Some(now_ms());
                Some((a.tracker.clone(), effects))
            })
            .ok()
            .flatten()
            .unwrap_or_else(|| (rec.tracker.clone(), vec![]));
        for fx in effects {
            self.on_effect(&rec, fx, &p);
        }
        match m.event {
            HookEvent::PreToolUse => {
                self.admit_tool(&rec, tracker.turn_origin == Some(TurnOrigin::User), &p, m.deadline_ms, conn)
            }
            // A rule, or the Inbox (held), or the pane when nobody answers in time.
            HookEvent::PermissionRequest => self.permission_request(&rec, &p, m.deadline_ms, conn),
            _ => None,
        }
    }

    fn on_effect(&self, rec: &AgentRecord, fx: Effect, p: &HookPayload) {
        let id = rec.id.as_str();
        let now = now_ms();
        let live = self.live_get(id);
        match fx {
            Effect::Ready => {
                if let Some(l) = &live {
                    l.pty.output.clear_trust_dialog();
                    l.ready_at_ms.store(now + delivery::AFTER_SESSION_START.as_millis() as u64, Ordering::SeqCst);
                }
            }
            Effect::TurnStarted { origin, markers } => {
                self.mark_delivered(id, &markers);
                if let Some(l) = &live {
                    l.ready_at_ms.store(0, Ordering::SeqCst);
                }
                match origin {
                    TurnOrigin::App => {} // admitted before delivery
                    TurnOrigin::User => {
                        self.decide_turn(rec, TurnKind::User, origin);
                    }
                    TurnOrigin::SelfStarted => {
                        self.decide_turn(rec, TurnKind::Continue, origin);
                    }
                }
            }
            Effect::QueuedMessage => {
                let markers = delivery::find_markers(p.prompt.as_deref().unwrap_or(""));
                self.mark_delivered(id, &markers);
            }
            Effect::TurnEnded | Effect::TurnEndedWithBackground => {
                self.house().index_dirty.insert(id.to_string());
                let delivered = |t: &TurnRow| t.agent_id == id && t.status == TurnStatus::Delivered;
                if self.store.read(|s| s.turns.iter().any(delivered)) {
                    let _ = self.store.write(|s| {
                        for turn in s.turns.iter_mut().filter(|t| delivered(t)) {
                            turn.status = TurnStatus::Done;
                        }
                    });
                }
                if let (Some(l), Effect::TurnEnded) = (&live, &fx) {
                    l.ready_at_ms.store(now + delivery::AFTER_STOP.as_millis() as u64, Ordering::SeqCst);
                }
                let releases = self.gov().turn_ended(&AgentId(id.into()), Timestamp::now());
                self.handle_releases(releases);
            }
            Effect::RateLimited { message } => {
                let (window, resets) = self
                    .usage
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .exhausted_window(id)
                    .unwrap_or((governor::Window::FiveHour, (now / 1000) as i64 + 3600));
                let resets_at = Timestamp::from_second(resets).unwrap_or_else(|_| Timestamp::now());
                let resp = self.gov().rate_limit_hit(Some(&AgentId(id.into())), window, resets_at, Timestamp::now());
                self.event(id, "rate_limited", json!({"window": window, "resets_at": resets, "message": message, "cancel_auto_continue": resp.cancel_auto_continue}));
                if resp.cancel_auto_continue {
                    // J9: cancel the CLI's own auto-continue; the Governor reschedules.
                    if let Some(l) = live {
                        std::thread::spawn(move || {
                            std::thread::sleep(ESC_AFTER_RATE_LIMIT);
                            let _ = l.pty.write(b"\x1b");
                        });
                    }
                }
            }
            Effect::Failed { error } => {
                let _ = self.store.write(|s| s.add_inbox(Some(id), "alert", format!("{id}: API error {error}")));
                let releases = self.gov().turn_ended(&AgentId(id.into()), Timestamp::now());
                self.handle_releases(releases);
            }
            Effect::NeedsUser => {}
        }
    }

    fn mark_delivered(&self, id: &str, markers: &[String]) {
        if markers.is_empty() {
            return;
        }
        let _ = self.store.write(|s| {
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id && markers.contains(&t.id)) {
                if matches!(t.status, TurnStatus::Queued | TurnStatus::Delivering) {
                    t.status = TurnStatus::Delivered;
                }
            }
        });
    }

    /// Admission point 2. Admitted → no output (the CLI's own permission flow
    /// applies). Not admitted → hold up to `cfg.hold` (and well inside
    /// orch-hook's deadline) for a release, then deny "paused by governor".
    fn admit_tool(
        &self,
        rec: &AgentRecord,
        interactive: bool,
        p: &HookPayload,
        deadline_ms: u64,
        conn: Option<&UnixStream>,
    ) -> Option<HookOutput> {
        let id = rec.id.as_str();
        let req = ToolCallRequest { agent: AgentId(id.into()), interactive };
        let ar =
            AdmissionRequest { agent_id: id.into(), point: AdmissionPoint::ToolCall, interactive, self_started: false };
        let d = self.gov().tool_call(&req, Timestamp::now());
        self.record_decision(ar, &d);
        let reason = match &d.effective {
            Verdict::Admit => return None,
            other => verdict_reason(other).unwrap_or_else(|| "paused by governor".into()),
        };
        // Hold.
        let budget = self.cfg.hold.min(Duration::from_millis(deadline_ms.saturating_sub(1_000)));
        let hold_id = delivery::new_turn_id();
        let summary = format!(
            "{} {}",
            p.tool_name.as_deref().unwrap_or("?"),
            p.tool_input.as_ref().map(|v| v.to_string()).unwrap_or_default()
        );
        let _ = self.store.write(|s| {
            s.holds.push(HoldRow {
                id: hold_id.clone(),
                agent_id: id.into(),
                tool_use_id: p.tool_use_id.clone(),
                kind: HoldKind::Governor,
                status: HoldStatus::Held,
                summary: summary.chars().take(200).collect(),
                created_at_ms: now_ms(),
                inbox_id: None,
            });
            if let Some(a) = s.agents.get_mut(id) {
                state::hold_started(&mut a.tracker);
            }
        });
        // Wait for a release: whichever thread receives it marks this hold
        // Allowed (`handle_releases`).
        let end = std::time::Instant::now() + budget;
        let hold_status = || self.store.read(|s| s.holds.iter().find(|h| h.id == hold_id).map(|h| h.status));
        let mut outcome = HoldStatus::Denied;
        while std::time::Instant::now() < end {
            if let Some(c) = conn {
                if conn_closed(c) {
                    outcome = HoldStatus::Interrupted;
                    break;
                }
            } else {
                std::thread::sleep(Duration::from_millis(250));
            }
            let releases = self.gov().tick(Timestamp::now());
            self.handle_releases(releases);
            if hold_status() == Some(HoldStatus::Allowed) {
                outcome = HoldStatus::Allowed;
                break;
            }
        }
        let _ = self.store.write(|s| {
            if let Some(h) = s.holds.iter_mut().find(|h| h.id == hold_id) {
                h.status = outcome;
            }
            if let Some(a) = s.agents.get_mut(id) {
                state::hold_ended(&mut a.tracker);
            }
        });
        self.event(id, "hold", json!({"id": hold_id, "outcome": outcome, "reason": reason}));
        match outcome {
            HoldStatus::Allowed | HoldStatus::Interrupted => None,
            _ => Some(HookOutput::pre_tool_use(
                ToolDecision::Deny,
                &format!(
                    "ORCH: paused by governor ({reason}). Stop here and end your turn; the orchestrator resumes you when there is budget."
                ),
            )),
        }
    }

    /// Carries out the Governor's releases. A release for an agent held in a
    /// PreToolUse allows that tool call; otherwise it admits the agent's queued
    /// turn (or a "continue" turn when none is queued).
    fn handle_releases(&self, releases: Vec<Release>) {
        for r in releases {
            let id = r.agent.0.clone();
            self.event(&id, "released", serde_json::to_value(&r).unwrap_or(Value::Null));
            let _ = self.store.write(|s| {
                let held = s
                    .holds
                    .iter_mut()
                    .find(|h| h.agent_id == id && h.kind == HoldKind::Governor && h.status == HoldStatus::Held);
                if let Some(h) = held {
                    h.status = HoldStatus::Allowed;
                    return;
                }
                let entry = r.entry.0;
                let existing = s
                    .turns
                    .iter_mut()
                    .filter(|t| {
                        t.agent_id == id && t.status == TurnStatus::Queued && t.admission != Admission::Admitted
                    })
                    .min_by_key(|t| (t.admission != Admission::Waiting(entry), t.created_at_ms));
                match existing {
                    Some(t) => t.admission = Admission::Admitted,
                    None => {
                        s.turns.push(TurnRow::queued(&id, CONTINUE_MESSAGE, TurnKind::Continue, Admission::Admitted))
                    }
                }
            });
        }
    }

    pub fn on_status(&self, m: &StatusMsg) {
        if !self.store.read(|s| s.agents.contains_key(&m.agent_id)) {
            return;
        }
        let sample = StatusSample::parse(&m.payload);
        let g = self.usage.lock().unwrap_or_else(|e| e.into_inner()).ingest(&m.agent_id, m.ts_ms, &sample);
        if let Some(g) = g {
            self.event(&m.agent_id, "usage_sample", serde_json::to_value(&g).unwrap_or(Value::Null));
            let mut gov = self.gov();
            let class = gov.cost_class(&g);
            let allocation = self.store.read(|s| s.agents.get(&m.agent_id).map(|a| a.allocation.clone()));
            self.db.add_usage(usage_record(&g, &sample, class, allocation));
            // FR-802: the plan week is aligned to the real weekly reset.
            if let Some(w) = g.weekly {
                let off = (w.resets_at.as_second() - gov.plan().reset_at.as_second()).abs();
                if off > 3600 && w.resets_at > g.ts {
                    let mut plan = gov.plan().clone();
                    plan.reset_at = w.resets_at;
                    plan.week_start = w.resets_at.checked_sub(168.hours()).unwrap_or(w.resets_at);
                    plan.id = format!("week-{}", plan.week_start.as_second());
                    if gov.new_week(plan.clone(), g.ts).is_ok() {
                        let _ = self.db.save_plan(&plan, orch_store::PlanStatus::Active);
                    }
                }
            }
            gov.ingest(&g);
        }
    }

    // ------------------------------------------------------------ loops

    /// The Governor's control loop (call every ~30 s). Also persists its
    /// recent actions, its queue, and (less often) forecast snapshots and the
    /// calibration.
    pub fn tick(&self) {
        let now = Timestamp::now();
        let releases = self.gov().tick(now);
        self.handle_releases(releases);
        let (actions, queue, plan_id, snap, calib) = {
            let g = self.gov();
            let n = g.actions().len();
            (g.actions()[n.saturating_sub(50)..].to_vec(), g.queue().to_vec(), g.plan().id.clone(), g.snapshot(now), g.calibration(now))
        };
        let _ = self.db.save_actions(self.boot_ms, &plan_id, &actions);
        let _ = self.db.replace_queue(self.boot_ms, &queue);
        let ms = now_ms();
        let (do_forecast, do_calib) = {
            let mut h = self.house();
            let f = ms.saturating_sub(h.last_forecast_ms) >= FORECAST_EVERY_MS;
            let c = ms.saturating_sub(h.last_calibration_ms) >= CALIBRATION_EVERY_MS;
            if f {
                h.last_forecast_ms = ms;
            }
            if c {
                h.last_calibration_ms = ms;
            }
            (f, c)
        };
        if do_forecast {
            let _ = self.db.add_forecast(&snap);
        }
        if do_calib {
            let _ = self.db.add_calibration(&calib);
        }
    }

    pub(crate) fn mark_full_index(&self) {
        self.house().index_all = true;
    }

    fn house(&self) -> std::sync::MutexGuard<'_, House> {
        self.house.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// FR-203: the live-agent cap now (base, lowered under memory pressure).
    pub fn live_cap(&self) -> u32 {
        resources::effective_cap(self.cfg.base_cap, self.house().pressure)
    }

    /// The Week planner's confirmed plan (FR-802): replaces the current one.
    pub fn set_plan(&self, plan: WeeklyPlan) -> Result<()> {
        self.gov().new_week(plan.clone(), Timestamp::now()).map_err(|e| anyhow!("{e:?}"))?;
        self.db.save_plan(&plan, orch_store::PlanStatus::Active)
    }

    /// The user is looking at or typing in this agent's pane (resets the idle timer).
    pub fn note_user_activity(&self, id: &str) {
        if let Some(l) = self.live_get(id) {
            l.last_user_input_ms.store(now_ms(), Ordering::SeqCst);
        }
    }

    /// Periodic upkeep (call every few seconds): memory pressure and the live
    /// cap, idle auto-suspend, transcript indexing, retention.
    pub fn housekeeping(&self) {
        let now = now_ms();
        let level = self.cfg.memory.level();
        let changed = {
            let mut h = self.house();
            let c = h.pressure != level;
            h.pressure = level;
            c
        };
        if changed {
            self.event("-", "memory_pressure", json!({"level": level, "cap": self.live_cap()}));
        }
        let queued: BTreeMap<String, usize> = self.store.read(|s| {
            let mut m = BTreeMap::new();
            for t in s.turns.iter().filter(|t| t.status == TurnStatus::Queued) {
                *m.entry(t.agent_id.clone()).or_default() += 1;
            }
            m
        });
        let live: Vec<LiveAgent> = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|(id, l)| {
                let rec = self.store.read(|s| s.agents.get(id).cloned())?;
                let last = [
                    rec.last_event_at_ms.unwrap_or(0),
                    l.last_user_input_ms.load(Ordering::SeqCst),
                    l.ready_at_ms.load(Ordering::SeqCst),
                    l.pty.spawned_at_ms,
                ]
                .into_iter()
                .max()
                .unwrap_or(0);
                Some(LiveAgent {
                    id: id.clone(),
                    idle: rec.tracker.state == AgentState::Idle,
                    background_tasks: rec.tracker.background_tasks,
                    queued_turns: queued.get(id).copied().unwrap_or(0),
                    attached: l.pty.output.is_attached(),
                    last_activity_ms: last,
                })
            })
            .collect();
        let idle_ms = self.cfg.idle_suspend.map(|d| d.as_millis() as u64);
        for (id, why) in resources::to_suspend(&live, self.live_cap(), idle_ms, now) {
            self.event(&id, "auto_suspend", json!({"why": why, "pressure": level}));
            if let Some(me) = self.me.upgrade() {
                // Suspending waits for the exit: never on the caller's loop.
                std::thread::spawn(move || {
                    let _ = me.suspend(&id);
                });
            }
        }
        let (do_index, do_purge) = {
            let mut h = self.house();
            let i = (!h.index_dirty.is_empty() || h.index_all) && now.saturating_sub(h.last_index_ms) >= INDEX_EVERY_MS;
            let p = now.saturating_sub(h.last_purge_ms) >= PURGE_EVERY_MS;
            if i {
                h.last_index_ms = now;
            }
            if p {
                h.last_purge_ms = now;
            }
            (i, p)
        };
        if do_index {
            let (all, dirty) = {
                let mut h = self.house();
                (std::mem::take(&mut h.index_all), std::mem::take(&mut h.index_dirty))
            };
            let files = self.transcript_files(if all { None } else { Some(&dirty) });
            match self.db.index_transcripts(&files) {
                Ok(st) if st.entries > 0 => self.event("-", "transcripts_indexed", json!(st)),
                Ok(_) => {}
                Err(e) => self.event("-", "index_error", json!(e.to_string())),
            }
        }
        if do_purge {
            match self.db.purge(now, self.cfg.retention) {
                Ok(n) if n > 0 => self.event("-", "purged", json!({"rows": n})),
                _ => {}
            }
        }
    }

    /// Transcripts to index: every known agent's session (found by id across
    /// `~/.claude/projects/*/`), plus, for a full pass, every session in a
    /// registered project's folders (its checkout and its worktrees).
    pub fn transcript_files(&self, only: Option<&std::collections::BTreeSet<String>>) -> Vec<orch_store::TranscriptFile> {
        let projects = self.db.projects(false).unwrap_or_default();
        let project_of = |repo: &Path| projects.iter().find(|p| p.path == repo).map(|p| p.id.clone());
        let mut files: Vec<orch_store::TranscriptFile> = vec![];
        for a in self.store.read(|s| s.agents.values().cloned().collect::<Vec<_>>()) {
            if only.is_some_and(|o| !o.contains(&a.id)) {
                continue;
            }
            if let Some(path) = crate::transcript::find(&self.cfg.projects_root, &a.session_id) {
                files.push(orch_store::TranscriptFile { path, session_id: a.session_id.clone(), project_id: project_of(&a.repo) });
            }
        }
        if only.is_none() {
            for p in &projects {
                let prefixes = [orch_store::encode_project_dir(&p.path), orch_store::encode_project_dir(&p.worktree_root)];
                let Ok(dirs) = std::fs::read_dir(&self.cfg.projects_root) else { break };
                for d in dirs.flatten() {
                    let name = d.file_name().to_string_lossy().into_owned();
                    let mine = prefixes.iter().any(|pre| name == *pre || name.starts_with(&format!("{pre}-")));
                    if !mine {
                        continue;
                    }
                    for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                        let path = f.path();
                        if path.extension().is_some_and(|e| e == "jsonl") && !files.iter().any(|x| x.path == path) {
                            let sid = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            files.push(orch_store::TranscriptFile { path, session_id: sid, project_id: Some(p.id.clone()) });
                        }
                    }
                }
            }
        }
        files
    }

    /// Delivery loop (call every ~200 ms): trust-dialog detection, then for
    /// each agent with queued work, admission and FR-206 delivery.
    pub fn pump(&self) {
        let now = now_ms();
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (id, l) in live {
            let seen = l.pty.output.trust_dialog.load(Ordering::Relaxed);
            let changed = self.store.read(|s| s.agents.get(&id).is_some_and(|a| a.trust_dialog != seen));
            if changed {
                let _ = self.store.write(|s| {
                    if let Some(a) = s.agents.get_mut(&id) {
                        a.trust_dialog = seen;
                    }
                    if seen {
                        s.add_inbox(Some(&id), "alert", format!("{id}: Claude Code asks to trust this folder. Attach and answer it yourself (the app never does)."));
                    }
                });
            }
        }
        let queued: Vec<(AgentRecord, TurnRow)> = self.store.read(|s| {
            let mut out: BTreeMap<String, (AgentRecord, TurnRow)> = BTreeMap::new();
            for t in s.turns.iter().filter(|t| t.status == TurnStatus::Queued) {
                if out.contains_key(&t.agent_id) {
                    continue;
                }
                if let Some(a) = s.agents.get(&t.agent_id) {
                    out.insert(t.agent_id.clone(), (a.clone(), t.clone()));
                }
            }
            out.into_values().collect()
        });
        for (rec, turn) in queued {
            if let Err(e) = self.pump_one(&rec, &turn, now) {
                self.event(&rec.id, "delivery_error", json!(e.to_string()));
            }
        }
    }

    fn admit_turn(&self, rec: &AgentRecord, turn: &TurnRow) -> bool {
        match turn.admission {
            Admission::Admitted => return true,
            Admission::Waiting(_) => return false, // in the Governor's queue until a release
            Admission::Pending => {}
        }
        let d = self.decide_turn(rec, turn.kind, TurnOrigin::App);
        let admission = match &d.effective {
            Verdict::Admit => Admission::Admitted,
            Verdict::Queue { entry: Some(e), .. } => Admission::Waiting(e.0),
            _ => Admission::Pending,
        };
        if admission != Admission::Pending {
            let _ = self.store.write(|s| {
                if let Some(t) = s.turns.iter_mut().find(|t| t.id == turn.id) {
                    t.admission = admission;
                }
            });
        }
        admission == Admission::Admitted
    }

    fn pump_one(&self, rec: &AgentRecord, turn: &TurnRow, now: u64) -> Result<()> {
        let Some(l) = self.live_get(&rec.id) else {
            // A suspended agent with work is resumed through the Governor first
            // (FR-403), and only when a live slot is free (FR-203).
            if rec.tracker.state == AgentState::Suspended && rec.flag.is_none() {
                let (live, cap) = (self.live_ids().len() as u32, self.live_cap());
                if live >= cap {
                    if self.house().cap_waiting.insert(rec.id.clone()) {
                        self.event(&rec.id, "cap_wait", json!({"live": live, "cap": cap}));
                    }
                    return Ok(());
                }
                self.house().cap_waiting.remove(&rec.id);
            }
            if rec.tracker.state == AgentState::Suspended && rec.flag.is_none() && self.admit_turn(rec, turn) {
                if let Err(e) = self.spawn(&rec.id, None) {
                    // Flag it so the next pump doesn't retry every 200 ms.
                    let msg = format!("{}: could not resume: {e:#}", rec.id);
                    self.store.write(|s| {
                        if let Some(a) = s.agents.get_mut(&rec.id) {
                            a.flag = Some(msg.clone());
                        }
                        s.add_inbox(Some(&rec.id), "alert", msg);
                    })?;
                    return Err(e);
                }
            }
            return Ok(());
        };
        let gate = GateInputs {
            live: !l.pty.has_exited(),
            idle: rec.tracker.state == AgentState::Idle,
            trust_dialog: rec.trust_dialog,
            background_tasks: rec.tracker.background_tasks,
            ready_at_ms: Some(l.ready_at_ms.load(Ordering::SeqCst)).filter(|t| *t > 0),
            last_user_input_ms: Some(l.last_user_input_ms.load(Ordering::SeqCst)).filter(|t| *t > 0),
            now_ms: now,
        };
        if delivery::can_deliver(&gate).is_err() || !self.admit_turn(rec, turn) {
            return Ok(());
        }
        let prepared = delivery::prepare(&turn.body, &turn.id, &self.cfg.paths.agent_dir(&rec.id));
        if let Some((path, body)) = &prepared.file {
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))?;
            std::fs::write(path, body)?;
        }
        l.ready_at_ms.store(0, Ordering::SeqCst);
        self.store.write(|s| {
            if let Some(t) = s.turns.iter_mut().find(|t| t.id == turn.id) {
                t.status = TurnStatus::Delivering;
                t.delivered_at_ms = Some(now);
            }
        })?;
        l.pty.write(&prepared.paste_bytes())?;
        std::thread::sleep(delivery::PASTE_TO_ENTER);
        l.pty.write(b"\r")?;
        self.event(&rec.id, "delivered", json!({"turn": turn.id, "chars": prepared.text.chars().count(), "file": prepared.file.as_ref().map(|f| f.0.clone())}));
        Ok(())
    }

    // ------------------------------------------------------------ terminal

    /// Bridges a client connection to the agent's PTY until either side
    /// closes. Keystrokes count as the user's typing (delivery waits 2 s).
    /// `initial` is input the client sent along with its request.
    pub fn attach(&self, id: &str, stream: UnixStream, initial: &[u8], rows: u16, cols: u16) -> Result<()> {
        let l = self.live_get(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        let _ = l.pty.resize(rows, cols);
        l.pty.output.set_attached(true);
        if !initial.is_empty() {
            l.last_user_input_ms.store(now_ms(), Ordering::SeqCst);
            l.pty.write(initial)?;
        }
        let mut input = stream.try_clone()?;
        let l2 = l.clone();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match input.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        l2.last_user_input_ms.store(now_ms(), Ordering::SeqCst);
                        if l2.pty.write(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut out = stream;
        // Redraw at the new size.
        let _ = l.pty.resize(rows, cols.saturating_sub(1).max(1));
        let _ = l.pty.resize(rows, cols);
        loop {
            let bytes = l.pty.output.read_wait(Duration::from_millis(250));
            if bytes.is_empty() {
                if l.pty.has_exited() && l.pty.output.is_closed() || reader.is_finished() {
                    break;
                }
                continue;
            }
            if out.write_all(&bytes).is_err() {
                break;
            }
        }
        l.pty.output.set_attached(false);
        let _ = out.shutdown(std::net::Shutdown::Both);
        let _ = reader.join();
        Ok(())
    }

    pub fn resize(&self, id: &str, rows: u16, cols: u16) -> Result<()> {
        self.live_get(id).ok_or_else(|| anyhow!("agent {id} is not running"))?.pty.resize(rows, cols)
    }

    /// Pull transport for a UI: everything buffered since the last read.
    pub fn read_output(&self, id: &str) -> Result<Vec<u8>> {
        Ok(self.live_get(id).ok_or_else(|| anyhow!("agent {id} is not running"))?.pty.read())
    }

    // ------------------------------------------------------------ views

    pub fn state_json(&self) -> Value {
        let s = self.store.read(|s| s.clone());
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner()).view();
        let mut queued: BTreeMap<&str, usize> = BTreeMap::new();
        for t in s.turns.iter().filter(|t| t.status == TurnStatus::Queued) {
            *queued.entry(t.agent_id.as_str()).or_default() += 1;
        }
        // OTLP tokens and cost this plan week, per agent.
        let week_start = self.gov().plan().week_start.as_millisecond().max(0) as u64;
        let tokens: BTreeMap<String, orch_store::UsageTotal> = self
            .db
            .usage_totals(week_start)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| Some((t.agent_id.clone()?, t)))
            .collect();
        let agents: Vec<Value> = s
            .agents
            .values()
            .map(|a| {
                let live = self.live_get(&a.id);
                let u = usage.agents.get(&a.id);
                json!({
                    "id": a.id,
                    "state": a.tracker.state,
                    "label": a.tracker.state.label(),
                    "pid": a.pid,
                    "live": live.is_some(),
                    "memory_mb": live.as_ref().and_then(|l| procs::footprint_mb(l.pty.pid)),
                    "session_id": a.session_id,
                    "worktree": a.worktree,
                    "model": u.and_then(|u| u.model.clone()).or(a.role.model.clone()),
                    "context_pct": u.and_then(|u| u.context_used_pct),
                    "cost_usd": u.map(|u| u.total_cost_usd),
                    "prompt_cache_expires_at": u.and_then(|u| u.prompt_cache_expires_at),
                    "current_tool": a.tracker.current_tool,
                    "background_tasks": a.tracker.background_tasks,
                    "turns": a.tracker.turns,
                    "queued_turns": queued.get(a.id.as_str()).copied().unwrap_or(0),
                    "queued_messages": a.tracker.queued_messages,
                    "last_event": a.last_event,
                    "last_event_at_ms": a.last_event_at_ms,
                    "last_error": a.tracker.last_error,
                    "trust_dialog": a.trust_dialog,
                    "flag": a.flag,
                    "otel_week": tokens.get(&a.id),
                })
            })
            .collect();
        json!({
            "agents": agents,
            "inbox": s.inbox.iter().filter(|i| i.pending()).collect::<Vec<_>>(),
            "turns": s.turns.iter().rev().take(20).collect::<Vec<_>>(),
            "warnings": self.warnings,
            "live_cap": {"base": self.cfg.base_cap, "effective": self.live_cap(), "pressure": self.house().pressure},
            "otlp": self.otlp_endpoint(),
        })
    }

    pub fn usage_json(&self) -> Value {
        let view = self.usage.lock().unwrap_or_else(|e| e.into_inner()).view();
        let snap = self.gov().snapshot(Timestamp::now());
        json!({
            "five_hour": view.five_hour,
            "weekly": view.weekly,
            "governor": {"used": snap.used, "provenance": snap.provenance, "plan": snap.plan, "ideal": snap.ideal, "governed": snap.governed, "error": snap.error, "state": snap.state, "five_hour": snap.five_hour},
            "agents": view.agents,
        })
    }

    pub fn governor_json(&self) -> Value {
        let g = self.gov();
        let now = Timestamp::now();
        let actions: Vec<_> = g.actions().iter().rev().take(20).collect();
        json!({
            "shadow": g.plan().shadow_mode,
            "plan": {"id": g.plan().id, "week_start": g.plan().week_start, "reset_at": g.plan().reset_at},
            "snapshot": g.snapshot(now),
            "queue": g.queue(),
            "actions": actions,
            "next": g.next_action_preview(now),
            "decisions": self.decisions.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().take(20).collect::<Vec<_>>(),
        })
    }
}

/// The Governor at start: the active plan when its week is still running
/// (else the default week), the stored policy (live cap from the config), and
/// this week's statusline samples replayed with their cost class (NFR-2).
fn restore_governor(db: &orch_store::Store, store: &Store, cfg: &RuntimeConfig, now: Timestamp) -> Result<Governor> {
    let mut policy = db.policy(POLICY_ID)?.unwrap_or_default();
    policy.live_caps = governor::LiveCaps::from_base(cfg.base_cap);
    db.save_policy(POLICY_ID, &policy)?;
    let mut plan = match db.active_plan()? {
        Some(p) if p.reset_at > now && p.week_start <= now => p,
        _ => default_plan(now, cfg.shadow),
    };
    plan.shadow_mode = cfg.shadow;
    db.save_plan(&plan, orch_store::PlanStatus::Active)?;
    let start = plan.week_start;
    let mut gov = Governor::new(plan, policy, start.min(now)).map_err(|e| anyhow!("{e:?}"))?;
    let allocs: BTreeMap<String, String> =
        store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.allocation.clone())).collect());
    let since = start.as_millisecond().max(0) as u64;
    for u in db.usage_since(since, Some(orch_store::UsageSource::Statusline))? {
        let Some(sample) = governor_sample(&u) else { continue };
        let class = match u.class.as_deref() {
            Some("autonomous") => governor::CostClass::Autonomous,
            Some("critical") => governor::CostClass::Critical,
            _ => governor::CostClass::Interactive,
        };
        let alloc = u.allocation.clone().or_else(|| u.agent_id.as_ref().and_then(|a| allocs.get(a).cloned()));
        gov.replay(&sample, class, alloc.map(Into::into));
    }
    let _ = gov.tick(now);
    Ok(gov)
}

fn reading(pct: Option<f64>, resets: Option<i64>) -> Option<governor::WindowReading> {
    Some(governor::WindowReading { used_pct: pct?, resets_at: Timestamp::from_second(resets?).ok()? })
}

/// A stored statusline row back into the Governor's sample.
fn governor_sample(u: &orch_store::UsageRecord) -> Option<governor::UsageSample> {
    Some(governor::UsageSample {
        ts: Timestamp::from_millisecond(u.ts_ms as i64).ok()?,
        agent: u.agent_id.clone().map(AgentId),
        cost_delta: u.cost_delta.unwrap_or(0.0),
        five_hour: reading(u.five_hour_pct, u.five_hour_resets_at),
        weekly: reading(u.weekly_pct, u.weekly_resets_at),
        provenance: if u.provenance == "estimated" { governor::Provenance::Estimated } else { governor::Provenance::Observed },
    })
}

fn usage_record(
    g: &governor::UsageSample,
    s: &StatusSample,
    class: governor::CostClass,
    allocation: Option<String>,
) -> orch_store::UsageRecord {
    let tag = |v: &dyn erased::Tag| v.tag();
    orch_store::UsageRecord {
        ts_ms: g.ts.as_millisecond().max(0) as u64,
        agent_id: g.agent.as_ref().map(|a| a.0.clone()),
        model: s.model.clone(),
        cost_estimate: s.total_cost_usd,
        cost_delta: Some(g.cost_delta),
        five_hour_pct: g.five_hour.map(|r| r.used_pct),
        five_hour_resets_at: g.five_hour.map(|r| r.resets_at.as_second()),
        weekly_pct: g.weekly.map(|r| r.used_pct),
        weekly_resets_at: g.weekly.map(|r| r.resets_at.as_second()),
        provenance: tag(&g.provenance),
        source: Some(orch_store::UsageSource::Statusline),
        class: Some(tag(&class)),
        allocation,
        ..Default::default()
    }
}

mod erased {
    /// A unit enum's serde name.
    pub trait Tag {
        fn tag(&self) -> String;
    }

    impl<T: serde::Serialize> Tag for T {
        fn tag(&self) -> String {
            match serde_json::to_value(self) {
                Ok(serde_json::Value::String(s)) => s,
                _ => String::new(),
            }
        }
    }
}

/// True when the peer closed the connection (EOF). Waits up to 250 ms.
pub(crate) fn conn_closed(c: &UnixStream) -> bool {
    let _ = c.set_read_timeout(Some(Duration::from_millis(250)));
    let mut b = [0u8; 1];
    match (&*c).read(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => !matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A runtime for unit tests: short holds, no idle timer, normal memory.
    pub(crate) fn test_cfg(d: &Path, shadow: bool) -> RuntimeConfig {
        let mut cfg = RuntimeConfig::new(AppPaths::new(d.join("app")));
        cfg.shadow = shadow;
        cfg.hold = Duration::from_millis(800);
        cfg.permission_hold = Duration::from_millis(300);
        cfg.idle_suspend = None;
        cfg.memory = Arc::new(crate::resources::FixedMemory::new(PressureLevel::Normal));
        cfg.projects_root = d.join("projects");
        cfg
    }

    pub(crate) fn test_runtime(shadow: bool) -> (tempfile::TempDir, Arc<Runtime>) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let rt = Runtime::new(test_cfg(d.path(), shadow)).unwrap();
        rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
        (d, rt)
    }

    pub(crate) fn test_agent(dir: &Path) -> AgentRecord {
        AgentRecord {
            id: "a1".into(),
            repo: dir.into(),
            worktree: dir.into(),
            role: Role::solo(),
            session_id: "s".into(),
            tracker: Default::default(),
            pid: None,
            pgid: None,
            allocation: DEFAULT_ALLOCATION.into(),
            tier: Tier::Normal,
            created_at_ms: 0,
            last_event: None,
            last_event_at_ms: None,
            spawns: 1,
            flag: None,
            trust_dialog: false,
        }
    }

    pub(crate) fn hook(event: &str, extra: Value) -> HookMsg {
        let mut payload = json!({"hook_event_name": event, "session_id": "s", "prompt_id": "p1"});
        payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        HookMsg {
            agent_id: "a1".into(),
            event: HookEvent::from(event.to_string()),
            ts_ms: now_ms(),
            deadline_ms: 540_000,
            payload,
        }
    }

    #[test]
    fn governor_hold_denies_autonomous_tools_and_never_blocks_the_user() {
        let (_d, rt) = test_runtime(false);
        rt.pause_all();
        // An app turn (marker) is autonomous: held, then denied "paused by governor".
        assert!(rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None).is_none());
        let t0 = std::time::Instant::now();
        let out = rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), None).expect("denied");
        assert!(t0.elapsed() >= Duration::from_millis(700), "held first");
        let o = &out.0["hookSpecificOutput"];
        assert_eq!(o["permissionDecision"], "deny");
        assert!(o["permissionDecisionReason"].as_str().unwrap().starts_with("ORCH: paused by governor"));
        let holds = rt.store.read(|s| s.holds.clone());
        assert_eq!(holds.last().unwrap().status, HoldStatus::Denied);
        // Esc in the pane kills orch-hook: EOF ends the hold at once, as interrupted.
        let (core_end, hook_end) = UnixStream::pair().unwrap();
        drop(hook_end);
        let t0 = std::time::Instant::now();
        assert!(rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), Some(&core_end)).is_none());
        assert!(t0.elapsed() < Duration::from_millis(600));
        assert_eq!(rt.store.read(|s| s.holds.last().unwrap().status), HoldStatus::Interrupted);
        // The user's own turn is never blocked, even under Pause all.
        rt.on_hook(&hook("Stop", json!({})), None);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt_id": "p2", "prompt": "typed by me"})), None);
        assert!(rt.on_hook(&hook("PreToolUse", json!({"prompt_id": "p2", "tool_name": "Bash"})), None).is_none());
        // Permission prompts are left to the pane (no decision).
        assert!(rt.on_hook(&hook("PermissionRequest", json!({"prompt_id": "p2"})), None).is_none());
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::WaitingForYou);
    }

    #[test]
    fn unknown_agent_fails_closed_for_tools_only() {
        let (_d, rt) = test_runtime(true);
        let mut m = hook("PreToolUse", json!({"tool_name": "Bash"}));
        m.agent_id = "ghost".into();
        let out = rt.on_hook(&m, None).expect("denied");
        assert_eq!(out.0["hookSpecificOutput"]["permissionDecision"], "deny");
        let mut m = hook("Stop", json!({}));
        m.agent_id = "ghost".into();
        assert!(rt.on_hook(&m, None).is_none());
    }

    #[test]
    fn shadow_mode_admits_but_logs_the_proposal() {
        let (_d, rt) = test_runtime(true);
        rt.pause_all();
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None);
        assert!(rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), None).is_none());
        let d = rt.decisions.lock().unwrap().back().cloned().unwrap();
        assert_eq!(d.verdict, AdmissionVerdict::Admit);
        assert_eq!(d.proposed, AdmissionVerdict::Deny);
    }

    #[test]
    fn statusline_and_rate_limit_reach_the_governor() {
        let (_d, rt) = test_runtime(false);
        rt.on_status(&StatusMsg {
            agent_id: "a1".into(),
            ts_ms: now_ms(),
            payload: json!({"session_id": "s", "cost": {"total_cost_usd": 0.01},
                "rate_limits": {"five_hour": {"used_percentage": 100, "resets_at": (now_ms() / 1000) as i64 + 3600}}}),
        });
        assert_eq!(rt.usage_json()["five_hour"]["used_pct"], 100.0);
        // A weekly reading moves the plan week onto the real reset.
        let reset = (now_ms() / 1000) as i64 + 2 * 24 * 3600 + 1234;
        rt.on_status(&StatusMsg {
            agent_id: "a1".into(),
            ts_ms: now_ms(),
            payload: json!({"session_id": "s", "cost": {"total_cost_usd": 0.02},
                "rate_limits": {"seven_day": {"used_percentage": 40, "resets_at": reset}}}),
        });
        assert_eq!(rt.gov().plan().reset_at.as_second(), reset);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None);
        rt.on_hook(
            &hook(
                "StopFailure",
                json!({"error": "rate_limit", "last_assistant_message": "You've hit your session limit"}),
            ),
            None,
        );
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::BlockedByLimit);
        let g = rt.governor_json();
        assert!(g["actions"].to_string().contains("limit"), "{g}");
    }

    #[test]
    fn restart_rebuilds_the_governor_from_stored_samples() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let reset = (now_ms() / 1000) as i64 + 3 * 24 * 3600;
        let (used, governed, plan_id) = {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
            // An app turn (admitted by the Governor before delivery: autonomous), two responses.
            let rec = rt.store.read(|s| s.agents["a1"].clone());
            assert!(rt.decide_turn(&rec, TurnKind::Message, TurnOrigin::App).effective.is_admit());
            rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None);
            for (cost, pct) in [(0.5, 30), (1.5, 32)] {
                rt.on_status(&StatusMsg {
                    agent_id: "a1".into(),
                    ts_ms: now_ms(),
                    payload: json!({"session_id": "s", "cost": {"total_cost_usd": cost},
                        "rate_limits": {"seven_day": {"used_percentage": pct, "resets_at": reset}}}),
                });
            }
            rt.pause_all(); // one Governor action to persist
            rt.tick();
            rt.db.flush().unwrap();
            assert!(!rt.db.actions(10).unwrap().is_empty(), "actions are persisted on tick");
            let snap = rt.gov().snapshot(Timestamp::now());
            (snap.used, snap.governed, rt.gov().plan().id.clone())
        };
        let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
        let snap = rt.gov().snapshot(Timestamp::now());
        assert_eq!(rt.gov().plan().id, plan_id, "the realigned plan week is restored");
        assert_eq!(rt.gov().plan().reset_at.as_second(), reset);
        assert!((snap.used - used).abs() < 0.01, "{} vs {used}", snap.used);
        assert!((snap.governed - governed).abs() < 0.01, "{} vs {governed}", snap.governed);
        let samples = rt.db.usage_since(0, Some(orch_store::UsageSource::Statusline)).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].class.as_deref(), Some("autonomous"));
    }

    #[test]
    fn otlp_requests_become_usage_samples() {
        let (_d, rt) = test_runtime(true);
        let ep = rt.start_otlp(0).unwrap();
        assert_eq!(rt.otlp_endpoint().as_deref(), Some(ep.as_str()));
        rt.on_api_requests(vec![
            crate::otlp::ApiRequest { ts_ms: 5, agent_id: Some("a1".into()), input_tokens: 10, output_tokens: 2, cost_usd: 0.01, ..Default::default() },
            crate::otlp::ApiRequest { ts_ms: 6, agent_id: Some("ghost".into()), input_tokens: 1, ..Default::default() },
        ]);
        rt.db.flush().unwrap();
        let t = rt.db.usage_totals(0).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].agent_id.as_deref(), t[0].tokens_in), (None, 1), "unknown agent: unattributed");
        assert_eq!((t[1].agent_id.as_deref(), t[1].tokens_in, t[1].tokens_out, t[1].cost), (Some("a1"), 10, 2, 0.01));
        rt.shutdown();
        assert!(rt.otlp_endpoint().is_none());
    }

    #[test]
    fn plan_week_is_monday_to_monday() {
        let now: Timestamp = "2026-10-21T15:00:00Z".parse().unwrap();
        let p = default_plan(now, true);
        assert!(p.week_start <= now && now < p.reset_at);
        assert_eq!(p.reset_at.as_second() - p.week_start.as_second(), 7 * 24 * 3600);
        assert!(p.shadow_mode);
    }

    #[test]
    fn spec_to_role() {
        let spec = AgentSpec {
            agent_id: "a".into(),
            repo: "/r".into(),
            model: Some("haiku".into()),
            permission_mode: Some("acceptEdits".into()),
            strict_mcp: Some(true),
            network_allow: vec!["example.org".into()],
            ..Default::default()
        };
        let r = role_from_spec(&spec).unwrap();
        assert_eq!(r.permission_mode, PermissionMode::AcceptEdits);
        assert!(r.strict_mcp && r.network_allow.contains(&"example.org".to_string()));
        let bad = AgentSpec { permission_mode: Some("bypassPermissions".into()), ..spec };
        assert!(role_from_spec(&bad).is_err());
    }
}
