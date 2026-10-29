//! The app's core: embeds the agent runtime and its Governor in the Tauri
//! process, serves the bus socket (hooks, statusline, `orchd` control
//! clients), holds permission prompts for the Inbox, and owns the shell and
//! editor PTYs. Every Tauri command is a thin wrapper over a method here.

use crate::LockOk;
use std::collections::BTreeMap;
use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{FETCH_EVERY, HOUSEKEEPING_EVERY, PUMP_EVERY, TICK_EVERY};
use anyhow::{Context, Result, anyhow, bail};
use governor::jiff::Timestamp;
use orch_git::git::real;
use orch_protocol::bus;
use orch_protocol::{
    AgentSpec, CtlReply, CtlRequest, Envelope, InboxDecision, Msg, PROTOCOL_VERSION, Reply, ReplyEnvelope, now_ms,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::editor;
use crate::kitty::{self, KittyImport};
use crate::memory::{self, AppMemory};
use crate::planner::{self, History, HistoryPoint, PlannerView};
use crate::services::{self, GitSummary, Project, Settings};
use crate::setup::{self, ClaudeCheck};
use crate::terms::{TermKind, Terms};
use crate::watch::MemoryWatch;

const GIT_EVERY: Duration = Duration::from_secs(10);
const MEMORY_EVERY: Duration = Duration::from_secs(5);
const HISTORY_EVERY: Duration = Duration::from_secs(60);
/// The diff pane renders at most this much patch (FR-107); the file list is always complete.
const DIFF_PATCH_MAX: usize = 2 << 20;

#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub home: PathBuf,
    pub claude: Option<PathBuf>,
    pub kitty_conf: PathBuf,
    /// Projects to register at start (development override; the registry
    /// keeps them).
    pub projects: Vec<PathBuf>,
    /// Run the J10 reconciler at start (resumes agents that were working).
    pub recover: bool,
    pub enforce: bool,
    /// The plan of a week with none stored (tests: `test_plan`, any date).
    pub default_plan: fn(governor::jiff::Timestamp, bool) -> governor::WeeklyPlan,
}

impl CoreConfig {
    /// `ORCH_HOME`, `ORCH_CLAUDE`, `ORCH_KITTY_CONF`, `ORCH_PROJECTS` (colon
    /// separated), `ORCH_NO_RECOVER`, `ORCH_ENFORCE`.
    pub fn from_env() -> CoreConfig {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = agent_runtime::paths::home_dir();
        CoreConfig {
            home: var("ORCH_HOME").map(PathBuf::from).unwrap_or_else(AppPaths::default_root),
            claude: var("ORCH_CLAUDE").map(PathBuf::from),
            kitty_conf: var("ORCH_KITTY_CONF")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config/kitty/kitty.conf")),
            projects: var("ORCH_PROJECTS").map(|v| v.split(':').map(PathBuf::from).collect()).unwrap_or_default(),
            recover: var("ORCH_NO_RECOVER").is_none(),
            enforce: var("ORCH_ENFORCE").is_some(),
            default_plan: agent_runtime::runtime::default_plan,
        }
    }
}

pub struct Core {
    pub paths: AppPaths,
    rt: Option<Arc<Runtime>>,
    /// Why agents are unavailable (another core owns the socket, no claude…).
    pub core_error: Option<String>,
    /// The store: the runtime's, or the app's own when the runtime is not up.
    pub db: orch_store::Store,
    pub settings: Settings,
    pub terms: Terms,
    pub kitty: Mutex<KittyImport>,
    pub editor_template: Mutex<String>,
    history: Mutex<History>,
    /// Governor actions of earlier starts this plan week (their ids restart
    /// at each start): shown in the log, not undoable.
    past_actions: Vec<governor::GovernorAction>,
    claude: Mutex<Option<ClaudeCheck>>,
    pub watch: Arc<MemoryWatch>,
    git: Mutex<Cached<BTreeMap<PathBuf, GitSummary>>>,
    memory: Mutex<Cached<AppMemory>>,
    /// The UI pane reading each agent's output: its generation, and bytes a
    /// stale reader (a closed pane's last pull) took for the current one.
    readers: Mutex<BTreeMap<String, Reader>>,
    reader_gen: std::sync::atomic::AtomicU64,
    agent_input: Mutex<BTreeMap<String, std::sync::mpsc::Sender<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    /// Shown as banners (from start, and raised later with `warn`).
    warnings: Mutex<Vec<String>>,
    /// The last failed manual task sync, per project (the Board shows it).
    pub(crate) sync_errors: Mutex<BTreeMap<String, String>>,
}

/// A value refreshed at most every so often.
#[derive(Default)]
struct Cached<T> {
    at: Option<Instant>,
    value: T,
}

impl<T> Cached<T> {
    fn stale(&self, every: Duration) -> bool {
        self.at.is_none_or(|t| t.elapsed() >= every)
    }

    fn set(&mut self, value: T) {
        self.value = value;
        self.at = Some(Instant::now());
    }
}

/// What `state()` reads about an agent from the store, besides the runtime's view.
struct AgentMeta {
    repo: PathBuf,
    worktree: PathBuf,
    branch: String,
    created_at_ms: u64,
    effort: Option<String>,
}

#[derive(Default)]
struct Reader {
    reader: u64,
    carry: Vec<u8>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct NewSession {
    pub project_id: String,
    pub name: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub base: Option<String>,
    pub message: Option<String>,
    pub tier: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TermInfo {
    pub id: u32,
    pub kind: TermKind,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct EditorOpen {
    /// The file went to the project's running nvim.
    pub reused: bool,
    pub term: Option<TermInfo>,
}

impl Core {
    /// Builds the core. A failure to start the runtime (another core on the
    /// socket, bad home) leaves the app usable without agents.
    pub fn start(cfg: CoreConfig) -> Arc<Core> {
        let paths = AppPaths::new(cfg.home.clone());
        let mut warnings = vec![];
        let started = Timestamp::now();
        let (rt, core_error) = match start_runtime(&cfg, &paths, &mut warnings) {
            Ok(rt) => (Some(rt), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let db = match &rt {
            Some(rt) => rt.db.clone(),
            None => open_store(&paths, &mut warnings),
        };
        let settings = Settings::new(db.clone());
        match settings.migrate_json(&paths.root.join("ui").join("settings.json")) {
            Ok(0) => {}
            Ok(n) => eprintln!("orchestrator: {n} settings imported from ui/settings.json"),
            Err(e) => warnings.push(format!("ui/settings.json not imported: {e:#}")),
        }
        if settings.get(setup::FIRST_RUN_KEY).is_none() {
            let _ = settings.set(setup::FIRST_RUN_KEY, json!(now_ms()));
        }
        let kitty_conf = settings
            .get(setup::KITTY_KEY)
            .and_then(|v| v.as_str().map(PathBuf::from))
            .unwrap_or(cfg.kitty_conf.clone());
        let kitty = kitty::import(&kitty_conf);
        let editor_template = settings
            .get(setup::EDITOR_KEY)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| editor::template_from_kitty(kitty.editor.as_deref()));
        // The Week planner's "actual" line and log across restarts.
        let (history, past_actions) = match &rt {
            Some(rt) => past_week(rt, started),
            None => (History::default(), vec![]),
        };
        let core = Arc::new(Core {
            paths,
            rt,
            core_error,
            db,
            settings,
            terms: Terms::default(),
            kitty: Mutex::new(kitty),
            editor_template: Mutex::new(editor_template),
            history: Mutex::new(history),
            past_actions,
            claude: Mutex::new(None),
            watch: Arc::new(MemoryWatch::default()),
            git: Mutex::new(Cached::default()),
            memory: Mutex::new(Cached::default()),
            readers: Mutex::new(BTreeMap::new()),
            reader_gen: std::sync::atomic::AtomicU64::new(0),
            agent_input: Mutex::new(BTreeMap::new()),
            stop: Arc::new(AtomicBool::new(false)),
            warnings: Mutex::new(warnings),
            sync_errors: Mutex::new(BTreeMap::new()),
        });
        // Claude Code's version and login (J0): `claude` starts slowly, off the startup path.
        {
            let claude = match &core.rt {
                Some(rt) => rt.cfg.claude.clone(),
                None => cfg.claude.clone().unwrap_or_else(|| RuntimeConfig::new(core.paths.clone()).claude),
            };
            let core2 = core.clone();
            std::thread::spawn(move || {
                let c = setup::check_claude(&claude);
                *core2.claude.lock_ok() = Some(c);
                if let Some(rt) = &core2.rt {
                    rt.touch(orch_protocol::Topic::Setup);
                }
            });
        }
        let rt2 = core.rt.clone();
        crate::watch::start(core.watch.clone(), core.paths.root.join("logs"), core.stop.clone(), move || {
            if let Some(rt) = &rt2 {
                rt.touch(orch_protocol::Topic::Memory);
            }
        });
        if let Some(rt) = &core.rt {
            if let Err(e) = core.serve() {
                eprintln!("orchestrator: bus server failed: {e:#}");
            }
            for path in &cfg.projects {
                if let Err(e) = core.project_add(path) {
                    core.warn(format!("ORCH_PROJECTS: {} not registered: {e:#}", path.display()));
                }
            }
            // Off the startup path (git per project): the projects of known
            // agents (registered by an earlier version or `orchd`), then J10.
            let (core2, rt, recover) = (core.clone(), rt.clone(), cfg.recover);
            std::thread::spawn(move || {
                let known = rt
                    .store
                    .read(|s| s.agents.values().map(|a| a.repo.clone()).collect::<std::collections::BTreeSet<_>>());
                for path in &known {
                    if core2.project_of(path, &core2.projects()).is_none() {
                        if let Err(e) = rt.add_project(path) {
                            eprintln!("orchestrator: {} not registered: {e:#}", path.display());
                        }
                    }
                }
                if recover {
                    if let Err(e) = rt.recover(true) {
                        eprintln!("orchestrator: recovery failed: {e:#}");
                    }
                }
            });
        }
        core
    }

    pub fn runtime(&self) -> Result<&Arc<Runtime>> {
        self.rt.as_ref().ok_or_else(|| anyhow!(self.core_error.clone().unwrap_or_else(|| "no core".into())))
    }

    /// Bus socket + the runtime's loops (pump 200 ms, tick 30 s, history 60 s).
    fn serve(self: &Arc<Self>) -> Result<()> {
        let rt = self.runtime()?.clone();
        let listener = orch_protocol::bind_listener(&self.paths.socket())?;
        let me = Arc::downgrade(self);
        let stop = self.stop.clone();
        std::thread::Builder::new().name("orch-loops".into()).spawn(move || {
            let mut last_tick = Instant::now();
            let mut last_hist: Option<Instant> = None;
            let mut last_house: Option<Instant> = None;
            while !stop.load(Ordering::SeqCst) {
                rt.pump();
                if last_tick.elapsed() >= TICK_EVERY {
                    rt.tick();
                    last_tick = Instant::now();
                }
                // Memory-pressure cap, idle suspend, transcript index, retention.
                if last_house.is_none_or(|t| t.elapsed() >= HOUSEKEEPING_EVERY) {
                    rt.housekeeping();
                    last_house = Some(Instant::now());
                }
                if last_hist.is_none_or(|t| t.elapsed() >= HISTORY_EVERY) {
                    if let Some(c) = me.upgrade() {
                        c.sample_history();
                    }
                    last_hist = Some(Instant::now());
                }
                std::thread::sleep(PUMP_EVERY);
            }
        })?;
        // Background git fetch (FR-103) on its own thread: network calls take a while.
        {
            let (rt, stop) = (self.runtime()?.clone(), self.stop.clone());
            std::thread::Builder::new().name("orch-fetch".into()).spawn(move || {
                let mut last: Option<Instant> = None;
                while !stop.load(Ordering::SeqCst) {
                    if last.is_none_or(|t| t.elapsed() >= FETCH_EVERY) {
                        rt.fetch_tick(&rt.cfg.fetch, rt.cfg.conditions.as_ref());
                        last = Some(Instant::now());
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })?;
        }
        let me = Arc::downgrade(self);
        let stop = self.stop.clone();
        std::thread::Builder::new().name("orch-bus".into()).spawn(move || accept_loop(listener, me, stop))?;
        Ok(())
    }

    fn sample_history(&self) {
        let Some(rt) = &self.rt else { return };
        let (p, week_start) = rt.with_governor(|g| {
            let s = g.snapshot(Timestamp::now());
            (
                HistoryPoint {
                    t_ms: s.ts.as_millisecond(),
                    used: s.used,
                    governed: s.governed,
                    five_hour: s.five_hour,
                },
                g.plan().week_start.as_millisecond(),
            )
        });
        self.history.lock_ok().push(p, week_start);
    }

    fn warn(&self, w: String) {
        eprintln!("orchestrator: {w}");
        self.warnings.lock_ok().push(w);
    }

    /// The Claude Code check (J0), once the background probe has answered.
    pub fn claude_check(&self) -> Option<ClaudeCheck> {
        self.claude.lock_ok().clone()
    }

    /// Graceful stop: suspend agents (resumed by id next time), close shells.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.agent_input.lock_ok().clear(); // ends the writer threads
        if let Some(rt) = &self.rt {
            // The planner's last point (the runtime stores one every 5 min).
            let snap = rt.with_governor(|g| g.snapshot(Timestamp::now()));
            let _ = rt.db.add_forecast(&snap);
            rt.shutdown();
        }
        self.terms.kill_all();
        let _ = UnixStream::connect(self.paths.socket()); // wake the accept loop
        let _ = std::fs::remove_file(self.paths.socket());
    }

    // ------------------------------------------------------------ hooks

    fn handle_conn(&self, s: UnixStream) {
        let Ok(rt) = self.runtime().cloned() else { return };
        let mut reader = BufReader::new(&s);
        let reply = |r: Reply| {
            let _ = bus::write_line(&mut &s, &ReplyEnvelope::new(r));
        };
        let env: Envelope = match bus::read_line(&mut reader) {
            Ok(e) => e,
            Err(e) => return reply(Reply::Error { message: e.to_string() }),
        };
        if env.v != PROTOCOL_VERSION {
            return reply(Reply::Error {
                message: format!("protocol version {} (core speaks {PROTOCOL_VERSION})", env.v),
            });
        }
        match env.msg {
            // PermissionRequest included: the runtime holds it as a store
            // Inbox item, and parks it when nobody answers in time (FR-705).
            Msg::Hook(m) => {
                let output = rt.on_hook(&m, Some(&s));
                reply(Reply::Hook { output });
            }
            Msg::Status(m) => {
                rt.on_status(&m);
                reply(Reply::Ack);
            }
            // orch-mcp: commits (and PR requests) the core runs for the agent.
            Msg::Mcp(m) => {
                let (is_error, text, data) = rt.on_mcp(&m, Some(&s));
                reply(Reply::Mcp { is_error, text, data });
            }
            Msg::Ctl(CtlRequest::Attach { agent_id, rows, cols }) => {
                if !rt.live_ids().contains(&agent_id) {
                    return reply(Reply::Ctl(CtlReply::err(format!("agent {agent_id} is not running"))));
                }
                if self.readers.lock_ok().contains_key(&agent_id) {
                    return reply(Reply::Ctl(CtlReply::err(format!("agent {agent_id} is open in the app"))));
                }
                reply(Reply::Ctl(CtlReply::ok(json!("attached"))));
                let initial = reader.buffer().to_vec();
                if let Ok(clone) = s.try_clone() {
                    let _ = rt.attach(&agent_id, clone, &initial, rows, cols);
                }
            }
            // The app owns its lifetime: `orchd stop` does not quit it.
            Msg::Ctl(CtlRequest::Shutdown) => reply(Reply::Ctl(CtlReply::err("quit the app to stop its core"))),
            Msg::Ctl(req) => reply(Reply::Ctl(agent_runtime::server::ctl(&rt, req))),
        }
    }

    // ------------------------------------------------------------ agents

    pub fn agent_start(&self, req: &NewSession) -> Result<String> {
        let rt = self.runtime()?;
        // J0: without a supported, logged-in Claude Code the app is read-only.
        if let Some(c) = self.claude_check() {
            c.ready()?;
        }
        let project = self.project(&req.project_id)?;
        let id = req.name.trim().to_string();
        if !agent_runtime::paths::valid_id(&id) {
            bail!("name {id:?}: use letters, digits, '-' or '_'");
        }
        let spec = AgentSpec {
            agent_id: id.clone(),
            repo: project.path.display().to_string(),
            slug: Some(id.clone()),
            base: req.base.clone().filter(|b| !b.is_empty()),
            model: req.model.clone().filter(|m| !m.is_empty()),
            effort: req.effort.clone().filter(|e| !e.is_empty()),
            permission_mode: req.permission_mode.clone().filter(|m| !m.is_empty()),
            first_message: req.message.clone().filter(|m| !m.trim().is_empty()),
            tier: req.tier.clone(),
            // Charged to the project's allocation when the plan has one (FR-802).
            allocation: rt
                .with_governor(|g| g.plan().allocations.iter().any(|a| a.id.0 == project.id))
                .then(|| project.id.clone()),
            ..Default::default()
        };
        rt.start(&spec)?;
        Ok(id)
    }

    /// The UI takes the agent's output (pull transport): with a consumer the
    /// buffer applies backpressure, and xterm.js answers terminal queries.
    fn live(&self, id: &str) -> Result<Arc<agent_runtime::runtime::Live>> {
        self.runtime()?.live(id).ok_or_else(|| anyhow!("agent {id} is not running"))
    }

    /// Returns the reader generation the pane passes to `agent_read`: a newer
    /// pane for the same agent makes older readers stale.
    pub fn agent_attach(&self, id: &str, rows: u16, cols: u16) -> Result<u64> {
        let l = self.live(id)?;
        l.pty.output.set_attached(true);
        // An idle TUI draws nothing new: a size change makes it redraw for the
        // new pane (as the runtime's own `attach` does).
        let _ = l.pty.resize(rows, cols.saturating_sub(1).max(1));
        let _ = l.pty.resize(rows, cols);
        let reader = self.reader_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let mut r = self.readers.lock_ok();
        let carry = r.remove(id).map(|old| old.carry).unwrap_or_default();
        r.insert(id.to_string(), Reader { reader, carry });
        Ok(reader)
    }

    /// Only the current reader detaches (a closed pane's detach can arrive
    /// after the reopened pane attached).
    pub fn agent_detach(&self, id: &str, reader: u64) {
        let mut r = self.readers.lock_ok();
        if r.get(id).is_some_and(|x| x.reader == reader) {
            r.remove(id);
            if let Some(l) = self.rt.as_ref().and_then(|rt| rt.live(id)) {
                l.pty.output.set_attached(false);
            }
        }
    }

    fn current_reader(&self, id: &str, reader: u64) -> bool {
        self.readers.lock_ok().get(id).is_some_and(|r| r.reader == reader)
    }

    /// Pull: everything buffered, waiting up to `wait` for output. Err when
    /// the process is gone (the pane then shows its state strip) or when a
    /// newer pane reads this agent; bytes a stale reader drained go to it.
    pub fn agent_read(&self, id: &str, reader: u64, wait: Duration) -> Result<Vec<u8>> {
        let l = self.live(id)?;
        {
            let mut r = self.readers.lock_ok();
            match r.get_mut(id) {
                Some(x) if x.reader == reader => {
                    if !x.carry.is_empty() {
                        return Ok(std::mem::take(&mut x.carry));
                    }
                }
                _ => bail!("stale reader for {id}"),
            }
        }
        let b = l.pty.output.read_wait(wait);
        if !self.current_reader(id, reader) {
            if let Some(x) = self.readers.lock_ok().get_mut(id) {
                x.carry.extend_from_slice(&b);
            }
            bail!("stale reader for {id}");
        }
        if b.is_empty() && l.pty.output.is_closed() {
            bail!("agent {id} exited");
        }
        Ok(b)
    }

    /// Queues the user's keystrokes for the agent's PTY, in order; a writer
    /// thread per agent does the (possibly blocking) write.
    pub fn agent_write(&self, id: &str, data: &[u8]) -> Result<()> {
        self.live(id)?;
        let rt = self.runtime()?.clone();
        let mut w = self.agent_input.lock_ok();
        let tx = match w.get(id) {
            Some(tx) => tx.clone(),
            None => {
                // Writers of agents no longer running (stopped, killed,
                // deleted) end here: dropping the sender ends the thread.
                let live = rt.live_ids();
                w.retain(|a, _| live.contains(a));
                let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                let agent = id.to_string();
                std::thread::Builder::new().name(format!("agent-{id}-in")).spawn(move || {
                    for b in rx {
                        // The process may have been resumed: look it up each time.
                        // FR-1311: typing (not terminal replies) may be a pane intervention.
                        let _ = rt.pane_input(&agent, &b);
                    }
                })?;
                w.insert(id.to_string(), tx.clone());
                tx
            }
        };
        tx.send(data.to_vec()).map_err(|_| anyhow!("agent {id}: input closed"))
    }

    pub fn agent_action(&self, id: &str, action: &str, message: Option<&str>) -> Result<Value> {
        let rt = self.runtime()?;
        Ok(match action {
            "interrupt" => {
                rt.interrupt(id)?;
                json!("esc sent")
            }
            "suspend" => json!(rt.suspend(id)?),
            "kill" => json!(rt.kill(id)?),
            "delete_plan" => rt.delete_plan(id)?,
            "resume" => {
                rt.resume(id, message)?;
                json!("resumed")
            }
            "deliver" => json!(rt.deliver_user(id, message.ok_or_else(|| anyhow!("no message"))?)?),
            // FR-805/811: the Governor holds this agent's autonomous work (until unpause).
            "pause" => json!(rt.pause_agent(id)?),
            "unpause" => json!(rt.resume_agent(id)),
            other => bail!("unknown agent action {other}"),
        })
    }

    /// FR-415: the agent again as a fresh session with its brief (agent menu, `startup` item).
    pub fn restart_fresh(&self, id: &str) -> Result<Value> {
        let session = self.runtime()?.restart_fresh(id, "restarted fresh by the user", "user")?;
        Ok(agent_runtime::startup::restarted(&session))
    }

    /// FR-202: kill (checked) and hide the agent, its worktree on request.
    pub fn agent_delete(&self, id: &str, remove_worktree: bool, confirm_discard: bool) -> Result<Value> {
        self.runtime()?.delete_agent(id, remove_worktree, confirm_discard)
    }

    /// Ends the feature's runs, deletes its agents, archives it.
    pub fn feature_delete(&self, feature_id: &str, remove_worktrees: bool, discard: bool) -> Result<Value> {
        self.runtime()?.delete_feature(feature_id, remove_worktrees, discard)
    }

    // ------------------------------------------------------------ terminals

    fn project(&self, id: &str) -> Result<Project> {
        self.projects().into_iter().find(|p| p.id == id).ok_or_else(|| anyhow!("no project {id}"))
    }

    /// cwd for a new pane (FR-1003): the agent's worktree, else the project
    /// root, else the home folder.
    pub fn pane_cwd(&self, project_id: Option<&str>, agent_id: Option<&str>) -> PathBuf {
        if let (Some(a), Some(rt)) = (agent_id, &self.rt) {
            if let Some(wt) = rt.store.read(|s| s.agents.get(a).map(|r| r.worktree.clone())) {
                return wt;
            }
        }
        if let Some(p) = project_id.and_then(|p| self.project(p).ok()) {
            return p.path;
        }
        agent_runtime::paths::home_dir()
    }

    pub fn term_spawn(&self, kind: TermKind, cwd: &Path, run: Option<&str>, rows: u16, cols: u16) -> Result<TermInfo> {
        let t = self.terms.spawn(kind, cwd, run, rows, cols)?;
        Ok(TermInfo { id: t.id, kind, cwd: t.cwd.clone() })
    }

    /// FR-1101/1102: reuse the project's nvim if it answers, else a new pane
    /// running the editor template (nvim listens on the project's socket).
    pub fn editor_open(
        &self,
        project_id: Option<&str>,
        agent_id: Option<&str>,
        file: Option<&str>,
        line: Option<u32>,
        rows: u16,
        cols: u16,
    ) -> Result<EditorOpen> {
        let cwd = self.pane_cwd(project_id, agent_id);
        let sock = editor::server_socket(&self.paths.root, project_id.unwrap_or("default"));
        match file {
            Some(f) => {
                let abs = if Path::new(f).is_absolute() { PathBuf::from(f) } else { cwd.join(f) };
                if editor::open_remote(&sock, &abs.display().to_string(), line)? {
                    return Ok(EditorOpen { reused: true, term: None });
                }
            }
            // No file: the project's nvim, if it runs, is the editor to show.
            None if orch_protocol::is_listening(&sock) => return Ok(EditorOpen { reused: true, term: None }),
            None => {}
        }
        if let Some(dir) = sock.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&sock); // stale: nobody answered on it
        let template = self.editor_template.lock_ok().clone();
        let cmd = editor::render(&template, file, line, Some(&sock));
        let term = self.term_spawn(TermKind::Editor, &cwd, Some(&cmd), rows, cols)?;
        Ok(EditorOpen { reused: false, term: Some(term) })
    }

    /// FR-1004: the real kitty at the same cwd (remote control when kitty.conf
    /// enables it, otherwise a new instance).
    pub fn open_in_kitty(&self, cwd: &Path) -> Result<()> {
        // Remote control first; `listen_on` may hold placeholders kitty expands
        // ({kitty_pid}), so a failure falls back to a new instance.
        let listen_on = self.kitty.lock_ok().listen_on.clone();
        if let Some(to) = listen_on.as_deref().filter(|t| !t.contains('{')) {
            let remote = std::process::Command::new("kitty")
                .args(["@", "--to", to, "launch", "--type=os-window", "--cwd"])
                .arg(cwd)
                .status();
            if remote.is_ok_and(|s| s.success()) {
                return Ok(());
            }
        }
        let st = std::process::Command::new("open")
            .args(["-na", "kitty", "--args", "--directory"])
            .arg(cwd)
            .status()
            .context("starting kitty")?;
        if !st.success() {
            bail!("kitty exited with {st}");
        }
        Ok(())
    }

    // ------------------------------------------------------------ inbox

    /// The user's action on a decision (api-contract §4): `{ok, lifecycle}`,
    /// the state after the call (usually decided or sent; the effect comes
    /// in the `now` event), plus `effect` as in P4.0.
    pub fn inbox_act(&self, item: &str, action: &str, message: Option<&str>) -> Result<Value> {
        let rt = self.runtime()?;
        let message = message.filter(|m| !m.trim().is_empty());
        // `decide` returns the lifecycle after the call.
        let done = |r: Value| json!({"ok": true, "effect": r["effect"], "lifecycle": r["lifecycle"], "result": r});
        // A synthesized decision (no stored row): its lifecycle is the answer.
        let by_user = |state: &'static str, effect: Option<&str>| {
            json!(agent_runtime::decision::Lifecycle {
                state,
                decided_by: Some("user".into()),
                effect: effect.map(str::to_string),
                ..Default::default()
            })
        };
        // A permission prompt, held or parked (FR-705): the runtime decides,
        // audits and acts (the hook answers, or the agent is told).
        if let Some(n) = item.strip_prefix("inbox-") {
            let n: u64 = n.parse()?;
            let found = rt.store.read(|st| st.inbox.iter().find(|i| i.id == n).map(|i| (i.status, i.payload.clone())));
            let open = found.as_ref().map(|(st, _)| *st);
            // "Always allow" for the agent's project, or for the agent itself when it has none.
            let in_project = found.is_some_and(|(_, p)| p.get("project_id").is_some_and(Value::is_string));
            let scope = (action == "always").then_some(if in_project { "project" } else { "agent" });
            let decision = match action {
                "approve" => InboxDecision::Approve,
                "reject" | "reply" => InboxDecision::Reject,
                "always" => InboxDecision::AlwaysAllow,
                // The dialog is in the pane while the hook is held: nothing to decide here.
                "in_pane" if open == Some(agent_runtime::store::InboxStatus::Pending) => {
                    return Ok(json!({"ok": true, "effect": "in_pane", "lifecycle": rt.lifecycle(n)}));
                }
                "in_pane" => bail!("the prompt is no longer in the pane"),
                other => bail!("{other} does not apply to a permission"),
            };
            let r = rt.decide(n, decision, scope, message)?;
            return Ok(done(r));
        }
        // A start that never happened (FR-415).
        if let Some(n) = item.strip_prefix("startup-") {
            let n: u64 = n.parse()?;
            let agent = rt.store.read(|s| s.inbox.iter().find(|i| i.id == n).and_then(|i| i.agent_id.clone()));
            let agent = agent.ok_or_else(|| anyhow!("no startup item {n}"))?;
            return match action {
                "restart_fresh" => {
                    let r = self.restart_fresh(&agent)?;
                    Ok(json!({"ok": true, "effect": "restarted", "lifecycle": rt.lifecycle(n), "result": r}))
                }
                "dismiss" => Ok(done(rt.decide(n, InboxDecision::Dismiss, None, None)?)),
                // Deleted first: a kill that fails leaves the card to try again.
                "delete" => {
                    let r = self.agent_delete(&agent, false, false)?;
                    let _ = rt.decide(n, InboxDecision::Dismiss, None, None);
                    rt.advance(n, agent_runtime::inbox::Stage::Effect("deleted", None));
                    Ok(json!({"ok": true, "effect": "deleted", "lifecycle": rt.lifecycle(n), "result": r}))
                }
                other => bail!("{other} does not apply to {item}"),
            };
        }
        if let Some(n) = item.strip_prefix("alert-") {
            let n: u64 = n.parse()?;
            let found = rt.store.read(|s| {
                s.inbox.iter().find(|i| i.id == n).map(|i| (i.kind.clone(), i.payload.clone(), i.agent_id.clone()))
            });
            let (kind, payload, agent) = found.ok_or_else(|| anyhow!("no item {n}"))?;
            // An agent that could not resume (`spawn_failed`): restarted fresh, then closed.
            if action == "restart_fresh" {
                let agent = agent.ok_or_else(|| anyhow!("{item} has no agent"))?;
                let r = self.restart_fresh(&agent)?;
                let _ = rt.decide(n, InboxDecision::Dismiss, None, None);
                rt.advance(n, agent_runtime::inbox::Stage::Effect("restarted", None));
                return Ok(json!({"ok": true, "effect": "restarted", "lifecycle": rt.lifecycle(n), "result": r}));
            }
            // An MCP sign-in alert: `open` and `reconnect` say where to go
            // (decisions D3); it closes once the server is connected again.
            if payload["alert"] == agent_runtime::registry::NEEDS_AUTH_ALERT
                && matches!(action, "open" | "open_settings" | "reconnect")
            {
                return self.mcp_alert_action(&payload, if action == "reconnect" { "reconnect" } else { "open" });
            }
            // A Notion comment (FR-505): read once opened; the UI opens the page.
            if (kind == "notion" || payload["source"] == "notion") && action == "open" {
                rt.decide(n, InboxDecision::Dismiss, None, None)?;
                return Ok(json!({"open": "url", "url": payload["page_url"], "lifecycle": rt.lifecycle(n)}));
            }
            // A budget item: applied or skipped (FR-807).
            if kind == "budget" && matches!(action, "approve" | "reject") {
                let d = if action == "approve" { InboxDecision::Approve } else { InboxDecision::Reject };
                return Ok(done(rt.decide(n, d, None, None)?));
            }
            return Ok(done(rt.decide(n, InboxDecision::Dismiss, None, None)?));
        }
        // Gates and team questions: the runtime decides (audited) and acts
        // (spec gate → the run moves on; PR gate → push + PR; reply → a turn).
        if let Some(n) = item.strip_prefix("gate-").or_else(|| item.strip_prefix("q-")) {
            let n: u64 = n.parse()?;
            let question = item.starts_with("q-");
            let decision = match action {
                "approve" => InboxDecision::Approve,
                "reject" | "dismiss" if question => InboxDecision::Dismiss,
                "reject" => InboxDecision::Reject,
                "reply" | "answer" => InboxDecision::Reply,
                other => bail!("{other} does not apply to {item}"),
            };
            if decision == InboxDecision::Reply && message.is_none() {
                bail!("empty reply");
            }
            return Ok(done(rt.decide(n, decision, None, message)?));
        }
        // The Governor's ask (FR-810): synthesized, so its lifecycle is the answer.
        if let Some(n) = item.strip_prefix("ask-") {
            let id = governor::ActionId(n.parse()?);
            let apply = match action {
                "approve" => true,
                "reject" => false,
                other => bail!("{other} does not apply to {item}"),
            };
            rt.answer_ask(id.0, apply)?;
            let effect = if apply { "applied" } else { "skipped" };
            return Ok(json!({"ok": true, "effect": effect,
                             "lifecycle": by_user("effect", Some(effect))}));
        }
        if let Some(a) = item.strip_prefix("wait-").or_else(|| item.strip_prefix("error-")) {
            if action == "reply" {
                let m = message.ok_or_else(|| anyhow!("empty reply"))?;
                // It comes after the prompt in the pane is answered: say what it answers.
                let text = if item.starts_with("wait-") {
                    format!("ORCH: the user answered from the Inbox while a prompt was open in your pane: {m}")
                } else {
                    m.to_string()
                };
                let effect = rt.delivered_effect(a);
                let how = if effect == "delivered" { "deliver" } else { "resume" };
                let r = self.agent_action(a, how, Some(&text))?;
                return Ok(json!({"ok": true, "effect": effect, "lifecycle": by_user("sent", None), "result": r}));
            }
            if matches!(action, "resume" | "reject") && item.starts_with("error-") {
                let r = self.agent_action(a, "resume", None)?;
                return Ok(json!({"ok": true, "effect": "resumed", "lifecycle": by_user("sent", None), "result": r}));
            }
        }
        bail!("{action} does not apply to {item}")
    }

    // ------------------------------------------------------------ views

    /// Git state of `paths`: every `GIT_EVERY`, or for paths not seen yet,
    /// one `git status` per path, in parallel.
    fn git_for(&self, paths: &std::collections::BTreeSet<PathBuf>) -> BTreeMap<PathBuf, GitSummary> {
        let mut g = self.git.lock_ok();
        let all = g.stale(GIT_EVERY);
        let todo: Vec<&PathBuf> = paths.iter().filter(|p| all || !g.value.contains_key(*p)).collect();
        if !todo.is_empty() {
            let fresh: Vec<(PathBuf, GitSummary)> = std::thread::scope(|sc| {
                let jobs: Vec<_> = todo.iter().map(|p| sc.spawn(|| ((*p).clone(), services::git_summary(p)))).collect();
                jobs.into_iter().filter_map(|j| j.join().ok()).collect()
            });
            let mut value = if all { BTreeMap::new() } else { std::mem::take(&mut g.value) };
            value.extend(fresh);
            if all {
                g.set(value);
            } else {
                g.value = value;
            }
        }
        g.value.iter().filter(|(p, _)| paths.contains(*p)).map(|(p, s)| (p.clone(), s.clone())).collect()
    }

    /// The project of an agent's repository: projects and agents both record
    /// the main checkout (FR-105), as resolved paths.
    fn project_of(&self, repo: &Path, projects: &[Project]) -> Option<String> {
        projects.iter().find(|p| p.path == repo).map(|p| p.id.clone())
    }

    fn memory(&self, agents_mb: f64) -> AppMemory {
        let mut m = self.memory.lock_ok();
        if m.stale(MEMORY_EVERY) {
            let pids: Vec<i32> =
                self.terms.ids().iter().filter_map(|i| self.terms.get(*i).ok()?.pid.map(|p| p as i32)).collect();
            m.set(memory::measure(&pids, agents_mb));
        }
        m.value.clone()
    }

    /// The open decisions (no git, no usage): what the notifier reads, also
    /// while the window is hidden.
    pub fn attention(&self) -> Vec<agent_runtime::decision::Decision> {
        self.rt.as_ref().map(|rt| rt.open_decisions()).unwrap_or_default()
    }

    /// Everything the shell shows, polled by the UI about once a second.
    pub fn state(&self) -> Value {
        let projects = self.projects();
        let mut agents = vec![];
        let mut warnings: Vec<Value> = self.warnings.lock_ok().iter().map(|w| json!(w)).collect();
        let (mut usage, mut queue, mut meta) = (Value::Null, vec![], BTreeMap::new());
        // Agents paused one by one (FR-805/811): idle ones have no queue entry.
        let mut paused = std::collections::BTreeSet::new();
        if let Some(rt) = &self.rt {
            let st = rt.state_json();
            agents = st["agents"].as_array().cloned().unwrap_or_default();
            warnings.extend(st["warnings"].as_array().cloned().unwrap_or_default());
            usage = rt.usage_json();
            if let Some(u) = usage.as_object_mut() {
                // Per-agent usage is already in `agents`, and `snapshot` replaces `governor`.
                u.remove("agents");
                u.remove("governor");
            }
            let (snapshot, reset, shadow);
            (queue, snapshot, reset, shadow, paused) = rt.with_governor(|g| {
                let now = Timestamp::now();
                let paused = g.paused_agents().iter().map(|a| a.0.clone()).collect();
                let plan = g.plan();
                (g.queue().to_vec(), g.snapshot(now), plan.reset_at.as_millisecond(), plan.shadow_mode, paused)
            });
            usage["live_cap"] = json!(snapshot.live_cap);
            usage["snapshot"] = json!(snapshot);
            usage["reset_at_ms"] = json!(reset);
            usage["shadow"] = json!(shadow);
            // One look at the Governor for every agent's note and off-hours flag.
            let ids: Vec<String> = agents.iter().map(|a| a["id"].as_str().unwrap_or("").to_string()).collect();
            let per_agent: Vec<(Option<String>, bool)> = rt.with_governor(|g| {
                ids.iter()
                    .map(|id| {
                        let a = governor::AgentId(id.clone());
                        (g.model_override(&a).map(|o| o.note()), g.running_off_hours(&a))
                    })
                    .collect()
            });
            for (a, (note, off)) in agents.iter_mut().zip(per_agent) {
                a["model_note"] = json!(note);
                a["off_hours"] = json!(off);
            }
            meta = rt.store.read(store_view);
        }
        let mut git_paths: std::collections::BTreeSet<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
        git_paths.extend(meta.values().map(|m: &AgentMeta| m.worktree.clone()));
        let git = self.git_for(&git_paths);
        let readers = self.readers.lock_ok().keys().cloned().collect::<std::collections::BTreeSet<_>>();
        let mut agents_mb = 0.0;
        for a in agents.iter_mut() {
            let id = a["id"].as_str().unwrap_or("").to_string();
            agents_mb += a["memory_mb"].as_f64().unwrap_or(0.0);
            if let Some(m) = meta.get(&id) {
                a["created_at_ms"] = json!(m.created_at_ms);
                a["effort"] = json!(m.effort);
                a["project_id"] = json!(self.project_of(&m.repo, &projects));
                a["branch"] = json!(m.branch);
                a["git"] = json!(git.get(&m.worktree));
            }
            if let Some(q) = queue.iter().find(|e| e.agent.0 == id) {
                a["queue_entry"] = json!(q);
            }
            a["paused"] = json!(paused.contains(&id));
            a["attached"] = json!(readers.contains(&id));
        }
        // `null` when the read failed: the UI keeps the features it has.
        let features = match self.rt.as_deref().map(crate::teams::features).transpose() {
            Ok(f) => json!(f.unwrap_or_default()),
            Err(e) => {
                warnings.push(json!(format!("features not read: {e:#}")));
                Value::Null
            }
        };
        // FR-816: the power assertion and the on-battery banner.
        let (power, banner) = self.rt.as_ref().map(|rt| rt.power_json()).unwrap_or((Value::Null, None));
        let banners: Vec<Value> = banner.into_iter().collect();
        let setup = self.setup_view(&projects);
        let projects: Vec<Value> = projects
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "path": p.path, "default_branch": p.default_branch,
                    "last_fetch_at_ms": p.last_fetch_at_ms, "last_fetch_error": p.last_fetch_error,
                    "git": git.get(&p.path),
                })
            })
            .collect();
        json!({
            "now_ms": now_ms(),
            "core": {"ok": self.rt.is_some(), "error": self.core_error, "home": self.paths.root, "user_home": agent_runtime::paths::home_dir()},
            "warnings": warnings,
            "projects": projects,
            "agents": agents,
            "usage": usage,
            "queue": queue,
            "features": features,
            "memory": self.memory(agents_mb),
            "memory_watch": self.watch.view(),
            "setup": setup,
            "power": power,
            "banners": banners,
        })
    }

    pub fn planner(&self) -> Result<PlannerView> {
        let rt = self.runtime()?;
        let five_reset =
            rt.usage_json()["five_hour"]["resets_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok());
        let live = rt.live_ids();
        let alloc_of: BTreeMap<String, String> =
            rt.store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.allocation.clone())).collect());
        let history = self.history.lock_ok();
        let mut v = rt.with_governor(|g| {
            planner::build(g, Timestamp::now(), &history, &self.past_actions, five_reset, &|alloc| {
                live.iter().filter(|id| alloc_of.get(*id).is_some_and(|a| a == alloc)).count()
            })
        });
        v.first_run_ms = self.settings.get(setup::FIRST_RUN_KEY).and_then(|v| v.as_i64());
        v.plan_set = self.settings.get(setup::PLAN_KEY).is_some();
        Ok(v)
    }

    /// Week planner and title-bar controls (FR-809, FR-811).
    pub fn governor_act(&self, action: &str, arg: Option<f64>) -> Result<Value> {
        let rt = self.runtime()?;
        let id = || arg.map(|a| a as u64).ok_or_else(|| anyhow!("{action} needs an id"));
        Ok(match action {
            "pause_all" => json!(rt.pause_all()),
            "resume_all" => json!(rt.resume_all()),
            "run_now" => {
                rt.run_now(id()?)?;
                json!(true)
            }
            "cancel" => {
                rt.cancel(id()?)?;
                json!("cancelled")
            }
            "undo" => {
                rt.undo(id()?)?;
                json!("undone")
            }
            "sprint" => json!(rt.sprint(arg)?),
            other => bail!("unknown governor action {other}"),
        })
    }
}

// ------------------------------------------------------------ projects (§4.1)

/// Through the runtime when it runs (audit, its worktree rows and agents);
/// registering also works on the app's own store without it.
impl Core {
    /// Registered projects, not archived (FR-101).
    pub fn projects(&self) -> Vec<Project> {
        self.db.projects(false).unwrap_or_default().into_iter().map(Project::from).collect()
    }

    /// FR-101: a local folder (its main checkout is registered).
    pub fn project_add(&self, path: &Path) -> Result<Project> {
        let p = match &self.rt {
            Some(rt) => rt.add_project(path)?,
            None => orch_git::project::register(&self.db, path)?,
        };
        Ok(p.into())
    }

    /// FR-101: clones `url` into `parent` (default `~/dev`), then registers it.
    pub fn project_clone(&self, url: &str, parent: Option<&Path>, name: Option<&str>) -> Result<Project> {
        let parent = parent.map(Path::to_path_buf).unwrap_or_else(|| agent_runtime::paths::home_dir().join("dev"));
        std::fs::create_dir_all(&parent)?;
        Ok(self.runtime()?.clone_project(url, &parent, name)?.into())
    }

    /// Status, worktrees (orphans flagged), branches, and with `with_prs` the
    /// open PRs with CI (FR-102, FR-104, FR-106).
    pub fn project_detail(&self, id: &str, with_prs: bool) -> Result<Value> {
        let mut v = self.runtime()?.project_detail(id, with_prs)?;
        if let Some(p) = self.db.project(id)? {
            v["trusted"] = json!(setup::trusted(&p.path));
        }
        Ok(v)
    }

    /// FR-103 on demand.
    pub fn project_fetch(&self, id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let out = rt.fetch_project(id, &rt.cfg.fetch, rt.cfg.conditions.as_ref())?;
        self.git.lock_ok().at = None; // ahead/behind changed
        Ok(json!(out))
    }

    /// FR-104: one worktree of the app's sibling folder; unmerged work needs `discard`.
    pub fn worktree_remove(&self, project: &str, path: &Path, delete_branch: bool, discard: bool) -> Result<Value> {
        Ok(json!(self.runtime()?.remove_worktree(project, path, delete_branch, discard)?))
    }

    /// The project, one of its git worktrees (the main checkout included) and the
    /// branch its work goes back to: the base it was created from, else the default.
    fn resolve_worktree(
        &self,
        project: &str,
        path: &Path,
    ) -> Result<(Project, orch_git::worktree::GitWorktree, String)> {
        let p = self.project(project)?;
        let want = real(path);
        let wt = orch_git::worktree::list(&p.path)?
            .into_iter()
            .find(|w| real(&w.path) == want)
            .ok_or_else(|| anyhow!("{} is not a worktree of {}", path.display(), p.name))?;
        // The store keys rows by the path the app created; git reports the real path.
        let base = self
            .rt
            .as_ref()
            .and_then(|rt| {
                rt.store.read(|s| {
                    s.worktrees
                        .get(&wt.path)
                        .or_else(|| s.worktrees.iter().find(|(k, _)| real(k) == want).map(|(_, w)| w))
                        .map(|w| w.base.clone())
                })
            })
            .unwrap_or_else(|| p.default_branch.clone());
        Ok((p, wt, base))
    }

    /// FR-107: the worktree's changes against its base, from their merge base.
    /// What a gate's preview shows: a spec gate's docs (markdown, read from
    /// the integration worktree), a PR gate's changed files against its base.
    pub fn gate_detail(&self, item: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let n: u64 = item.strip_prefix("gate-").ok_or_else(|| anyhow!("{item} is not a gate"))?.parse()?;
        let it = rt
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == n).cloned())
            .ok_or_else(|| anyhow!("no Inbox item {n}"))?;
        let p = &it.payload;
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let wt = PathBuf::from(s("worktree"));
        match s("action").as_str() {
            "spec_approval" => {
                let slug = s("feature");
                let docs: Vec<Value> = agent_runtime::team::feature_docs(&wt, &slug)
                    .into_iter()
                    .map(|name| {
                        let text = agent_runtime::team::read_feature_doc(&wt, &slug, &name)
                            .unwrap_or_else(|e| format!("({e:#})"));
                        json!({"name": name, "text": text})
                    })
                    .collect();
                // For the docs' images (`feature_asset`); a gate from before it
                // names only its run, which names the feature.
                let feature_id = Some(s("feature_id"))
                    .filter(|f| !f.is_empty())
                    .or_else(|| rt.db.run(&s("run")).ok().flatten().map(|r| r.feature_id));
                Ok(json!({"action": "spec_approval", "summary": s("summary"), "docs": docs, "feature_id": feature_id}))
            }
            "push_pr" => {
                let d = orch_git::diff::diff(&wt, &s("base"), DIFF_PATCH_MAX)?;
                Ok(json!({"action": "push_pr", "title": s("title"), "body": s("body"), "branch": s("branch"),
                          "base": s("base"), "files": d.files, "truncated": d.truncated}))
            }
            // A hold the core found still open at a restart (no payload of its own).
            "restart_gate" => Ok(json!({"action": "restart_gate", "detail": s("detail")})),
            other => bail!("unknown gate {other:?}"),
        }
    }

    pub fn worktree_diff(&self, project: &str, path: &Path) -> Result<Value> {
        let (_, wt, base) = self.resolve_worktree(project, path)?;
        let mut v = json!(orch_git::diff::diff(&wt.path, &base, DIFF_PATCH_MAX)?);
        v["branch"] = json!(wt.branch);
        Ok(v)
    }

    /// The user's own "Create PR" (the UI confirms first): pushes the worktree's
    /// branch with the user's credentials and opens the PR into its base (ADR-009).
    pub fn worktree_create_pr(
        &self,
        project: &str,
        path: &Path,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String> {
        let (p, wt, base) = self.resolve_worktree(project, path)?;
        let branch = wt.branch.clone().ok_or_else(|| anyhow!("the worktree has no branch (detached HEAD)"))?;
        if branch == base || branch == p.default_branch {
            bail!("{branch} is the base branch: there is nothing to open a PR from");
        }
        if title.trim().is_empty() {
            bail!("a PR needs a title");
        }
        let r = orch_git::gh::push(&wt.path, "origin", &branch)
            .and_then(|_| orch_git::gh::create_pr(&wt.path, &base, &branch, title, body, draft));
        let _ = self.db.audit(
            "user",
            None,
            "pr",
            Some(&branch),
            if r.is_ok() { "ok" } else { "failed" },
            json!({"base": base, "result": r.as_ref().map_err(|e| e.to_string())}),
        );
        r
    }

    /// J11 step 1: worktrees, branches and agents of the project.
    pub fn removal_plan(&self, id: &str) -> Result<Value> {
        self.runtime()?.removal_plan(id)
    }

    /// J11: stops and archives the project's agents, removes merged worktrees
    /// and the confirmed ones, keeps the rest, deletes or archives history.
    pub fn project_remove(&self, id: &str, choices: &orch_git::project::RemovalChoices) -> Result<Value> {
        Ok(json!(self.runtime()?.remove_project(id, choices)?))
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The helper binaries Claude Code launches for each agent (FR-1201).
const HELPERS: [&str; 3] = ["orch-hook", "orch-status", "orch-mcp"];

fn start_runtime(cfg: &CoreConfig, paths: &AppPaths, warnings: &mut Vec<String>) -> Result<Arc<Runtime>> {
    if orch_protocol::is_listening(&paths.socket()) {
        bail!("another core is running on {} (orchd?): stop it to run agents in the app", paths.socket().display());
    }
    std::fs::create_dir_all(&paths.root)?;
    let mut rc = RuntimeConfig::new(paths.clone());
    if let Some(c) = &cfg.claude {
        rc.claude = c.clone();
    }
    // FR-812: shadow mode unless ORCH_ENFORCE; the runtime then applies the
    // user's stored choice (`governor.enforce`, also set by `orchd enforce`).
    rc.shadow = !cfg.enforce;
    rc.default_plan = cfg.default_plan;
    // Unit tests never read or write the user's Keychain.
    #[cfg(test)]
    {
        rc.secrets = Arc::new(orch_secrets::MemorySecrets::default());
        // Nor Linear: an in-memory one per backend.
        rc.linear = Some(agent_runtime::linear::LinearFactory(Arc::new(|| {
            Box::new(orch_tasks::linear::fake::FakeLinear::default())
        })));
    }
    // Helpers next to the app binary (target/<profile>, or Contents/MacOS in a
    // bundle) are copied to <home>/bin, a stable path outside the .app.
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let changed = HELPERS.iter().any(|n| {
            let meta = |p: PathBuf| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
            meta(dir.join(n))
                .is_some_and(|src| meta(paths.bin_dir().join(n)).is_none_or(|dst| dst.0 != src.0 || dst.1 < src.1))
        });
        if changed && HELPERS.iter().all(|n| dir.join(n).is_file()) {
            if let Err(e) = agent_runtime::preflight::install_helpers(&dir, &paths.bin_dir()) {
                warnings.push(format!("helpers: {e:#}"));
            }
        }
    }
    if agent_runtime::preflight::check_helpers(&rc.helpers).is_err() {
        warnings.push(format!(
            "orch-hook/orch-status/orch-mcp missing in {}: build them (cargo build -p orch-hook -p orch-status -p orch-mcp); agents can't start",
            paths.bin_dir().display()
        ));
    }
    let rt = Runtime::new(rc)?;
    // The local OTLP receiver (FR-801): agents spawned from now on export to it.
    if let Err(e) = rt.start_otlp(0) {
        warnings.push(format!("OTLP receiver not started: {e:#}"));
    }
    Ok(rt)
}

/// The app's own store when the runtime did not start (another core owns
/// the socket): projects and settings still work. A store that cannot be
/// opened (a newer schema) gives a scratch one, with a warning.
fn open_store(paths: &AppPaths, warnings: &mut Vec<String>) -> orch_store::Store {
    match orch_store::Store::open(&paths.db_file()) {
        Ok(db) => db,
        Err(e) => {
            warnings.push(format!(
                "store {} not opened ({e:#}): projects and settings are not kept",
                paths.db_file().display()
            ));
            let tmp = std::env::temp_dir().join(format!("orchestrator-{}.db", std::process::id()));
            orch_store::Store::open(&tmp).expect("a scratch store in the temp folder")
        }
    }
}

/// The plan week so far, from the store: the "actual" line (forecast
/// snapshots, every 5 min while the app ran) and the log of earlier starts.
fn past_week(rt: &Runtime, started: Timestamp) -> (History, Vec<governor::GovernorAction>) {
    let week_start = rt.with_governor(|g| g.plan().week_start);
    let since = week_start.as_millisecond().max(0) as u64;
    let mut h = History::default();
    for f in rt.db.forecasts_since(since).unwrap_or_default() {
        h.push(
            HistoryPoint { t_ms: f.ts.as_millisecond(), used: f.used, governed: f.governed, five_hour: f.five_hour },
            since as i64,
        );
    }
    let actions = rt
        .db
        .actions(500)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.ts < started) // the planner keeps this plan week's
        .collect();
    (h, actions)
}

/// Agent metadata, in one pass over the store.
fn store_view(s: &agent_runtime::store::StateFile) -> BTreeMap<String, AgentMeta> {
    s.agents
        .values()
        .map(|a| {
            let m = AgentMeta {
                repo: a.repo.clone(),
                worktree: a.worktree.clone(),
                branch: s.worktrees.get(&a.worktree).map(|w| w.branch.clone()).unwrap_or_default(),
                created_at_ms: a.created_at_ms,
                effort: a.role.effort.clone(),
            };
            (a.id.clone(), m)
        })
        .collect()
}

fn accept_loop(listener: UnixListener, core: std::sync::Weak<Core>, stop: Arc<AtomicBool>) {
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(s) = conn else { continue };
        let Some(c) = core.upgrade() else { break };
        std::thread::spawn(move || c.handle_conn(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn test_core(d: &Path) -> Arc<Core> {
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        orch_git::fixtures::git(&repo, &["init", "-q", "-b", "main"]);
        orch_git::fixtures::git(
            &repo,
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"],
        );
        core_on(&d.join("home"), &repo)
    }

    #[test]
    fn integrations_registry_and_mcp_alerts() {
        let d = tempfile::Builder::new().prefix("orchapp").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let core = test_core(d.path());
        let v = core.integrations_view().unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 5);
        assert_eq!(v["items"][1]["status"], "not_configured");
        // The registry starts with the penpot connector.
        let reg = core.mcp_registry_view(None).unwrap();
        assert_eq!(reg["servers"][0]["id"], "penpot");
        assert_eq!(reg["project_overrides"], json!({}));
        let s: orch_team::mcp::McpServer = serde_json::from_value(
            json!({"id": "figma", "name": "Figma", "kind": "http", "url": "https://figma.invalid/mcp", "allow_secret_env": false}),
        )
        .unwrap();
        let scope: orch_team::Scope = serde_json::from_value(json!({"kind": "project", "id": "repo"})).unwrap();
        assert_eq!(core.mcp_server_save(s, scope.clone()).unwrap()["status"], "unknown");
        assert_eq!(
            core.mcp_registry_view(Some("repo")).unwrap()["project_overrides"]["figma"]["url"],
            "https://figma.invalid/mcp"
        );
        assert_eq!(core.mcp_server_delete("figma", scope).unwrap()["deleted"], true);
        // A needs-auth alert carries its payload; open/reconnect say where to go.
        let rt = core.runtime().unwrap().clone();
        let penpot = rt.db.mcp_server(&orch_team::Scope::Global, "penpot").unwrap().unwrap();
        rt.set_mcp_status(&orch_team::Scope::Global, &penpot, orch_team::mcp::McpStatus::NeedsAuth, Some("x")).unwrap();
        let items = decisions(&core);
        let al = items.iter().find(|i| i["payload"]["alert"] == "mcp_needs_auth").unwrap().clone();
        assert_eq!((al["level"].as_u64(), al["payload"]["server"].as_str()), (Some(2), Some("penpot")));
        let id = al["id"].as_str().unwrap();
        assert_eq!(core.inbox_act(id, "reconnect", None).unwrap()["open"], "url");
        assert_eq!(core.inbox_act(id, "open_settings", None).unwrap(), json!({"open": "settings", "server": "penpot"}));
        let r = core.inbox_act(id, "dismiss", None).unwrap();
        assert_eq!((r["effect"].as_str(), r["lifecycle"]["state"].as_str()), (Some("dismissed"), Some("dismissed")));
        // A project override (http) gets its own reconnect target.
        let over: orch_team::mcp::McpServer = serde_json::from_value(
            json!({"id": "penpot", "name": "Penpot", "kind": "http", "url": "https://penpot.invalid/mcp", "allow_secret_env": false}),
        )
        .unwrap();
        let pscope: orch_team::Scope = serde_json::from_value(json!({"kind": "project", "id": "repo"})).unwrap();
        core.mcp_server_save(over, pscope).unwrap();
        let pay = |project: Value| json!({"alert": "mcp_needs_auth", "server": "penpot", "project": project});
        assert_eq!(core.mcp_alert_action(&pay(json!("repo")), "reconnect").unwrap()["run"], "claude /mcp");
        assert_eq!(core.mcp_alert_action(&pay(Value::Null), "reconnect").unwrap()["open"], "url");
        core.shutdown();
    }

    #[test]
    fn presets_duplicate_and_flags_through_the_core() {
        let d = tempfile::Builder::new().prefix("orchapp").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let core = test_core(d.path());
        let v = core.teams_view(None).unwrap();
        let fs = v["teams"].as_array().unwrap().iter().find(|t| t["team"]["id"] == "full-stack").unwrap().clone();
        assert_eq!(
            (fs["preset"].as_bool(), fs["starter"].as_str(), fs["update_available"].as_bool()),
            (Some(true), Some("full-stack"), Some(false))
        );
        let scope = |v: Value| -> orch_team::Scope { serde_json::from_value(v).unwrap() };
        let (g, p) = (scope(json!({"kind": "global"})), scope(json!({"kind": "project", "id": "repo"})));
        let dup = core.team_duplicate("full-stack", &g, &p, Some("shop-team")).unwrap();
        assert_eq!(dup["team"]["scope"], json!({"kind": "project", "id": "repo"}));
        assert_eq!(core.team_set_preset("shop-team", &p, true).unwrap()["preset"], true);
        assert_eq!(core.role_duplicate("ui", &g, &g, None).unwrap()["id"], "ui-copy");
        assert!(core.preset_update("shop-team", &p).unwrap_err().to_string().contains("not a starter"));
        assert_eq!(core.preset_update("full-stack", &g).unwrap()["version"], 1, "already current");
        assert!(core.artifact_preview("nope", 1).is_err());
        core.shutdown();
    }

    fn record(id: &str, repo: PathBuf) -> agent_runtime::store::AgentRecord {
        agent_runtime::store::AgentRecord {
            id: id.into(),
            worktree: repo.clone(),
            repo,
            role: agent_runtime::role::Role::solo(),
            session_id: "s".into(),
            tracker: Default::default(),
            pid: None,
            pgid: None,
            allocation: "default".into(),
            tier: governor::Tier::Normal,
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
        }
    }

    fn tmp() -> tempfile::TempDir {
        // Short path: the socket must fit in 104 bytes.
        tempfile::Builder::new().prefix("orchapp").tempdir_in(orch_protocol::test_tmp()).unwrap()
    }

    /// The decisions of the `now` snapshot, recomputed.
    fn decisions(core: &Core) -> Vec<Value> {
        let rt = core.runtime().unwrap();
        rt.now_pump(true);
        rt.now_snapshot()["decisions"].as_array().cloned().unwrap_or_default()
    }

    #[test]
    fn state_has_the_shell_s_inputs() {
        let d = tmp();
        let core = test_core(d.path());
        assert!(core.core_error.is_none(), "{:?}", core.core_error);
        let s = core.state();
        assert_eq!(s["core"]["ok"], true);
        assert_eq!(s["projects"][0]["id"], "repo");
        assert!(s.get("inbox").is_none(), "decisions come from the now push");
        assert!(s["usage"]["snapshot"]["state"].is_string(), "{}", s["usage"]);
        assert!(s["usage"]["governor"].is_null() && s["usage"]["agents"].is_null(), "dropped from the poll");
        assert!(s["memory"]["core_mb"].as_f64().unwrap() > 0.0);
        let p = core.planner().unwrap();
        assert!(p.curve.len() > 100);
        core.shutdown();
    }

    #[test]
    fn teams_launch_board_feature_and_gates_through_the_core() {
        let d = tmp();
        let core = test_core(d.path());
        let v = core.teams_view(Some("repo")).unwrap();
        let ids: Vec<&str> = v["teams"].as_array().unwrap().iter().map(|t| t["team"]["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"full-stack") && ids.contains(&"bugfix-squad"), "presets installed: {ids:?}");
        assert!(v["teams"][0]["rules"].is_array());
        // Saving a changed team bumps its version; an invalid one is kept as a draft.
        let mut t =
            v["teams"].as_array().unwrap().iter().find(|t| t["team"]["id"] == "bugfix-squad").unwrap()["team"].clone();
        t["nodes"][1]["position"] = json!([10.0, 20.0]);
        let saved = core.team_save(t.clone()).unwrap();
        assert_eq!(saved["team"]["version"], 2);
        let mut bad = t.clone();
        bad["id"] = json!("draft");
        bad["nodes"][0]["is_lead"] = json!(false);
        assert!(!core.team_save(bad).unwrap()["issues"].as_array().unwrap().is_empty());
        let exported = core.team_export("bugfix-squad", None).unwrap();
        assert!(exported.contains("\"format\": 1"));
        // Launch: the feature shows in app_state; the Feature pane and the Board read it.
        let run = core
            .team_launch(agent_runtime::team::LaunchSpec {
                project_id: "repo".into(),
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                title: "Fix X".into(),
                brief: "b".into(),
                base: None,
                off_hours: false,
            })
            .unwrap();
        let s = core.state();
        let f = &s["features"][0];
        assert_eq!((f["slug"].as_str(), f["run"]["status"].as_str()), (Some("fx"), Some("spec")));
        let fid = f["id"].as_str().unwrap().to_string();
        let fv = core.feature_view(&fid).unwrap();
        assert_eq!(fv["agents"][0]["id"], "fx-lead-1");
        assert_eq!(fv["agents"][0]["node"], "lead");
        let t = core
            .task_create(json!({"project_id": "repo", "title": "By hand", "run_id": run["id"], "node": "back"}))
            .unwrap();
        let b = core.board_view("repo").unwrap();
        assert_eq!(b["backend"], "local");
        assert_eq!(b["tasks"][0]["title"], "By hand");
        assert_eq!(b["runs"][0]["nodes"].as_array().unwrap().len(), 3);
        let t = core.task_update(t["id"].as_u64().unwrap(), Some("backlog"), None, Some("later")).unwrap();
        assert_eq!((t["status"].as_str(), t["notes"].as_array().unwrap().len()), (Some("backlog"), 2));
        // A message from the user; pause and resume the team.
        assert!(core.message_send(run["id"].as_str().unwrap(), "lead", "hello").is_ok());
        assert_eq!(core.run_act(run["id"].as_str().unwrap(), "pause").unwrap()["status"], "paused");
        assert_eq!(core.run_act(run["id"].as_str().unwrap(), "resume").unwrap()["status"], "spec");
        // Project settings (FR-109).
        let ps = core
            .project_settings_set(
                "repo",
                serde_json::from_value(json!({"settings": {"default_team": "bugfix-squad"}, "backend": {"type": "local"}, "track_orchestrator_dir": true})).unwrap(),
            )
            .unwrap();
        assert_eq!(ps["settings"]["default_team"], "bugfix-squad");
        // A runtime gate is a level-1 gate in the Inbox and decides through the runtime.
        let rt = core.runtime().unwrap().clone();
        rt.store
            .write(|s| {
                s.add_inbox_item(
                    Some("fx-lead-1"),
                    "gate",
                    "Spec gate · fx".into(),
                    json!({"action": "spec_approval", "run": run["id"], "feature": "fx", "feature_id": fid, "summary": "s"}),
                    20,
                )
            })
            .unwrap();
        rt.set_run_state(run["id"].as_str().unwrap(), orch_team::RunState::SpecGate, "test").unwrap();
        let gate = decisions(&core).into_iter().find(|i| i["kind"] == "gate").unwrap();
        assert_eq!((gate["level"].as_u64(), gate["gate"].as_str()), (Some(1), Some("spec")));
        // The spec gate names its feature, so the docs' images can be read.
        assert_eq!(core.gate_detail(gate["id"].as_str().unwrap()).unwrap()["feature_id"], json!(fid));
        let r = core.inbox_act(gate["id"].as_str().unwrap(), "approve", None).unwrap();
        assert_eq!(r["lifecycle"]["state"], "sent", "the lead is told");
        assert_eq!(core.state()["features"][0]["run"]["status"], "planning");
        assert!(core.inbox_act(gate["id"].as_str().unwrap(), "approve", None).is_err(), "decided once");
        // A feature archived under its team stays listed, flagged, while one
        // of its agents is not deleted; it goes once none is left.
        let mut fr = rt.db.feature(&fid).unwrap().unwrap();
        fr.archived = true;
        rt.db.upsert_feature(&fr).unwrap();
        let s = core.state();
        assert_eq!(
            (s["features"][0]["id"].as_str(), s["features"][0]["archived"].as_bool()),
            (Some(&*fid), Some(true))
        );
        rt.delete_agent("fx-lead-1", false, false).unwrap();
        assert!(core.state()["features"].as_array().unwrap().is_empty());
        core.shutdown();
    }

    #[test]
    fn a_second_core_on_the_same_home_is_refused() {
        let d = tmp();
        let a = test_core(d.path());
        let b = test_core(d.path());
        assert!(b.core_error.as_deref().unwrap_or("").contains("another core"));
        assert!(
            b.agent_start(&NewSession { project_id: "repo".into(), name: "x".into(), ..Default::default() }).is_err()
        );
        a.shutdown();
    }

    #[test]
    fn permission_prompt_is_held_until_the_inbox_answers() {
        let d = tmp();
        let core = test_core(d.path());
        let rt = core.runtime().unwrap().clone();
        rt.store.write(|s| s.agents.insert("a1".into(), record("a1", d.path().join("repo")))).unwrap();
        // What orch-hook sends, on the real socket.
        let sock = core.paths.socket();
        let ask = move |cmd: &str, deadline_ms: u64| {
            let s = bus::connect(&sock, Duration::from_secs(1)).unwrap();
            let msg = Envelope::new(Msg::Hook(orch_protocol::HookMsg {
                agent_id: "a1".into(),
                event: orch_protocol::HookEvent::PermissionRequest,
                ts_ms: now_ms(),
                deadline_ms,
                payload: json!({"hook_event_name": "PermissionRequest", "session_id": "s",
                    "tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": "/w"}),
            }));
            bus::write_line(&mut &s, &msg).unwrap();
            let r: ReplyEnvelope = bus::read_line(&mut BufReader::new(&s)).unwrap();
            r
        };
        let behavior = |r: ReplyEnvelope| match r.reply {
            Reply::Hook { output: Some(o) } => o.0["hookSpecificOutput"]["decision"].clone(),
            other => panic!("{other:?}"),
        };
        let ask2 = ask.clone();
        let hook = std::thread::spawn(move || ask2("pnpm add x", 30_000));
        let mut item = None;
        for _ in 0..50 {
            let s = core.state();
            if let Some(i) = s["inbox"]["items"].as_array().and_then(|i| i.first()).cloned() {
                item = Some(i);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let item = item.expect("the prompt reached the Inbox");
        assert_eq!(item["kind"], "permission");
        assert!(item["id"].as_str().unwrap().starts_with("inbox-"), "the store's item: {item}");
        assert_eq!(item["preview"], "pnpm add x");
        assert_eq!((item["payload"]["cwd"].as_str(), item["status"].as_str()), (Some("/w"), Some("open")));
        assert!(core.state().get("permissions").is_none(), "no second list");
        assert_eq!(core.state()["agents"][0]["state"], "waiting_for_you");
        let id = item["id"].as_str().unwrap().to_string();
        assert_eq!(core.inbox_act(&id, "in_pane", None).unwrap()["effect"], "in_pane");
        assert_eq!(core.inbox_act(&id, "approve", None).unwrap()["effect"], "allowed");
        assert_eq!(behavior(hook.join().unwrap())["behavior"], "allow");
        // Answered: not shown again as "waiting for you" while the next hook is on its way.
        assert_eq!(core.state()["inbox"]["badge"], 0);
        assert!(core.inbox_act(&id, "approve", None).is_err());

        // Nobody answers within the hold (here 2.5 s of deadline − 2 s): denied and parked.
        let d = behavior(ask("pnpm add y", 2_500));
        assert_eq!(
            (d["behavior"].as_str(), d["message"].as_str()),
            (Some("deny"), Some(agent_runtime::inbox::PARKED_MESSAGE))
        );
        let parked = core.state()["inbox"]["items"][0].clone();
        assert_eq!(
            (parked["status"].as_str(), parked["payload"]["tool_input"]["command"].as_str()),
            (Some("parked"), Some("pnpm add y"))
        );
        assert!(!parked["actions"].as_array().unwrap().contains(&json!("in_pane")));
        assert!(core.inbox_act(parked["id"].as_str().unwrap(), "in_pane", None).is_err());
        let r = core.inbox_act(parked["id"].as_str().unwrap(), "approve", None).unwrap();
        assert_eq!(r["effect"], "resumed");
        // The retry runs without a second prompt.
        assert_eq!(behavior(ask("pnpm add y", 30_000))["behavior"], "allow");
        let actors: Vec<String> = rt.db.audit_log(Some("a1"), 10).unwrap().into_iter().rev().map(|e| e.actor).collect();
        assert_eq!(actors, ["user", "expired", "user", "user"]);
        core.shutdown();
    }

    #[test]
    fn agents_of_a_linked_worktree_project_stay_under_it() {
        let d = tmp();
        let main = d.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
        let nested = main.join(".claude/worktrees/feat");
        git(&main, &["worktree", "add", "-q", "-b", "feat", nested.to_str().unwrap()]);
        let core = core_on(&d.path().join("home"), &nested);
        // The runtime records the main checkout as the agent's repo (FR-105).
        let main = main.canonicalize().unwrap();
        core.runtime().unwrap().store.write(|s| s.agents.insert("a1".into(), record("a1", main.clone()))).unwrap();
        let projects = core.projects();
        assert_eq!(projects.len(), 1, "registering the worktree registers its main checkout, once");
        assert_eq!(projects[0].path, main);
        assert_eq!(core.state()["agents"][0]["project_id"], json!(projects[0].id));
        core.shutdown();
    }

    #[test]
    fn opening_the_editor_without_a_file_keeps_a_running_nvim() {
        let d = tmp();
        let core = test_core(d.path());
        let sock = editor::server_socket(&core.paths.root, "repo");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        let _nvim = UnixListener::bind(&sock).unwrap(); // stands in for nvim --listen
        let r = core.editor_open(Some("repo"), None, None, None, 24, 80).unwrap();
        assert!(r.reused && r.term.is_none());
        assert!(sock.exists(), "the live server's socket is kept");
        core.shutdown();
    }

    #[test]
    fn shell_panes_and_cwd_fallbacks() {
        let d = tmp();
        let core = test_core(d.path());
        assert_eq!(core.pane_cwd(Some("repo"), Some("nobody")), d.path().join("repo").canonicalize().unwrap());
        assert_eq!(core.pane_cwd(None, None), agent_runtime::paths::home_dir());
        let t = core.term_spawn(TermKind::Shell, &d.path().join("repo"), Some("/bin/echo hi-there"), 24, 80).unwrap();
        let term = core.terms.get(t.id).unwrap();
        let mut out = String::new();
        while let Some(b) = term.output.read_wait(Duration::from_secs(5)) {
            out.push_str(&String::from_utf8_lossy(&b));
            if out.contains("hi-there") {
                break;
            }
        }
        assert!(out.contains("hi-there"), "{out:?}");
        core.terms.kill(t.id);
        core.shutdown();
    }

    #[test]
    fn governor_controls() {
        let d = tmp();
        let core = test_core(d.path());
        core.governor_act("pause_all", None).unwrap();
        let p = core.planner().unwrap();
        let pause = p.log.iter().find(|l| l.action.kind_name() == "pause_all").expect("logged");
        assert!(pause.can_undo);
        core.governor_act("undo", Some(pause.action.id.0 as f64)).unwrap();
        assert!(core.governor_act("run_now", None).is_err());
        assert!(core.governor_act("nope", None).is_err());
        core.shutdown();
    }

    /// Starts a core on `home` with `repo` registered (the restart tests reuse the home).
    /// A core on `home` with `repo` registered, on the test plan (any date).
    fn core_on(home: &Path, repo: &Path) -> Arc<Core> {
        Core::start(CoreConfig {
            home: home.to_path_buf(),
            claude: Some(home.join("no-claude")),
            kitty_conf: home.join("kitty.conf"),
            projects: vec![repo.to_path_buf()],
            recover: false,
            enforce: false,
            default_plan: agent_runtime::runtime::test_plan,
        })
    }

    #[test]
    fn the_planner_history_survives_a_restart() {
        let d = tmp();
        let core = test_core(d.path());
        let rt = core.runtime().unwrap().clone();
        let reset = rt.with_governor(|g| g.plan().reset_at);
        // An observed weekly reading earlier this week, as the statusline stores it.
        let t0 = (Timestamp::now().as_millisecond() - 60_000) as u64;
        rt.db.add_usage(orch_store::UsageRecord {
            ts_ms: t0,
            weekly_pct: Some(42.0),
            weekly_resets_at: Some(reset.as_second()),
            provenance: "observed".into(),
            source: Some(orch_store::UsageSource::Statusline),
            ..Default::default()
        });
        rt.db.flush().unwrap();
        // Restore what the runtime would have seen live: replay through a restart first.
        core.shutdown();
        drop((core, rt));
        let home = d.path().join("home");
        let core = core_on(&home, &d.path().join("repo"));
        let rt = core.runtime().unwrap().clone();
        let used = rt.with_governor(|g| g.snapshot(Timestamp::now()).used_central);
        assert!((used - 42.0).abs() < 2.0, "usage replayed: {used}");
        rt.tick(); // a forecast snapshot (every 5 min) and the actions so far
        core.governor_act("pause_all", None).unwrap();
        rt.tick();
        let before = core.planner().unwrap();
        assert!(!before.log.is_empty() && before.log.iter().all(|l| !l.past));
        core.shutdown(); // stores the last point
        drop((core, rt));

        let core = core_on(&home, &d.path().join("repo"));
        let p = core.planner().unwrap();
        assert!(p.actual.len() >= 2, "the actual line covers the earlier run: {:?}", p.actual);
        assert!(p.actual.iter().all(|h| h.t_ms >= p.week_start_ms));
        assert!(p.actual.iter().any(|h| (h.used - 42.0).abs() < 10.0), "the second run's points: {:?}", p.actual);
        let past: Vec<_> = p.log.iter().filter(|l| l.past).collect();
        assert!(past.iter().any(|l| l.action.kind_name() == "pause_all"), "earlier actions in the log");
        assert!(past.iter().all(|l| !l.can_undo));
        let keys: std::collections::BTreeSet<_> = p.log.iter().map(|l| l.key.clone()).collect();
        assert_eq!(keys.len(), p.log.len(), "log keys are unique across starts");
        assert!(p.shadow && p.first_run_ms.is_some());
        core.shutdown();
    }

    #[test]
    fn enforcing_is_chosen_once_and_kept_across_restarts() {
        let d = tmp();
        let core = test_core(d.path());
        assert!(core.planner().unwrap().shadow, "shadow mode by default (FR-812)");
        // Pause all survives the rebuilt Governor; stored actions are not overwritten.
        let rt = core.runtime().unwrap().clone();
        core.governor_act("pause_all", None).unwrap();
        rt.tick();
        core.set_enforce(true).unwrap();
        assert!(!core.planner().unwrap().shadow);
        assert!(rt.with_governor(|g| g.is_paused_all()), "Pause all carried over");
        rt.tick();
        rt.db.flush().unwrap();
        let pauses = rt.db.actions(50).unwrap().iter().filter(|a| a.kind_name() == "pause_all").count();
        assert_eq!(pauses, 2, "the first start's pause row is kept next to the re-applied one");
        core.governor_act("resume_all", None).unwrap();
        // A sprint needs working time left today and in the week: the test
        // plan (`test_plan`) has both at any date and hour.
        let sprint = rt.governor_act(|g, now| (g.start_sprint(5.0, now), vec![]));
        assert!(sprint.is_ok(), "{sprint:?}");
        // A sprint carries over a switch, both ways (it used to be refused).
        core.set_enforce(false).unwrap();
        assert!(rt.with_governor(|g| g.sprint().is_some()), "sprint carried over");
        core.set_enforce(true).unwrap();
        drop(rt);
        core.shutdown();
        drop(core);
        let core = core_on(&d.path().join("home"), &d.path().join("repo"));
        assert!(!core.planner().unwrap().shadow, "kept");
        core.set_enforce(false).unwrap();
        assert!(core.planner().unwrap().shadow);
        core.shutdown();
    }

    /// D8 and the Board on a Linear-backed project (api-contract §7).
    #[test]
    fn switching_to_linear_asks_first_then_pushes_open_tasks() {
        let d = tmp();
        let core = test_core(d.path());
        assert_eq!(core.linear_teams().unwrap()["teams"][0]["key"], "ENG");
        let st = core.linear_states("ENG").unwrap();
        assert_eq!(st["states"][3]["default_column"], "review");
        assert_eq!(st["projects"][0]["name"], "Shop");
        let mk = |title: &str| core.task_create(json!({"project_id": "repo", "title": title})).unwrap();
        let (a, b) = (mk("Open one"), mk("Done one"));
        core.task_update(b["id"].as_u64().unwrap(), Some("done"), None, None).unwrap();
        let form = |migrate: Option<&str>| -> crate::teams::ProjectSettingsForm {
            serde_json::from_value(json!({"settings": {}, "backend": {"type": "linear", "team": "ENG"},
                "track_orchestrator_dir": true, "migrate": migrate}))
            .unwrap()
        };
        let r = core.project_settings_set("repo", form(None)).unwrap();
        assert_eq!(r, json!({"confirm": {"kind": "linear_migrate", "open": 1, "done": 1}}), "nothing saved yet");
        assert_eq!(core.project_settings("repo").unwrap()["backend"]["type"], "local");
        assert!(core.project_settings_set("repo", form(Some("maybe"))).is_err());
        core.project_settings_set("repo", form(Some("push_open"))).unwrap();
        let v = core.board_view("repo").unwrap();
        assert_eq!((v["backend"].as_str(), v["linear_team"].as_str()), (Some("linear"), Some("ENG")));
        assert!(v["last_sync_at_ms"].as_u64().is_some(), "{v}");
        let task = |id: &Value| v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == *id).unwrap().clone();
        let (ta, tb) = (task(&a["id"]), task(&b["id"]));
        assert_eq!((ta["backend"].as_str(), ta["external_id"].as_str()), (Some("linear"), Some("ENG-1")));
        assert_eq!((ta["remote_won"].as_bool(), ta["off_hours"].as_bool()), (Some(false), Some(false)));
        assert_eq!((tb["backend"].as_str(), tb["external_id"].as_str()), (Some("local"), None), "done: never pushed");
        let s = core.board_sync("repo").unwrap();
        assert_eq!((s["pushed"].as_u64(), s["pulled"].as_u64()), (Some(0), Some(0)));
        assert!(s["errors"].as_array().unwrap().is_empty());
        // Another Linear team is a migration too; `leave` keeps the old issues untouched.
        let other = |migrate: Option<&str>| -> crate::teams::ProjectSettingsForm {
            serde_json::from_value(json!({"settings": {}, "backend": {"type": "linear", "team": "OPS"},
                "track_orchestrator_dir": true, "migrate": migrate}))
            .unwrap()
        };
        let r = core.project_settings_set("repo", other(None)).unwrap();
        assert_eq!(r["confirm"]["kind"], "linear_migrate", "{r}");
        core.project_settings_set("repo", other(Some("leave"))).unwrap();
        let v = core.board_view("repo").unwrap();
        assert_eq!(v["linear_team"], "OPS");
        let ta = v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == a["id"]).unwrap().clone();
        assert_eq!(ta["backend"], "local", "not written to ENG's issue any more");
        // The same team again: no question.
        assert!(core.project_settings_set("repo", other(None)).unwrap().get("confirm").is_none());
        core.shutdown();
    }

    #[test]
    fn the_first_plan_keeps_the_week_s_usage() {
        let d = tmp();
        let core = test_core(d.path());
        let rt = core.runtime().unwrap().clone();
        let reset = rt.with_governor(|g| g.plan().reset_at);
        rt.db.add_usage(orch_store::UsageRecord {
            ts_ms: now_ms() - 60_000,
            weekly_pct: Some(30.0),
            weekly_resets_at: Some(reset.as_second()),
            provenance: "observed".into(),
            source: Some(orch_store::UsageSource::Statusline),
            ..Default::default()
        });
        rt.db.flush().unwrap();
        let mut form = core.plan_form().unwrap();
        assert_eq!(form.allocations.len(), 1, "even split over the one project");
        assert_eq!(form.allocations[0].id, "repo");
        assert_eq!(form.per_model_weights, Some(false), "FR-814 per-model weights: off by default");
        form.reserve_pct = 15.0;
        form.per_model_weights = Some(true);
        core.set_plan(&form).unwrap();
        let p = core.planner().unwrap();
        assert_eq!(p.reserve_pct, 15.0);
        assert!(rt.with_governor(|g| g.policy().per_model_weights), "the policy option reaches the Governor");
        assert_eq!((p.off_hours.allowance_pct, p.off_hours.windows.len()), (0.0, 0), "no allowance: no windows");
        assert_eq!(core.plan_form().unwrap().per_model_weights, Some(true));
        assert_eq!(core.reports_list().unwrap()["reports"][0]["id"], "current");
        assert_eq!(core.report_view("current").unwrap()["in_progress"], true);
        assert!(p.plan_set);
        assert!(
            (p.snapshot.used_central - 30.0).abs() < 2.0,
            "the week's samples replayed into the new plan: {}",
            p.snapshot.used_central
        );
        assert!(p.allocations.iter().any(|a| a.id == "repo"));
        core.shutdown();
    }

    #[test]
    fn diff_and_pr_work_on_the_projects_own_worktrees_only() {
        let d = tmp();
        let core = test_core(d.path());
        let (_keep, repo) = orch_git::fixtures::scratch_repo();
        let p = core.project_add(&repo).unwrap();
        let wt = core
            .runtime()
            .unwrap()
            .create_worktree(&repo, &orch_protocol::WorktreeKindSpec::Solo { slug: "d1".into() }, None)
            .unwrap();
        std::fs::write(wt.path.join("new.txt"), "hello\n").unwrap();
        let v = core.worktree_diff(&p.id, &wt.path).unwrap();
        assert_eq!(v["against"], "main");
        assert!(v["files"].as_array().unwrap().iter().any(|f| f["path"] == "new.txt"));
        assert!(v["branch"].as_str().unwrap().starts_with("orch/"));
        // The main checkout is a worktree too; a stranger folder is not.
        assert!(core.worktree_diff(&p.id, &repo).is_ok());
        assert!(core.worktree_diff(&p.id, d.path()).unwrap_err().to_string().contains("not a worktree"));
        // Nothing to open a PR from on the base branch; a PR needs a title.
        assert!(core.worktree_create_pr(&p.id, &repo, "t", "", false).unwrap_err().to_string().contains("base branch"));
        assert!(core.worktree_create_pr(&p.id, &wt.path, " ", "", false).unwrap_err().to_string().contains("title"));
        core.shutdown();
    }

    #[test]
    fn projects_live_in_the_store_and_leave_per_j11() {
        let d = tmp();
        let core = test_core(d.path());
        assert_eq!(core.projects().len(), 1);
        let (_keep, other) = orch_git::fixtures::scratch_repo();
        let p = core.project_add(&other).unwrap();
        assert_eq!(core.project_add(&other).unwrap(), p, "idempotent");
        assert!(core.project_add(&d.path().join("missing")).is_err());
        let detail = core.project_detail(&p.id, false).unwrap();
        assert_eq!(detail["status"]["branch"], "main");
        assert!(detail["worktrees"].is_array() && detail["branches"].is_array());
        let st = core.state();
        let row = st["projects"].as_array().unwrap().iter().find(|x| x["id"] == json!(p.id)).unwrap().clone();
        assert_eq!(row["git"]["branch"], "main");
        assert_eq!(row["default_branch"], "main");
        // A worktree with unmerged work is kept unless confirmed (FR-104, J11).
        let rt = core.runtime().unwrap();
        let wt =
            rt.create_worktree(&other, &orch_protocol::WorktreeKindSpec::Solo { slug: "w1".into() }, None).unwrap();
        orch_git::fixtures::commit_file(&wt.path, "x.txt", "x", "work");
        let own = other.join("mine");
        orch_git::fixtures::git(&other, &["worktree", "add", "-q", "-b", "mine", own.to_str().unwrap()]);
        assert!(
            core.worktree_remove(&p.id, &own, true, true).unwrap_err().to_string().contains("only removes its own")
        );
        assert!(own.exists());
        // A worktree whose folder is gone is still the app's to prune.
        let gone =
            rt.create_worktree(&other, &orch_protocol::WorktreeKindSpec::Solo { slug: "gone".into() }, None).unwrap();
        std::fs::remove_dir_all(&gone.path).unwrap();
        core.worktree_remove(&p.id, &gone.path, true, false).unwrap();
        let plan = core.removal_plan(&p.id).unwrap();
        assert_eq!(plan["plan"]["worktrees"].as_array().unwrap().len(), 1);
        let report = core
            .project_remove(
                &p.id,
                &orch_git::project::RemovalChoices {
                    remove_merged: true,
                    discard: vec![],
                    history: orch_git::project::History::Delete,
                },
            )
            .unwrap();
        assert_eq!(report["kept"].as_array().unwrap().len(), 1, "{report}");
        assert!(wt.path.exists());
        assert_eq!(core.projects().len(), 1);
        // Projects persist: a new core on the same home lists the first one.
        core.shutdown();
        drop(core);
        let core = core_on(&d.path().join("home"), &d.path().join("repo"));
        assert_eq!(core.projects().len(), 1);
        core.shutdown();
    }

    #[test]
    fn the_first_version_s_settings_file_is_imported() {
        let d = tmp();
        let ui = d.path().join("home/ui");
        std::fs::create_dir_all(&ui).unwrap();
        std::fs::write(ui.join("settings.json"), r#"{"layout": {"focused": "p1"}}"#).unwrap();
        let core = test_core(d.path());
        assert_eq!(core.settings.get("layout"), Some(json!({"focused": "p1"})));
        assert!(!ui.join("settings.json").exists());
        core.set_editor("hx {file}:{line}").unwrap();
        assert!(core.set_editor("  ").is_err());
        assert_eq!(core.setup_view(&core.projects())["editor"], "hx {file}:{line}");
        core.shutdown();
        drop(core);
        let core = core_on(&d.path().join("home"), &d.path().join("repo"));
        assert_eq!(*core.editor_template.lock_ok(), "hx {file}:{line}", "kept");
        core.shutdown();
    }
}
