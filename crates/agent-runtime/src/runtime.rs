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
use crate::project_files::{self, SpawnFile};
use crate::pty::PtyProcess;
use crate::reconcile::{self, ReconcileEnv, Report};
use crate::resources::{self, LiveAgent, MemoryProbe, PressureFilter, SystemMemory};
use crate::role::{PermissionMode, Role};
use crate::state::{self, AgentState, Effect, StopIntent, TurnOrigin};
use crate::store::{Admission, AgentRecord, HoldKind, HoldRow, HoldStatus, Store, TurnRow, TurnStatus};
use crate::usage::UsageCollector;
use crate::worktree;

pub use governor::DEFAULT_ALLOCATION;
/// The CLI shows its "continuing automatically at …" banner just after the
/// StopFailure hook returns; Esc sent before it would cancel nothing.
const ESC_AFTER_RATE_LIMIT: Duration = Duration::from_secs(1);
/// An agent blocked by a limit stays so at least this long after its last
/// hook, even when the Governor holds no limit (§3.2).
const LIMIT_GRACE_MS: u64 = 60_000;
/// The flag of an agent whose start just failed, while FR-415 decides (`on_exit`).
const CHECKING_START: &str = "checking its failed start";
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
    /// FR-203: the base live cap for this run (`orchd serve --cap`), not
    /// stored. `None` = the `governor.live_cap` setting, else
    /// [`DEFAULT_LIVE_CAP`](crate::live_cap::DEFAULT_LIVE_CAP).
    pub base_cap: Option<u32>,
    /// How long a PreToolUse waits for the Governor before denying (ADR-007: "a few seconds").
    pub hold: Duration,
    pub rows: u16,
    pub cols: u16,
    /// FR-705: how long a PermissionRequest waits for an Inbox decision
    /// before it is denied and parked (the `permissions.hold_secs` setting
    /// overrides it). Zero = straight to the pane.
    pub permission_hold: Duration,
    /// FR-204: suspend an idle agent after this long (`None` = never).
    pub idle_suspend: Option<Duration>,
    /// FR-203: macOS memory pressure lowers the live-agent cap.
    pub memory: Arc<dyn MemoryProbe>,
    /// FR-203 hysteresis on that level (D4: 60 s up, 120 s down).
    pub pressure_filter: PressureFilter,
    /// FR-415: a spawn with no SessionStart (and no trust dialog) after this
    /// long gets a `startup` Inbox item.
    pub startup_timeout: Duration,
    /// NFR-8 retention of raw events, snapshots and samples.
    pub retention: orch_store::Retention,
    /// FR-103 background fetch.
    pub fetch: orch_git::fetch::FetchPolicy,
    /// Power and network state for the fetch throttle.
    pub conditions: Arc<dyn orch_git::fetch::Conditions>,
    /// Replaces `gh` for the GitHub task backend (tests).
    pub gh: Option<crate::team::GhFactory>,
    /// Replaces the PR gate's push + `gh pr create` (tests): the PR's URL.
    pub open_pr: Option<crate::inbox::PrOpener>,
    /// Integration tokens and MCP env secrets (NFR-3): the Keychain.
    pub secrets: Arc<dyn orch_secrets::Secrets>,
    /// The core's own network calls (MCP probes, token tests).
    pub net: Arc<dyn crate::net::Net>,
    /// The plan of a week with none stored ([`default_plan`]; tests use
    /// [`test_plan`], which holds at any date, hour and time zone).
    pub default_plan: fn(Timestamp, bool) -> WeeklyPlan,
    /// FR-816: keeps the Mac awake on AC while off-hours work waits or runs.
    pub power: Arc<dyn crate::power::PowerAssertion>,
    /// Replaces the Linear GraphQL transport (tests).
    pub linear: Option<crate::linear::LinearFactory>,
    /// Replaces the Notion transport (tests).
    pub notion: Option<crate::notion::NotionFactory>,
    /// Replaces the PTY + `claude` process of a spawn (tests): the chaos suite
    /// runs a fake CLI in process.
    pub terminal: Option<TerminalFactory>,
}

/// Starts the terminal of a spawn; the closure runs when the "process" exits.
#[derive(Clone)]
pub struct TerminalFactory(pub Arc<TerminalFn>);

/// What starts a terminal: the compiled command, and what runs on exit.
pub type TerminalFn = dyn Fn(&crate::compile::Compiled, crate::pty::OnExit) -> Result<PtyProcess> + Send + Sync;

impl std::fmt::Debug for TerminalFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalFactory")
    }
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
            base_cap: None,
            hold: Duration::from_secs(5),
            rows: 50,
            cols: 160,
            permission_hold: Duration::from_secs(300),
            idle_suspend: Some(Duration::from_secs(10 * 60)),
            memory: Arc::new(SystemMemory),
            pressure_filter: PressureFilter::default(),
            startup_timeout: Duration::from_secs(30),
            retention: orch_store::Retention::default(),
            fetch: orch_git::fetch::FetchPolicy::default(),
            conditions: Arc::new(orch_git::fetch::SystemConditions),
            gh: None,
            open_pr: None,
            secrets: Arc::new(orch_secrets::Keychain),
            net: Arc::new(crate::net::SystemNet),
            default_plan,
            power: Arc::new(crate::power::Caffeinate::default()),
            linear: None,
            notion: None,
            terminal: None,
        }
    }
}

/// A live agent process and its delivery clocks.
pub struct Live {
    pub pty: PtyProcess,
    /// 0 = input not allowed yet (before SessionStart + 1.5 s, or during a turn).
    pub ready_at_ms: AtomicU64,
    pub last_user_input_ms: AtomicU64,
    /// The `--model` it was started with (FR-819 switches at a task boundary).
    pub model: Option<String>,
    /// The task `model` was chosen for (0 = none): a task keeps its model
    /// between its turns, only a new task is a boundary.
    pub task: AtomicU64,
    /// The last paste of a turn (activity for least-recently-active, FR-203).
    pub pasted_at_ms: AtomicU64,
    /// Started with `--resume` (else a new session, the fresh retry of
    /// FR-415 included): the exit rule of FR-415.
    pub resumed: bool,
}

pub struct Runtime {
    pub cfg: RuntimeConfig,
    pub store: Store,
    /// The SQLite database (shared with the git service and the UI's queries).
    pub db: orch_store::Store,
    gov: Mutex<Governor>,
    /// One live-cap change at a time ([`Runtime::set_live_caps`]).
    pub(crate) live_cap_change: Mutex<()>,
    /// FR-203: the base live cap now (the Governor's `LiveCaps` and the
    /// process cap derive from it); [`Runtime::set_live_caps`] changes it.
    base_cap: std::sync::atomic::AtomicU32,
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
    pub(crate) teaming: std::sync::atomic::AtomicBool,
    /// A task-backend sync ([`Runtime::sync_task_backends`]) is running: on
    /// its own thread, off the team pass (§3.6).
    syncing: std::sync::atomic::AtomicBool,
    /// Chaos suite (§3.8): the team-tick step that panics ([`Runtime::inject_tick_fault`]).
    pub(crate) tick_fault: Mutex<Option<String>>,
    /// A paste-watchdog and spool pass ([`Runtime::handover_tick`]) is running.
    pub(crate) watching: std::sync::atomic::AtomicBool,
    /// Held while a spool is replayed: a live hook waits for the sweep's
    /// replay of older events, so events apply in their order (§3.3).
    pub(crate) replaying: Mutex<()>,
    /// Each project's task backend, kept across calls.
    pub(crate) task_backends: orch_tasks::Backends,
    /// One per project: serialises its task syncs ([`Runtime::sync_tasks`]).
    pub(crate) sync_gates: Mutex<std::collections::HashMap<String, std::sync::Arc<crate::team::SyncGate>>>,
    /// Projects whose Board was opened in this process: the background task
    /// sync covers their remote backend too.
    pub(crate) watched_projects: Mutex<std::collections::BTreeSet<String>>,
    /// The starter roles and presets were checked this run.
    pub(crate) presets_checked: std::sync::atomic::AtomicBool,
    /// FR-505: one Notion publish or poll per feature at a time, and the
    /// background poll pass running.
    pub(crate) notion_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    pub(crate) notion_polling: std::sync::atomic::AtomicBool,
    /// The Linear API key, read once (cleared when the user changes it).
    pub(crate) linear_token: crate::linear::TokenCache,
    /// FR-604: (project, server) probes running in the background.
    pub(crate) probing: Mutex<std::collections::BTreeSet<(Option<String>, String)>>,
    /// Told of every Inbox outcome ([`Runtime::set_outcome_sink`]).
    pub(crate) outcome_sink: Mutex<Option<crate::inbox::OutcomeSink>>,
    /// The `now` publisher (P4.1): last world sent, rev, sinks.
    pub(crate) now: crate::now::Now,
    /// Worktrees open in a Diff pane, watched for the `worktree:<path>` topic.
    pub(crate) watches: crate::worktree_watch::Watches,
    /// The PR watcher's fetch clock; held for each PR check.
    pub(crate) pr_watch: Mutex<crate::pr::PrWatch>,
}

/// Clocks and flags of the periodic work (`tick`, `housekeeping`).
#[derive(Default)]
pub(crate) struct House {
    last_forecast_ms: u64,
    last_calibration_ms: u64,
    /// The fit last stored (FR-814: a row on each fit change).
    last_fit: Option<governor::Calibrator>,
    last_purge_ms: u64,
    last_index_ms: u64,
    /// FR-1311: each agent's wait episode already recorded as a pane intervention.
    pub(crate) interventions: BTreeMap<String, crate::intervention::Wait>,
    /// FR-203: the probe's level with hysteresis; its `effective` sets the cap.
    pressure: PressureFilter,
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
    pub(crate) watchdog: std::collections::BTreeSet<String>,
    /// Last FR-604 probe of the servers the running agents use.
    last_mcp_probe_ms: u64,
    /// Files of its `messages/` folder each agent referenced in a tool call
    /// during its running turn (a Read, a `cat`): its long messages are read
    /// when it ends (§3.4).
    pub(crate) referenced: BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Agents that called `inbox` during their running turn.
    pub(crate) inbox_called: std::collections::BTreeSet<String>,
    /// Agents that asked the user (Inbox question) during their running turn.
    asked: std::collections::BTreeSet<String>,
    /// Stuck team members already told to their lead, by run.
    supervised: BTreeMap<String, std::collections::BTreeSet<String>>,
    /// FR-816: last read of the power source, what it said, and the night
    /// the on-battery alert went out.
    pub(crate) last_power_ms: u64,
    pub(crate) power: crate::pacing::PowerState,
    pub(crate) battery_alerted: Option<String>,
    /// FR-505: last look for Notion pages due a comment poll.
    pub(crate) last_notion_ms: u64,
    /// FR-816: the off-hours marks of tasks and runs (the store's, cached).
    pub(crate) off_hours_marks: Option<crate::pacing::OffHoursMarks>,
    /// The PTY tail of each agent's last process that exited unasked (a
    /// crash): shown when its restarts are used up (§3.5).
    pub(crate) crash_tails: BTreeMap<String, String>,
    /// `delivery` decisions raised this run of the core, by episode key
    /// (§3.7): a dismissed one is not raised again for the same episode.
    pub(crate) delivery_raised: std::collections::BTreeSet<String>,
    /// Agents whose model switch could not stop the process: their task runs
    /// on the current model instead of waiting for a switch that never comes.
    pub(crate) switch_failed: std::collections::BTreeSet<String>,
    /// When the user last retried each message of a `delivery` decision (§3.7).
    pub(crate) retried_at: BTreeMap<u64, u64>,
    /// When the user last retried the decision of each episode key (§3.7):
    /// the episode's clock starts again from then.
    pub(crate) retried_keys: BTreeMap<String, u64>,
}

/// A hook's event detail (FR-1311, the run report): the common fields, plus
/// the type and text of a Notification, the call id of a tool event and the
/// permission mode a SessionStart reports.
pub(crate) fn hook_event_detail(event: &HookEvent, p: &HookPayload) -> Value {
    let mut d = json!({"tool": p.tool_name, "prompt_id": p.prompt_id, "source": p.source, "error": p.error});
    let mut put = |k: &str, v: &Option<String>| {
        if let Some(v) = v {
            d[k] = json!(v);
        }
    };
    match event {
        HookEvent::Notification => {
            put("notification_type", &p.notification_type);
            put("message", &p.message);
        }
        HookEvent::SessionStart => put("permission_mode", &p.permission_mode),
        HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure
        | HookEvent::PermissionRequest
        | HookEvent::PermissionDenied => put("tool_use_id", &p.tool_use_id),
        _ => {}
    }
    d
}

/// A team lead (the node that merges): outside the process cap. Not the
/// tier alone, which a specialist's role can also set to critical.
pub(crate) fn reserved_slot(a: &AgentRecord) -> bool {
    a.team.is_some() && a.role.orch_tools.iter().any(|t| t == "merge")
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
/// FR-814: the estimator is stored on each fit change, at most this often.
pub const CALIBRATION_EVERY_MS: u64 = 60 * 1000;
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

/// For tests: a plan no date can break. The week runs from three days
/// before `now` to four days after, every hour is working time, and the
/// zone is the fixed offset where `now` is between 12:00 and 13:00: no test
/// starts near the end of a day or of the week, whatever the date, the
/// system time zone or a DST change.
#[doc(hidden)]
pub fn test_plan(now: Timestamp, shadow: bool) -> WeeklyPlan {
    use governor::jiff::civil::time;
    let utc_hour = now.to_zoned(TimeZone::UTC).hour();
    // Etc/GMT counts west as positive: Etc/GMT-5 is UTC+5.
    let tz_name = format!("Etc/GMT{:+}", utc_hour - 12);
    let mut plan = default_plan(now, shadow);
    plan.week_start = now.checked_sub(72.hours()).unwrap_or(now);
    plan.reset_at = now.checked_add(96.hours()).unwrap_or(now);
    plan.id = format!("week-{}", plan.week_start.as_second());
    plan.calendar = WorkingCalendar::every_day(&tz_name, time(0, 0, 0, 0), time(23, 59, 59, 0));
    plan
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
        let mut warnings = vec![];
        let base_cap = match cfg.base_cap {
            // `orchd serve --cap`: a bad value fails the start.
            Some(c) => {
                crate::live_cap::check_live_cap(c)?;
                c
            }
            None => crate::live_cap::stored(&db).unwrap_or_else(|w| {
                warnings.push(w);
                crate::live_cap::DEFAULT_LIVE_CAP
            }),
        };
        let (gov, released) = restore_governor(&db, &store, &cfg, base_cap, cfg.shadow, None, now)?;
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
        warnings.extend(crate::env::parent_env_warnings());
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
        let pressure = cfg.pressure_filter;
        let rt = Arc::new_cyclic(|me| Runtime {
            user_statusline: crate::env::user_statusline_command(&home),
            git_identity,
            cfg,
            store,
            db,
            gov: Mutex::new(gov),
            live_cap_change: Mutex::new(()),
            base_cap: std::sync::atomic::AtomicU32::new(base_cap),
            usage: Mutex::new(UsageCollector::new()),
            live: Mutex::new(BTreeMap::new()),
            decisions: Mutex::new(VecDeque::new()),
            login_path,
            warnings,
            me: me.clone(),
            boot_ms: AtomicU64::new(now_ms()),
            // The first purge waits a day like the others (not during the boot rush).
            house: Mutex::new(House { index_all: true, last_purge_ms: now_ms(), pressure, ..Default::default() }),
            otlp: Mutex::new(None),
            indexing: std::sync::atomic::AtomicBool::new(false),
            teaming: std::sync::atomic::AtomicBool::new(false),
            syncing: std::sync::atomic::AtomicBool::new(false),
            tick_fault: Mutex::new(None),
            watching: std::sync::atomic::AtomicBool::new(false),
            replaying: Mutex::new(()),
            task_backends: orch_tasks::Backends::default(),
            sync_gates: Mutex::default(),
            watched_projects: Mutex::default(),
            presets_checked: std::sync::atomic::AtomicBool::new(false),
            notion_locks: Mutex::new(BTreeMap::new()),
            notion_polling: std::sync::atomic::AtomicBool::new(false),
            linear_token: Default::default(),
            probing: Default::default(),
            pr_watch: Default::default(),
            outcome_sink: Mutex::new(None),
            now: Default::default(),
            watches: Default::default(),
        });
        rt.handle_releases(released);
        rt.backfill_report(now);
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
        let payload = json!({"reason": reason});
        if self.add_item(None, "budget", body, payload, 0).is_ok() {
            let _ = self.db.set_setting(RESERVE_ALERT_KEY, &json!(week));
            self.house().reserve_alerted = Some(Some(week));
        }
    }

    /// Tells the Governor how many `claude` processes run and the memory cap
    /// on them (FR-203); spawns it held wait for a free slot.
    pub(crate) fn sync_process_slots(&self) {
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

    /// A slash command the app typed has no marker: its prompt is matched
    /// to its turn and given the marker, so the turn reads as the app's.
    fn bind_command(&self, id: &str, p: &mut HookPayload) {
        let Some(prompt) = p.prompt.as_deref().map(str::trim).filter(|p| p.starts_with('/')) else { return };
        let turn = self.store.read(|s| {
            s.turns
                .iter()
                .find(|t| {
                    t.agent_id == id && t.command && t.status == TurnStatus::Delivering && t.body.trim() == prompt
                })
                .map(|t| t.id.clone())
        });
        if let Some(t) = turn {
            p.prompt = Some(format!("{} {prompt}", delivery::marker(&t)));
        }
    }

    /// Queues a slash command for the agent (`/code-review …`), typed as is
    /// once it is idle and admitted like any app turn.
    pub fn deliver_command(&self, id: &str, command: &str) -> Result<String> {
        let mut turn = TurnRow::queued(id, command, TurnKind::Task, Admission::Pending);
        turn.command = true;
        self.deliver_row(turn)
    }

    pub(crate) fn mark_asked(&self, id: &str) {
        self.house().asked.insert(id.to_string());
    }

    pub(crate) fn asked(&self, id: &str) -> bool {
        self.house().asked.contains(id)
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
        self.undeliver(msgs);
    }

    /// The team messages of turns taken back are delivered again later.
    pub(crate) fn undeliver(&self, msgs: Result<Vec<u64>>) {
        if let Ok(msgs) = msgs
            && !msgs.is_empty()
        {
            let _ = self.db.undeliver_messages(&msgs);
        }
    }

    /// The Stop policy: a question left in the pane keeps a team agent's
    /// turn going, once. Team messages never block a Stop (FR-403): the ones
    /// injected but not seen go back to the queue at TurnEnded.
    fn stop_check(&self, rec: &AgentRecord, p: &HookPayload) -> Option<HookOutput> {
        self.question_at_stop(rec, p).map(HookOutput::stop_block)
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
    pub(crate) fn decide_turn(
        &self,
        rec: &AgentRecord,
        kind: TurnKind,
        origin: TurnOrigin,
        needs_spawn: bool,
    ) -> Decision {
        if origin != TurnOrigin::User {
            self.sync_off_hours(rec);
        }
        let model = self.turn_model(rec);
        let req = TurnRequest {
            agent: AgentId(rec.id.clone()),
            allocation: rec.allocation.clone().into(),
            tier: rec.tier,
            interactive: origin == TurnOrigin::User,
            kind,
            self_started: origin == TurnOrigin::SelfStarted,
            ceiling: None,
            needs_spawn,
            model,
            // §3.5: a team's messages and nudges carry its coordination: the
            // Governor admits them like a user's turn, but paces their tool
            // calls as autonomous work (the Governor holds them for a pause
            // or a limit).
            coordination: kind == TurnKind::Message && rec.team.is_some(),
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
            self.parked_at_restart(&report.parked);
            // Injected before the restart and never acknowledged: delivered again.
            self.requeue_injected_unseen(None, "restart");
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
            resume_failed: false,
            start_fresh: false,
            spawned_at_ms: None,
            crash_restarts: vec![],
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

    /// FR-112: the project's files for this spawn, re-checked; one that now
    /// fails the checks is skipped and logged, never refusing the spawn.
    fn spawn_project_files(&self, id: &str, project: Option<&str>, home: &Path) -> Vec<SpawnFile> {
        let Some(id_p) = project else { return vec![] };
        let p = match self.db.project(id_p) {
            Ok(Some(p)) => p,
            Ok(None) => return vec![],
            Err(e) => {
                self.event(id, "project_file_skipped", json!({"project": id_p, "reason": format!("{e:#}")}));
                return vec![];
            }
        };
        let files = p.typed_settings().external_files;
        if files.is_empty() {
            return vec![];
        }
        // Without the other projects the bounds would miss their repositories
        // and worktrees: no files rather than unchecked ones.
        let bounds = match self.project_file_bounds(&p, home) {
            Ok(b) => b,
            Err(e) => {
                self.event(id, "project_file_skipped", json!({"project": p.id, "reason": format!("{e:#}")}));
                return vec![];
            }
        };
        let (out, skipped) = project_files::at_spawn(&files, &bounds);
        for why in skipped {
            self.event(id, "project_file_skipped", json!({"project": p.id, "reason": why}));
        }
        out
    }

    /// What `p`'s files may not touch: the same bounds when adding and at spawn.
    pub fn project_file_bounds(&self, p: &orch_store::Project, home: &Path) -> Result<project_files::Bounds> {
        let all = self.db.projects(true)?;
        Ok(project_files::Bounds::of(p, &all, home, &self.cfg.paths.root))
    }

    /// The live agents of `p` (FR-112: the Files section's note), matched
    /// like `agent_project` but without a query per agent.
    pub fn live_in_project(&self, p: &orch_store::Project) -> usize {
        let live = self.live_ids();
        self.store.read(|s| {
            live.iter()
                .filter_map(|id| s.agents.get(id))
                .filter(|rec| match &rec.team {
                    Some(t) => t.project_id == p.id,
                    None => rec.repo == p.path,
                })
                .count()
        })
    }

    /// Spawns `claude` for an existing agent: `--session-id` on first start,
    /// `--resume` afterwards (the default when `mode` is `None`).
    pub fn spawn(&self, id: &str, mode: Option<SessionMode>) -> Result<()> {
        if self.live_get(id).is_some() {
            bail!("agent {id} is already running");
        }
        self.refresh_team_role(id);
        let rec = self.store.read(|s| s.agents.get(id).cloned()).ok_or_else(|| anyhow!("no agent {id}"))?;
        if rec.tracker.state == AgentState::Archived {
            bail!("agent {id} is deleted");
        }
        let mode = mode.unwrap_or_else(|| SessionMode::Resume(rec.session_id.clone()));
        let resumed = matches!(mode, SessionMode::Resume(_));
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
        let project = self.agent_project(&rec);
        let project_files = self.spawn_project_files(id, project.as_deref(), &env.home);
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
            project_files,
        };
        // Registry refs and opted-in Keychain env values, for this spawn's
        // files only (FR-601, NFR-3); a ref that can't work refuses the spawn.
        let mut role = self.resolve_mcp(&rec.role, project.as_deref(), true)?;
        // FR-819: a downgrade in force runs the agent on its fallback model.
        role.model = self.turn_model(&rec);
        let compiled = compile(&role, &ctx);
        compiled.write()?;
        self.probe_in_background(project, rec.role.mcp_refs.clone());
        let weak = self.me.clone();
        let id_owned = id.to_string();
        // The pid is known only after the spawn; the exit callback needs it so a
        // late exit of an old process never touches a newer one.
        let pid_cell = Arc::new(std::sync::OnceLock::<i32>::new());
        let pid_for_exit = pid_cell.clone();
        let on_exit: Box<dyn FnOnce(Option<u32>) + Send> = Box::new(move |status| {
            if let (Some(rt), Some(pid)) = (weak.upgrade(), pid_for_exit.get()) {
                rt.on_exit(&id_owned, *pid, status);
            }
        });
        let pty = match &self.cfg.terminal {
            Some(t) => (t.0)(&compiled, on_exit)?,
            None => PtyProcess::spawn(&compiled, self.cfg.rows, self.cfg.cols, None, on_exit)?,
        };
        let (pid, pgid) = (pty.pid, pty.pgid);
        let _ = pid_cell.set(pid);
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id.into(),
            Arc::new(Live {
                pty,
                ready_at_ms: AtomicU64::new(0),
                last_user_input_ms: AtomicU64::new(0),
                model: role.model.clone(),
                task: AtomicU64::new(rec.team.as_ref().and_then(|t| t.task).unwrap_or(0)),
                pasted_at_ms: AtomicU64::new(0),
                resumed,
            }),
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

    pub(crate) fn on_exit(&self, id: &str, pid: i32, status: Option<u32>) {
        let gone = {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            if live.get(id).is_some_and(|l| l.pty.pid == pid) { live.remove(id) } else { None }
        };
        // Asked for (suspend, kill, delete) or not, and before SessionStart:
        // only an unexpected exit of a start is a failed start. Read before
        // `on_process_gone` resets them.
        let failed_start = self
            .store
            .write(|s| {
                let a = s.agents.get_mut(id)?;
                let failed = a.tracker.stop_intent == StopIntent::None && a.tracker.state == AgentState::Starting;
                if a.pid == Some(pid) {
                    a.on_process_gone();
                    // FR-415 decides about a failed start first: until then the
                    // pump must not resume it as a crash (§3.5), which would
                    // race the fresh restart.
                    if failed && a.flag.is_none() {
                        a.flag = Some(CHECKING_START.into());
                    }
                }
                Some(failed)
            })
            .ok()
            .flatten()
            .unwrap_or(false);
        let checked = || {
            let _ = self.store.write(|s| {
                if let Some(a) = s.agents.get_mut(id)
                    && a.flag.as_deref() == Some(CHECKING_START)
                {
                    a.flag = None;
                }
            });
        };
        // FR-203: a pasted turn the process never started goes back to the
        // queue (not on a late exit of an older process: the turns are a newer one's).
        if gone.is_some() {
            self.requeue_unstarted(id);
        }
        let releases = self.gov().turn_ended(&AgentId(id.into()), Timestamp::now());
        self.handle_releases(releases);
        self.event(id, "exited", json!({"status": status}));
        let Some(l) = gone else {
            checked();
            return;
        };
        if failed_start {
            // FR-415 owns a start that died: one fresh restart (not counted
            // as a crash restart), then the user (`resume_failed`).
            self.check_quick_exit(id, &l, status);
        }
        checked();
        // §3.5: a crash FR-415 left alone (a resume with a session, or an
        // exit after the start): the pump resumes it when it has work.
        if self.store.read(|s| s.agents.get(id).is_some_and(crate::startup::crashed)) {
            self.note_crash(id, &l);
            self.continue_after_crash(id);
        }
    }

    /// Stops the process and keeps the session (FR-204).
    pub fn suspend(&self, id: &str) -> Result<Vec<String>> {
        self.stop(id, StopIntent::Suspend)
    }

    /// Asks for a suspend and returns at once (§3.6): the stop waits up to
    /// 5 s for the exit, never on the pump or the team pass. The stop intent
    /// is set now, so the pump pastes nothing meanwhile and a second request
    /// is a no-op. True only when this call asked for the stop (a live
    /// agent with no stop under way).
    pub(crate) fn suspend_in_background(&self, id: &str, why: &'static str) -> bool {
        if self.live_get(id).is_none() {
            return false;
        }
        let first = self
            .store
            .write(|s| match s.agents.get_mut(id) {
                Some(a) if a.tracker.stop_intent == StopIntent::None => {
                    a.tracker.stop_intent = StopIntent::Suspend;
                    true
                }
                _ => false,
            })
            .unwrap_or(false);
        let Some(me) = self.me.upgrade().filter(|_| first) else { return first };
        let id = id.to_string();
        std::thread::spawn(move || {
            if let Err(e) = me.suspend(&id) {
                // Not stopped: the agent takes its work on its current process.
                let _ = me.store.write(|s| {
                    if let Some(a) = s.agents.get_mut(&id) {
                        a.tracker.stop_intent = StopIntent::None;
                    }
                });
                if why == "model_switch" {
                    me.house().switch_failed.insert(id.clone());
                }
                me.event(&id, "suspend_failed", json!({"why": why, "error": format!("{e:#}")}));
            }
        });
        true
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
            // A fake terminal has no signal to end it.
            l.pty.fake_exit(Some(143));
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
        // The user's resume of a crashed agent: its restart budget starts again.
        self.clear_crash(id, "user");
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
        self.unwatch_all();
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
        self.on_hook_reply(m, conn).0
    }

    /// [`Runtime::on_hook`], plus the team messages the output carries (to
    /// confirm once printed). Every state change is applied before it returns.
    pub fn on_hook_reply(&self, m: &HookMsg, conn: Option<&UnixStream>) -> (Option<HookOutput>, Vec<u64>) {
        if !m.acks.is_empty() {
            self.on_hook_ack(&m.agent_id, &m.acks);
        }
        // Events orch-hook spooled while the core was away come first, in order.
        self.replay_spool(&m.agent_id);
        self.apply_hook(m, conn, false)
    }

    /// One hook event. `replay`: a spooled event applied late, with no
    /// answer to give (no hold, no injection, no Stop block).
    pub(crate) fn apply_hook(
        &self,
        m: &HookMsg,
        conn: Option<&UnixStream>,
        replay: bool,
    ) -> (Option<HookOutput>, Vec<u64>) {
        let id = m.agent_id.as_str();
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else {
            self.event(id, "hook_unknown_agent", json!(m.event));
            // Fail closed: an agent the core doesn't know is not governed.
            return (if replay { None } else { HookOutput::fail_closed(&m.event) }, vec![]);
        };
        let mut p = HookPayload::parse(&m.payload);
        if m.event == HookEvent::UserPromptSubmit {
            self.bind_command(id, &mut p);
        }
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
        let mut detail = hook_event_detail(&m.event, &p);
        if replay {
            detail["replayed"] = json!(true);
        }
        self.event(id, m.event.as_str(), detail);
        // Before any state change: a blocked Stop keeps the turn running.
        if m.event == HookEvent::Stop
            && !replay
            && let Some(out) = self.stop_check(&rec, &p)
        {
            self.event(id, "stop_blocked", json!({"reason": out.0["reason"]}));
            return (Some(out), vec![]);
        }
        // §3.4: a long message's file read in a tool call (Read, `cat`).
        if matches!(m.event, HookEvent::PostToolUse | HookEvent::PostToolUseFailure) {
            self.note_referenced(id, &p);
        }
        // Applied under the store lock: concurrent hooks (parallel tool calls)
        // must not overwrite each other's transitions.
        let (tracker, effects) = self
            .store
            .write(|s| {
                let a = s.agents.get_mut(id)?;
                let effects = state::apply(&mut a.tracker, &m.event, &p);
                let now = now_ms();
                // The call's start and short input: the record's activity, and
                // an MCP call running long is an external wait (P4.1).
                if m.event == HookEvent::PreToolUse {
                    let tool = p.tool_name.as_deref().unwrap_or("");
                    let input = p.tool_input.as_ref().unwrap_or(&Value::Null);
                    a.tracker.tool_started_ms = Some(now);
                    a.tracker.tool_brief =
                        crate::inbox::main_arg(tool, input).map(|b| crate::attention::copy::cut(&b, 60));
                } else if a.tracker.current_tool.is_none() {
                    a.tracker.tool_started_ms = None;
                    a.tracker.tool_brief = None;
                }
                a.last_event = Some(m.event.to_string());
                a.last_event_at_ms = Some(now);
                Some((a.tracker.clone(), effects))
            })
            .ok()
            .flatten()
            .unwrap_or_else(|| (rec.tracker.clone(), vec![]));
        for fx in effects {
            self.on_effect(&rec, fx, &p);
        }
        // FR-604: the agents' calls to registry servers tell their auth status.
        self.observe_mcp_call(&rec, &m.event, &p);
        // A pane showing this worktree (the Diff) refetches.
        if m.event == HookEvent::PostToolUse
            && matches!(p.tool_name.as_deref(), Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Bash"))
        {
            self.touch_worktree(&rec.worktree);
        }
        // A solo session's finished turn is an outcome of the digest (D8):
        // its last assistant message, not the prompt it was sent.
        if m.event == HookEvent::Stop
            && rec.team.is_none()
            && let Some(msg) = p.last_assistant_message.as_deref().filter(|t| !t.trim().is_empty())
        {
            self.event(id, "turn.done", json!({"summary": crate::team::cut(msg, 280)}));
        }
        if matches!(m.event, HookEvent::Stop | HookEvent::StopFailure) {
            self.suspend_if_parked(id);
        }
        if replay {
            return (None, vec![]);
        }
        let out = match m.event {
            HookEvent::PreToolUse => self
                .plan_lock(&rec, p.tool_name.as_deref().unwrap_or(""))
                .or_else(|| self.forward_ask_user_question(&rec, &p))
                .or_else(|| {
                    self.admit_tool(&rec, tracker.turn_origin == Some(TurnOrigin::User), &p, m.deadline_ms, conn)
                }),
            // A rule, or the Inbox (held), or the pane when nobody answers in time.
            HookEvent::PermissionRequest => self.permission_request(&rec, &p, m.deadline_ms, conn),
            // FR-403: a busy team agent gets its messages at its next tool
            // call, never inside a subagent's (they wait for its own).
            HookEvent::PostToolUse | HookEvent::PostToolUseFailure
                if rec.team.is_some() && !crate::handover::in_subagent(&p) =>
            {
                let injected = self.inject_messages(&rec, tracker.prompt_id.as_deref());
                return injected
                    .map(|(t, ids)| (Some(HookOutput::additional_context(&m.event, &t)), ids))
                    .unwrap_or((None, vec![]));
            }
            _ => None,
        };
        (out, vec![])
    }

    fn on_effect(&self, rec: &AgentRecord, fx: Effect, p: &HookPayload) {
        let id = rec.id.as_str();
        let now = now_ms();
        let live = self.live_get(id);
        match fx {
            Effect::Ready => {
                self.started(id);
                if let Some(l) = &live {
                    l.pty.output.clear_trust_dialog();
                    l.ready_at_ms.store(now + delivery::AFTER_SESSION_START.as_millis() as u64, Ordering::SeqCst);
                }
            }
            Effect::TurnStarted { origin, markers } => {
                self.end_wait_episode(id);
                self.answers_resumed(id);
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
                // Idle either way (§3.2): with background work, only its
                // tasks and continues wait for it.
                self.end_turn(rec, live.as_deref(), now);
            }
            Effect::RateLimited { message } => {
                let model_limit =
                    self.usage.lock().unwrap_or_else(|e| e.into_inner()).exhausted_model_window(id, message.as_deref());
                if let Some((name, resets)) = model_limit {
                    // A per-model weekly limit (FR-815): only that model's turns wait.
                    let now_ts = Timestamp::now();
                    let resets_at = Timestamp::from_second(resets).unwrap_or(now_ts);
                    let releases = {
                        let mut g = self.gov();
                        g.model_limit_hit(&name, resets_at, now_ts);
                        g.turn_ended(&AgentId(id.into()), now_ts)
                    };
                    self.handle_releases(releases);
                    self.event(
                        id,
                        "rate_limited",
                        json!({"model_window": name, "resets_at": resets, "message": message}),
                    );
                    return;
                }
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
                // `alert: api_error` puts the agent in error (attention rule 1).
                let payload = json!({"alert": "api_error", "error": error});
                let _ = self.add_item(Some(id), "alert", format!("{id}: API error {error}"), payload, 0);
                // §3.2: the process lives on, idle; its turn is over.
                let ended = self.end_turn(rec, live.as_deref(), now);
                self.continue_after_failure(rec, &ended, &error);
            }
            Effect::NeedsUser => {}
        }
    }

    /// The agent's turn is over (Stop, or a StopFailure it lives through):
    /// its delivered turns are done, its messages settled (§3.4), input may
    /// go out again after [`delivery::AFTER_STOP`], and the Governor is told.
    /// The turns that ended.
    fn end_turn(&self, rec: &AgentRecord, live: Option<&Live>, now: u64) -> Vec<TurnRow> {
        let id = rec.id.as_str();
        self.house().index_dirty.insert(id.to_string());
        // A command whose prompt never reached a hook ends with the turn too.
        let delivered = |t: &TurnRow| {
            t.agent_id == id && (t.status == TurnStatus::Delivered || t.command && t.status == TurnStatus::Delivering)
        };
        self.house().asked.remove(id);
        let ended = self
            .store
            .write(|s| {
                let mut ended: Vec<TurnRow> = vec![];
                for turn in s.turns.iter_mut() {
                    if delivered(turn) {
                        turn.status = TurnStatus::Done;
                        ended.push(turn.clone());
                    } else if turn.agent_id == id && turn.status == TurnStatus::Delivering {
                        // A paste still unacknowledged after another
                        // turn: its watchdog clock starts again now.
                        turn.delivered_at_ms = Some(now);
                    }
                }
                ended
            })
            .unwrap_or_default();
        // §3.4: read only when fully seen; injected but never seen → queued.
        self.settle_messages(rec, &ended, now);
        if let Some(l) = live {
            l.ready_at_ms.store(now + delivery::AFTER_STOP.as_millis() as u64, Ordering::SeqCst);
        }
        let releases = self.gov().turn_ended(&AgentId(id.into()), Timestamp::now());
        self.handle_releases(releases);
        ended
    }

    /// §3.2: a turn that failed (StopFailure, not a rate limit) is followed by
    /// one Continue turn, unless one is queued already. When the failed turn
    /// was that Continue, nothing is queued: [`Runtime::on_repeated_failure`].
    fn continue_after_failure(&self, rec: &AgentRecord, ended: &[TurnRow], error: &str) {
        let id = rec.id.as_str();
        if ended.iter().any(|t| t.after_failure) {
            self.on_repeated_failure(rec, error);
            return;
        }
        let queued = self.store.write(|s| {
            let has = s
                .turns
                .iter()
                .any(|t| t.agent_id == id && t.status == TurnStatus::Queued && t.kind == TurnKind::Continue);
            if !has {
                let text = format!("ORCH: your last turn failed ({error}). Carry on, or report where you are.");
                let mut t = TurnRow::queued(id, &text, TurnKind::Continue, Admission::Pending);
                t.after_failure = true;
                s.turns.push(t);
            }
            !has
        });
        if queued.unwrap_or(false) {
            self.delivery_event(
                id,
                "recovered",
                json!({"agent": id, "from_state": "error", "how": "continue", "error": error}),
            );
        }
    }

    /// §3.5 (F3): a process that died in the middle of a turn leaves work that
    /// nobody asks for again, and with nothing queued the pump never restarts
    /// it. The interrupted turn is closed as done and one Continue turn is
    /// queued, which the crash restart then carries to the new process.
    pub(crate) fn continue_after_crash(&self, id: &str) {
        let queued = self.store.write(|s| {
            let mut interrupted = false;
            for t in s.turns.iter_mut().filter(|t| t.agent_id == id && t.status == TurnStatus::Delivered) {
                t.status = TurnStatus::Done;
                interrupted = true;
            }
            let has = s
                .turns
                .iter()
                .any(|t| t.agent_id == id && t.status == TurnStatus::Queued && t.kind == TurnKind::Continue);
            if interrupted && !has {
                let text = "ORCH: your session crashed mid-turn and was resumed. Check where you were and carry on.";
                s.turns.push(TurnRow::queued(id, text, TurnKind::Continue, Admission::Pending));
            }
            interrupted && !has
        });
        if queued.unwrap_or(false) {
            self.delivery_event(id, "recovered", json!({"agent": id, "from_state": "crashed", "how": "continue"}));
        }
    }

    /// The Continue turn queued after a failure failed too: no loop. The
    /// event, and a `delivery` decision for the user (and the agent's Lead),
    /// beside the `api_error` alert the failure already raised.
    pub(crate) fn on_repeated_failure(&self, rec: &AgentRecord, error: &str) {
        self.delivery_event(&rec.id, "continue_failed", json!({"agent": rec.id, "error": error}));
        // One decision while it is open: every further failed Continue is the same episode.
        let open = self.store.read(|s| {
            s.inbox.iter().any(|i| {
                i.kind == "delivery"
                    && i.open()
                    && i.agent_id.as_deref() == Some(rec.id.as_str())
                    && i.payload["blocker"] == "not_idle:error"
            })
        });
        if open {
            return;
        }
        let now = now_ms();
        self.raise_delivery(
            rec,
            crate::stuck::Raise {
                episode: "delivery",
                blocker: "not_idle:error".into(),
                since_ms: now,
                messages: vec![],
                turns: vec![],
                key: format!("{}:repeated_failure:{now}", rec.id),
                stuck_event: true,
            },
        );
    }

    /// Marks the turns `markers` delivered and, for a turn that started at
    /// `started`, records its start (one store write).
    pub(crate) fn mark_delivered(&self, id: &str, markers: &[String], started: Option<u64>) {
        if markers.is_empty() && started.is_none() {
            return;
        }
        let seen = self
            .store
            .write(|s| {
                let mut seen: Vec<(String, Vec<u64>)> = vec![];
                for t in s.turns.iter_mut().filter(|t| t.agent_id == id && markers.contains(&t.id)) {
                    if matches!(t.status, TurnStatus::Queued | TurnStatus::Delivering) {
                        t.status = TurnStatus::Delivered;
                        if !t.messages.is_empty() {
                            seen.push((t.id.clone(), t.messages.clone()));
                        }
                    }
                }
                if let Some(now) = started
                    && let Some(a) = s.agents.get_mut(id)
                {
                    a.tracker.turn_started_ms = Some(now);
                }
                seen
            })
            .unwrap_or_default();
        // The CLI took the paste: its messages are seen.
        for (turn, msgs) in seen {
            let _ = self.db.see_messages(&msgs, now_ms());
            self.delivery_event(id, "seen", json!({"messages": msgs, "via": "turn", "turn": turn}));
        }
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
            // §3.2: released past its limit, the agent takes input again.
            self.limit_lifted(&id, "release");
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
                    // Its own turn first, then by class (§3.5), then queue order.
                    .min_by_key(|t| {
                        (t.admission != Admission::Waiting(entry), delivery::class_rank(t.kind), t.created_at_ms)
                    });
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

    /// §3.2: an agent blocked by a limit is idle again (the Governor
    /// released it, or no limit holds it any more): its waiting turns are
    /// pasted as usual. No-op for any other state.
    pub(crate) fn limit_lifted(&self, id: &str, how: &str) {
        let blocked = |s: &crate::store::StateFile| {
            s.agents.get(id).is_some_and(|a| a.tracker.state == AgentState::BlockedByLimit)
        };
        // A read first: most releases are for agents that are not blocked.
        if !self.store.read(blocked) {
            return;
        }
        let lifted = self
            .store
            .write(|s| match s.agents.get_mut(id) {
                Some(a) if a.tracker.state == AgentState::BlockedByLimit => {
                    a.tracker.state = AgentState::Idle;
                    true
                }
                _ => false,
            })
            .unwrap_or(false);
        if !lifted {
            return;
        }
        if let Some(l) = self.live_get(id) {
            l.ready_at_ms.store(now_ms(), Ordering::SeqCst);
        }
        self.delivery_event(id, "recovered", json!({"agent": id, "from_state": "blocked_by_limit", "how": how}));
    }

    /// Housekeeping: agents blocked by a limit no longer in force (no release
    /// comes for one the Governor holds no entry for: interactive, shadow
    /// mode, or its auto-continue allowed).
    pub fn lift_limits(&self, now: u64) {
        // A minute after the hit at least: a limit the Governor could not
        // place (a model it doesn't know) must not bounce every pass.
        let blocked: Vec<String> = self.store.read(|s| {
            s.agents
                .values()
                .filter(|a| a.tracker.state == AgentState::BlockedByLimit)
                .filter(|a| now.saturating_sub(a.last_event_at_ms.unwrap_or(0)) >= LIMIT_GRACE_MS)
                .map(|a| a.id.clone())
                .collect()
        });
        for id in blocked {
            let agent = AgentId(id.clone());
            if !self.with_governor(|g| g.limit_in_force(&agent, Timestamp::now())) {
                self.limit_lifted(&id, "limit_over");
            }
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
            g.forget_agent(&agent);
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
            self.touch(orch_protocol::Topic::Usage);
            self.event(&m.agent_id, "usage_sample", serde_json::to_value(&g).unwrap_or(Value::Null));
            let allocation = self.store.read(|s| s.agents.get(&m.agent_id).map(|a| a.allocation.clone()));
            let _ = self.advance_week(
                g.ts,
                |gov| {
                    let class = gov.cost_class(&g);
                    self.db.add_usage(usage_record(&g, &sample, class, allocation));
                    // FR-802: the plan week is aligned to the real weekly reset.
                    let w = g.weekly.filter(|w| !governor::same_reset(w.resets_at, gov.plan().reset_at))?;
                    (w.resets_at > g.ts).then(|| {
                        let mut plan = gov.plan().clone();
                        plan.reset_at = w.resets_at;
                        plan.week_start = w.resets_at.checked_sub(168.hours()).unwrap_or(w.resets_at);
                        plan.id = format!("week-{}", plan.week_start.as_second());
                        plan
                    })
                },
                |gov| gov.ingest(&g),
            );
        }
    }

    // ------------------------------------------------------------ loops

    /// The Governor's control loop (call every ~30 s). Also persists its
    /// recent actions, its queue, and (less often) forecast snapshots and the
    /// calibration.
    pub fn tick(&self) {
        let now = Timestamp::now();
        // The week ended with no reading of the next one yet: the next plan
        // week starts at the reset (a later reading realigns it).
        let next = |g: &mut Governor| {
            (now >= g.plan().reset_at).then(|| {
                let mut next = g.plan().clone();
                next.week_start = next.reset_at;
                next.reset_at = next.reset_at.checked_add(168.hours()).unwrap_or(next.reset_at);
                next.id = format!("week-{}", next.week_start.as_second());
                next
            })
        };
        if let Err(e) = self.advance_week(now, next, |_| {}) {
            self.event("-", "week_switch_failed", json!(e.to_string()));
        }
        self.governor_act(|g, now| ((), g.tick(now)));
        self.reserve_alert();
        let (actions, queue, plan_id, snap, calib, est, week_start) = {
            let g = self.gov();
            let n = g.actions().len();
            (
                g.actions()[n.saturating_sub(50)..].to_vec(),
                g.queue().to_vec(),
                g.plan().id.clone(),
                g.snapshot(now),
                g.calibration(now),
                g.estimator(),
                g.plan().week_start,
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
            let fit_changed = h.last_fit.as_ref() != Some(&est.calibrator);
            (
                due(&mut h.last_forecast_ms, ms, FORECAST_EVERY_MS),
                fit_changed && due(&mut h.last_calibration_ms, ms, CALIBRATION_EVERY_MS),
            )
        };
        if do_forecast {
            let _ = self.db.add_forecast(&snap);
        }
        if do_calib && self.db.save_estimator("fit", week_start, &calib, &est).is_ok() {
            self.house().last_fit = Some(est.calibrator);
        }
        // Time-based reasons and waits ("≈14:20", an MCP call past 5 min).
        self.now_pump(true);
    }

    /// Under the Governor's lock: `next` says which plan to move to (if any),
    /// the switch happens, then `after` runs (also when the switch failed).
    /// With the lock released, the week that ended gets its report (FR-818;
    /// it reads a whole week of the store).
    pub(crate) fn advance_week(
        &self,
        now: Timestamp,
        next: impl FnOnce(&mut Governor) -> Option<WeeklyPlan>,
        after: impl FnOnce(&mut Governor),
    ) -> Result<()> {
        let (switched, ended) = {
            let mut gov = self.gov();
            let switched = match next(&mut gov) {
                Some(plan) => self.switch_week(&mut gov, plan, now),
                None => Ok(None),
            };
            after(&mut gov);
            let ended = switched.as_ref().ok().cloned().flatten();
            (switched.map(|_| ()), ended)
        };
        if let Some((plan, calib)) = ended {
            if let Err(e) = self.finish_week(&plan, &calib, now) {
                self.event("-", "weekly_report_failed", json!(e.to_string()));
            }
        }
        switched
    }

    /// Moves the Governor to `plan` and stores it. A new plan week stores
    /// the estimator it starts with (FR-814: a restart restores it and
    /// replays the week). Returns the week that ended, if one did, with its
    /// calibration (for [`Runtime::advance_week`]'s report). A plan realigned
    /// within its week (FR-802) ends none.
    fn switch_week(
        &self,
        gov: &mut Governor,
        plan: WeeklyPlan,
        now: Timestamp,
    ) -> Result<Option<(WeeklyPlan, governor::Calibration)>> {
        let old = gov.plan().clone();
        let ended = plan.week_start.as_second() >= old.reset_at.as_second() - governor::SAME_RESET_S;
        let calib = gov.calibration(now);
        if ended {
            // The week's last actions, for its report.
            let n = gov.actions().len();
            let _ = self.db.save_actions(
                self.boot_ms.load(Ordering::SeqCst),
                &old.id,
                &gov.actions()[n.saturating_sub(200)..],
            );
        }
        gov.new_week(plan.clone(), now).map_err(|e| anyhow!("{e:?}"))?;
        self.db.save_plan(&plan, orch_store::PlanStatus::Active)?;
        if gov.plan().week_start != old.week_start {
            let base = gov.week_estimator().clone();
            self.db.save_estimator("week", plan.week_start, &base.calibrator.snapshot(now), &base)?;
        }
        Ok(ended.then_some((old, calib)))
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

    pub(crate) fn house(&self) -> std::sync::MutexGuard<'_, House> {
        self.house.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// FR-203: the live-agent cap now (base, lowered under memory pressure).
    pub fn live_cap(&self) -> u32 {
        resources::effective_cap(self.base_cap(), self.memory_pressure())
    }

    /// FR-203: the base live cap in effect (the setting, `--cap`, else 3).
    pub fn base_cap(&self) -> u32 {
        self.base_cap.load(Ordering::SeqCst)
    }

    pub(crate) fn set_base_cap(&self, cap: u32) {
        self.base_cap.store(cap, Ordering::SeqCst);
    }

    /// FR-203: the memory pressure level that sets the cap (with hysteresis).
    pub fn memory_pressure(&self) -> resources::PressureLevel {
        self.house().pressure.effective
    }

    /// The Week planner's confirmed plan (FR-802): replaces the current one.
    pub fn set_plan(&self, plan: WeeklyPlan) -> Result<()> {
        self.advance_week(Timestamp::now(), |_| Some(plan), |_| {})
    }

    /// Changes the stored pacing policy (the Week planner's options). It
    /// applies at the next Governor rebuild ([`Runtime::replan`]).
    pub fn update_policy(&self, f: impl FnOnce(&mut governor::PacingPolicy)) -> Result<()> {
        let mut p = self.db.policy(POLICY_ID)?.unwrap_or_default();
        f(&mut p);
        self.db.save_policy(POLICY_ID, &p)
    }

    /// A plan edited during its week (the first plan, J0; shadow mode on or
    /// off, FR-812): stored, then the Governor is rebuilt from it and this
    /// week's samples are replayed, so the week's accounting is kept (unlike
    /// `set_plan`, which starts a new week). The queue, pauses, deferrals, the
    /// ask and sprints carry over (`Governor::carry`); a sprint keeps its boost
    /// on the new curve.
    pub fn replan(&self, plan: WeeklyPlan) -> Result<()> {
        self.rebuild_governor(Some(plan))
    }

    /// [`Runtime::replan`] on `plan`, or on the current plan (`None`: the
    /// policy or the base cap changed), read under the Governor's lock.
    pub(crate) fn rebuild_governor(&self, plan: Option<WeeklyPlan>) -> Result<()> {
        let now = Timestamp::now();
        let released = {
            // Held through the rebuild: a turn queued meanwhile on the old
            // Governor would miss the carry and wait on an entry that is gone.
            let mut old = self.gov();
            let plan = plan.unwrap_or_else(|| old.plan().clone());
            if plan.reset_at <= now || plan.week_start > now {
                bail!("the plan does not cover now");
            }
            let carry = old.carry();
            self.db.save_plan(&plan, orch_store::PlanStatus::Active)?;
            let (gov, released) = restore_governor(
                &self.db,
                &self.store,
                &self.cfg,
                self.base_cap(),
                plan.shadow_mode,
                Some(carry),
                now,
            )?;
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
        if due(&mut self.house().last_mcp_probe_ms, now, crate::registry::PROBE_EVERY_MS) {
            self.probe_used_servers();
        }
        self.power_tick(false);
        self.notion_tick();
        let raw = self.cfg.memory.level();
        let (changed, level) = {
            let mut h = self.house();
            let c = h.pressure.feed(raw, now);
            (c.is_some(), h.pressure.effective)
        };
        if changed {
            self.event("-", "memory_pressure", json!({"raw": raw, "effective": level, "cap": self.live_cap()}));
        }
        self.sync_process_slots();
        self.lift_limits(now);
        // One pass: queued turns per agent, agents with a pasted turn, and
        // agents with an open `startup` item (both kept live, FR-203/415).
        let (queued, delivering, startup_open) = self.store.read(|s| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            let mut delivering = std::collections::BTreeSet::new();
            for t in &s.turns {
                match t.status {
                    TurnStatus::Queued => *m.entry(t.agent_id.clone()).or_default() += 1,
                    TurnStatus::Delivering => {
                        delivering.insert(t.agent_id.clone());
                    }
                    _ => {}
                }
            }
            let startup: std::collections::BTreeSet<String> =
                s.inbox.iter().filter(|i| i.open() && i.kind == "startup").filter_map(|i| i.agent_id.clone()).collect();
            (m, delivering, startup)
        });
        // A lead's prompt cache: kept alive until it goes cold (FR-204).
        let warm: BTreeMap<String, u64> = {
            let ids = self.live_ids();
            // Only a lead's is used.
            let ids: Vec<String> =
                self.store.read(|s| ids.into_iter().filter(|id| s.agents.get(id).is_some_and(reserved_slot)).collect());
            let u = self.usage.lock().unwrap_or_else(|e| e.into_inner());
            ids.into_iter()
                .filter_map(|id| {
                    let t = u.agent(&id)?.prompt_cache_expires_at?;
                    Some((id, (t.max(0) as u64) * 1000))
                })
                .collect()
        };
        // A snapshot: no store lock is taken while holding `live`.
        let procs: Vec<(String, Arc<Live>)> =
            self.live.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let live: Vec<LiveAgent> = procs
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
                    l.pasted_at_ms.load(Ordering::SeqCst),
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
                    warm_until_ms: warm.get(id).copied().unwrap_or(0),
                    delivering: delivering.contains(id),
                    startup_blocked: startup_open.contains(id),
                })
            })
            .collect();
        self.watch_startups(now);
        let idle_ms = self.cfg.idle_suspend.map(|d| d.as_millis() as u64);
        let suspend = resources::to_suspend(&live, self.live_cap(), idle_ms, now);
        let freeing = !suspend.is_empty();
        for (id, why) in suspend {
            let why = if why == resources::OVER_CAP { resources::over_cap_reason(level) } else { why };
            self.event(&id, "auto_suspend", json!({"why": why, "pressure": level}));
            if let Some(me) = self.me.upgrade() {
                // Suspending waits for the exit: never on the caller's loop.
                std::thread::spawn(move || {
                    let _ = me.suspend(&id);
                });
            }
        }
        // §3.5 (F6): no slot freed above, and a member's messages wait for one.
        if !freeing {
            self.free_slot_for_messages(&live, self.live_cap(), now);
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
        // §3.1/§3.3: the paste watchdog and the live spool replay (reads
        // transcripts): never on the delivery loop.
        if !self.watching.swap(true, Ordering::SeqCst) {
            if let Some(me) = self.me.upgrade() {
                std::thread::spawn(move || {
                    // A panic never leaves the guard set (the watchdog would stop for good).
                    me.tick_step("-", "handover", || me.handover_tick(now_ms()));
                    me.watching.store(false, Ordering::SeqCst);
                });
            } else {
                self.watching.store(false, Ordering::SeqCst);
            }
        }
        // Teams: dispatch, messages, run progress. Each run and step under
        // its own panic guard, and the `teaming` guard always released (§3.6).
        if !self.teaming.swap(true, Ordering::SeqCst) {
            if let Some(me) = self.me.upgrade() {
                std::thread::spawn(move || {
                    me.tick_step("-", "team_tick", || me.team_tick());
                    me.tick_step("-", "watchdog", || me.watchdog(now_ms()));
                    me.teaming.store(false, Ordering::SeqCst);
                });
            } else {
                self.teaming.store(false, Ordering::SeqCst);
            }
        }
        // Task backends (GitHub/Linear network calls): their own thread and
        // guard, off the delivery path (§3.6).
        if !self.syncing.load(Ordering::SeqCst) && self.task_sync_due() && !self.syncing.swap(true, Ordering::SeqCst) {
            if let Some(me) = self.me.upgrade() {
                std::thread::spawn(move || {
                    me.tick_step("-", "task_sync", || me.sync_task_backends());
                    me.syncing.store(false, Ordering::SeqCst);
                });
            } else {
                self.syncing.store(false, Ordering::SeqCst);
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
                });
                // The Inbox card is synthesized from `trust_dialog` (D6): nothing stored.
                self.event(&id, "trust_dialog", json!({"seen": seen}));
            }
        }
        // Every agent's queued turns, in queue order.
        let queued: Vec<(AgentRecord, Vec<TurnRow>)> = self.store.read(|s| {
            let mut out: BTreeMap<String, Vec<TurnRow>> = BTreeMap::new();
            for t in s.turns.iter().filter(|t| t.status == TurnStatus::Queued) {
                out.entry(t.agent_id.clone()).or_default().push(t.clone());
            }
            out.into_iter().filter_map(|(id, turns)| Some((s.agents.get(&id)?.clone(), turns))).collect()
        });
        // The Governor's queue, read once for every agent's waiting turn.
        let entries: std::collections::BTreeSet<u64> =
            if queued.iter().flat_map(|(_, ts)| ts).any(|t| matches!(t.admission, Admission::Waiting(_))) {
                self.with_governor(|g| g.queue().iter().map(|q| q.id.0).collect())
            } else {
                Default::default()
            };
        for (rec, turns) in queued {
            // An entry gone without a release (dropped for a critical turn):
            // ask again rather than wait forever (next pass). A release that
            // comes meanwhile admits the turn anyway.
            let mut stale = vec![];
            for t in &turns {
                if let Admission::Waiting(e) = t.admission
                    && !entries.contains(&e)
                {
                    stale.push((t.id.clone(), e));
                }
            }
            if !stale.is_empty() {
                let _ = self.store.write(|s| {
                    for (id, e) in &stale {
                        let t = s.turns.iter_mut().find(|t| &t.id == id && t.admission == Admission::Waiting(*e));
                        if let Some(t) = t {
                            t.admission = Admission::Pending;
                        }
                    }
                });
                for (turn, entry) in &stale {
                    self.event(&rec.id, "stale_admission", json!({"turn": turn, "entry": entry}));
                }
            }
            // §3.5 (F5): the first turn by priority that can move (a stale
            // turn is still `Waiting` in this snapshot: skipped this pass).
            let Some(turn) = delivery::pick_turn(&turns, rec.tracker.background_tasks) else {
                continue;
            };
            if let Err(e) = self.pump_one(&rec, turn, now) {
                self.event(&rec.id, "delivery_error", json!(e.to_string()));
            }
        }
        self.flush_watches();
        self.now_pump(false);
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
        // An answer sent to the agent now waits in the Governor's queue.
        if let Verdict::Queue { estimated_start, .. } = &d.effective
            && matches!(admission, Admission::Waiting(_))
        {
            let now = now_ms();
            let eta = estimated_start.map(|t| {
                let tz = governor::jiff::tz::TimeZone::system();
                format!("≈{}", crate::attention::clock(t.as_millisecond().max(0) as u64, now, &tz))
            });
            self.answers_queued(&rec.id, eta);
        }
        admission == Admission::Admitted
    }

    /// One agent's turn picked by [`delivery::pick_turn`]: admission, a
    /// resume or a restart when the process is gone, then the paste.
    fn pump_one(&self, rec: &AgentRecord, turn: &TurnRow, now: u64) -> Result<()> {
        // FR-819: a new task is where the model may change (never mid-turn).
        if turn.kind == TurnKind::Task && turn.admission == Admission::Pending && self.task_boundary(rec) {
            return Ok(());
        }
        let Some(l) = self.live_get(&rec.id) else {
            // A suspended agent with work is resumed through the Governor
            // (FR-403), which also holds the spawn until a process slot is
            // free under memory pressure (FR-203).
            // A team agent that never ran starts the same way (with its own
            // session id) once its first turn is admitted.
            let never_started = rec.never_started();
            // §3.5 (F3): a crashed agent with work resumes the same way, at
            // most twice an hour; then a `startup` item waits for the user.
            // The user's own words resume a crashed agent whose budget is used up.
            let by_user = turn.kind == TurnKind::User;
            let crashed = crate::startup::crashed(rec) && (by_user || self.startup_item(&rec.id).is_none());
            // FR-705: parked on the user's decision, only their own words resume it.
            let suspended = (rec.tracker.state == AgentState::Suspended || never_started || crashed)
                && rec.flag.is_none()
                && (turn.kind == TurnKind::User || !self.is_parked(&rec.id));
            if suspended && crashed {
                if by_user {
                    self.clear_crash(&rec.id, "user");
                } else if !self.crash_restart(rec, now) {
                    return Ok(());
                }
            }
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
                // A first start, or a fresh session queued by the lead (FR-415).
                let new = never_started || rec.start_fresh;
                if let Err(e) = self.spawn(&rec.id, new.then(|| SessionMode::New(rec.session_id.clone()))) {
                    // Flag it so the next pump doesn't retry every 200 ms.
                    let msg = format!("{}: could not resume: {e:#}", rec.id);
                    self.store.write(|s| {
                        if let Some(a) = s.agents.get_mut(&rec.id) {
                            a.flag = Some(msg.clone());
                        }
                        let payload = json!({"alert": "spawn_failed", "error": format!("{e:#}"),
                                             "cmd": self.cfg.claude.display().to_string()});
                        s.add_inbox_item(Some(&rec.id), "alert", msg, payload, 0);
                    })?;
                    return Err(e);
                }
                if crashed {
                    self.crash_restarted(&rec.id, now);
                }
            }
            return Ok(());
        };
        // A stop under way (a model switch, a fresh session for the next
        // task): the turn goes to the next process.
        if rec.tracker.stop_intent != StopIntent::None {
            return Ok(());
        }
        let gate = GateInputs {
            live: !l.pty.has_exited(),
            idle: rec.tracker.state == AgentState::Idle,
            trust_dialog: rec.trust_dialog,
            background_tasks: rec.tracker.background_tasks,
            waits_on_background: delivery::waits_on_background(turn.kind),
            ready_at_ms: Some(l.ready_at_ms.load(Ordering::SeqCst)).filter(|t| *t > 0),
            last_user_input_ms: Some(l.last_user_input_ms.load(Ordering::SeqCst)).filter(|t| *t > 0),
            now_ms: now,
        };
        if delivery::can_deliver(&gate).is_err() || !self.admit_turn(rec, turn, false) {
            return Ok(());
        }
        // The agent's next queued turns ride along (one turn, one paste), up
        // to a slash command, which is typed alone and as is. Each one is
        // admitted first: a pause or a budget wait holds it like any turn.
        let (batch, prepared) = if turn.command {
            let text = turn.body.trim().to_string();
            (vec![turn.clone()], delivery::Prepared { turn_id: turn.id.clone(), text, file: None })
        } else {
            // The others in priority order (§3.5), up to a command, one held by
            // the Governor, one waiting for background work, or a new task
            // (its model is settled when it is picked itself, FR-819).
            let bg = rec.tracker.background_tasks;
            let next: Vec<TurnRow> = self.store.read(|s| {
                let mine = s.turns.iter().filter(|t| t.agent_id == rec.id && t.status == TurnStatus::Queued);
                delivery::by_priority(mine)
                    .into_iter()
                    .filter(|t| t.id != turn.id)
                    .take_while(|t| {
                        !t.command
                            && delivery::can_move(t, bg)
                            && !(t.kind == TurnKind::Task && t.admission == Admission::Pending)
                    })
                    .cloned()
                    .collect()
            });
            let mut batch = vec![turn.clone()];
            for t in next {
                if !self.admit_turn(rec, &t, false) {
                    break;
                }
                batch.push(t);
            }
            let parts: Vec<(&str, &str)> = batch.iter().map(|t| (t.id.as_str(), t.body.as_str())).collect();
            let prepared = delivery::prepare_many(&parts, &self.cfg.paths.agent_dir(&rec.id));
            (batch, prepared)
        };
        if let Some((path, body)) = &prepared.file {
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))?;
            std::fs::write(path, body)?;
        }
        // §3.1: a turn pasted before that the CLI did take is never typed twice.
        if batch.iter().any(|t| t.attempts > 0) && self.already_in_transcript(rec, &batch) {
            return Ok(());
        }
        let ids: Vec<&str> = batch.iter().map(|t| t.id.as_str()).collect();
        let file = prepared.file.as_ref().and_then(|f| f.0.file_name()).map(|n| n.to_string_lossy().into_owned());
        // Claimed in one write, only if every turn is still queued: one an
        // injection dropped meanwhile (its messages given at a tool call,
        // §3.2) is never pasted from this pass's snapshot.
        let claimed = self.store.write(|s| {
            let queued = |id: &str| s.turns.iter().any(|t| t.id == id && t.status == TurnStatus::Queued);
            if !ids.iter().all(|id| queued(id)) {
                return false;
            }
            for t in s.turns.iter_mut().filter(|t| ids.contains(&t.id.as_str())) {
                t.status = TurnStatus::Delivering;
                t.delivered_at_ms = Some(now);
                t.enter_retried = false;
                t.file = file.clone();
            }
            true
        })?;
        if !claimed {
            return Ok(());
        }
        l.ready_at_ms.store(0, Ordering::SeqCst);
        // FR-206: team messages are handed at the paste, seen when its prompt
        // starts. Handed before the Enter: a fast UserPromptSubmit finds them.
        for t in batch.iter().filter(|t| !t.messages.is_empty()) {
            let _ = self.db.hand_messages(&t.messages, &rec.id, "turn", Some(&t.id), now);
            self.delivery_event(&rec.id, "handed", json!({"messages": t.messages, "via": "turn", "turn": t.id}));
        }
        l.pty.write(&prepared.paste_bytes())?;
        std::thread::sleep(delivery::PASTE_TO_ENTER);
        l.pty.write(b"\r")?;
        l.pasted_at_ms.store(now, Ordering::SeqCst);
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
            self.pane_write(id, &l, initial)?;
        }
        let mut input = stream.try_clone()?;
        let mut out = stream;
        // Scoped: the reader borrows the runtime and the process, and is
        // joined before `attach` returns.
        std::thread::scope(|sc| {
            let l = &l;
            let reader = sc.spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match input.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if self.pane_write(id, l, &buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
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
        });
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
            "inbox": s.inbox.iter().filter(|i| i.open()).collect::<Vec<_>>(),
            "turns": s.turns.iter().rev().take(20).collect::<Vec<_>>(),
            "warnings": self.warnings,
            "live_cap": {"base": self.base_cap(), "effective": self.live_cap(), "pressure": self.memory_pressure(), "raw_pressure": self.cfg.memory.level()},
            "otlp": self.otlp_endpoint(),
        })
    }

    /// The latest reading of each window, across agents.
    pub(crate) fn usage_view(&self) -> crate::usage::UsageView {
        self.usage.lock().unwrap_or_else(|e| e.into_inner()).view()
    }

    pub fn usage_json(&self) -> Value {
        let view = self.usage_view();
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
/// (else the default week), the stored policy (live caps from `base_cap`),
/// this week's statusline samples replayed with their cost class (NFR-2), and
/// the carry-over state (`carry`, else the stored one): pauses, deferrals, the
/// ask, sprints and the queue entries whose turns still wait for them. A turn
/// that waited on an entry that did not come back is asked again.
fn restore_governor(
    db: &orch_store::Store,
    store: &Store,
    cfg: &RuntimeConfig,
    base_cap: u32,
    shadow: bool,
    carry: Option<governor::Carry>,
    now: Timestamp,
) -> Result<(Governor, Vec<Release>)> {
    let mut policy = db.policy(POLICY_ID)?.unwrap_or_default();
    policy.live_caps = governor::LiveCaps::from_base(base_cap);
    db.save_policy(POLICY_ID, &policy)?;
    let mut plan = match db.active_plan()? {
        Some(p) if p.reset_at > now && p.week_start <= now => p,
        // The app did not run through the reset: the last plan rolled
        // forward by whole weeks keeps the real reset time (as `tick` does).
        Some(p) if p.reset_at <= now => roll_forward(p, now),
        _ => (cfg.default_plan)(now, shadow),
    };
    plan.shadow_mode = shadow;
    db.save_plan(&plan, orch_store::PlanStatus::Active)?;
    let start = plan.week_start;
    let samples = week_samples(db, start, None)?;
    // Created when the app first saw this week, as the live Governor was: it
    // trusts W = 0 before the week's first reading only if that was at the
    // reset (FR-814). Either way the reset anchors the calibration.
    let seen_from = samples.first().map_or(now, |(_, s)| s.ts).clamp(start, now.max(start));
    let mut gov = Governor::new(plan.clone(), policy.clone(), seen_from).map_err(|e| anyhow!("{e:?}"))?;
    let aliases = project_aliases(db)?;
    let allocs: BTreeMap<String, String> =
        store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.allocation.clone())).collect());
    // FR-814: the estimator the week started with, then the week replayed,
    // gives the live run's calibration back.
    match db.week_estimator(start)? {
        Some(e) => gov.restore_estimator(e),
        None => {
            if let Some(e) = rebuild_estimator(db, &plan, &policy, &allocs, &aliases)? {
                gov.restore_estimator(e);
            }
            let base = gov.week_estimator().clone();
            db.save_estimator("week", start, &base.calibrator.snapshot(seen_from), &base)?;
        }
    }
    gov.set_aliases(aliases);
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

/// Project name → project id, for agent and usage records made when team
/// agents were charged to their project's name.
fn project_aliases(db: &orch_store::Store) -> Result<BTreeMap<String, governor::AllocationId>> {
    Ok(name_aliases(db.projects(true)?.into_iter().map(|p| (p.id, p.name))))
}

/// `(id, name)` pairs as name → id aliases. A name two projects share is
/// ambiguous and left out (those records go to `default`).
pub(crate) fn name_aliases(
    projects: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, governor::AllocationId> {
    let mut out: BTreeMap<String, Option<governor::AllocationId>> = BTreeMap::new();
    for (id, name) in projects {
        out.entry(name).and_modify(|v| *v = None).or_insert(Some(id.into()));
    }
    out.into_iter().filter_map(|(n, id)| Some((n, id?))).collect()
}

/// `p` moved by whole weeks to the week that holds `now`.
fn roll_forward(mut p: WeeklyPlan, now: Timestamp) -> WeeklyPlan {
    let week = 168 * 3600;
    let behind = now.as_second() - p.reset_at.as_second();
    let n = behind.div_euclid(week) + 1;
    let shift = governor::jiff::SignedDuration::from_secs(n * week);
    p.week_start = p.reset_at.checked_add(shift - governor::jiff::SignedDuration::from_secs(week)).unwrap_or(now);
    p.reset_at = p.reset_at.checked_add(shift).unwrap_or(now);
    p.id = format!("week-{}", p.week_start.as_second());
    p
}

/// The week's statusline samples, oldest first, with their stored rows.
fn week_samples(
    db: &orch_store::Store,
    from: Timestamp,
    to: Option<Timestamp>,
) -> Result<Vec<(orch_store::UsageRecord, governor::UsageSample)>> {
    let to_ms = to.map(|t| t.as_millisecond().max(0) as u64);
    Ok(db
        .usage_between(from.as_millisecond().max(0) as u64, to_ms, Some(orch_store::UsageSource::Statusline))?
        .into_iter()
        .filter_map(|u| governor_sample(&u).map(|s| (u, s)))
        .collect())
}

/// FR-814, a start after a reset the app did not see run through: the
/// estimator of the last week stored, with that week's samples replayed on
/// it, is what the live run would have carried into this week.
fn rebuild_estimator(
    db: &orch_store::Store,
    plan: &WeeklyPlan,
    policy: &governor::PacingPolicy,
    allocs: &BTreeMap<String, String>,
    aliases: &BTreeMap<String, governor::AllocationId>,
) -> Result<Option<governor::EstimatorState>> {
    let Some((prev_start, base)) = db.week_estimator_before(plan.week_start)? else { return Ok(None) };
    let prev_reset = prev_start.checked_add(168.hours()).unwrap_or(plan.week_start).min(plan.week_start);
    let prev_plan = WeeklyPlan {
        id: format!("week-{}", prev_start.as_second()),
        week_start: prev_start,
        reset_at: prev_reset,
        ..plan.clone()
    };
    let samples = week_samples(db, prev_start, Some(prev_reset))?;
    let Ok(mut prev) = Governor::new(prev_plan, policy.clone(), prev_start) else { return Ok(Some(base)) };
    prev.restore_estimator(base);
    prev.set_aliases(aliases.clone());
    for (u, sample) in samples {
        let class = u.class.as_deref().and_then(orch_store::untag).unwrap_or(governor::CostClass::Interactive);
        let alloc = u.allocation.clone().or_else(|| u.agent_id.as_ref().and_then(|a| allocs.get(a).cloned()));
        prev.replay(&sample, class, alloc.map(Into::into));
    }
    Ok(Some(prev.estimator()))
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
        model: u.model.clone(),
        model_windows: u.model_windows.clone(),
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
        // What the Governor was given, so a replay weighs it the same (FR-814).
        model: g.model.clone().or_else(|| s.model.clone()),
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
        model_windows: g.model_windows.clone(),
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
        // Never the real Keychain or network from a test.
        cfg.secrets = Arc::new(orch_secrets::MemorySecrets::default());
        cfg.net = Arc::new(crate::net::fake::FakeNet::default());
        cfg.shadow = shadow;
        cfg.hold = Duration::from_millis(800);
        cfg.permission_hold = Duration::from_millis(300);
        cfg.idle_suspend = None;
        cfg.memory = Arc::new(crate::resources::FixedMemory::new(crate::resources::PressureLevel::Normal));
        cfg.projects_root = d.join("projects");
        cfg.default_plan = test_plan;
        cfg.power = Arc::new(crate::power::FakePower::default());
        cfg
    }

    pub(crate) fn test_runtime(shadow: bool) -> (tempfile::TempDir, Arc<Runtime>) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let rt = Runtime::new(test_cfg(d.path(), shadow)).unwrap();
        rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
        (d, rt)
    }

    /// A live process standing for `id` on `model`. Like a real one, its exit
    /// reaches [`Runtime::on_exit`], so a stop removes the live entry at once.
    pub(crate) fn insert_test_live(rt: &Runtime, id: &str, model: Option<&str>, task: u64, _dir: &Path) {
        let pid = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let (me, agent, p2) = (rt.me.clone(), id.to_string(), pid.clone());
        let pty = crate::pty::echoing_fake(Box::new(move |status| {
            if let Some(rt) = me.upgrade() {
                rt.on_exit(&agent, p2.load(Ordering::SeqCst), status);
            }
        }));
        pid.store(pty.pid, Ordering::SeqCst);
        insert_live(rt, id, pty, model, task);
    }

    /// `id` is live behind `pty` (a fake terminal), ready for input.
    pub(crate) fn insert_test_live_with(rt: &Runtime, id: &str, pty: PtyProcess) {
        insert_live(rt, id, pty, None, 0);
        if let Some(l) = rt.live_get(id) {
            l.ready_at_ms.store(1, Ordering::SeqCst);
        }
    }

    fn insert_live(rt: &Runtime, id: &str, pty: PtyProcess, model: Option<&str>, task: u64) {
        let live = Live {
            pty,
            ready_at_ms: AtomicU64::new(0),
            last_user_input_ms: AtomicU64::new(0),
            model: model.map(Into::into),
            task: AtomicU64::new(task),
            pasted_at_ms: AtomicU64::new(0),
            resumed: false,
        };
        rt.live.lock().unwrap().insert(id.into(), Arc::new(live));
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
            resume_failed: false,
            start_fresh: false,
            spawned_at_ms: None,
            crash_restarts: vec![],
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
            acks: vec![],
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
        // A permission nobody answers is denied and parked (FR-705): the turn goes on to its end.
        let out = rt.on_hook(&hook("PermissionRequest", json!({"prompt_id": "p2"})), None).expect("parked");
        assert_eq!(out.0["hookSpecificOutput"]["decision"]["message"], crate::inbox::PARKED_MESSAGE);
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::Working);
    }

    #[test]
    fn hook_events_keep_their_specific_fields() {
        let p = |v: Value| HookPayload::parse(&v);
        let n = hook_event_detail(
            &HookEvent::Notification,
            &p(json!({"notification_type": "permission_prompt", "message": "Claude needs your permission"})),
        );
        assert_eq!(
            (n["notification_type"].as_str(), n["message"].as_str()),
            (Some("permission_prompt"), Some("Claude needs your permission"))
        );
        let t = hook_event_detail(&HookEvent::PreToolUse, &p(json!({"tool_name": "Bash", "tool_use_id": "tu1"})));
        assert_eq!((t["tool"].as_str(), t["tool_use_id"].as_str()), (Some("Bash"), Some("tu1")));
        let s =
            hook_event_detail(&HookEvent::SessionStart, &p(json!({"source": "startup", "permission_mode": "auto"})));
        assert_eq!(s["permission_mode"], "auto");
        let plain = hook_event_detail(&HookEvent::Stop, &p(json!({"permission_mode": "auto"})));
        assert!(plain.get("permission_mode").is_none() && plain.get("tool_use_id").is_none());
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

    fn events_of(rt: &Runtime, kind: &str) -> Vec<Value> {
        rt.db.flush().unwrap();
        rt.db.events(0, Some("a1"), Some(kind), 50).unwrap().into_iter().map(|e| e.detail).collect()
    }

    fn turns_of(rt: &Runtime) -> Vec<TurnRow> {
        rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == "a1").cloned().collect())
    }

    /// §3.2 (F2): a StopFailure the process lives through leaves the agent
    /// idle, its turn over, and one Continue queued; a failure of that
    /// Continue queues nothing more (the T3 hook point).
    #[test]
    fn a_failed_turn_leaves_the_agent_idle_with_one_continue_and_no_loop() {
        let (_d, rt) = test_runtime(false);
        let mut t = TurnRow::queued("a1", "work", TurnKind::Task, Admission::Admitted);
        t.status = TurnStatus::Delivered;
        let first = t.id.clone();
        rt.store.write(|s| s.turns.push(t)).unwrap();
        rt.on_hook(&hook("StopFailure", json!({"error": "server_error"})), None);
        let a = rt.store.read(|s| s.agents["a1"].clone());
        assert_eq!(a.tracker.state, AgentState::Idle);
        assert_eq!(a.tracker.last_error.as_deref(), Some("server_error"));
        let ts = turns_of(&rt);
        assert_eq!(ts.iter().find(|t| t.id == first).unwrap().status, TurnStatus::Done, "the failed turn is over");
        let cont: Vec<_> = ts.iter().filter(|t| t.kind == TurnKind::Continue).collect();
        assert_eq!(cont.len(), 1);
        assert!(cont[0].after_failure && cont[0].body.contains("your last turn failed (server_error)"));
        assert_eq!(events_of(&rt, "recovered")[0]["from_state"], "error");
        // Another failure before it ran: still one.
        rt.on_hook(&hook("StopFailure", json!({"error": "server_error"})), None);
        assert_eq!(turns_of(&rt).iter().filter(|t| t.kind == TurnKind::Continue).count(), 1);
        // That Continue ran and failed too: nothing queued, the hook point fires.
        rt.store
            .write(|s| s.turns.iter_mut().find(|t| t.after_failure).unwrap().status = TurnStatus::Delivered)
            .unwrap();
        rt.on_hook(&hook("StopFailure", json!({"error": "overloaded"})), None);
        let queued = turns_of(&rt).into_iter().filter(|t| t.status == TurnStatus::Queued).count();
        assert_eq!(queued, 0, "no loop");
        assert_eq!(events_of(&rt, "continue_failed")[0]["error"], "overloaded");
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::Idle);
    }

    /// §3.2 (F2): an agent blocked by a limit is idle again when the
    /// Governor releases it, or once no limit holds it (no release ever
    /// comes for one it holds no entry for), not within a minute of the hit.
    #[test]
    fn a_limit_released_or_over_gives_the_agent_back() {
        let (_d, rt) = test_runtime(false);
        let block = |rt: &Runtime, at: u64| {
            rt.store
                .write(|s| {
                    let a = s.agents.get_mut("a1").unwrap();
                    a.tracker.state = AgentState::BlockedByLimit;
                    a.last_event_at_ms = Some(at);
                })
                .unwrap()
        };
        let state = |rt: &Runtime| rt.store.read(|s| s.agents["a1"].tracker.state);
        let now = now_ms();
        block(&rt, now);
        rt.lift_limits(now + 1_000);
        assert_eq!(state(&rt), AgentState::BlockedByLimit, "not within a minute of the hit");
        // A limit in force holds it.
        let reset = Timestamp::now() + std::time::Duration::from_secs(3600);
        rt.governor_act(|g, now| (g.rate_limit_hit(None, governor::Window::FiveHour, reset, now), Vec::new()));
        rt.lift_limits(now + LIMIT_GRACE_MS + 1);
        assert_eq!(state(&rt), AgentState::BlockedByLimit);
        // A Governor release gives it back at once.
        rt.handle_releases(vec![Release {
            entry: governor::EntryId(999),
            agent: AgentId("a1".into()),
            allocation: DEFAULT_ALLOCATION.into(),
            kind: TurnKind::Continue,
            self_started: false,
            service: Default::default(),
        }]);
        assert_eq!(state(&rt), AgentState::Idle);
        assert_eq!(events_of(&rt, "recovered")[0]["how"], "release");
        // No limit at all (a fresh runtime): over once the grace passed.
        let (_d2, rt2) = test_runtime(false);
        block(&rt2, now);
        rt2.lift_limits(now + LIMIT_GRACE_MS + 1);
        assert_eq!(state(&rt2), AgentState::Idle);
        assert_eq!(events_of(&rt2, "recovered")[0]["how"], "limit_over");
    }

    /// §3.5 (F3): a crashed agent with queued work is resumed by the pump,
    /// at most twice an hour; then a `startup` item (`failed: "crash"`).
    /// One the user killed, or a start FR-415 owns, is left alone.
    #[test]
    fn a_crashed_agent_with_work_is_resumed_twice_an_hour_then_waits() {
        let (_d, rt) = test_runtime(true);
        let set = |rt: &Runtime, f: &dyn Fn(&mut AgentRecord)| {
            rt.store.write(|s| f(s.agents.get_mut("a1").unwrap())).unwrap();
        };
        let flag = |rt: &Runtime| rt.store.read(|s| s.agents["a1"].flag.clone());
        let crash_item = |rt: &Runtime| {
            rt.store.read(|s| s.inbox.iter().find(|i| i.open() && crate::startup::is_crash_item(i, "a1")).cloned())
        };
        // Work that is not the user's own words: it is what the budget counts.
        let work = rt.deliver_kind("a1", "hello", TurnKind::Message).unwrap();
        // Killed by the user: never restarted.
        set(&rt, &|a| a.tracker.state = AgentState::Killed);
        rt.pump();
        assert!(flag(&rt).is_none() && crash_item(&rt).is_none());
        // A failed start is FR-415's (a fresh restart, then the user).
        set(&rt, &|a| {
            a.tracker.state = AgentState::Error;
            a.resume_failed = true;
        });
        rt.pump();
        assert!(flag(&rt).is_none() && crash_item(&rt).is_none());
        // A crash: the pump resumes it (the test has no `claude`, so the
        // spawn fails and flags it: it was attempted).
        set(&rt, &|a| a.resume_failed = false);
        rt.pump();
        assert!(flag(&rt).is_some_and(|f| f.contains("could not resume")), "{:?}", flag(&rt));
        // Two restarts in the hour already: a `startup` item, no spawn.
        let now = now_ms();
        set(&rt, &|a| {
            a.flag = None;
            a.crash_restarts = vec![now - 50 * 60_000, now - 5 * 60_000];
        });
        rt.pump();
        assert!(flag(&rt).is_none(), "no spawn past the budget");
        let item = crash_item(&rt).expect("crash item");
        assert_eq!(item.payload["restarts"], 2);
        rt.pump();
        let crash_items =
            |rt: &Runtime| rt.store.read(|s| s.inbox.iter().filter(|i| i.payload["failed"] == "crash").count());
        assert_eq!(crash_items(&rt), 1, "raised once");
        // Dismissed: not raised again for the same used-up budget.
        rt.store
            .write(|s| {
                s.inbox.iter_mut().find(|i| i.id == item.id).unwrap().status = orch_store::InboxStatus::Dismissed
            })
            .unwrap();
        rt.pump();
        assert_eq!(crash_items(&rt), 1, "a dismissed item is not raised again");
        rt.store.write(|s| s.inbox.iter_mut().find(|i| i.id == item.id).unwrap().status = item.status).unwrap();
        // The user's own message resumes it whatever the budget says: the
        // spawn is attempted (the test has no `claude`, so it fails and flags),
        // the budget starts again and the item closes.
        rt.store.write(|s| s.turns.retain(|t| t.id != work)).unwrap();
        rt.deliver_user("a1", "back again").unwrap();
        rt.pump();
        assert!(flag(&rt).is_some_and(|f| f.contains("could not resume")), "{:?}", flag(&rt));
        assert!(crash_item(&rt).is_none(), "the item closes");
        // The lead's (or the user's) resume: the budget starts again, the item closes.
        rt.clear_crash("a1", "lead");
        assert!(crash_item(&rt).is_none());
        assert!(rt.store.read(|s| s.agents["a1"].crash_restarts.is_empty()));
        // An hour-old restart no longer counts.
        assert!(crate::startup::crash_restart_allowed(&[now - 3_600_000, now - 60_000], now));
        assert!(!crate::startup::crash_restart_allowed(&[now - 59 * 60_000, now - 60_000], now));
    }

    #[test]
    fn restart_rebuilds_the_governor_from_stored_samples() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let reset = (now_ms() / 1000) as i64 + 3 * 24 * 3600;
        let (live, plan_id) = {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
            // An app turn (admitted by the Governor before delivery: autonomous), two responses.
            let rec = rt.store.read(|s| s.agents["a1"].clone());
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
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let waiting =
            |rt: &Runtime| rt.store.read(|s| s.turns.iter().find(|t| t.agent_id == "a1").map(|t| t.admission));
        let entry = {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
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
    fn project_names_alias_their_ids_unless_two_projects_share_one() {
        let pairs = [("shop", "shop-api"), ("a", "dup"), ("b", "dup")];
        let aliases = name_aliases(pairs.map(|(id, n)| (id.to_string(), n.to_string())));
        assert_eq!(aliases.get("shop-api"), Some(&governor::AllocationId::from("shop")));
        assert!(!aliases.contains_key("dup"), "ambiguous: left to `default`");
    }

    #[test]
    fn replan_keeps_running_turns_and_restart_keeps_held_work() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        {
            let rt = Runtime::new(test_cfg(d.path(), false)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(d.path()))).unwrap();
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
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
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
                ..Default::default()
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

    fn msg(rt: &Runtime, id: u64) -> orch_store::Message {
        rt.db.messages_by_id(&[id]).unwrap().remove(0)
    }

    #[test]
    fn messages_are_read_when_their_turn_ends_not_when_queued() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        let m = team_message(&rt, "T-2 done");
        let tid = rt.deliver_messages_turn("a1", "ORCH: 1 new message", vec![m]).unwrap();
        rt.db.hand_messages(&[m], "a1", "turn", Some(&tid), 5).unwrap();
        assert!(rt.inject_messages(&rt.store.read(|s| s.agents["a1"].clone()), None).is_none(), "handed once");
        assert_eq!(msg(&rt, m).delivery_state(), orch_store::DeliveryState::Handed, "pasted is not seen");
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": format!("[t:{tid}] ORCH: 1 new message")})), None);
        assert_eq!(msg(&rt, m).delivery_state(), orch_store::DeliveryState::Seen, "its prompt started: seen");
        assert_eq!(read_at(&rt, m), None, "started is not read");
        rt.on_hook(&hook("Stop", json!({})), None);
        assert!(read_at(&rt, m).is_some(), "read when the turn ends");
    }

    #[test]
    fn a_dropped_turn_gives_its_messages_back() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        let m = team_message(&rt, "T-2 done");
        let tid = rt.deliver_messages_turn("a1", "ORCH: 1 new message", vec![m]).unwrap();
        rt.db.hand_messages(&[m], "a1", "turn", Some(&tid), 5).unwrap();
        rt.drop_queued_turns("a1");
        let rec = rt.store.read(|s| s.agents["a1"].clone());
        assert!(rt.inject_messages(&rec, None).unwrap().0.contains("T-2 done"), "delivered again");
    }

    /// FR-403 (fix-team-getting-stucked §3.3): a Stop is never blocked for
    /// messages; one injected and confirmed is read at the turn's end, one
    /// injected but never confirmed goes back to the queue as a new turn.
    #[test]
    fn a_message_that_came_mid_turn_is_read_or_handed_back_at_stop_never_blocked() {
        let (_d, rt) = test_runtime(true);
        seat(&rt);
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "review T-1"})), None);
        let m = team_message(&rt, "T-2 done");
        let (ctx, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Edit"})), None);
        assert!(ctx.expect("injected").0.to_string().contains("T-2 done"));
        assert_eq!(ack, [m]);
        let h = msg(&rt, m);
        assert_eq!(
            (h.delivery_state(), h.via.as_deref(), h.turn_id.as_deref()),
            (orch_store::DeliveryState::Handed, Some("inject"), Some("p1"))
        );
        rt.on_hook_ack("a1", &ack);
        assert_eq!(msg(&rt, m).delivery_state(), orch_store::DeliveryState::Seen);
        assert!(rt.on_hook(&hook("Stop", json!({})), None).is_none(), "no Stop block for messages");
        assert!(read_at(&rt, m).is_some(), "seen in full: read");
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::Idle);

        // Injected, never confirmed: queued again at the Stop, then a turn.
        rt.on_hook(&hook("UserPromptSubmit", json!({"prompt": "next", "prompt_id": "p2"})), None);
        let m2 = team_message(&rt, "T-3 done");
        let (_, ack) = rt.on_hook_reply(&hook("PostToolUse", json!({"tool_name": "Edit", "prompt_id": "p2"})), None);
        assert_eq!(ack, [m2]);
        assert!(rt.on_hook(&hook("Stop", json!({"prompt_id": "p2"})), None).is_none());
        let q = msg(&rt, m2);
        assert_eq!(
            (q.delivery_state(), q.attempts, q.last_error.as_deref()),
            (orch_store::DeliveryState::Queued, 1, Some("no_ack"))
        );
        let rec = rt.store.read(|s| s.agents["a1"].clone());
        assert!(rt.inject_messages(&rec, None).unwrap().0.contains("T-3 done"), "delivered again");
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
        assert!(
            rt.on_hook(&hook("Stop", json!({"last_assistant_message": q, "stop_hook_active": true})), None).is_none()
        );
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
    fn the_test_plan_holds_at_any_hour() {
        // The plan depends on the UTC hour only: each hour and the last
        // instant before the next, on a few dates (a leap day, a year end, a
        // DST change in Europe): a sprint can start with most of the day left.
        for day in ["2024-02-29", "2026-03-29", "2026-10-25", "2026-12-31"] {
            let midnight: Timestamp = format!("{day}T00:00:00Z").parse().unwrap();
            for h in 0..24 {
                let hour = midnight.checked_add(h.hours()).unwrap();
                for now in [hour, hour.checked_add(3_599_900.milliseconds()).unwrap()] {
                    let mut g = Governor::new(test_plan(now, false), governor::PacingPolicy::default(), now).unwrap();
                    let tz = TimeZone::get(&g.plan().calendar.time_zone).unwrap();
                    let end = g.curve().end_of_working_day(now, &tz).unwrap();
                    assert!(end.as_second() - now.as_second() >= 11 * 3600 - 1, "{now}: the day ends at {end}");
                    assert!(g.start_sprint(5.0, now).is_ok(), "{now}");
                }
            }
        }
    }

    /// For [`estimator_and_report_carry_across_restarts_and_the_reset`]: the
    /// week before [`test_plan`]'s.
    fn last_week_plan(now: Timestamp, shadow: bool) -> WeeklyPlan {
        let mut p = test_plan(now, shadow);
        p.week_start = p.week_start.checked_sub(168.hours()).unwrap();
        p.reset_at = p.reset_at.checked_sub(168.hours()).unwrap();
        p.id = format!("week-{}", p.week_start.as_second());
        p
    }

    /// [`last_week_plan`] with its reset 5 h earlier than the default plan's.
    fn odd_last_week_plan(now: Timestamp, shadow: bool) -> WeeklyPlan {
        let mut p = last_week_plan(now, shadow);
        p.week_start = p.week_start.checked_sub(5.hours()).unwrap();
        p.reset_at = p.reset_at.checked_sub(5.hours()).unwrap();
        p.id = format!("week-{}", p.week_start.as_second());
        p
    }

    /// FR-814 and FR-818 through the runtime: the estimator a week starts
    /// with is stored, so a restart mid-week and a restart after a reset the
    /// app did not run through both get the live run's calibration back; the
    /// week that ended gets its report, at the reset or at the next start.
    #[test]
    fn estimator_and_report_carry_across_restarts_and_the_reset() {
        let t0 = Timestamp::now();
        let (w1, w2) = (last_week_plan(t0, false), test_plan(t0, false));
        let at =
            |t: Timestamp, h: f64| t.checked_add(governor::jiff::SignedDuration::from_secs_f64(h * 3600.0)).unwrap();
        // Truth: 10 cost per weekly pp, plus 0.05 pp per sample outside the app.
        // One response every 30 min for 60 h of each week.
        let week = |p: &WeeklyPlan, cost0: f64| -> Vec<StatusMsg> {
            let (mut cost, mut w) = (cost0, 0.0);
            (0..120)
                .map(|i| {
                    let dc = 1.0 + (i % 5) as f64;
                    cost += dc;
                    w += dc / 10.0 + 0.05;
                    StatusMsg {
                        agent_id: "a1".into(),
                        ts_ms: at(p.week_start, 1.0 + i as f64 * 0.5).as_millisecond() as u64,
                        payload: json!({"session_id": "s", "model": {"id": "claude-sonnet-4-5"}, "cost": {"total_cost_usd": cost},
                            "rate_limits": {"seven_day": {"used_percentage": w.floor(), "resets_at": p.reset_at.as_second()}}}),
                    }
                })
                .collect()
        };
        let (s1, s2) = (week(&w1, 0.0), week(&w2, 360.0)); // one session: its cost keeps adding up
        let cfg = |d: &Path, plan: fn(Timestamp, bool) -> WeeklyPlan| RuntimeConfig {
            default_plan: plan,
            ..test_cfg(d, false)
        };
        let k = |rt: &Runtime| rt.gov().calibration(t0);
        let close = |a: &governor::Calibration, b: &governor::Calibration, what: &str| {
            let c = |x: f64, y: f64| (x - y).abs() <= 1e-6 * x.abs().max(1.0);
            assert!(
                c(a.cost_per_weekly_pct, b.cost_per_weekly_pct)
                    && c(a.unattributed_rate, b.unattributed_rate)
                    && c(a.confidence, b.confidence),
                "{what}:\n{a:?}\n{b:?}"
            );
        };
        let reports = |rt: &Runtime| -> Vec<String> {
            rt.reports_list().unwrap()["reports"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].as_str().unwrap().to_string())
                .collect()
        };

        // Live: week 1, then the reset (the first reading of week 2), then week 2.
        let a = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let live = {
            let rt = Runtime::new(cfg(a.path(), last_week_plan)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(a.path()))).unwrap();
            // Made a moment after `w1`: its id can be a second later.
            let week1_id = rt.gov().plan().id.clone();
            for m in s1.iter().chain(&s2) {
                rt.on_status(m);
            }
            assert_eq!(rt.gov().plan().week_start.as_second(), w2.week_start.as_second(), "switched at the reset");
            rt.tick();
            rt.db.flush().unwrap();
            assert_eq!(reports(&rt), ["current".to_string(), week1_id.clone()], "the ended week's report is stored");
            let r = rt.report_view(&week1_id).unwrap();
            assert_eq!(r["in_progress"], false);
            assert!(r["totals"]["weekly_pct"].as_f64().unwrap() > 30.0, "{r}");
            assert_eq!(r["by_model"][0]["key"], "sonnet", "{r}");
            let digest = rt.db.events(0, None, Some("weekly_report"), 5).unwrap();
            assert_eq!(digest[0].detail["report"], week1_id.as_str(), "a report_ready outcome");
            assert_eq!(rt.report_view("current").unwrap()["in_progress"], true);
            let k = k(&rt);
            assert!(k.confidence > 0.5 && (k.cost_per_weekly_pct - 10.0).abs() < 3.0, "{k:?}");
            k
        };

        // Restart mid-week 2: the week's stored estimator, the week replayed.
        let rt = Runtime::new(cfg(a.path(), test_plan)).unwrap();
        close(&k(&rt), &live, "restart mid-week");
        assert_eq!(reports(&rt).len(), 2, "no second report of the same week");
        drop(rt);

        // The app stopped in week 1 (whose reset is not the default plan's:
        // 5 h earlier) and starts again in week 2. Week 1 was never switched
        // away from live, so this restart rebuilds the estimator from week
        // 1's, rolls the plan forward to the real reset, and makes week 1's
        // report at start.
        let b = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let plan1 = {
            let rt = Runtime::new(cfg(b.path(), odd_last_week_plan)).unwrap();
            rt.store.write(|s| s.agents.insert("a1".into(), test_agent(b.path()))).unwrap();
            let plan1 = rt.gov().plan().clone();
            for m in &week(&plan1, 0.0) {
                rt.on_status(m);
            }
            rt.db.flush().unwrap();
            plan1
        };
        let next_start = plan1.reset_at;
        {
            let db = orch_store::Store::open(&AppPaths::new(b.path().join("app")).db_file()).unwrap();
            assert!(db.week_estimator(next_start).unwrap().is_none(), "no stored base for week 2: the rebuild runs");
            assert!(db.reports().unwrap().is_empty());
        }
        let rt = Runtime::new(cfg(b.path(), test_plan)).unwrap();
        rt.db.flush().unwrap();
        let plan2 = rt.gov().plan().clone();
        assert_eq!(
            (plan2.week_start, plan2.reset_at.as_second()),
            (next_start, plan1.reset_at.as_second() + 168 * 3600),
            "the last plan rolled forward, not the default week"
        );
        let list = rt.reports_list().unwrap();
        assert_eq!(list["reports"].as_array().unwrap().len(), 2, "week 1's report, made at start: {list}");
        assert_eq!(list["reports"][1]["id"], json!(plan1.id));
        let ms_iso = |t: Timestamp| Timestamp::from_millisecond(t.as_millisecond()).unwrap().to_string();
        assert_eq!(list["reports"][1]["reset_at"], json!(ms_iso(plan1.reset_at)), "the real reset");
        assert!(rt.db.week_estimator(next_start).unwrap().is_some(), "the rebuilt base is stored");
        // Its agents' processes ended with the old core: their cost counts from 0 again.
        for m in &week(&plan2, 0.0) {
            rt.on_status(m);
        }
        close(&k(&rt), &live, "restart across the reset");

        // Without the carry-over, week 2 alone gives another fit.
        let mut cold = Governor::new(plan2.clone(), governor::PacingPolicy::default(), plan2.week_start).unwrap();
        for (u, smp) in week_samples(&rt.db, plan2.week_start, None).unwrap() {
            let class = u.class.as_deref().and_then(orch_store::untag).unwrap_or(governor::CostClass::Interactive);
            cold.replay(&smp, class, None);
        }
        assert!((cold.calibration(t0).confidence - live.confidence).abs() > 1e-3, "{:?}", cold.calibration(t0));
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
