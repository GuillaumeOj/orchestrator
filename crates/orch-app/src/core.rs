//! The app's core: embeds the agent runtime and its Governor in the Tauri
//! process, serves the bus socket (hooks, statusline, `orchd` control
//! clients), holds permission prompts for the Inbox, and owns the shell and
//! editor PTYs. Every Tauri command is a thin wrapper over a method here.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{PUMP_EVERY, TICK_EVERY};
use anyhow::{Context, Result, anyhow, bail};
use governor::jiff::Timestamp;
use orch_protocol::bus;
use orch_protocol::{
    AgentSpec, CtlReply, CtlRequest, Envelope, HookEvent, HookPayload, Msg, PROTOCOL_VERSION, Reply, ReplyEnvelope,
    now_ms,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::editor;
use crate::inbox::{self, AgentFacts, Alert, Ask, Done};
use crate::kitty::{self, KittyImport};
use crate::memory::{self, AppMemory};
use crate::permissions::{Answer, PermissionBroker};
use crate::planner::{self, History, HistoryPoint, PlannerView};
use crate::services::{GitSummary, JsonFileSettings, Project, ProjectRegistry, SettingsStore, InMemoryProjects};
use crate::terms::{TermKind, Terms};

/// How long the pane strip and the Inbox hide a "waiting for you" item after
/// the user answered (the next hook moves the agent on).
const ANSWERED_GRACE_MS: u64 = 5_000;
const GIT_EVERY: Duration = Duration::from_secs(10);
const MEMORY_EVERY: Duration = Duration::from_secs(5);
const HISTORY_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub home: PathBuf,
    pub claude: Option<PathBuf>,
    pub kitty_conf: PathBuf,
    /// Projects to register at start (the stub registry has no persistence).
    pub projects: Vec<PathBuf>,
    /// Run the J10 reconciler at start (resumes agents that were working).
    pub recover: bool,
    pub enforce: bool,
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
        }
    }
}

pub struct Core {
    pub paths: AppPaths,
    rt: Option<Arc<Runtime>>,
    /// Why agents are unavailable (another core owns the socket, no claude…).
    pub core_error: Option<String>,
    pub broker: Arc<PermissionBroker>,
    pub projects: Box<dyn ProjectRegistry>,
    pub settings: Box<dyn SettingsStore>,
    pub terms: Terms,
    pub kitty: KittyImport,
    pub editor_template: String,
    history: Mutex<History>,
    git: Mutex<(Option<Instant>, BTreeMap<PathBuf, GitSummary>)>,
    memory: Mutex<(Option<Instant>, AppMemory)>,
    /// Project folder → its repository's main checkout (FR-105), resolved once.
    mains: Mutex<BTreeMap<PathBuf, Option<PathBuf>>>,
    answered: Mutex<BTreeMap<String, u64>>,
    attached: Mutex<std::collections::BTreeSet<String>>,
    stop: Arc<AtomicBool>,
    pub warnings: Vec<String>,
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
        let projects = InMemoryProjects::seeded(cfg.projects.clone());
        let settings = JsonFileSettings::open(paths.root.join("ui").join("settings.json"));
        let kitty = kitty::import(&cfg.kitty_conf);
        let editor_template = editor::template_from_kitty(kitty.editor.as_deref());
        let (rt, core_error) = match start_runtime(&cfg, &paths, &mut warnings) {
            Ok(rt) => (Some(rt), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let core = Arc::new(Core {
            paths,
            rt,
            core_error,
            broker: Arc::new(PermissionBroker::default()),
            projects: Box::new(projects),
            settings: Box::new(settings),
            terms: Terms::default(),
            kitty,
            editor_template,
            history: Mutex::new(History::default()),
            git: Mutex::new((None, BTreeMap::new())),
            memory: Mutex::new((None, AppMemory::default())),
            mains: Mutex::new(BTreeMap::new()),
            answered: Mutex::new(BTreeMap::new()),
            attached: Mutex::new(Default::default()),
            stop: Arc::new(AtomicBool::new(false)),
            warnings,
        });
        if let Some(rt) = &core.rt {
            // Projects of known agents, unless a registered folder already maps to them.
            let known = rt.store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.repo.clone())).collect::<Vec<_>>());
            for (id, repo) in known {
                if core.project_of(&id, &repo, &core.projects.list()).is_none() {
                    let _ = core.projects.add(&repo);
                }
            }
            if let Err(e) = core.serve() {
                eprintln!("orchestrator: bus server failed: {e:#}");
            }
            if cfg.recover {
                let (c, rt) = (core.clone(), rt.clone());
                std::thread::spawn(move || {
                    if let Err(e) = rt.recover(true) {
                        eprintln!("orchestrator: recovery failed: {e:#}");
                    }
                    drop(c);
                });
            }
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
            while !stop.load(Ordering::SeqCst) {
                rt.pump();
                if last_tick.elapsed() >= TICK_EVERY {
                    rt.tick();
                    last_tick = Instant::now();
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
                HistoryPoint { t_ms: s.ts.as_millisecond(), used: s.used, governed: s.governed, five_hour: s.five_hour },
                g.plan().week_start.as_millisecond(),
            )
        });
        self.history.lock().unwrap_or_else(|e| e.into_inner()).push(p, week_start);
    }

    /// Graceful stop: suspend agents (resumed by id next time), close shells.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(rt) = &self.rt {
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
            Msg::Hook(m) if m.event == HookEvent::PermissionRequest => {
                // The state machine first (Waiting for you); an unknown agent fails closed.
                if let Some(out) = rt.on_hook(&m, None) {
                    return reply(Reply::Hook { output: Some(out) });
                }
                let p = HookPayload::parse(&m.payload);
                let id = self.broker.open(&m.agent_id, &p, now_ms());
                // Answer before orch-hook's own deadline; then the pane's dialog shows.
                let hold = Duration::from_millis(m.deadline_ms.saturating_sub(2_000).max(1_000));
                let outcome = self.broker.wait(&id, Instant::now() + hold, Duration::from_millis(250), || peer_closed(&s));
                reply(Reply::Hook { output: PermissionBroker::hook_output(&outcome) });
            }
            Msg::Hook(m) => {
                // A prompt held for the Inbox but answered in the pane: this call's
                // next hook, or the turn's end, closes it.
                let call_done = matches!(
                    m.event,
                    HookEvent::PostToolUse | HookEvent::PostToolUseFailure | HookEvent::PermissionDenied
                );
                let turn_done = matches!(
                    m.event,
                    HookEvent::Stop | HookEvent::StopFailure | HookEvent::UserPromptSubmit | HookEvent::SessionEnd
                );
                if call_done || turn_done {
                    let p = HookPayload::parse(&m.payload);
                    let tool_use = if call_done { p.tool_use_id.as_deref() } else { None };
                    self.broker.answered_in_pane(&m.agent_id, tool_use);
                }
                let output = rt.on_hook(&m, Some(&s));
                reply(Reply::Hook { output });
            }
            Msg::Status(m) => {
                rt.on_status(&m);
                reply(Reply::Ack);
            }
            Msg::Ctl(CtlRequest::Attach { agent_id, rows, cols }) => {
                if !rt.live_ids().contains(&agent_id) {
                    return reply(Reply::Ctl(CtlReply::err(format!("agent {agent_id} is not running"))));
                }
                if self.attached.lock().unwrap_or_else(|e| e.into_inner()).contains(&agent_id) {
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
            ..Default::default()
        };
        rt.start(&spec)?;
        // The runtime records the main checkout (FR-105); remember which project
        // folder the session was started from, for the sidebar.
        self.settings.set(&format!("agent-project:{id}"), json!(project.id))?;
        Ok(id)
    }

    /// The UI takes the agent's output (pull transport): with a consumer the
    /// buffer applies backpressure, and xterm.js answers terminal queries.
    pub fn agent_attach(&self, id: &str) -> Result<()> {
        let l = self.runtime()?.live(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        l.pty.output.set_attached(true);
        self.attached.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        Ok(())
    }

    pub fn agent_detach(&self, id: &str) {
        self.attached.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if let Some(l) = self.rt.as_ref().and_then(|rt| rt.live(id)) {
            l.pty.output.set_attached(false);
        }
    }

    /// Pull: everything buffered, waiting up to `wait` for output. Err when
    /// the process is gone (the pane then shows its state strip).
    pub fn agent_read(&self, id: &str, wait: Duration) -> Result<Vec<u8>> {
        let l = self.runtime()?.live(id).ok_or_else(|| anyhow!("agent {id} is not running"))?;
        let b = l.pty.output.read_wait(wait);
        if b.is_empty() && l.pty.output.is_closed() {
            bail!("agent {id} exited");
        }
        Ok(b)
    }

    pub fn agent_write(&self, id: &str, data: &[u8]) -> Result<()> {
        self.runtime()?.live(id).ok_or_else(|| anyhow!("agent {id} is not running"))?.user_input(data)
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
            "resume" => {
                rt.resume(id, message)?;
                json!("resumed")
            }
            "deliver" => json!(rt.deliver(id, message.ok_or_else(|| anyhow!("no message"))?)?),
            other => bail!("unknown agent action {other}"),
        })
    }

    // ------------------------------------------------------------ terminals

    fn project(&self, id: &str) -> Result<Project> {
        self.projects.list().into_iter().find(|p| p.id == id).ok_or_else(|| anyhow!("no project {id}"))
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
        if let Some(f) = file {
            let abs = if Path::new(f).is_absolute() { PathBuf::from(f) } else { cwd.join(f) };
            if editor::open_remote(&sock, &abs.display().to_string(), line)? {
                return Ok(EditorOpen { reused: true, term: None });
            }
        }
        if let Some(dir) = sock.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&sock); // stale: nobody answered on it
        let cmd = editor::render(&self.editor_template, file, line, Some(&sock));
        let term = self.term_spawn(TermKind::Editor, &cwd, Some(&cmd), rows, cols)?;
        Ok(EditorOpen { reused: false, term: Some(term) })
    }

    /// FR-1004: the real kitty at the same cwd (remote control when kitty.conf
    /// enables it, otherwise a new instance).
    pub fn open_in_kitty(&self, cwd: &Path) -> Result<()> {
        let st = match &self.kitty.listen_on {
            Some(to) => std::process::Command::new("kitty")
                .args(["@", "--to", to, "launch", "--type=os-window", "--cwd"])
                .arg(cwd)
                .status(),
            None => std::process::Command::new("open").args(["-na", "kitty", "--args", "--directory"]).arg(cwd).status(),
        }
        .context("starting kitty")?;
        if !st.success() {
            bail!("kitty exited with {st}");
        }
        Ok(())
    }

    // ------------------------------------------------------------ inbox

    pub fn inbox_act(&self, item: &str, action: &str, message: Option<&str>) -> Result<Value> {
        let rt = self.runtime()?;
        if let Some(_n) = item.strip_prefix("perm-") {
            let agent = self.broker.list().into_iter().find(|p| p.id == item).map(|p| p.agent_id);
            let answer = match action {
                "approve" => Answer::Allow,
                "reject" => Answer::Deny { message: message.map(str::to_string) },
                "in_pane" => Answer::InPane,
                other => bail!("{other} does not apply to a permission"),
            };
            if !self.broker.answer(item, answer) {
                bail!("this permission prompt is no longer open");
            }
            if let Some(a) = agent {
                self.answered.lock().unwrap_or_else(|e| e.into_inner()).insert(a, now_ms());
            }
            return Ok(json!("answered"));
        }
        if let Some(n) = item.strip_prefix("alert-") {
            let n: u64 = n.parse()?;
            rt.store.write(|s| {
                if let Some(i) = s.inbox.iter_mut().find(|i| i.id == n) {
                    i.pending = false;
                }
            })?;
            return Ok(json!("dismissed"));
        }
        if let Some(n) = item.strip_prefix("ask-") {
            let id = governor::ActionId(n.parse()?);
            let accept = action == "approve";
            rt.governor_act(|g, now| (g.answer_ask(id, accept, now), vec![])).map_err(|e| anyhow!("{e:?}"))?;
            return Ok(json!("answered"));
        }
        if let Some(a) = item.strip_prefix("wait-").or_else(|| item.strip_prefix("error-")) {
            if action == "reply" {
                let m = message.ok_or_else(|| anyhow!("empty reply"))?;
                return self.agent_action(a, if rt.live(a).is_some() { "deliver" } else { "resume" }, Some(m));
            }
            if action == "reject" && item.starts_with("error-") {
                return self.agent_action(a, "resume", None);
            }
        }
        bail!("{action} does not apply to {item}")
    }

    fn inbox_items(&self, agents: &[Value]) -> Vec<inbox::InboxItem> {
        let Some(rt) = &self.rt else { return vec![] };
        let now = now_ms();
        let answered = self.answered.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let facts: Vec<AgentFacts> = agents
            .iter()
            .filter_map(|a| {
                let id = a["id"].as_str()?.to_string();
                let mut state = a["state"].as_str().unwrap_or("").to_string();
                if state == "waiting_for_you" && answered.get(&id).is_some_and(|t| now.saturating_sub(*t) < ANSWERED_GRACE_MS)
                {
                    state = "working".into();
                }
                let interesting = state == "waiting_for_you" || state == "error" || a["trust_dialog"] == true;
                let tail = interesting.then(|| rt.live(&id).map(|l| l.pty.output.tail_text())).flatten();
                Some(AgentFacts {
                    since_ms: a["last_event_at_ms"].as_u64().unwrap_or(0),
                    trust_dialog: a["trust_dialog"] == true,
                    last_error: a["last_error"].as_str().map(str::to_string),
                    tail,
                    id,
                    state,
                })
            })
            .collect();
        let (alerts, done) = rt.store.read(|s| {
            let alerts: Vec<Alert> = s
                .inbox
                .iter()
                .filter(|i| i.pending)
                .map(|i| Alert { id: i.id, agent_id: i.agent_id.clone(), body: i.body.clone(), created_ms: i.created_at_ms })
                .collect();
            let done: Vec<Done> = s
                .turns
                .iter()
                .rev()
                .filter(|t| t.status == agent_runtime::store::TurnStatus::Done)
                .take(10)
                .map(|t| Done {
                    turn_id: t.id.clone(),
                    agent_id: t.agent_id.clone(),
                    body: t.body.clone(),
                    at_ms: t.delivered_at_ms.unwrap_or(t.created_at_ms),
                })
                .collect();
            (alerts, done)
        });
        let ask = rt.with_governor(|g| {
            let (id, _) = g.next_action_preview(Timestamp::now()).pending_ask?;
            let a = g.action(id)?;
            let ask = a.ask.as_ref()?;
            Some(Ask {
                id: id.0,
                reason: a.reason.clone(),
                effect: a.expected_effect.clone(),
                created_ms: a.ts.as_millisecond() as u64,
                deadline_ms: ask.deadline.as_millisecond(),
                default_apply: matches!(ask.default, governor::AskDefault::Apply),
            })
        });
        inbox::build(&inbox::Inputs { permissions: &self.broker.list(), agents: &facts, alerts: &alerts, ask, done: &done })
    }

    // ------------------------------------------------------------ views

    fn git_for(&self, paths: &[PathBuf]) -> BTreeMap<PathBuf, GitSummary> {
        let mut g = self.git.lock().unwrap_or_else(|e| e.into_inner());
        let stale = g.0.is_none_or(|t| t.elapsed() >= GIT_EVERY) || paths.iter().any(|p| !g.1.contains_key(p));
        if stale {
            g.1 = paths.iter().map(|p| (p.clone(), self.projects.git_summary(p))).collect();
            g.0 = Some(Instant::now());
        }
        g.1.clone()
    }

    fn main_of(&self, path: &Path) -> Option<PathBuf> {
        let mut m = self.mains.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(path.to_path_buf()).or_insert_with(|| agent_runtime::worktree::main_checkout(path).ok()).clone()
    }

    /// The project an agent belongs to: the one it was started from, else the
    /// project whose folder (or main checkout) is the agent's repository.
    fn project_of(&self, agent_id: &str, repo: &Path, projects: &[Project]) -> Option<String> {
        if let Some(id) = self.settings.get(&format!("agent-project:{agent_id}")).and_then(|v| v.as_str().map(String::from)) {
            if projects.iter().any(|p| p.id == id) {
                return Some(id);
            }
        }
        let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let repo = real(repo);
        projects
            .iter()
            .find(|p| real(&p.path) == repo)
            .or_else(|| projects.iter().find(|p| self.main_of(&p.path).is_some_and(|m| real(&m) == repo)))
            .map(|p| p.id.clone())
    }

    fn memory(&self, agents_mb: f64) -> AppMemory {
        let mut m = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        if m.0.is_none_or(|t| t.elapsed() >= MEMORY_EVERY) {
            let pids: Vec<i32> =
                self.terms.ids().iter().filter_map(|i| self.terms.get(*i).ok()?.pid.map(|p| p as i32)).collect();
            m.1 = memory::measure(&pids, agents_mb);
            m.0 = Some(Instant::now());
        }
        m.1.clone()
    }

    /// Everything the shell shows, polled by the UI about once a second.
    pub fn state(&self) -> Value {
        let projects = self.projects.list();
        let (mut agents, warnings, usage, queue, worktrees) = match &self.rt {
            Some(rt) => {
                let st = rt.state_json();
                let usage = rt.usage_json();
                let (queue, snapshot) = rt.with_governor(|g| {
                    (serde_json::to_value(g.queue()).unwrap_or(Value::Null), g.snapshot(Timestamp::now()))
                });
                let mut usage = usage;
                usage["live_cap"] = json!(snapshot.live_cap);
                usage["snapshot"] = serde_json::to_value(&snapshot).unwrap_or(Value::Null);
                let (reset, shadow) = rt.with_governor(|g| (g.plan().reset_at.as_millisecond(), g.plan().shadow_mode));
                usage["reset_at_ms"] = json!(reset);
                usage["shadow"] = json!(shadow);
                let worktrees: BTreeMap<String, (PathBuf, PathBuf, String, u64, Option<String>)> = rt.store.read(|s| {
                    s.agents
                        .values()
                        .map(|a| {
                            let branch = s.worktrees.get(&a.worktree).map(|w| w.branch.clone()).unwrap_or_default();
                            (a.id.clone(), (a.repo.clone(), a.worktree.clone(), branch, a.created_at_ms, a.role.effort.clone()))
                        })
                        .collect()
                });
                (st["agents"].as_array().cloned().unwrap_or_default(), st["warnings"].clone(), usage, queue, worktrees)
            }
            None => (vec![], json!([]), Value::Null, json!([]), BTreeMap::new()),
        };
        let mut git_paths: Vec<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
        git_paths.extend(worktrees.values().map(|w| w.1.clone()));
        let git = self.git_for(&git_paths);
        let mut agents_mb = 0.0;
        for a in agents.iter_mut() {
            let id = a["id"].as_str().unwrap_or("").to_string();
            agents_mb += a["memory_mb"].as_f64().unwrap_or(0.0);
            if let Some((repo, wt, branch, created, effort)) = worktrees.get(&id) {
                a["created_at_ms"] = json!(created);
                a["effort"] = json!(effort);
                a["project_id"] = json!(self.project_of(&id, repo, &projects));
                a["branch"] = json!(branch);
                a["git"] = json!(git.get(wt));
            }
            if let Some(q) = queue.as_array().and_then(|q| q.iter().find(|e| e["agent"] == json!(id))) {
                a["queue_entry"] = q.clone();
            }
            a["attached"] = json!(self.attached.lock().unwrap_or_else(|e| e.into_inner()).contains(&id));
        }
        let items = self.inbox_items(&agents);
        let projects: Vec<Value> = projects
            .iter()
            .map(|p| json!({"id": p.id, "name": p.name, "path": p.path, "git": git.get(&p.path)}))
            .collect();
        let mut all_warnings: Vec<Value> = warnings.as_array().cloned().unwrap_or_default();
        all_warnings.extend(self.warnings.iter().map(|w| json!(w)));
        json!({
            "now_ms": now_ms(),
            "core": {"ok": self.rt.is_some(), "error": self.core_error, "home": self.paths.root, "socket": self.paths.socket()},
            "warnings": all_warnings,
            "projects": projects,
            "agents": agents,
            "usage": usage,
            "queue": queue,
            "inbox": {"items": items, "badge": inbox::badge(&items)},
            "permissions": self.broker.list(),
            "memory": self.memory(agents_mb),
            "terms": self.terms.ids(),
        })
    }

    pub fn planner(&self) -> Result<PlannerView> {
        let rt = self.runtime()?;
        let five_reset = rt.usage_json()["five_hour"]["resets_at"].as_i64().and_then(|s| Timestamp::from_second(s).ok());
        let live = rt.live_ids();
        let alloc_of: BTreeMap<String, String> =
            rt.store.read(|s| s.agents.values().map(|a| (a.id.clone(), a.allocation.clone())).collect());
        let history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        Ok(rt.with_governor(|g| {
            planner::build(g, Timestamp::now(), &history, five_reset, &|alloc| {
                live.iter().filter(|id| alloc_of.get(*id).is_some_and(|a| a == alloc)).count()
            })
        }))
    }

    /// Week planner and title-bar controls (FR-809, FR-811).
    pub fn governor_act(&self, action: &str, arg: Option<f64>) -> Result<Value> {
        let rt = self.runtime()?;
        let id = || arg.map(|a| a as u64).ok_or_else(|| anyhow!("{action} needs an id"));
        Ok(match action {
            "pause_all" => json!(rt.pause_all()),
            "resume_all" => json!(rt.resume_all()),
            "run_now" => {
                let e = governor::EntryId(id()?);
                json!(rt.governor_act(|g, now| {
                    let r = g.run_now(e, now);
                    (r.is_some(), r.into_iter().collect())
                }))
            }
            "undo" => {
                let a = governor::ActionId(id()?);
                rt.governor_act(|g, now| match g.undo(a, now) {
                    Ok(r) => (Ok(()), r),
                    Err(e) => (Err(anyhow!("{e:?}")), vec![]),
                })?;
                json!("undone")
            }
            "sprint" => {
                let pp = arg.unwrap_or(5.0);
                let a = rt.governor_act(|g, now| (g.start_sprint(pp, now), vec![])).map_err(|e| anyhow!("{e:?}"))?;
                json!(a.0)
            }
            other => bail!("unknown governor action {other}"),
        })
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn start_runtime(cfg: &CoreConfig, paths: &AppPaths, warnings: &mut Vec<String>) -> Result<Arc<Runtime>> {
    if orch_protocol::is_listening(&paths.socket()) {
        bail!(
            "another core is running on {} (orchd?): stop it to run agents in the app",
            paths.socket().display()
        );
    }
    std::fs::create_dir_all(&paths.root)?;
    let mut rc = RuntimeConfig::new(paths.clone());
    if let Some(c) = &cfg.claude {
        rc.claude = c.clone();
    }
    rc.shadow = !cfg.enforce;
    // Helpers next to the app binary (target/<profile>, or Contents/MacOS in a
    // bundle) are copied to <home>/bin, a stable path outside the .app.
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        if dir.join("orch-hook").is_file() && dir.join("orch-status").is_file() {
            if let Err(e) = agent_runtime::preflight::install_helpers(&dir, &paths.bin_dir()) {
                warnings.push(format!("helpers: {e:#}"));
            }
        }
    }
    if agent_runtime::preflight::check_helpers(&rc.helpers).is_err() {
        warnings.push(format!(
            "orch-hook/orch-status missing in {}: build them (cargo build -p orch-hook -p orch-status); agents can't start",
            paths.bin_dir().display()
        ));
    }
    Runtime::new(rc)
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

/// True when orch-hook closed its end (Esc in the pane kills it).
fn peer_closed(s: &UnixStream) -> bool {
    let mut b = [0u8; 1];
    // SAFETY: a non-blocking peek into a local buffer.
    let n = unsafe { libc::recv(s.as_raw_fd(), b.as_mut_ptr().cast(), 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
    n == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn test_core(d: &Path) -> Arc<Core> {
        let repo = d.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Core::start(CoreConfig {
            home: d.join("home"),
            claude: Some(d.join("no-claude")),
            kitty_conf: d.join("kitty.conf"),
            projects: vec![repo],
            recover: false,
            enforce: false,
        })
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
        }
    }

    fn tmp() -> tempfile::TempDir {
        // Short path: the socket must fit in 104 bytes.
        tempfile::Builder::new().prefix("orchapp").tempdir_in("/tmp").unwrap()
    }

    #[test]
    fn state_has_the_shell_s_inputs() {
        let d = tmp();
        let core = test_core(d.path());
        assert!(core.core_error.is_none(), "{:?}", core.core_error);
        let s = core.state();
        assert_eq!(s["core"]["ok"], true);
        assert_eq!(s["projects"][0]["id"], "repo");
        assert_eq!(s["inbox"]["badge"], 0);
        assert!(s["usage"]["governor"]["state"].is_string(), "{}", s["usage"]);
        assert!(s["memory"]["core_mb"].as_f64().unwrap() > 0.0);
        let p = core.planner().unwrap();
        assert!(p.curve.len() > 100);
        core.shutdown();
    }

    #[test]
    fn a_second_core_on_the_same_home_is_refused() {
        let d = tmp();
        let a = test_core(d.path());
        let b = test_core(d.path());
        assert!(b.core_error.as_deref().unwrap_or("").contains("another core"));
        assert!(b.agent_start(&NewSession { project_id: "repo".into(), name: "x".into(), ..Default::default() }).is_err());
        a.shutdown();
    }

    #[test]
    fn permission_prompt_is_held_until_the_inbox_answers() {
        let d = tmp();
        let core = test_core(d.path());
        let rt = core.runtime().unwrap().clone();
        rt.store
            .write(|s| {
                s.agents.insert(
                    "a1".into(),
                    record("a1", d.path().join("repo")),
                )
            })
            .unwrap();
        // What orch-hook sends, on the real socket.
        let sock = core.paths.socket();
        let hook = std::thread::spawn(move || {
            let s = bus::connect(&sock, Duration::from_secs(1)).unwrap();
            let msg = Envelope::new(Msg::Hook(orch_protocol::HookMsg {
                agent_id: "a1".into(),
                event: HookEvent::PermissionRequest,
                ts_ms: now_ms(),
                deadline_ms: 30_000,
                payload: json!({"hook_event_name": "PermissionRequest", "session_id": "s",
                    "tool_name": "Bash", "tool_input": {"command": "pnpm add x"}}),
            }));
            bus::write_line(&mut &s, &msg).unwrap();
            let r: ReplyEnvelope = bus::read_line(&mut BufReader::new(&s)).unwrap();
            r
        });
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
        assert_eq!(item["preview"], "pnpm add x");
        assert_eq!(core.state()["agents"][0]["state"], "waiting_for_you");
        core.inbox_act(item["id"].as_str().unwrap(), "approve", None).unwrap();
        let r = hook.join().unwrap();
        match r.reply {
            Reply::Hook { output: Some(o) } => assert_eq!(o.0["hookSpecificOutput"]["decision"]["behavior"], "allow"),
            other => panic!("{other:?}"),
        }
        // Answered: not shown again as "waiting for you" while the next hook is on its way.
        assert_eq!(core.state()["inbox"]["badge"], 0);
        assert!(core.inbox_act(item["id"].as_str().unwrap(), "approve", None).is_err());
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
        let core = Core::start(CoreConfig {
            home: d.path().join("home"),
            claude: Some(d.path().join("no-claude")),
            kitty_conf: d.path().join("kitty.conf"),
            projects: vec![nested],
            recover: false,
            enforce: false,
        });
        // The runtime records the main checkout as the agent's repo (FR-105).
        let main = main.canonicalize().unwrap();
        core.runtime().unwrap().store.write(|s| s.agents.insert("a1".into(), record("a1", main.clone()))).unwrap();
        assert_eq!(core.state()["agents"][0]["project_id"], "feat", "found through the project's main checkout");
        core.settings.set("agent-project:a1", json!("nope")).unwrap();
        assert_eq!(core.state()["agents"][0]["project_id"], "feat", "an unknown remembered project is ignored");
        assert_eq!(core.projects.list().len(), 1, "the main checkout was not added as a second project");
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
}
