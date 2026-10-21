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
    AgentId, BudgetAllocation, Decision, Governor, PacingPolicy, Release, Tier, ToolCallRequest, TurnKind, TurnRequest,
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
use crate::pty::PtyProcess;
use crate::reconcile::{self, ReconcileEnv, Report};
use crate::role::{PermissionMode, Role};
use crate::state::{self, AgentState, Effect, StopIntent, TurnOrigin};
use crate::store::{AgentRecord, HoldKind, HoldRow, HoldStatus, Store, TurnRow, TurnStatus};
use crate::usage::UsageCollector;
use crate::worktree;

pub const DEFAULT_ALLOCATION: &str = "default";
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

pub struct Runtime {
    pub cfg: RuntimeConfig,
    pub store: Store,
    gov: Mutex<Governor>,
    usage: Mutex<UsageCollector>,
    live: Mutex<BTreeMap<String, Arc<Live>>>,
    decisions: Mutex<VecDeque<AdmissionDecision>>,
    events: Mutex<Option<std::fs::File>>,
    login_path: String,
    user_statusline: Option<String>,
    git_identity: (String, String),
    pub warnings: Vec<String>,
    me: Weak<Runtime>,
}

fn ts_now() -> Timestamp {
    Timestamp::now()
}

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
        let store = Store::open(&cfg.paths.state_file())?;
        let policy = PacingPolicy { live_caps: governor::LiveCaps::from_base(cfg.base_cap), ..Default::default() };
        let now = ts_now();
        let gov = Governor::new(default_plan(now, cfg.shadow), policy, now).map_err(|e| anyhow!("{e:?}"))?;
        let home = crate::paths::home_dir();
        let shell = std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| "/bin/zsh".into());
        let login_path = crate::env::login_path(&shell, &home);
        let mut warnings = crate::env::parent_env_warnings();
        match preflight::claude_version(&cfg.claude).and_then(|v| cfg.version_range.check(v)) {
            Ok(VersionCheck::Untested(v)) => {
                warnings.push(format!("claude {v} is newer than the last tested version: run the contract tests (R2)"))
            }
            Ok(VersionCheck::Supported(_)) => {}
            Err(e) => warnings.push(format!("claude: {e}")),
        }
        if let Err(e) = preflight::check_git() {
            warnings.push(e.to_string());
        }
        let events =
            std::fs::OpenOptions::new().create(true).append(true).open(cfg.paths.state_dir().join("events.jsonl")).ok();
        Ok(Arc::new_cyclic(|me| Runtime {
            user_statusline: crate::env::user_statusline_command(&home),
            git_identity: crate::env::user_git_identity(),
            cfg,
            store,
            gov: Mutex::new(gov),
            usage: Mutex::new(UsageCollector::new()),
            live: Mutex::new(BTreeMap::new()),
            decisions: Mutex::new(VecDeque::new()),
            events: Mutex::new(events),
            login_path,
            warnings,
            me: me.clone(),
        }))
    }

    fn gov(&self) -> std::sync::MutexGuard<'_, Governor> {
        self.gov.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn live_get(&self, id: &str) -> Option<Arc<Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    pub fn live_ids(&self) -> Vec<String> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    fn event(&self, agent: &str, kind: &str, detail: Value) {
        if let Some(f) = self.events.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = writeln!(f, "{}", json!({"ts_ms": now_ms(), "agent": agent, "kind": kind, "detail": detail}));
        }
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

    fn turn_request(&self, rec: &AgentRecord, kind: TurnKind, interactive: bool, self_started: bool) -> TurnRequest {
        TurnRequest {
            agent: AgentId(rec.id.clone()),
            allocation: rec.allocation.clone().into(),
            tier: rec.tier,
            interactive,
            kind,
            self_started,
            ceiling: None,
        }
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
            repo: repo.clone(),
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
        env.otel_endpoint = self.cfg.otel_endpoint.clone();
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
        let pty = PtyProcess::spawn(
            &compiled,
            self.cfg.rows,
            self.cfg.cols,
            None,
            Box::new(move |status| {
                if let Some(rt) = weak.upgrade() {
                    rt.on_exit(&id_owned, status);
                }
            }),
        )?;
        let (pid, pgid) = (pty.pid, pty.pgid);
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id.into(),
            Arc::new(Live { pty, ready_at_ms: AtomicU64::new(0), last_user_input_ms: AtomicU64::new(0) }),
        );
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.pid = Some(pid);
                a.pgid = Some(pgid);
                a.spawns += 1;
                a.flag = None;
                a.trust_dialog = false;
                a.tracker.state = AgentState::Starting;
                a.tracker.stop_intent = StopIntent::None;
                a.tracker.background_tasks = 0;
            }
        })?;
        self.event(id, "spawned", json!({"pid": pid, "args": compiled.args}));
        Ok(())
    }

    fn on_exit(&self, id: &str, status: Option<u32>) {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                state::on_exit(&mut a.tracker);
                a.pid = None;
                a.pgid = None;
            }
        });
        let releases = self.gov().turn_ended(&AgentId(id.into()), ts_now());
        self.handle_releases(releases, None);
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
                    state::on_exit(&mut a.tracker);
                    a.pid = None;
                    a.pgid = None;
                }
            })?;
        } else if let Some(l) = live {
            // Wait (bounded) for the exit callback so callers see the new state.
            for _ in 0..100 {
                if l.pty.has_exited() {
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
        let tid = delivery::new_turn_id();
        self.store.write(|s| {
            s.turns.push(TurnRow {
                id: tid.clone(),
                agent_id: id.into(),
                body: text.into(),
                status: TurnStatus::Queued,
                created_at_ms: now_ms(),
                delivered_at_ms: None,
                admitted: false,
                governor_entry: None,
            })
        })?;
        Ok(tid)
    }

    /// FR-811 Pause all. Returns the Governor action id.
    pub fn pause_all(&self) -> u64 {
        self.gov().pause_all(ts_now()).0
    }

    /// Lifts Pause all; returns how many queued turns were released.
    pub fn resume_all(&self) -> usize {
        let releases = self.gov().resume_all(ts_now());
        let n = releases.len();
        self.handle_releases(releases, None);
        n
    }

    /// Graceful stop of the core: suspend every live agent.
    pub fn shutdown(&self) {
        for id in self.live_ids() {
            let _ = self.suspend(&id);
        }
    }

    // ------------------------------------------------------------ hooks

    /// One hook from orch-hook. `conn` is watched for EOF while a PreToolUse
    /// is held (Esc in the pane kills orch-hook and no hook event reports it).
    pub fn on_hook(&self, m: &HookMsg, conn: Option<&UnixStream>) -> Option<HookOutput> {
        let id = m.agent_id.as_str();
        let Some(rec) = self.store.read(|s| s.agents.get(id).cloned()) else {
            self.event(id, "hook_unknown_agent", json!(m.event));
            return None;
        };
        let p = HookPayload::parse(&m.payload);
        self.event(
            id,
            m.event.as_str(),
            json!({"tool": p.tool_name, "prompt_id": p.prompt_id, "source": p.source, "error": p.error}),
        );
        let mut tracker = rec.tracker.clone();
        let effects = state::apply(&mut tracker, &m.event, &p);
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker = tracker.clone();
                a.last_event = Some(m.event.to_string());
                a.last_event_at_ms = Some(now_ms());
            }
        });
        for fx in effects {
            self.on_effect(&rec, fx, &p);
        }
        match m.event {
            HookEvent::PreToolUse => {
                self.admit_tool(&rec, tracker.turn_origin == Some(TurnOrigin::User), &p, m.deadline_ms, conn)
            }
            // No decision: the dialog shows in the pane for the user (no Inbox UI yet).
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
                let req = match origin {
                    TurnOrigin::App => None, // admitted before delivery
                    TurnOrigin::User => Some(self.turn_request(rec, TurnKind::User, true, false)),
                    TurnOrigin::SelfStarted => Some(self.turn_request(rec, TurnKind::Continue, false, true)),
                };
                if let Some(req) = req {
                    let ar = AdmissionRequest {
                        agent_id: id.into(),
                        point: AdmissionPoint::TurnStart,
                        interactive: req.interactive,
                        self_started: req.self_started,
                    };
                    let d = self.gov().request_turn(req, ts_now());
                    self.record_decision(ar, &d);
                }
            }
            Effect::QueuedMessage => {
                let markers = delivery::find_markers(p.prompt.as_deref().unwrap_or(""));
                self.mark_delivered(id, &markers);
            }
            Effect::TurnEnded | Effect::TurnEndedWithBackground(_) => {
                let _ = self.store.write(|s| {
                    for turn in s.turns.iter_mut().filter(|x| x.agent_id == id && x.status == TurnStatus::Delivered) {
                        turn.status = TurnStatus::Done;
                    }
                });
                if let (Some(l), Effect::TurnEnded) = (&live, &fx) {
                    l.ready_at_ms.store(now + delivery::AFTER_STOP.as_millis() as u64, Ordering::SeqCst);
                }
                let releases = self.gov().turn_ended(&AgentId(id.into()), ts_now());
                self.handle_releases(releases, None);
            }
            Effect::RateLimited { message } => {
                let (window, resets) = self
                    .usage
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .exhausted_window(id)
                    .unwrap_or((governor::Window::FiveHour, (now / 1000) as i64 + 3600));
                let resets_at = Timestamp::from_second(resets).unwrap_or_else(|_| ts_now());
                let resp = self.gov().rate_limit_hit(Some(&AgentId(id.into())), window, resets_at, ts_now());
                self.event(id, "rate_limited", json!({"window": window, "resets_at": resets, "message": message, "cancel_auto_continue": resp.cancel_auto_continue}));
                if resp.cancel_auto_continue {
                    // J9: cancel the CLI's own auto-continue; the Governor reschedules.
                    if let Some(l) = live {
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(1));
                            let _ = l.pty.write(b"\x1b");
                        });
                    }
                }
            }
            Effect::Failed { error } => {
                let _ = self.store.write(|s| s.add_inbox(Some(id), "alert", format!("{id}: API error {error}")));
                let releases = self.gov().turn_ended(&AgentId(id.into()), ts_now());
                self.handle_releases(releases, None);
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
        let d = self.gov().tool_call(&req, ts_now());
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
            });
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker.state = AgentState::Queued;
            }
        });
        let end = std::time::Instant::now() + budget;
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
            let releases = self.gov().tick(ts_now());
            if self.handle_releases(releases, Some(id)) {
                outcome = HoldStatus::Allowed;
                break;
            }
        }
        let _ = self.store.write(|s| {
            if let Some(h) = s.holds.iter_mut().find(|h| h.id == hold_id) {
                h.status = outcome;
            }
            if let Some(a) = s.agents.get_mut(id) {
                if a.tracker.state == AgentState::Queued {
                    a.tracker.state = AgentState::Working;
                }
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

    /// Carries out the Governor's releases. Returns whether `holding` got one
    /// (that tool call is then allowed instead of a new turn being delivered).
    fn handle_releases(&self, releases: Vec<Release>, holding: Option<&str>) -> bool {
        let mut held_released = false;
        for r in releases {
            let id = r.agent.0.clone();
            self.event(&id, "released", serde_json::to_value(&r).unwrap_or(Value::Null));
            if holding == Some(id.as_str()) && !held_released {
                held_released = true;
                continue;
            }
            let _ = self.store.write(|s| {
                let entry = r.entry.0;
                let existing = s
                    .turns
                    .iter_mut()
                    .filter(|t| t.agent_id == id && t.status == TurnStatus::Queued && !t.admitted)
                    .min_by_key(|t| (t.governor_entry != Some(entry), t.created_at_ms));
                match existing {
                    Some(t) => {
                        t.admitted = true;
                        t.governor_entry = Some(entry);
                    }
                    None => s.turns.push(TurnRow {
                        id: delivery::new_turn_id(),
                        agent_id: id.clone(),
                        body: CONTINUE_MESSAGE.into(),
                        status: TurnStatus::Queued,
                        created_at_ms: now_ms(),
                        delivered_at_ms: None,
                        admitted: true,
                        governor_entry: Some(entry),
                    }),
                }
            });
        }
        held_released
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
            // FR-802: the plan week is aligned to the real weekly reset.
            if let Some(w) = g.weekly {
                let off = (w.resets_at.as_second() - gov.plan().reset_at.as_second()).abs();
                if off > 3600 && w.resets_at > g.ts {
                    let mut plan = gov.plan().clone();
                    plan.reset_at = w.resets_at;
                    plan.week_start = w.resets_at.checked_sub(168.hours()).unwrap_or(w.resets_at);
                    plan.id = format!("week-{}", plan.week_start.as_second());
                    let _ = gov.new_week(plan, g.ts);
                }
            }
            gov.ingest(&g);
        }
    }

    // ------------------------------------------------------------ loops

    /// The Governor's control loop (call every ~30 s).
    pub fn tick(&self) {
        let releases = self.gov().tick(ts_now());
        self.handle_releases(releases, None);
    }

    /// Delivery loop (call every ~200 ms): trust-dialog detection, then for
    /// each agent with queued work, admission and FR-206 delivery.
    pub fn pump(&self) {
        let now = now_ms();
        for (id, l) in self.live.lock().unwrap_or_else(|e| e.into_inner()).clone() {
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
        if turn.admitted {
            return true;
        }
        if turn.governor_entry.is_some() {
            return false; // waiting in the Governor's queue for a release
        }
        let kind = if turn.body == CONTINUE_MESSAGE || turn.body == reconcile::RESUME_MESSAGE {
            TurnKind::Continue
        } else {
            TurnKind::Message
        };
        let req = self.turn_request(rec, kind, false, false);
        let ar = AdmissionRequest {
            agent_id: rec.id.clone(),
            point: AdmissionPoint::TurnStart,
            interactive: false,
            self_started: false,
        };
        let d = self.gov().request_turn(req, ts_now());
        self.record_decision(ar, &d);
        let (admitted, entry) = match &d.effective {
            Verdict::Admit => (true, None),
            Verdict::Queue { entry, .. } => (false, entry.map(|e| e.0)),
            Verdict::Deny { .. } => (false, None),
        };
        let _ = self.store.write(|s| {
            if let Some(t) = s.turns.iter_mut().find(|t| t.id == turn.id) {
                t.admitted = admitted;
                t.governor_entry = entry;
            }
        });
        admitted
    }

    fn pump_one(&self, rec: &AgentRecord, turn: &TurnRow, now: u64) -> Result<()> {
        let Some(l) = self.live_get(&rec.id) else {
            // A suspended agent with work is resumed through the Governor first (FR-403).
            if rec.tracker.state == AgentState::Suspended && rec.flag.is_none() && self.admit_turn(rec, turn) {
                self.spawn(&rec.id, None)?;
            }
            return Ok(());
        };
        let gate = GateInputs {
            live: !l.pty.has_exited(),
            idle: rec.tracker.state == AgentState::Idle,
            paused: false,
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
    pub fn attach(&self, id: &str, stream: UnixStream, rows: u16, cols: u16) -> Result<()> {
        let l = self.live_get(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        let _ = l.pty.resize(rows, cols);
        l.pty.output.set_attached(true);
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
            let bytes = l.pty.read();
            if bytes.is_empty() {
                if l.pty.has_exited() && l.pty.output.is_closed() || reader.is_finished() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
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
                    "queued_turns": s.turns.iter().filter(|t| t.agent_id == a.id && t.status == TurnStatus::Queued).count(),
                    "last_event": a.last_event,
                    "trust_dialog": a.trust_dialog,
                    "flag": a.flag,
                })
            })
            .collect();
        json!({
            "agents": agents,
            "inbox": s.inbox.iter().filter(|i| i.pending).collect::<Vec<_>>(),
            "turns": s.turns.iter().rev().take(20).collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
    }

    pub fn usage_json(&self) -> Value {
        let view = self.usage.lock().unwrap_or_else(|e| e.into_inner()).view();
        let snap = self.gov().snapshot(ts_now());
        json!({
            "five_hour": view.five_hour,
            "weekly": view.weekly,
            "governor": {"used": snap.used, "provenance": snap.provenance, "plan": snap.plan, "error": snap.error, "state": snap.state, "five_hour": snap.five_hour},
            "agents": view.agents,
        })
    }

    pub fn governor_json(&self) -> Value {
        let g = self.gov();
        let now = ts_now();
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

/// True when the peer closed the connection (EOF). Waits up to 250 ms.
fn conn_closed(c: &UnixStream) -> bool {
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
mod tests {
    use super::*;

    fn test_runtime(shadow: bool) -> (tempfile::TempDir, Arc<Runtime>) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let mut cfg = RuntimeConfig::new(AppPaths::new(d.path().join("app")));
        cfg.shadow = shadow;
        cfg.hold = Duration::from_millis(800);
        cfg.projects_root = d.path().join("projects");
        let rt = Runtime::new(cfg).unwrap();
        let rec = AgentRecord {
            id: "a1".into(),
            repo: d.path().into(),
            worktree: d.path().into(),
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
        };
        rt.store.write(|s| s.agents.insert("a1".into(), rec)).unwrap();
        (d, rt)
    }

    fn hook(event: &str, extra: Value) -> HookMsg {
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
