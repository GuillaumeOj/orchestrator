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
    AgentId, BudgetAllocation, Decision, Governor, Release, Tier, ToolCallRequest, TurnKind, TurnRequest, Verdict,
    WeeklyPlan, WorkingCalendar,
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
use crate::procs::{self, AgentMatch, SystemProcs};
use crate::pty::PtyProcess;
use crate::reconcile::{self, ReconcileEnv, Report};
use crate::resources::{self, LiveAgent, MemoryProbe, PressureLevel, SystemMemory};
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
    /// Replaces `gh` for the GitHub task backend (tests).
    pub gh: Option<crate::team::GhFactory>,
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
            gh: None,
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
    /// Core start time: Governor action ids restart at each boot (and at each
    /// `replan`, which takes a new key so stored actions are not overwritten).
    boot_ms: AtomicU64,
    house: Mutex<House>,
    /// The local OTLP receiver once started ([`Runtime::start_otlp`]).
    otlp: Mutex<Option<crate::otlp::OtlpReceiver>>,
    /// A transcript index pass is running.
    indexing: std::sync::atomic::AtomicBool,
    /// A team pass ([`Runtime::team_tick`]) is running.
    teaming: std::sync::atomic::AtomicBool,
    /// Each project's task backend, kept across calls.
    pub(crate) task_backends: orch_tasks::Backends,
    /// The starter roles and presets were checked this run.
    pub(crate) presets_checked: std::sync::atomic::AtomicBool,
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
    /// What `tick` last saved of the actions and the queue.
    last_saved: (Vec<governor::GovernorAction>, Vec<governor::QueueEntry>),
    /// What was last stored of the Governor's carry-over state.
    last_carry: Option<governor::Carry>,
    /// Agents the user ran now (FR-809): their next spawn may exceed the memory cap (FR-203).
    spawn_override: std::collections::BTreeSet<String>,
    /// The plan week (start, ms) whose reserve warning reached the Inbox, once read from the store.
    reserve_alerted: Option<Option<i64>>,
    /// Last sync of the remote task backends (FR-503).
    last_task_sync_ms: u64,
    /// Watchdog alerts already raised, by `<agent>:<turn start>` (FR-407).
    watchdog: std::collections::BTreeSet<String>,
    /// Team messages given to a busy agent during its running turn: read
    /// when it ends, reminded at its Stop first.
    injected: BTreeMap<String, Vec<u64>>,
    /// Agents that asked the user (Inbox question) during their running turn.
    asked: std::collections::BTreeSet<String>,
    /// Stuck team members already told to their lead, by run.
    supervised: BTreeMap<String, std::collections::BTreeSet<String>>,
}

/// A team lead (compiled at the critical tier): outside the process cap.
pub(crate) fn reserved_slot(a: &AgentRecord) -> bool {
    a.team.is_some() && a.tier == Tier::Critical
}

/// True (and restamps `last`) when `every` ms have passed since `last`.
fn due(last: &mut u64, now: u64, every: u64) -> bool {
    let d = now.saturating_sub(*last) >= every;
    if d {
        *last = now;
    }
    d
}

pub const FORECAST_EVERY_MS: u64 = 5 * 60 * 1000;
pub const CALIBRATION_EVERY_MS: u64 = 60 * 60 * 1000;
pub const PURGE_EVERY_MS: u64 = 24 * 3600 * 1000;
pub const INDEX_EVERY_MS: u64 = 60 * 1000;
/// FR-503: how often remote task backends (GitHub) are synced.
pub const TASK_SYNC_EVERY_MS: u64 = 5 * 60 * 1000;
pub const POLICY_ID: &str = "default";
/// Settings key of the Governor's carry-over state (ADR-007 "State across restarts").
pub const CARRY_KEY: &str = "governor.carry";
/// Sprint mode's default extra pp for today (FR-811).
pub const DEFAULT_SPRINT_PP: f64 = 5.0;
/// Settings key: the user's FR-812 choice, `true` = enforce. Every way of
/// switching (the app, `orchd enforce`) stores it here, and every start reads it.
pub const ENFORCE_KEY: &str = "governor.enforce";
/// Where the app stored the choice before the runtime did.
const LEGACY_ENFORCE_KEY: &str = "ui.governor.enforce";

/// FR-812: enforcing was chosen (by the app or `orchd enforce on`).
pub fn enforce_chosen(db: &orch_store::Store) -> Option<bool> {
    [ENFORCE_KEY, LEGACY_ENFORCE_KEY].iter().find_map(|k| db.setting(k).ok().flatten()?.as_bool())
}
/// Settings key: the plan week (start, ms) whose reserve warning reached the Inbox.
pub const RESERVE_ALERT_KEY: &str = "governor.reserve_alerted";

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
    pub fn new(mut cfg: RuntimeConfig) -> Result<Arc<Runtime>> {
        std::fs::create_dir_all(cfg.paths.agents_dir())?;
        std::fs::create_dir_all(cfg.paths.state_dir())?;
        orch_protocol::check_socket_path(&cfg.paths.socket())?;
        let db = orch_store::Store::open(&cfg.paths.db_file())?;
        let store = Store::open(db.clone(), Some(&cfg.paths.legacy_state_file()))?;
        let now = Timestamp::now();
        // `cfg.shadow = false` forces enforcement (`serve --enforce`, ORCH_ENFORCE);
        // otherwise the user's stored choice decides (FR-812).
        cfg.shadow = cfg.shadow && enforce_chosen(&db) != Some(true);
        let (gov, released) = restore_governor(&db, &store, &cfg, cfg.shadow, None, now)?;
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
        let rt = Arc::new_cyclic(|me| Runtime {
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
            boot_ms: AtomicU64::new(now_ms()),
            // The first purge waits a day like the others (not during the boot rush).
            house: Mutex::new(House { index_all: true, last_purge_ms: now_ms(), ..Default::default() }),
            otlp: Mutex::new(None),
            indexing: std::sync::atomic::AtomicBool::new(false),
            teaming: std::sync::atomic::AtomicBool::new(false),
            task_backends: orch_tasks::Backends::default(),
            presets_checked: std::sync::atomic::AtomicBool::new(false),
        });
        rt.handle_releases(released);
        Ok(rt)
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
        self.save_carry();
        r
    }

    /// Stores the Governor's carry-over state when it changed, so a restart
    /// or a replan keeps pauses, deferrals, the ask, sprints and the queue.
    fn save_carry(&self) {
        // Running turns are not stored: a restart has no use for them (their
        // processes end with the core), and their cost changes with every sample.
        let carry = governor::Carry { live: Vec::new(), ..self.gov().carry() };
        let mut h = self.house();
        if h.last_carry.as_ref() != Some(&carry) {
            if let Ok(v) = serde_json::to_value(&carry) {
                if self.db.set_setting(CARRY_KEY, &v).is_ok() {
                    h.last_carry = Some(carry);
                }
            }
        }
    }

    /// FR-807: the reserve for interactive work is used up. A budget item in
    /// the Inbox (level 3), once per plan week, also in shadow mode (a warning).
    fn reserve_alert(&self) {
        let (week, reason) = {
            let g = self.gov();
            let start = g.plan().week_start;
            let week = start.as_millisecond();
            // This week's only: the log keeps earlier weeks' actions.
            let a = g
                .actions()
                .iter()
                .rev()
                .take_while(|a| a.ts >= start)
                .find(|a| matches!(a.action, governor::ActionKind::ReserveExhausted));
            (week, a.map(|a| a.reason.clone()))
        };
        let Some(reason) = reason else { return };
        // The week already alerted: kept in memory, read from the store once.
        let alerted = *self
            .house()
            .reserve_alerted
            .get_or_insert_with(|| self.db.setting(RESERVE_ALERT_KEY).ok().flatten().and_then(|v| v.as_i64()));
        if alerted == Some(week) {
            return;
        }
        let body = format!(
            "The reserve for your own work is used up\n{reason}. Interactive work is never blocked; agents are squeezed harder until the reset."
        );
        if self.store.write(|s| s.add_inbox(None, "budget", body)).is_ok() {
            let _ = self.db.set_setting(RESERVE_ALERT_KEY, &json!(week));
            self.house().reserve_alerted = Some(Some(week));
        }
    }

    /// Tells the Governor how many `claude` processes run and the memory cap
    /// on them (FR-203); spawns it held wait for a free slot.
    fn sync_process_slots(&self) {
        let releases = self.gov().set_process_slots(self.live_count(), self.live_cap(), Timestamp::now());
        self.handle_releases(releases);
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

    pub(crate) fn mark_asked(&self, id: &str) {
        self.house().asked.insert(id.to_string());
    }

    pub(crate) fn asked(&self, id: &str) -> bool {
        self.house().asked.contains(id)
    }

    pub(crate) fn note_injected(&self, id: &str, ids: &[u64]) {
        self.house().injected.entry(id.to_string()).or_default().extend(ids);
    }

    /// Drops an agent's queued turns; the team messages they carried are
    /// delivered again later.
    pub(crate) fn drop_queued_turns(&self, id: &str) {
        let msgs = self.store.write(|s| {
            let mut msgs: Vec<u64> = vec![];
            s.turns.retain(|t| {
                let drop = t.agent_id == id && t.status == TurnStatus::Queued;
                if drop {
                    msgs.extend(&t.messages);
                }
                !drop
            });
            msgs
        });
        if let Ok(msgs) = msgs
            && !msgs.is_empty()
        {
            let _ = self.db.undeliver_messages(&msgs);
        }
    }

    /// A Stop that should not end the turn yet: team messages that came in
    /// during it (as tool-call context) are listed once more, so a report
    /// that arrived during the user's turn is not lost in it. Once per turn:
    /// the CLI sets `stop_hook_active` on the Stop that follows a block.
    fn stop_reminder(&self, rec: &AgentRecord, p: &HookPayload) -> Option<HookOutput> {
        if rec.team.is_none() || p.stop_hook_active == Some(true) {
            return None;
        }
        let ids = self.house().injected.get(&rec.id).cloned().unwrap_or_default();
        let msgs = self.unread_of(rec, &ids);
        if msgs.is_empty() {
            return None;
        }
        let list: String = msgs
            .iter()
            .map(|m| format!("\n- {} from {}: {}", m.kind.replace('_', " "), m.from_agent.as_deref().unwrap_or(&m.from_node), m.body))
            .collect();
        Some(HookOutput::stop_block(&format!(
            "ORCH: before you stop: {} team message(s) reached you during this turn:{list}\n\nAct on each one now (review and merge a report, answer with mcp__orch__send_message, update the task). If you already have, end your turn.",
            msgs.len()
        )))
    }

    /// Live processes under the memory cap (FR-203): a team lead has a slot
    /// of its own, so it never waits for one and never takes one.
    fn live_count(&self) -> u32 {
        let ids = self.live_ids();
        self.store.read(|s| ids.iter().filter(|id| !s.agents.get(*id).is_some_and(reserved_slot)).count()) as u32
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
    fn decide_turn(&self, rec: &AgentRecord, kind: TurnKind, origin: TurnOrigin, needs_spawn: bool) -> Decision {
        let req = TurnRequest {
            agent: AgentId(rec.id.clone()),
            allocation: rec.allocation.clone().into(),
            tier: rec.tier,
            interactive: origin == TurnOrigin::User,
            kind,
            self_started: origin == TurnOrigin::SelfStarted,
            ceiling: None,
            needs_spawn,
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
            team: None,
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
        if rec.tracker.state == AgentState::Archived {
            bail!("agent {id} is deleted");
        }
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
        // A Resume or a restart outside a release: a turn queued for a process
        // slot would otherwise wait for the slot this agent now holds.
        let agent = AgentId(id.into());
        self.governor_act(|g, now| ((), g.spawned(&agent, now)));
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
        let (log, left) = match intent {
            StopIntent::Suspend => (
                procs::kill_tree(
                    &pgid.into_iter().collect::<Vec<_>>(),
                    &pid.into_iter().collect::<Vec<_>>(),
                    Duration::from_secs(5),
                ),
                vec![],
            ),
            _ => {
                let agent_dir = self.cfg.paths.agent_dir(id);
                let wt = real(&rec.worktree);
                let me = [std::process::id() as i32];
                procs::kill_verified(
                    &SystemProcs,
                    &AgentMatch { pgid, agent_dir: &agent_dir, worktree: &wt, exclude: &me },
                    &pgid.into_iter().collect::<Vec<_>>(),
                    &pid.into_iter().collect::<Vec<_>>(),
                    Duration::from_secs(5),
                    Duration::from_secs(2),
                )
            }
        };
        if !left.is_empty() {
            // Not verified: keep pid/pgid so a retry finds the same processes.
            let what: Vec<String> = left.iter().map(|f| format!("{} {}", f.proc.pid, f.proc.cmd)).collect();
            let err = format!("{} process(es) survived the kill: {}", what.len(), what.join("; "));
            let _ = self.store.write(|s| {
                if let Some(a) = s.agents.get_mut(id) {
                    a.tracker.last_error = Some(err.clone());
                }
            });
            self.event(id, "stopped", json!({"intent": intent, "log": log, "survivors": left}));
            bail!("agent {id}: {err}");
        }
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
            self.deliver_user(id, m)?;
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
        self.deliver_kind(id, text, TurnKind::Message)
    }

    /// Queues the user's own words (an Inbox answer, a gate decision, a
    /// message from the app): admitted like typing in the pane is, past the
    /// Governor and the memory cap.
    pub fn deliver_user(&self, id: &str, text: &str) -> Result<String> {
        self.deliver_kind(id, text, TurnKind::User)
    }

    /// FR-811 Pause all. Returns the Governor action id.
    pub fn pause_all(&self) -> u64 {
        self.governor_act(|g, now| (g.pause_all(now).0, Vec::new()))
    }

    /// Lifts Pause all; returns how many queued turns were released.
    pub fn resume_all(&self) -> usize {
        self.governor_act(|g, now| {
            let r = g.resume_all(now);
            (r.len(), r)
        })
    }

    /// FR-805/811: pause one agent at its next tool boundary (until Resume).
    pub fn pause_agent(&self, id: &str) -> Result<u64> {
        if !self.store.read(|s| s.agents.contains_key(id)) {
            bail!("no agent {id}");
        }
        Ok(self.governor_act(|g, now| (g.pause_agent(&AgentId(id.to_owned()), now).0, Vec::new())))
    }

    /// Lifts an agent's pause; returns how many queued turns were released.
    pub fn resume_agent(&self, id: &str) -> usize {
        self.governor_act(|g, now| {
            let r = g.resume_agent(&AgentId(id.to_owned()), now);
            (r.len(), r)
        })
    }

    /// "Run now" on a queued entry (FR-809): past every cap, the memory cap included.
    pub fn run_now(&self, entry: u64) -> Result<()> {
        let agent = self.gov().queue().iter().find(|e| e.id.0 == entry).map(|e| e.agent.0.clone());
        let Some(agent) = agent else { bail!("no queued entry {entry}") };
        // Set before the release, so the pump never sees the turn without it;
        // only for an agent with no process (it would linger otherwise).
        if self.live_get(&agent).is_none() {
            self.house().spawn_override.insert(agent.clone());
        }
        let done = self.governor_act(|g, now| match g.run_now(governor::EntryId(entry), now) {
            Some(r) => (true, vec![r]),
            None => (false, Vec::new()),
        });
        if !done {
            self.house().spawn_override.remove(&agent);
            bail!("no queued entry {entry}");
        }
        Ok(())
    }

    /// Undoes a Governor action from the log (FR-809).
    pub fn undo(&self, action: u64) -> Result<()> {
        self.governor_act(|g, now| match g.undo(governor::ActionId(action), now) {
            Ok(r) => (Ok(()), r),
            Err(e) => (Err(anyhow!("{e}")), Vec::new()),
        })
    }

    /// Sprint mode (FR-811): `extra_pp` more today, borrowed from the rest of the week.
    /// `None` borrows [`DEFAULT_SPRINT_PP`].
    pub fn sprint(&self, extra_pp: Option<f64>) -> Result<u64> {
        let pp = extra_pp.unwrap_or(DEFAULT_SPRINT_PP);
        Ok(self.governor_act(|g, now| (g.start_sprint(pp, now), Vec::new()))?.0)
    }

    /// Answers a Governor ask (FR-810): accept applies it.
    pub fn answer_ask(&self, action: u64, accept: bool) -> Result<()> {
        Ok(self.governor_act(|g, now| (g.answer_ask(governor::ActionId(action), accept, now), Vec::new()))?)
    }

    /// FR-812: shadow mode on (log only) or off (enforce), from now on.
    /// The choice is stored, so the next start keeps it.
    pub fn set_shadow(&self, shadow: bool) -> Result<()> {
        let mut plan = self.gov().plan().clone();
        plan.shadow_mode = shadow;
        self.replan(plan)?;
        self.db.set_setting(ENFORCE_KEY, &json!(!shadow))
    }

    /// Drops a queued entry and cancels the turn waiting on it.
    pub fn cancel(&self, entry: u64) -> Result<()> {
        let agent = self.governor_act(|g, now| (g.cancel(governor::EntryId(entry), now), Vec::new()));
        let Some(agent) = agent else { bail!("no queued entry {entry}") };
        let msgs = self.store.write(|s| {
            let mut msgs = vec![];
            for t in s.turns.iter_mut().filter(|t| t.agent_id == agent.0 && t.status == TurnStatus::Queued) {
                if t.admission == Admission::Waiting(entry) {
                    t.status = TurnStatus::Cancelled;
                    msgs.extend(&t.messages);
                }
            }
            msgs
        })?;
        if !msgs.is_empty() {
            self.db.undeliver_messages(&msgs)?;
        }
        Ok(())
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
        // A permission prompt held for the Inbox but answered in the pane (the
        // CLI shows its dialog while the hook is held): this call's next hook,
        // or the turn's end, closes the Inbox item and ends the hold.
        let call_done =
            matches!(m.event, HookEvent::PostToolUse | HookEvent::PostToolUseFailure | HookEvent::PermissionDenied);
        let turn_done = matches!(
            m.event,
            HookEvent::Stop | HookEvent::StopFailure | HookEvent::UserPromptSubmit | HookEvent::SessionEnd
        );
        if call_done || turn_done {
            self.answered_in_pane(id, if call_done { p.tool_use_id.as_deref() } else { None });
        }
        self.event(
            id,
            m.event.as_str(),
            json!({"tool": p.tool_name, "prompt_id": p.prompt_id, "source": p.source, "error": p.error}),
        );
        // Before any state change: a blocked Stop keeps the turn running.
        if m.event == HookEvent::Stop {
            if let Some(out) = self.stop_reminder(&rec, &p) {
                self.event(id, "stop_blocked", json!({"why": "messages"}));
                return Some(out);
            }
            if let Some(out) = self.question_at_stop(&rec, &p) {
                self.event(id, "stop_blocked", json!({"why": "question"}));
                return Some(out);
            }
        }
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
            HookEvent::PreToolUse => self.forward_ask_user_question(&rec, &p).or_else(|| {
                self.admit_tool(&rec, tracker.turn_origin == Some(TurnOrigin::User), &p, m.deadline_ms, conn)
            }),
            // A rule, or the Inbox (held), or the pane when nobody answers in time.
            HookEvent::PermissionRequest => self.permission_request(&rec, &p, m.deadline_ms, conn),
            // FR-403: a busy team agent gets its messages at its next tool call.
            HookEvent::PostToolUse | HookEvent::PostToolUseFailure if rec.team.is_some() => {
                self.inject_messages(&rec).map(|t| HookOutput::additional_context(&m.event, &t))
            }
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
                self.mark_delivered(id, &markers, Some(now));
                if let Some(l) = &live {
                    l.ready_at_ms.store(0, Ordering::SeqCst);
                }
                match origin {
                    TurnOrigin::App => {} // admitted before delivery
                    TurnOrigin::User => {
                        self.close_pane_questions(id);
                        self.decide_turn(rec, TurnKind::User, origin, false);
                    }
                    TurnOrigin::SelfStarted => {
                        self.decide_turn(rec, TurnKind::Continue, origin, false);
                    }
                }
            }
            Effect::QueuedMessage => {
                let markers = delivery::find_markers(p.prompt.as_deref().unwrap_or(""));
                self.mark_delivered(id, &markers, None);
            }
            Effect::TurnEnded | Effect::TurnEndedWithBackground => {
                self.house().index_dirty.insert(id.to_string());
                let delivered = |t: &TurnRow| t.agent_id == id && t.status == TurnStatus::Delivered;
                let mut read = {
                    let mut h = self.house();
                    h.asked.remove(id);
                    h.injected.remove(id).unwrap_or_default()
                };
                if self.store.read(|s| s.turns.iter().any(delivered)) {
                    let carried = self.store.write(|s| {
                        let mut carried: Vec<u64> = vec![];
                        for turn in s.turns.iter_mut().filter(|t| delivered(t)) {
                            turn.status = TurnStatus::Done;
                            carried.extend(&turn.messages);
                        }
                        carried
                    });
                    read.extend(carried.unwrap_or_default());
                }
                if !read.is_empty() {
                    let _ = self.db.ack_messages(&read, now);
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

    /// Marks the turns `markers` delivered and, for a turn that started at
    /// `started`, records its start (one store write).
    fn mark_delivered(&self, id: &str, markers: &[String], started: Option<u64>) {
        if markers.is_empty() && started.is_none() {
            return;
        }
        let _ = self.store.write(|s| {
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id && markers.contains(&t.id)) {
                if matches!(t.status, TurnStatus::Queued | TurnStatus::Delivering) {
                    t.status = TurnStatus::Delivered;
                }
            }
            if let Some(now) = started
                && let Some(a) = s.agents.get_mut(id)
            {
                a.tracker.turn_started_ms = Some(now);
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
        let mut gone = vec![];
        for r in releases {
            let id = r.agent.0.clone();
            // A deleted agent's last Governor entry: nothing to run, and the
            // slot the release took is given back below.
            if self.store.read(|s| s.agents.get(&id).is_none_or(|a| a.tracker.state == AgentState::Archived)) {
                gone.push(r.agent);
                continue;
            }
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
        for agent in gone {
            let releases = self.gov().turn_ended(&agent, Timestamp::now());
            self.handle_releases(releases);
        }
    }

    /// Forgets a deleted agent in the Governor: its queued entries are
    /// dropped, its pause lifted and its running turn (if any) ended, so it
    /// holds no slot and no release ever comes for it.
    pub(crate) fn governor_forget(&self, id: &str) {
        let agent = AgentId(id.to_owned());
        self.governor_act(|g, now| {
            let entries: Vec<_> = g.queue().iter().filter(|e| e.agent == agent).map(|e| e.id).collect();
            for e in entries {
                g.cancel(e, now);
            }
            let mut releases = g.resume_agent(&agent, now);
            releases.extend(g.turn_ended(&agent, now));
            ((), releases)
        });
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
        self.governor_act(|g, now| ((), g.tick(now)));
        self.reserve_alert();
        let (actions, queue, plan_id, snap, calib) = {
            let g = self.gov();
            let n = g.actions().len();
            (
                g.actions()[n.saturating_sub(50)..].to_vec(),
                g.queue().to_vec(),
                g.plan().id.clone(),
                g.snapshot(now),
                g.calibration(now),
            )
        };
        // Written only when they changed (outcomes and undo change after the fact).
        let saved = (actions, queue);
        let changed = {
            let mut h = self.house();
            let c = h.last_saved != saved;
            if c {
                h.last_saved = saved.clone();
            }
            c
        };
        if changed {
            let boot = self.boot_ms.load(Ordering::SeqCst);
            let _ = self.db.save_actions(boot, &plan_id, &saved.0);
            let _ = self.db.replace_queue(boot, &saved.1);
        }
        let ms = now_ms();
        let (do_forecast, do_calib) = {
            let mut h = self.house();
            (
                due(&mut h.last_forecast_ms, ms, FORECAST_EVERY_MS),
                due(&mut h.last_calibration_ms, ms, CALIBRATION_EVERY_MS),
            )
        };
        if do_forecast {
            let _ = self.db.add_forecast(&snap);
        }
        if do_calib {
            let _ = self.db.add_calibration(&calib);
        }
    }

    /// True the first time `key` is seen (one watchdog alert per turn).
    pub(crate) fn watchdog_once(&self, key: String) -> bool {
        self.house().watchdog.insert(key)
    }

    /// Forgets the keys of turns that are no longer flagged.
    pub(crate) fn watchdog_keep(&self, keys: &std::collections::BTreeSet<String>) {
        self.house().watchdog.retain(|k| keys.contains(k));
    }

    /// True the first time `key` is seen in `run` (one note to the lead per
    /// stuck member and episode); keys of `run` not in `keys` are forgotten.
    pub(crate) fn supervised(&self, run: &str, keys: &std::collections::BTreeSet<String>) -> Vec<String> {
        let mut h = self.house();
        let seen = h.supervised.entry(run.to_string()).or_default();
        seen.retain(|k| keys.contains(k));
        keys.iter().filter(|k| seen.insert((*k).clone())).cloned().collect()
    }

    /// Every 5 minutes: time to sync the remote task backends.
    pub(crate) fn task_sync_due(&self) -> bool {
        due(&mut self.house().last_task_sync_ms, now_ms(), TASK_SYNC_EVERY_MS)
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

    /// A plan edited during its week (the first plan, J0; shadow mode on or
    /// off, FR-812): stored, then the Governor is rebuilt from it and this
    /// week's samples are replayed, so the week's accounting is kept (unlike
    /// `set_plan`, which starts a new week). The queue, pauses, deferrals, the
    /// ask and sprints carry over (`Governor::carry`); a sprint keeps its boost
    /// on the new curve.
    pub fn replan(&self, plan: WeeklyPlan) -> Result<()> {
        let now = Timestamp::now();
        if plan.reset_at <= now || plan.week_start > now {
            bail!("the plan does not cover now");
        }
        let released = {
            // Held through the rebuild: a turn queued meanwhile on the old
            // Governor would miss the carry and wait on an entry that is gone.
            let mut old = self.gov();
            let carry = old.carry();
            self.db.save_plan(&plan, orch_store::PlanStatus::Active)?;
            let (gov, released) =
                restore_governor(&self.db, &self.store, &self.cfg, plan.shadow_mode, Some(carry), now)?;
            // The new Governor's action ids start again: store them under a new key.
            let prev = self.boot_ms.load(Ordering::SeqCst);
            self.boot_ms.store(now_ms().max(prev + 1), Ordering::SeqCst);
            self.house().last_saved = Default::default();
            *old = gov;
            released
        };
        self.handle_releases(released);
        self.save_carry();
        Ok(())
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
        self.sync_process_slots();
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
                // A stop already under way (a suspend waits up to 5 s for the exit).
                if rec.tracker.stop_intent != StopIntent::None {
                    return None;
                }
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
                    reserved: reserved_slot(&rec),
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
            let pending = !h.index_dirty.is_empty() || h.index_all;
            (pending && due(&mut h.last_index_ms, now, INDEX_EVERY_MS), due(&mut h.last_purge_ms, now, PURGE_EVERY_MS))
        };
        if do_index && !self.indexing.swap(true, Ordering::SeqCst) {
            let (all, dirty) = {
                let mut h = self.house();
                (std::mem::take(&mut h.index_all), std::mem::take(&mut h.index_dirty))
            };
            // Reading transcripts can take seconds (first pass): never on the delivery loop.
            if let Some(me) = self.me.upgrade() {
                std::thread::spawn(move || {
                    let files = me.transcript_files(if all { None } else { Some(&dirty) });
                    match me.db.index_transcripts(&files) {
                        Ok(st) if st.entries > 0 => me.event("-", "transcripts_indexed", json!(st)),
                        Ok(_) => {}
                        Err(e) => me.event("-", "index_error", json!(e.to_string())),
                    }
                    me.indexing.store(false, Ordering::SeqCst);
                });
            } else {
                self.indexing.store(false, Ordering::SeqCst);
            }
        }
        // Teams: dispatch, messages, run progress (network calls with GitHub).
        if !self.teaming.swap(true, Ordering::SeqCst) {
            if let Some(me) = self.me.upgrade() {
                std::thread::spawn(move || {
                    me.team_tick();
                    me.watchdog(now_ms());
                    if me.task_sync_due() {
                        me.sync_task_backends();
                    }
                    me.teaming.store(false, Ordering::SeqCst);
                });
            } else {
                self.teaming.store(false, Ordering::SeqCst);
            }
        }
        if do_purge {
            // Deletes across several tables: never on the delivery loop.
            let (db, retention, me) = (self.db.clone(), self.cfg.retention, self.me.clone());
            std::thread::spawn(move || {
                if let (Ok(n), Some(me)) = (db.purge(now, retention), me.upgrade()) {
                    if n > 0 {
                        me.event("-", "purged", json!({"rows": n}));
                    }
                }
            });
        }
    }

    /// Transcripts to index: every known agent's session (found by id across
    /// `~/.claude/projects/*/`), plus, for a full pass, every session in a
    /// registered project's folders (its checkout and its worktrees).
    pub fn transcript_files(
        &self,
        only: Option<&std::collections::BTreeSet<String>>,
    ) -> Vec<orch_store::TranscriptFile> {
        let projects = self.db.projects(false).unwrap_or_default();
        let project_of = |repo: &Path| projects.iter().find(|p| p.path == repo).map(|p| p.id.clone());
        let mut files: Vec<orch_store::TranscriptFile> = vec![];
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for a in self.store.read(|s| s.agents.values().cloned().collect::<Vec<_>>()) {
            if only.is_some_and(|o| !o.contains(&a.id)) {
                continue;
            }
            // Already indexed: its path is known; else search every project folder.
            let known = self.db.transcript_path(&a.session_id).ok().flatten().filter(|p| p.is_file());
            if let Some(path) = known.or_else(|| crate::transcript::find(&self.cfg.projects_root, &a.session_id)) {
                if seen.insert(path.clone()) {
                    files.push(orch_store::TranscriptFile {
                        path,
                        session_id: a.session_id.clone(),
                        project_id: project_of(&a.repo),
                    });
                }
            }
        }
        if only.is_none() {
            for d in std::fs::read_dir(&self.cfg.projects_root).into_iter().flatten().flatten() {
                let name = d.file_name().to_string_lossy().into_owned();
                let Some(pid) = project_for_dir(&projects, &name) else { continue };
                for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                    let path = f.path();
                    if path.extension().is_some_and(|e| e == "jsonl") && seen.insert(path.clone()) {
                        let sid = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        files.push(orch_store::TranscriptFile { path, session_id: sid, project_id: Some(pid.clone()) });
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

    fn admit_turn(&self, rec: &AgentRecord, turn: &TurnRow, needs_spawn: bool) -> bool {
        match turn.admission {
            Admission::Admitted => return true,
            Admission::Waiting(_) => return false, // in the Governor's queue until a release
            Admission::Pending => {}
        }
        let origin = if turn.kind == TurnKind::User { TurnOrigin::User } else { TurnOrigin::App };
        let d = self.decide_turn(rec, turn.kind, origin, needs_spawn);
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
        if let Admission::Waiting(e) = turn.admission
            && !self.gov().queue().iter().any(|q| q.id.0 == e)
        {
            // Its entry is gone without a release (dropped for a critical
            // turn): ask again rather than wait forever. A release that comes
            // meanwhile admits the turn anyway.
            self.store.write(|s| {
                let stale = s.turns.iter_mut().find(|t| t.id == turn.id && t.admission == Admission::Waiting(e));
                if let Some(t) = stale {
                    t.admission = Admission::Pending;
                }
            })?;
            self.event(&rec.id, "stale_admission", json!({"turn": turn.id, "entry": e}));
            return Ok(());
        }
        let Some(l) = self.live_get(&rec.id) else {
            // A suspended agent with work is resumed through the Governor
            // (FR-403), which also holds the spawn until a process slot is
            // free under memory pressure (FR-203).
            // A team agent that never ran starts the same way (with its own
            // session id) once its first turn is admitted.
            let never_started = rec.never_started();
            let suspended = (rec.tracker.state == AgentState::Suspended || never_started) && rec.flag.is_none();
            if suspended && turn.admission == Admission::Pending {
                self.sync_process_slots();
            }
            // A lead's process has its own slot: it waits for none.
            let reserved = reserved_slot(rec);
            if suspended && self.admit_turn(rec, turn, !reserved) {
                // FR-203 at spawn time too: processes start and stop between
                // admission and here (another resume, a restored turn). Without
                // a slot the turn goes back to the queue as a memory wait,
                // unless the user ran it now.
                let (live, cap) = (self.live_count(), self.live_cap());
                let forced = self.house().spawn_override.remove(&rec.id) || turn.kind == TurnKind::User || reserved;
                if live >= cap && !forced {
                    let (agent, alloc) = (AgentId(rec.id.clone()), rec.allocation.clone().into());
                    let e = self
                        .governor_act(|g, now| (g.hold_spawn(&agent, &alloc, rec.tier, live, cap, now), Vec::new()));
                    self.event(&rec.id, "spawn_held", json!({"live": live, "cap": cap, "entry": e.0}));
                    self.store.write(|s| {
                        if let Some(t) = s.turns.iter_mut().find(|t| t.id == turn.id) {
                            t.admission = Admission::Waiting(e.0);
                        }
                    })?;
                    return Ok(());
                }
                if let Err(e) = self.spawn(&rec.id, never_started.then(|| SessionMode::New(rec.session_id.clone()))) {
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
        if delivery::can_deliver(&gate).is_err() || !self.admit_turn(rec, turn, false) {
            return Ok(());
        }
        // The agent's other queued turns ride along: one turn, one paste.
        let batch: Vec<TurnRow> = self.store.read(|s| {
            s.turns
                .iter()
                .filter(|t| {
                    t.agent_id == rec.id
                        && t.status == TurnStatus::Queued
                        && (t.id == turn.id || matches!(t.admission, Admission::Pending | Admission::Admitted))
                })
                .cloned()
                .collect()
        });
        let parts: Vec<(&str, &str)> = batch.iter().map(|t| (t.id.as_str(), t.body.as_str())).collect();
        let prepared = delivery::prepare_many(&parts, &self.cfg.paths.agent_dir(&rec.id));
        if let Some((path, body)) = &prepared.file {
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))?;
            std::fs::write(path, body)?;
        }
        l.ready_at_ms.store(0, Ordering::SeqCst);
        let ids: Vec<&str> = batch.iter().map(|t| t.id.as_str()).collect();
        self.store.write(|s| {
            for t in s.turns.iter_mut().filter(|t| ids.contains(&t.id.as_str())) {
                t.status = TurnStatus::Delivering;
                t.delivered_at_ms = Some(now);
            }
        })?;
        l.pty.write(&prepared.paste_bytes())?;
        std::thread::sleep(delivery::PASTE_TO_ENTER);
        l.pty.write(b"\r")?;
        self.event(&rec.id, "delivered", json!({"turn": turn.id, "turns": ids, "chars": prepared.text.chars().count(), "file": prepared.file.as_ref().map(|f| f.0.clone())}));
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
                    // P2: the role and, for a team agent, its seat (run, node, task).
                    "role": a.role.name,
                    "team": a.team,
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

/// The project a `~/.claude/projects/<name>` folder belongs to: the folder of
/// its checkout exactly, or a folder under its worktree root (sessions started
/// in a subfolder of the checkout can't be told apart from a sibling project
/// with the same encoded prefix, so they are left out). The longest match wins.
fn project_for_dir(projects: &[orch_store::Project], name: &str) -> Option<String> {
    projects
        .iter()
        .filter_map(|p| {
            let main = orch_store::encode_project_dir(&p.path);
            let root = orch_store::encode_project_dir(&p.worktree_root);
            let len = if name == main {
                main.len()
            } else if name == root || name.starts_with(&format!("{root}-")) {
                root.len()
            } else {
                return None;
            };
            Some((len, p.id.clone()))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, id)| id)
}

/// The Governor at start: the active plan when its week is still running
/// (else the default week), the stored policy (live cap from the config),
/// this week's statusline samples replayed with their cost class (NFR-2), and
/// the carry-over state (`carry`, else the stored one): pauses, deferrals, the
/// ask, sprints and the queue entries whose turns still wait for them. A turn
/// that waited on an entry that did not come back is asked again.
fn restore_governor(
    db: &orch_store::Store,
    store: &Store,
    cfg: &RuntimeConfig,
    shadow: bool,
    carry: Option<governor::Carry>,
    now: Timestamp,
) -> Result<(Governor, Vec<Release>)> {
    let mut policy = db.policy(POLICY_ID)?.unwrap_or_default();
    policy.live_caps = governor::LiveCaps::from_base(cfg.base_cap);
    db.save_policy(POLICY_ID, &policy)?;
    let mut plan = match db.active_plan()? {
        Some(p) if p.reset_at > now && p.week_start <= now => p,
        _ => default_plan(now, shadow),
    };
    plan.shadow_mode = shadow;
    db.save_plan(&plan, orch_store::PlanStatus::Active)?;
    let start = plan.week_start;
    let since = start.as_millisecond().max(0) as u64;
    let samples: Vec<_> = db
        .usage_since(since, Some(orch_store::UsageSource::Statusline))?
        .into_iter()
        .filter_map(|u| governor_sample(&u).map(|s| (u, s)))
        .collect();
    // Created when the app first saw this week, as the live Governor was: it
    // trusts W = 0 before the week's first reading only if that was at the
    // reset (FR-814). Either way the reset anchors the calibration.
    let seen_from = samples.first().map_or(now, |(_, s)| s.ts).clamp(start, now.max(start));
    let mut gov = Governor::new(plan, policy, seen_from).map_err(|e| anyhow!("{e:?}"))?;
    let allocs: BTreeMap<String, String> =
        store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.allocation.clone())).collect());
    for (u, sample) in samples {
        let class = u.class.as_deref().and_then(orch_store::untag).unwrap_or(governor::CostClass::Interactive);
        let alloc = u.allocation.clone().or_else(|| u.agent_id.as_ref().and_then(|a| allocs.get(a).cloned()));
        gov.replay(&sample, class, alloc.map(Into::into));
    }
    let mut released = Vec::new();
    // A replan carries the live Governor's state, running turns included; a
    // restart the stored one, without running turns (their processes ended
    // with the old core; the reconciler resumes those agents through the queue).
    let carry = match carry {
        Some(c) => Some(c),
        // Stored without running turns (`save_carry`).
        None => db.setting(CARRY_KEY)?.and_then(|v| serde_json::from_value::<governor::Carry>(v).ok()),
    };
    if let Some(carry) = carry {
        // No queued work is lost: an entry stays while its agent exists. An
        // entry with no turn row (a held tool call, a rate-limit reschedule)
        // becomes a "continue" turn on release; a turn row that was asked
        // again meanwhile finds the agent's entry and waits on it.
        let agents: std::collections::BTreeSet<String> = store.read(|s| s.agents.keys().cloned().collect());
        released = gov.restore(carry, |e| agents.contains(&e.agent.0), now);
    }
    // Ticked after the restore, so a carried ask is not replaced by a fresh one.
    released.extend(gov.tick(now));
    // The caller hands `released` to `handle_releases`, which admits their turns.
    let kept: std::collections::BTreeSet<u64> =
        gov.queue().iter().map(|e| e.id.0).chain(released.iter().map(|r| r.entry.0)).collect();
    store.write(|s| {
        for t in s.turns.iter_mut().filter(|t| t.status == TurnStatus::Queued) {
            if matches!(t.admission, Admission::Waiting(e) if !kept.contains(&e)) {
                t.admission = Admission::Pending;
            }
        }
    })?;
    Ok((gov, released))
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
        provenance: orch_store::untag(&u.provenance).unwrap_or(governor::Provenance::Observed),
    })
}

fn usage_record(
    g: &governor::UsageSample,
    s: &StatusSample,
    class: governor::CostClass,
    allocation: Option<String>,
) -> orch_store::UsageRecord {
    use orch_store::tag;
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
            team: None,
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
        let (live, plan_id) = {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
            // An app turn (admitted by the Governor before delivery: autonomous), two responses.
            let rec = rt.store.read(|s| s.agents["a1"].clone());
            // Off-hours allowed, so the turn is admitted at any time of day.
            let plan = rt.gov().plan().clone();
            rt.replan(WeeklyPlan { off_hours_allowance_pct: 10.0, ..plan }).unwrap();
            assert!(rt.decide_turn(&rec, TurnKind::Message, TurnOrigin::App, false).effective.is_admit());
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
            let g = rt.gov();
            (g.snapshot(Timestamp::now()), g.plan().id.clone())
        };
        let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
        let snap = rt.gov().snapshot(Timestamp::now());
        assert_eq!(rt.gov().plan().id, plan_id, "the realigned plan week is restored");
        assert_eq!(rt.gov().plan().reset_at.as_second(), reset);
        // The usage before the app (30 % since the reset, 4 days ago) gets the
        // same outside-the-app forecast and plan live and after the restart.
        let same = |a: f64, b: f64| (a - b).abs() < 0.01;
        assert!(same(snap.used, live.used) && same(snap.governed, live.governed), "{snap:?} vs {live:?}");
        assert!(same(snap.unattributed_forecast, live.unattributed_forecast), "{snap:?} vs {live:?}");
        assert!(same(snap.ideal, live.ideal) && snap.state == live.state, "{snap:?} vs {live:?}");
        let samples = rt.db.usage_since(0, Some(orch_store::UsageSource::Statusline)).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].class.as_deref(), Some("autonomous"));
    }

    #[test]
    fn pauses_and_waiting_turns_survive_a_replan_and_a_restart() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let waiting =
            |rt: &Runtime| rt.store.read(|s| s.turns.iter().find(|t| t.agent_id == "a1").map(|t| t.admission));
        let entry = {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
            let plan = rt.gov().plan().clone();
            rt.replan(WeeklyPlan { off_hours_allowance_pct: 10.0, ..plan }).unwrap();
            rt.pause_agent("a1").unwrap();
            rt.pause_all();
            rt.deliver("a1", "later").unwrap();
            let (rec, turn) = rt.store.read(|s| (s.agents["a1"].clone(), s.turns[0].clone()));
            assert!(!rt.admit_turn(&rec, &turn, false));
            let Some(Admission::Waiting(e)) = waiting(&rt) else { panic!("queued: {:?}", waiting(&rt)) };
            // A replan keeps the queue and the pauses (it used to be refused).
            let plan = rt.gov().plan().clone();
            rt.replan(WeeklyPlan { reserve_pct: 15.0, ..plan }).unwrap();
            assert_eq!(rt.gov().queue().iter().map(|q| q.id.0).collect::<Vec<_>>(), vec![e]);
            assert!(rt.gov().is_paused_all());
            rt.tick();
            rt.db.flush().unwrap();
            e
        };
        let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
        assert_eq!(waiting(&rt), Some(Admission::Waiting(entry)), "the turn keeps its entry");
        assert_eq!(rt.gov().queue()[0].id.0, entry);
        assert!(rt.gov().paused_agents().contains(&AgentId("a1".into())));
        assert_eq!(rt.resume_all(), 0, "a1 is still paused on its own");
        assert_eq!(rt.resume_agent("a1"), 1);
        assert_eq!(waiting(&rt), Some(Admission::Admitted));
    }

    #[test]
    fn replan_keeps_running_turns_and_restart_keeps_held_work() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
            let plan = rt.gov().plan().clone();
            rt.replan(WeeklyPlan { off_hours_allowance_pct: 10.0, ..plan }).unwrap();
            // An app turn is running.
            let rec = rt.store.read(|s| s.agents["a1"].clone());
            assert!(rt.decide_turn(&rec, TurnKind::Message, TurnOrigin::App, false).effective.is_admit());
            rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] go"})), None);
            // A replan (an enforce switch) keeps it: still admitted, still counted.
            let plan = rt.gov().plan().clone();
            rt.replan(WeeklyPlan { reserve_pct: 12.0, ..plan }).unwrap();
            assert!(rt.gov().is_live(&AgentId("a1".into())));
            assert_eq!(rt.gov().live_autonomous(), 1);
            assert!(rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), None).is_none(), "tool admitted");
            // Paused mid-turn: its next tool call is denied and the turn waits
            // as a queue entry with no turn row.
            rt.pause_agent("a1").unwrap();
            assert!(rt.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash"})), None).is_some(), "held");
            assert_eq!(rt.gov().queue().len(), 1);
            assert_eq!(rt.store.read(|s| s.turns.iter().filter(|t| t.status == TurnStatus::Queued).count()), 0);
            rt.tick();
            rt.db.flush().unwrap();
        }
        // The restart keeps that work (it used to be dropped).
        let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
        assert_eq!(rt.gov().queue().len(), 1);
        assert!(!rt.gov().is_live(&AgentId("a1".into())), "its process ended with the old core");
        assert_eq!(rt.resume_agent("a1"), 1);
        let continued = rt
            .store
            .read(|s| s.turns.iter().any(|t| t.status == TurnStatus::Queued && t.admission == Admission::Admitted));
        assert!(continued, "a continue turn is admitted on release");
    }

    #[test]
    fn the_enforce_choice_is_kept_across_restarts() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        {
            let rt = Runtime::new(test_cfg(d.path(), true)).unwrap();
            assert!(rt.gov().plan().shadow_mode);
            rt.set_shadow(false).unwrap(); // `orchd enforce on` takes this path too
            rt.db.flush().unwrap();
        }
        let rt = Runtime::new(test_cfg(d.path(), true)).unwrap();
        assert!(!rt.gov().plan().shadow_mode, "kept");
        assert!(!rt.cfg.shadow);
        rt.set_shadow(true).unwrap();
        rt.db.flush().unwrap();
        drop(rt);
        let rt = Runtime::new(test_cfg(d.path(), true)).unwrap();
        assert!(rt.gov().plan().shadow_mode, "back to shadow, kept too");
        // The app's earlier key still counts.
        rt.db.set_setting(ENFORCE_KEY, &Value::Null).unwrap();
        rt.db.set_setting("ui.governor.enforce", &json!(true)).unwrap();
        assert_eq!(enforce_chosen(&rt.db), Some(true));
    }

    #[test]
    fn cancel_drops_the_entry_and_its_turn() {
        let (_d, rt) = test_runtime(false);
        rt.pause_all();
        rt.deliver("a1", "later").unwrap();
        let (rec, turn) = rt.store.read(|s| (s.agents["a1"].clone(), s.turns[0].clone()));
        assert!(!rt.admit_turn(&rec, &turn, false));
        let e = rt.gov().queue()[0].id.0;
        rt.cancel(e).unwrap();
        assert!(rt.gov().queue().is_empty());
        assert_eq!(rt.store.read(|s| s.turns[0].status), TurnStatus::Cancelled);
        assert!(rt.cancel(e).is_err());
    }

    #[test]
    fn otlp_requests_become_usage_samples() {
        let (_d, rt) = test_runtime(true);
        let ep = rt.start_otlp(0).unwrap();
        assert_eq!(rt.otlp_endpoint().as_deref(), Some(ep.as_str()));
        rt.on_api_requests(vec![
            crate::otlp::ApiRequest {
                ts_ms: 5,
                agent_id: Some("a1".into()),
                input_tokens: 10,
                output_tokens: 2,
                cost_usd: 0.01,
                ..Default::default()
            },
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
    fn housekeeping_indexes_agent_transcripts_off_the_loop() {
        let (d, rt) = test_runtime(true);
        let dir = d.path().join("projects/-old-folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("s.jsonl"),
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"refactor the parser\"}}\n",
        )
        .unwrap();
        rt.housekeeping();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        let mut hits = vec![];
        while hits.is_empty() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
            hits = rt.db.search_transcripts("parser", None, 5).unwrap();
        }
        assert_eq!(hits.len(), 1, "found by session id in any project folder");
        assert_eq!(hits[0].session_id, "s");
    }

    #[test]
    fn transcript_folders_map_to_the_right_project() {
        let pr = |id: &str, path: &str| {
            let mut p: orch_store::Project = serde_json::from_value(json!({
                "id": id, "name": id, "path": path, "default_branch": "main", "remotes": [],
                "worktree_root": format!("{path}.worktrees"), "task_backend": {}, "track_orchestrator_dir": true,
                "settings": {}, "created_at_ms": 0, "last_active_at_ms": 0, "last_fetch_at_ms": null,
                "last_fetch_error": null, "archived": false}))
            .unwrap();
            p.path = path.into();
            p
        };
        let ps = vec![pr("demo", "/x/demo"), pr("demo-api", "/x/demo-api")];
        assert_eq!(project_for_dir(&ps, "-x-demo").as_deref(), Some("demo"));
        assert_eq!(project_for_dir(&ps, "-x-demo-api").as_deref(), Some("demo-api"), "not the sibling's prefix");
        assert_eq!(project_for_dir(&ps, "-x-demo-worktrees-solo-fix").as_deref(), Some("demo"));
        assert_eq!(project_for_dir(&ps, "-x-demo-api-worktrees-solo-a").as_deref(), Some("demo-api"));
        assert_eq!(project_for_dir(&ps, "-x-other"), None);
    }

    fn team_message(rt: &Runtime, body: &str) -> u64 {
        rt.db
            .add_message(&orch_store::Message {
                id: 0,
                team_run_id: "r1".into(),
                from_node: "back".into(),
                from_agent: Some("back-1".into()),
                to_node: "lead".into(),
                to_agent: Some("a1".into()),
                kind: "report".into(),
                body: body.into(),
                r#ref: Value::Null,
                created_at_ms: 1,
                delivered_at_ms: None,
                read_at_ms: None,
            })
            .unwrap()
    }

    fn seat(rt: &Runtime) {
        rt.store
            .write(|s| {
                s.agents.get_mut("a1").unwrap().team = Some(crate::store::TeamSeat {
                    run_id: "r1".into(),
                    node: "lead".into(),
                    feature_id: "f1".into(),
                    project_id: "p1".into(),
                    task: None,
                })
            })
            .unwrap();
    }

    fn read_at(rt: &Runtime, id: u64) -> Option<u64> {
        rt.db.messages("r1", None, false).unwrap().into_iter().find(|m| m.id == id).unwrap().read_at_ms
    }

    #[test]
    fn messages_are_read_when_their_turn_ends_not_when_queued() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        let m = team_message(&rt, "T-2 done");
        let tid = rt.deliver_messages_turn("a1", "ORCH: 1 new message", vec![m]).unwrap();
        rt.db.deliver_messages(&[m], "a1", 5).unwrap();
        assert!(rt.inject_messages(&rt.store.read(|s| s.agents["a1"].clone())).is_none(), "delivered once");
        assert_eq!(read_at(&rt, m), None, "queued is not read");
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": format!("[t:{tid}] ORCH: 1 new message")})), None);
        assert_eq!(read_at(&rt, m), None, "started is not read either");
        rt.on_hook(&hook("Stop", json!({})), None);
        assert!(read_at(&rt, m).is_some(), "read when the turn ends");
    }

    #[test]
    fn a_dropped_turn_gives_its_messages_back() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        let m = team_message(&rt, "T-2 done");
        rt.deliver_messages_turn("a1", "ORCH: 1 new message", vec![m]).unwrap();
        rt.db.deliver_messages(&[m], "a1", 5).unwrap();
        rt.drop_queued_turns("a1");
        let rec = rt.store.read(|s| s.agents["a1"].clone());
        assert!(rt.inject_messages(&rec).unwrap().contains("T-2 done"), "delivered again");
    }

    #[test]
    fn a_message_that_came_mid_turn_is_reminded_once_at_stop() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "review T-1"})), None);
        let m = team_message(&rt, "T-2 done");
        let ctx = rt.on_hook(&hook("PostToolUse", json!({"tool_name": "Edit"})), None).expect("injected");
        assert!(ctx.0.to_string().contains("T-2 done"));
        let out = rt.on_hook(&hook("Stop", json!({})), None).expect("blocked");
        assert_eq!(out.0["decision"], "block");
        assert!(out.0["reason"].as_str().unwrap().contains("T-2 done"));
        assert_eq!(read_at(&rt, m), None);
        // The Stop after the block ends the turn.
        assert!(rt.on_hook(&hook("Stop", json!({"stop_hook_active": true})), None).is_none());
        assert!(read_at(&rt, m).is_some());
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::Idle);
    }

    fn questions(rt: &Runtime) -> Vec<crate::store::InboxItem> {
        rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "question" && i.pending()).cloned().collect())
    }

    #[test]
    fn a_plain_text_question_is_sent_back_once_then_reaches_the_inbox() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] ORCH: report"})), None);
        let q = "I merged T-1.\n\nDecision for you: do you want that column?";
        let out = rt.on_hook(&hook("Stop", json!({"last_assistant_message": q})), None).expect("blocked");
        assert!(out.0["reason"].as_str().unwrap().contains("mcp__orch__ask_user"));
        assert!(questions(&rt).is_empty());
        assert!(rt.on_hook(&hook("Stop", json!({"last_assistant_message": q, "stop_hook_active": true})), None).is_none());
        let items = questions(&rt);
        assert_eq!(items.len(), 1);
        assert!(items[0].body.contains("do you want that column?"));
        // The user answers in the pane instead: the item closes.
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "no dollars"})), None);
        assert!(questions(&rt).is_empty());
    }

    #[test]
    fn ask_user_question_in_an_app_turn_goes_to_the_inbox() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "[t:0123abcd] ORCH: task"})), None);
        let input = json!({"questions": [{"question": "Which db?", "options": [{"label": "SQLite"}]}]});
        let out = rt
            .on_hook(&hook("PreToolUse", json!({"tool_name": "AskUserQuestion", "tool_input": input})), None)
            .expect("denied");
        assert_eq!(out.0["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(questions(&rt)[0].payload["options"], json!(["SQLite"]));
        // It asked: the Stop on its question is not sent back.
        assert!(rt.on_hook(&hook("Stop", json!({"last_assistant_message": "Which db?"})), None).is_none());
        assert_eq!(questions(&rt).len(), 1);
    }

    #[test]
    fn the_users_answer_is_admitted_past_a_pause() {
        let (_d, rt) = test_runtime(false);
        rt.pause_all();
        rt.deliver_user("a1", "go with ux's proposal").unwrap();
        let (rec, turn) = rt.store.read(|s| (s.agents["a1"].clone(), s.turns[0].clone()));
        assert!(rt.admit_turn(&rec, &turn, false));
        rt.deliver("a1", "an app turn").unwrap();
        let turn = rt.store.read(|s| s.turns[1].clone());
        assert!(!rt.admit_turn(&rec, &turn, false));
    }

    #[test]
    fn a_turn_whose_entry_vanished_asks_again() {
        let (_d, rt) = test_runtime(false);
        rt.pause_all();
        rt.deliver("a1", "later").unwrap();
        let (rec, turn) = rt.store.read(|s| (s.agents["a1"].clone(), s.turns[0].clone()));
        assert!(!rt.admit_turn(&rec, &turn, false));
        let Admission::Waiting(e) = rt.store.read(|s| s.turns[0].admission) else { panic!() };
        // Dropped without a release (e.g. replaced by a critical turn).
        rt.gov().cancel(governor::EntryId(e), Timestamp::now());
        assert_eq!(rt.store.read(|s| s.turns[0].admission), Admission::Waiting(e));
        rt.pump();
        assert_eq!(rt.store.read(|s| s.turns[0].admission), Admission::Pending);
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
