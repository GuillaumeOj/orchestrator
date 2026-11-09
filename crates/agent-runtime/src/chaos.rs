//! The chaos suite (fix-team-getting-stucked §3.8): one scenario per failure
//! mode F1–F9 and the delivery invariant, with no process, no PTY and no
//! socket. A fake CLI runs on a thread behind a fake terminal
//! ([`PtyProcess::fake`]); it takes what the core types (a bracketed paste, Enter, Esc),
//! and answers the way Claude Code does: through the core's hook entry
//! ([`Runtime::serve_hook`], plus the ack orch-hook sends) and a transcript file.
//! Its misbehaviours are switches set per test. Time is the real clock, with
//! the bounds shortened through settings, or a later `now` given to the
//! watchdogs once the fake CLI has gone quiet.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use governor::TurnKind;
use orch_protocol::{HookEvent, HookMsg, Reply, StatusMsg, now_ms};
use serde_json::{Value, json};

use crate::handover::{ACK_KEY, INVARIANT_KEY, SLOT_WAIT_KEY};
use crate::pty::PtyProcess;
use crate::runtime::tests::{insert_test_live_with, test_agent, test_cfg};
use crate::runtime::{Runtime, TerminalFactory};
use crate::state::AgentState;
use crate::store::{TurnRow, TurnStatus};

/// What the fake CLI does wrong, per test.
#[derive(Default)]
pub(crate) struct Switches {
    /// The next Enter is lost: the text stays in the input box.
    pub swallow_enter: bool,
    /// A dialog eats every key until a lone Esc.
    pub modal: bool,
    /// ... and comes back after each Esc.
    pub modal_always: bool,
    /// The next turn's hooks never reach the core (the transcript has it).
    pub drop_hooks: bool,
    /// The next Stop never reaches the core.
    pub drop_stop: bool,
    /// The next Stop is a StopFailure with this error.
    pub stop_failure: Option<String>,
    /// Every Stop is.
    pub stop_failure_sticky: Option<String>,
    /// The next Stop carries a running background task.
    pub bg: bool,
    /// The process dies after the next UserPromptSubmit.
    pub exit_mid_turn: bool,
    /// ... after every one.
    pub crash_always: bool,
    /// The next PostToolUse waits this long.
    pub delay_post: Option<Duration>,
    /// The next turn's UserPromptSubmit waits this long (a slow CLI).
    pub delay_submit: Option<Duration>,
    /// The next PostToolUse comes from a subagent.
    pub subagent: bool,
    /// The model reads any `full text: <path>` it is shown.
    pub read_full: bool,
    /// The statusline's 5 h usage (%).
    pub usage: Option<f64>,
}

/// The fake CLI of one agent, seen from the test.
#[derive(Clone, Default)]
pub(crate) struct Cli {
    pub sw: Arc<Mutex<Switches>>,
    /// What the model saw, in order: `POST …` (a tool reply), `DONE[n] …`.
    pub screen: Arc<Mutex<Vec<String>>>,
    /// Chunks typed into the terminal and not handled yet.
    pending: Arc<AtomicUsize>,
    /// Turns the CLI started.
    pub turns: Arc<AtomicUsize>,
    /// SessionStart went to the core.
    pub started: Arc<std::sync::atomic::AtomicBool>,
}

impl Cli {
    pub fn set(&self, f: impl FnOnce(&mut Switches)) {
        f(&mut self.sw.lock().unwrap());
    }

    pub fn saw(&self, needle: &str) -> bool {
        self.screen.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    pub fn lines(&self) -> Vec<String> {
        self.screen.lock().unwrap().clone()
    }

    /// Everything typed into it was handled.
    pub fn quiet(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }
}

/// The CLI's own state while it runs.
struct Machine {
    rt: Weak<Runtime>,
    agent: String,
    sid: String,
    cli: Cli,
    out: Arc<crate::pty::Output>,
    exit: Arc<crate::pty::FakeExit>,
    /// The input box.
    input: String,
    n: usize,
    dead: bool,
    /// When the 5 h window resets (s): fixed, like the real one.
    resets5: u64,
    resets7: u64,
    last_status_ms: u64,
}

impl Machine {
    fn rt(&self) -> Option<Arc<Runtime>> {
        self.rt.upgrade()
    }

    fn say(&self, line: String) {
        self.out.push(format!("{line}\r\n").as_bytes());
        self.cli.screen.lock().unwrap().push(line);
    }

    fn transcript(&self, rt: &Runtime, line: Value) {
        let dir = rt.cfg.projects_root.join("-fake");
        let _ = std::fs::create_dir_all(&dir);
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.jsonl", self.sid)));
        if let Ok(f) = f.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }

    /// One hook, the way orch-hook does it: the core's reply, then the ack.
    fn hook(&self, rt: &Runtime, drop: bool, event: &str, extra: Value) -> Option<Value> {
        if drop {
            return None;
        }
        let mut payload = json!({
            "hook_event_name": event, "session_id": self.sid, "prompt_id": format!("p{}", self.n)
        });
        payload.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        let m = HookMsg {
            agent_id: self.agent.clone(),
            event: HookEvent::from(event.to_string()),
            ts_ms: now_ms(),
            deadline_ms: 540_000,
            payload,
            acks: vec![],
        };
        let (mut out, mut acks) = (None, vec![]);
        rt.serve_hook(&m, None, |reply| {
            if let Reply::Hook { output, ack } = reply {
                out = output.map(|o| o.0);
                acks = ack;
            }
            true
        });
        if !acks.is_empty() {
            rt.on_hook_ack(&self.agent, &acks);
        }
        out
    }

    fn status(&mut self, rt: &Runtime, drop: bool, used: f64, cost: f64) {
        if drop {
            return;
        }
        // Each reading is newer than the last, as a real one is.
        self.last_status_ms = now_ms().max(self.last_status_ms + 1);
        let reset5 = self.resets5;
        rt.on_status(&StatusMsg {
            agent_id: self.agent.clone(),
            ts_ms: self.last_status_ms,
            payload: json!({
                "session_id": self.sid,
                "rate_limits": {
                    "five_hour": {"used_percentage": used, "resets_at": reset5},
                    "seven_day": {"used_percentage": 34, "resets_at": self.resets7},
                },
                "cost": {"total_cost_usd": cost},
            }),
        });
    }

    fn start(&mut self, resumed: bool) {
        if let Some(rt) = self.rt() {
            self.hook(&rt, false, "SessionStart", json!({"source": if resumed { "resume" } else { "startup" }}));
            self.status(&rt, false, 0.0, 0.0);
        }
        self.cli.started.store(true, Ordering::SeqCst);
    }

    /// Bytes typed into the terminal.
    fn typed(&mut self, bytes: &[u8]) {
        if self.dead {
            return;
        }
        let sw = &self.cli.sw;
        if bytes == b"\x1b" {
            // Esc closes a dialog; an idle CLI has nothing to interrupt.
            let mut sw = sw.lock().unwrap();
            sw.modal = sw.modal_always;
            return;
        }
        if sw.lock().unwrap().modal {
            return;
        }
        if bytes == b"\r" {
            if std::mem::take(&mut sw.lock().unwrap().swallow_enter) {
                return;
            }
            let text = std::mem::take(&mut self.input);
            if !text.trim().is_empty() {
                self.turn(text.trim());
            }
            return;
        }
        // The terminal echoes what is typed.
        self.out.push(bytes);
        let s = String::from_utf8_lossy(bytes).replace("\x1b[200~", "").replace("\x1b[201~", "");
        self.input.push_str(&s);
    }

    fn turn(&mut self, prompt: &str) {
        let Some(rt) = self.rt() else { return };
        self.n += 1;
        self.cli.turns.fetch_add(1, Ordering::SeqCst);
        let n = self.n;
        let (drop_all, exit, delay, delay_submit, sub, read_full, usage) = {
            let mut sw = self.cli.sw.lock().unwrap();
            let exit = std::mem::take(&mut sw.exit_mid_turn) || sw.crash_always;
            (
                std::mem::take(&mut sw.drop_hooks),
                exit,
                sw.delay_post.take(),
                sw.delay_submit.take(),
                std::mem::take(&mut sw.subagent),
                sw.read_full,
                sw.usage,
            )
        };
        if let Some(d) = delay_submit {
            std::thread::sleep(d);
        }
        self.transcript(&rt, json!({"type": "user", "message": {"role": "user", "content": prompt}}));
        self.hook(&rt, drop_all, "UserPromptSubmit", json!({"prompt": prompt}));
        if exit {
            self.say("FAKE CLAUDE: crashed".into());
            self.dead = true;
            self.exit.exit(Some(1));
            return;
        }
        self.hook(
            &rt,
            drop_all,
            "PreToolUse",
            json!({"tool_name": "Bash", "tool_input": {"command": "echo hi"}, "tool_use_id": format!("tu{n}")}),
        );
        if read_full && let Some(i) = prompt.find("full text: ") {
            let path = prompt[i + "full text: ".len()..].split_whitespace().next().unwrap_or("");
            self.hook(&rt, drop_all, "PostToolUse", json!({"tool_name": "Read", "tool_input": {"file_path": path}}));
        }
        if let Some(d) = delay {
            std::thread::sleep(d);
        }
        let extra = if sub { json!({"tool_name": "Bash", "agent_id": "sub1"}) } else { json!({"tool_name": "Bash"}) };
        if let Some(out) = self.hook(&rt, drop_all, "PostToolUse", extra) {
            self.say(format!("POST[{n}]: {out}"));
        }
        self.status(&rt, drop_all, usage.unwrap_or(10.0 + n as f64), 0.01 * n as f64);
        self.transcript(
            &rt,
            json!({"type": "assistant", "message": {"role": "assistant", "stop_reason": "end_turn", "content": "ok"}}),
        );
        let (drop_stop, failure, bg) = {
            let mut sw = self.cli.sw.lock().unwrap();
            let failure = sw.stop_failure.take().or_else(|| sw.stop_failure_sticky.clone());
            (std::mem::take(&mut sw.drop_stop), failure, std::mem::take(&mut sw.bg))
        };
        if drop_stop {
            // The Stop never arrives: the core still believes in the turn.
        } else if let Some(err) = failure {
            if err == "rate_limit" {
                // The plan is spent: the statusline said so just before.
                self.status(&rt, drop_all, 100.0, 0.01);
            }
            self.hook(
                &rt,
                drop_all,
                "StopFailure",
                json!({"error": err, "last_assistant_message": format!("FAKE {err}")}),
            );
            if err == "rate_limit" {
                // The statusline refreshes now and then: the plan reads under
                // 100 % again (the core may place the limit a moment after
                // the hook, so it reads more than once).
                for i in 0..3 {
                    std::thread::sleep(Duration::from_millis(100));
                    self.status(&rt, drop_all, 20.0 + i as f64, 0.01);
                }
            }
        } else {
            let tasks = if bg { json!([{"id": "b1", "type": "shell", "status": "running"}]) } else { json!([]) };
            self.hook(&rt, drop_all, "Stop", json!({"background_tasks": tasks}));
        }
        self.say(format!("DONE[{n}] {prompt}"));
    }
}

/// A fake CLI for `agent`, as the terminal a spawn (or a test) gets.
fn terminal_for(
    rt: Weak<Runtime>,
    agent: &str,
    sid: &str,
    resumed: bool,
    cli: Cli,
    on_exit: Box<dyn FnOnce(Option<u32>) + Send>,
) -> PtyProcess {
    let (tx, rx) = channel::<Vec<u8>>();
    let pending = cli.pending.clone();
    let pty = PtyProcess::fake(
        Box::new(move |b| {
            pending.fetch_add(1, Ordering::SeqCst);
            let _ = tx.send(b.to_vec());
        }),
        on_exit,
    );
    let mut m = Machine {
        rt,
        agent: agent.into(),
        sid: sid.into(),
        out: pty.output.clone(),
        exit: pty.fake_handle().expect("a fake"),
        cli: cli.clone(),
        input: String::new(),
        n: 0,
        dead: false,
        resets5: now_ms() / 1000 + 3 * 3600,
        resets7: now_ms() / 1000 + 3 * 86_400,
        last_status_ms: 0,
    };
    std::thread::spawn(move || {
        // A process takes a moment to start: the spawn records it first.
        std::thread::sleep(Duration::from_millis(150));
        m.start(resumed);
        while let Ok(chunk) = rx.recv() {
            m.typed(&chunk);
            m.cli.pending.fetch_sub(1, Ordering::SeqCst);
        }
    });
    pty
}

/// A runtime whose spawns start a fake CLI (one per agent, in `clis`).
pub(crate) struct Sim {
    pub _d: tempfile::TempDir,
    pub rt: Arc<Runtime>,
    clis: Arc<Mutex<std::collections::BTreeMap<String, Cli>>>,
}

impl Sim {
    pub fn new(tweak: impl FnOnce(&mut crate::runtime::RuntimeConfig)) -> Sim {
        let d = tempfile::Builder::new().prefix("orchsim").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let mut cfg = test_cfg(d.path(), false);
        // spawn() checks a version and the helpers; nothing of them runs.
        let claude = d.path().join("claude");
        let exec = |p: &std::path::Path, body: &str| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(p, body).unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        exec(&claude, "#!/bin/sh\necho '2.1.302 (Claude Code)'\n");
        cfg.claude = claude;
        let bin = cfg.paths.bin_dir();
        std::fs::create_dir_all(&bin).unwrap();
        for h in ["orch-hook", "orch-status", "orch-mcp"] {
            exec(&bin.join(h), "#!/bin/sh\n");
        }
        cfg.helpers = crate::compile::HelperPaths::in_dir(&bin);
        let cell: Arc<OnceLock<Weak<Runtime>>> = Arc::new(OnceLock::new());
        let clis: Arc<Mutex<std::collections::BTreeMap<String, Cli>>> = Default::default();
        let (c2, k2) = (cell.clone(), clis.clone());
        cfg.terminal = Some(TerminalFactory(Arc::new(move |compiled, on_exit| {
            let agent = compiled.agent_dir.file_name().unwrap().to_string_lossy().to_string();
            let a = &compiled.args;
            let at = a.iter().position(|x| x == "--resume" || x == "--session-id").expect("a session flag");
            let (resumed, sid) = (a[at] == "--resume", a[at + 1].clone());
            let cli = k2.lock().unwrap().entry(agent.clone()).or_default().clone();
            Ok(terminal_for(c2.get().cloned().unwrap_or_default(), &agent, &sid, resumed, cli, on_exit))
        })));
        tweak(&mut cfg);
        let rt = Runtime::new(cfg).unwrap();
        let _ = cell.set(Arc::downgrade(&rt));
        Sim { _d: d, rt, clis }
    }

    /// The fake CLI of `agent` (the same one across its restarts).
    pub fn cli(&self, agent: &str) -> Cli {
        self.clis.lock().unwrap().entry(agent.into()).or_default().clone()
    }

    /// A plain agent `a1` in a git worktree of its own, idle and live with a
    /// fake CLI already running.
    pub fn agent_a1(&self) -> Cli {
        self.agent_a1_unattached();
        self.attach("a1")
    }

    /// The record of `a1` in a git worktree of its own (returned, canonical),
    /// not started.
    pub fn agent_a1_unattached(&self) -> std::path::PathBuf {
        let wt = crate::paths::real(self._d.path()).join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "i"],
        ] {
            assert!(std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success());
        }
        let mut a = test_agent(self._d.path());
        a.repo = wt.clone();
        a.worktree = wt.clone();
        self.rt.store.write(|s| s.agents.insert("a1".into(), a)).unwrap();
        wt
    }

    /// Gives `agent` a live fake CLI now (as if it had been started and had
    /// passed SessionStart), idle and ready for input.
    pub fn attach(&self, agent: &str) -> Cli {
        let cli = self.cli(agent);
        let sid = self.rt.store.read(|s| s.agents[agent].session_id.clone());
        let (weak, id) = (Arc::downgrade(&self.rt), agent.to_string());
        let pid_cell = Arc::new(OnceLock::<i32>::new());
        let pid_for_exit = pid_cell.clone();
        let on_exit: Box<dyn FnOnce(Option<u32>) + Send> = Box::new(move |status| {
            if let (Some(rt), Some(pid)) = (weak.upgrade(), pid_for_exit.get()) {
                rt.on_exit(&id, *pid, status);
            }
        });
        let pty = terminal_for(Arc::downgrade(&self.rt), agent, &sid, false, cli.clone(), on_exit);
        let _ = pid_cell.set(pty.pid);
        cli.started.store(false, Ordering::SeqCst);
        let (pid, pgid) = (pty.pid, pty.pgid);
        insert_test_live_with(&self.rt, agent, pty);
        let end = Instant::now() + Duration::from_secs(5);
        while !cli.started.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.rt
            .store
            .write(|s| {
                let a = s.agents.get_mut(agent).unwrap();
                a.on_spawned(pid, pgid);
                a.spawns = a.spawns.max(1);
                a.tracker.state = AgentState::Idle;
            })
            .unwrap();
        cli
    }

    /// One pass of what the server loop does.
    ///
    /// The paste watchdog is given a clock 30 s ahead once every fake CLI is
    /// quiet (judged after the pump and the tick, which may have typed), so its
    /// 20 s bound costs nothing; `ack_secs` stays the setting it is.
    pub fn step(&self) {
        self.rt.pump();
        self.rt.team_tick();
        let quiet = self.clis.lock().unwrap().values().all(|c| c.quiet());
        self.rt.paste_watchdog(now_ms() + if quiet { 30_000 } else { 0 });
        // The watchdog's later clock must not hold the agent back for real.
        let ids: Vec<String> = self.clis.lock().unwrap().keys().cloned().collect();
        for l in ids.iter().filter_map(|id| self.rt.live(id)) {
            if l.ready_at_ms.load(Ordering::SeqCst) > now_ms() + 1_000 {
                l.ready_at_ms.store(now_ms(), Ordering::SeqCst);
            }
        }
        self.rt.housekeeping();
        std::thread::sleep(Duration::from_millis(20));
    }

    /// Runs the loop until `done`, within `secs`.
    pub fn until(&self, what: &str, secs: u64, mut done: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            self.step();
            if done() {
                return;
            }
        }
        let turns = self.rt.store.read(|s| {
            s.turns
                .iter()
                .map(|t| format!("{} {:?} attempts={} body={}", t.id, t.status, t.attempts, t.body))
                .collect::<Vec<_>>()
        });
        let lim = self.rt.with_governor(|g| {
            format!(
                "{:?}",
                g.actions().iter().rev().take(6).map(|a| format!("{:?} {}", a.action, a.reason)).collect::<Vec<_>>()
            )
        });
        let agents = self.rt.store.read(|s| {
            s.agents
                .values()
                .map(|a| {
                    format!(
                        "{} {:?} flag={:?} resume_failed={} crashes={}",
                        a.id,
                        a.tracker.state,
                        a.flag,
                        a.resume_failed,
                        a.crash_restarts.len()
                    )
                })
                .collect::<Vec<_>>()
        });
        let inbox = self
            .rt
            .store
            .read(|s| s.inbox.iter().map(|i| format!("{} {} {}", i.kind, i.body, i.payload)).collect::<Vec<_>>());
        let clis: Vec<_> = self.clis.lock().unwrap().iter().map(|(k, c)| (k.clone(), c.lines())).collect();
        panic!(
            "timed out waiting for {what}\ngov: {lim}\nagents: {agents:#?}\ninbox: {inbox:#?}\nturns: {turns:#?}\nscreens: {clis:#?}"
        );
    }

    /// A bugfix-squad run past its spec gate (lead `fx-lead-1`, nobody running).
    pub fn team(&self) -> (tempfile::TempDir, orch_store::TeamRun) {
        let (rd, repo) = orch_git::fixtures::scratch_repo();
        let p = orch_git::project::register(&self.rt.db, &repo).unwrap();
        let (roles, team) = orch_team::presets::bugfix_squad();
        for r in &roles {
            self.rt.db.upsert_role(r, 1).unwrap();
        }
        self.rt.db.save_team(&team, &roles, 1).unwrap();
        let run = crate::team::tests::approved(&self.rt, &p.id);
        (rd, run)
    }

    /// The agent's unread messages.
    pub fn unread(&self, run: &str, node: &str, agent: &str) -> usize {
        self.rt.db.messages(run, Some((node, Some(agent))), true).unwrap().len()
    }

    /// The run timeline's events of a kind.
    pub fn run_events(&self, run: &str, kind: &str) -> Vec<Value> {
        self.rt.db.flush().unwrap();
        self.rt.db.run_events(run, 1000).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.detail).collect()
    }

    pub fn turn(&self, id: &str) -> TurnRow {
        self.rt.store.read(|s| s.turns.iter().find(|t| t.id == id).cloned().expect("a turn"))
    }

    pub fn status(&self, id: &str) -> TurnStatus {
        self.turn(id).status
    }

    pub fn state(&self, agent: &str) -> AgentState {
        self.rt.store.read(|s| s.agents[agent].tracker.state)
    }

    pub fn events(&self, agent: &str, kind: &str) -> Vec<Value> {
        self.rt.db.flush().unwrap();
        self.rt.db.events(0, Some(agent), Some(kind), 200).unwrap().into_iter().map(|e| e.detail).collect()
    }

    /// Open `delivery` / `startup` ... items of a kind.
    pub fn items(&self, kind: &str) -> Vec<crate::store::InboxItem> {
        self.rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == kind && i.pending()).cloned().collect())
    }

    /// A user message for the agent; its turn id.
    pub fn say(&self, agent: &str, text: &str) -> String {
        self.rt.deliver_kind(agent, text, TurnKind::User).unwrap()
    }

    /// The agent takes the next message: delivered, seen by the model, done.
    pub fn next_message_arrives(&self, agent: &str, cli: &Cli) {
        let t = self.say(agent, "and the next one");
        self.until("the next message done", 30, || self.status(&t) == TurnStatus::Done);
        assert!(cli.saw(&format!("[t:{t}]")), "the model saw {t}: {:?}", cli.lines());
        self.until("idle again", 10, || self.state(agent) == AgentState::Idle);
    }

    /// Delivery bounds that cost nothing: 1 s ack, 1 s invariant and slot wait.
    pub fn short_bounds(&self) {
        for k in [ACK_KEY, INVARIANT_KEY, SLOT_WAIT_KEY] {
            self.rt.db.set_setting(k, &json!(1)).unwrap();
        }
    }
}

#[cfg(test)]
mod scenarios {
    use super::*;
    use crate::store::TurnStatus as T;

    fn sim_with_agent() -> (Sim, Cli) {
        let sim = Sim::new(|_| {});
        sim.short_bounds();
        let cli = sim.agent_a1();
        (sim, cli)
    }

    /// F1: the Enter was lost, the text sits in the input box. The watchdog
    /// (20 s in production) sends Enter once more.
    #[test]
    fn f1_a_swallowed_enter_is_sent_again() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.swallow_enter = true);
        let t = sim.say("a1", "second");
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        assert!(cli.saw(&format!("[t:{t}]")));
        assert!(sim.events("a1", "redelivered").iter().any(|e| e["how"] == "enter"));
        sim.next_message_arrives("a1", &cli);
    }

    /// F1: a dialog eats every key until Esc. The watchdog sends Esc once and
    /// puts the turn back; the paste then goes through.
    #[test]
    fn f1_a_modal_gets_an_esc_and_the_turn_is_pasted_again() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.modal = true);
        let t = sim.say("a1", "second");
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        assert!(cli.saw(&format!("[t:{t}]")));
        assert!(sim.events("a1", "redelivered").iter().any(|e| e["how"] == "esc"));
        sim.next_message_arrives("a1", &cli);
    }

    /// F1 (a lost hook): the CLI took the paste but its hooks never reached the
    /// core. The marker is in the transcript: applied late, no second paste.
    #[test]
    fn f1_a_lost_hook_is_applied_late_from_the_transcript() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.drop_hooks = true);
        let t = sim.say("a1", "second");
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        assert!(sim.events("a1", "recovered").iter().any(|e| e["how"] == "late_hook"));
        assert!(sim.events("a1", "redelivered").is_empty(), "no second paste: {:?}", sim.events("a1", "redelivered"));
        sim.next_message_arrives("a1", &cli);
    }

    /// F1 (bound): a paste nobody ever acknowledges fails after 3 attempts with
    /// a `delivery` decision, and the agent takes its next turn.
    #[test]
    fn f1_three_unacknowledged_pastes_raise_a_delivery_decision() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| (s.modal, s.modal_always) = (true, true));
        let t = sim.say("a1", "second");
        sim.until("the turn to fail", 30, || sim.status(&t) == T::Failed);
        let items = sim.items("delivery");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].payload["blocker"], "no_ack");
        assert!(!sim.events("a1", "delivery_failed").is_empty());
        cli.set(|s| (s.modal, s.modal_always) = (false, false));
        sim.next_message_arrives("a1", &cli);
    }

    /// F2: an API error (the process lives) leaves the agent idle with one
    /// Continue turn; a second failure on that turn raises a decision, no loop.
    #[test]
    fn f2_a_server_error_gets_one_continue_and_a_second_failure_a_decision() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.stop_failure = Some("server_error".into()));
        sim.say("a1", "second");
        sim.until("the Continue turn", 30, || {
            cli.saw("your last turn failed (server_error)") && sim.state("a1") == AgentState::Idle
        });
        assert!(sim.events("a1", "recovered").iter().any(|e| e["from_state"] == "error"));
        // The same error on the Continue turn: a decision instead of another.
        cli.set(|s| s.stop_failure_sticky = Some("server_error".into()));
        sim.say("a1", "third");
        sim.until("a decision", 30, || !sim.items("delivery").is_empty());
        assert!(!sim.events("a1", "continue_failed").is_empty(), "the second failure stops the chain");
        cli.set(|s| s.stop_failure_sticky = None);
        sim.until("idle", 10, || sim.state("a1") == AgentState::Idle);
        sim.next_message_arrives("a1", &cli);
    }

    /// F2: a rate limit blocks the agent; once the limit is over (the plan reads
    /// under 100 % again, a minute after the hit) it is idle and takes messages.
    #[test]
    fn f2_a_rate_limited_agent_is_idle_again_when_the_limit_is_over() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.stop_failure = Some("rate_limit".into()));
        sim.say("a1", "second");
        sim.until("blocked by the limit", 30, || sim.state("a1") == AgentState::BlockedByLimit);
        sim.until("idle again", 30, || {
            sim.rt.lift_limits(now_ms() + 61_000);
            sim.state("a1") == AgentState::Idle
        });
        sim.next_message_arrives("a1", &cli);
    }

    /// F2: a Stop with a background task leaves the agent idle, not working:
    /// a message is delivered (Claude Code queues input while shells run).
    #[test]
    fn f2_background_tasks_do_not_hold_a_message() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.bg = true);
        let t = sim.say("a1", "second");
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        sim.until("idle with a task", 10, || sim.state("a1") == AgentState::Idle);
        assert_eq!(sim.rt.store.read(|s| s.agents["a1"].tracker.background_tasks), 1);
        sim.next_message_arrives("a1", &cli);
    }

    /// F3: the process dies mid-turn. The next message restarts the agent with
    /// `--resume` and is pasted into the new process, no app restart.
    #[test]
    fn f3_a_crashed_agent_is_resumed_and_gets_its_message() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.exit_mid_turn = true);
        sim.say("a1", "this one dies");
        sim.until("the crash", 30, || sim.state("a1") == AgentState::Error);
        let t = sim.say("a1", "second");
        sim.until("the turn done", 40, || sim.status(&t) == T::Done);
        assert!(cli.saw(&format!("[t:{t}]")));
        let ev = sim.events("a1", "recovered");
        assert!(ev.iter().any(|e| e["from_state"] == "crashed" && e["how"] == "resume"), "{ev:?}");
        sim.next_message_arrives("a1", &cli);
    }

    /// F3: nobody writes again after the crash. The interrupted turn gets one
    /// Continue turn, which restarts the agent: it does not stay down.
    #[test]
    fn f3_a_crash_mid_turn_queues_a_continue_and_the_agent_restarts_by_itself() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.exit_mid_turn = true);
        let t = sim.say("a1", "this one dies");
        sim.until("the Continue in the new process", 40, || cli.saw("your session crashed mid-turn and was resumed"));
        sim.until("idle again", 20, || sim.state("a1") == AgentState::Idle);
        assert_eq!(sim.status(&t), T::Done, "the interrupted turn is closed");
        let ev = sim.events("a1", "recovered");
        assert!(ev.iter().any(|e| e["from_state"] == "crashed" && e["how"] == "continue"), "{ev:?}");
        assert!(ev.iter().any(|e| e["from_state"] == "crashed" && e["how"] == "resume"), "{ev:?}");
        sim.next_message_arrives("a1", &cli);
    }

    /// F3 (bound): at most 2 automatic restarts an hour; the third crash is a
    /// `startup` decision (`failed: crash`), and the user's resume works.
    #[test]
    fn f3_an_agent_that_keeps_crashing_raises_a_decision_after_two_restarts() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.crash_always = true);
        let crash_item = || sim.items("startup").iter().any(|i| i.payload["failed"] == "crash");
        for i in 0..6 {
            if crash_item() {
                break;
            }
            // Work that is not the user's own: it is what the budget counts.
            sim.rt.deliver_kind("a1", &format!("message {i}"), TurnKind::Message).unwrap();
            sim.until("a crash", 40, || sim.state("a1") == AgentState::Error || crash_item());
        }
        sim.until("a crash decision", 40, crash_item);
        assert_eq!(
            sim.events("a1", "recovered")
                .iter()
                .filter(|e| e["from_state"] == "crashed" && e["how"] == "resume")
                .count(),
            2
        );
        cli.set(|s| s.crash_always = false);
        // The user's own message resumes it, whatever the used-up budget says.
        sim.say("a1", "back again");
        sim.until("the resumed agent", 40, || cli.saw("back again"));
        assert!(sim.items("startup").iter().all(|i| i.payload["failed"] != "crash"), "the crash item is closed");
    }

    /// F4: the Stop never reached the core, so the agent looks `Working`. The
    /// watchdog reads the transcript, finds the turn ended and applies the Stop
    /// (15 min in production).
    #[test]
    fn f4_a_lost_stop_is_applied_from_the_transcript() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.drop_stop = true);
        let t = sim.say("a1", "second");
        sim.until("working with no Stop", 30, || {
            cli.saw(&format!("[t:{t}]")) && sim.state("a1") == AgentState::Working
        });
        sim.rt.watchdog(now_ms() + 16 * 60_000);
        sim.until("idle after the late Stop", 10, || sim.state("a1") == AgentState::Idle);
        let ev = sim.events("a1", "recovered");
        assert!(ev.iter().any(|e| e["from_state"] == "working_silent" && e["how"] == "late_hook"), "{ev:?}");
        sim.next_message_arrives("a1", &cli);
    }

    /// F5: an autonomous turn held by the Governor (the 5 h guard) no longer
    /// blocks the user's answer queued behind it.
    #[test]
    fn f5_a_held_turn_does_not_block_the_one_behind_it() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.usage = Some(90.0));
        let warm = sim.say("a1", "read the usage");
        sim.until("the usage read", 30, || sim.status(&warm) == T::Done);
        let task = TurnRow::queued("a1", "background chore", TurnKind::Task, crate::store::Admission::Pending);
        let task_id = task.id.clone();
        sim.rt.store.write(|s| s.turns.push(task)).unwrap();
        let answer = sim.say("a1", "the user answers");
        sim.until("the answer done", 30, || sim.status(&answer) == T::Done);
        assert!(!cli.saw("background chore"), "the held task did not run first");
        assert_ne!(sim.status(&task_id), T::Done);
    }

    const LEAD: &str = "fx-lead-1";

    /// A team whose lead runs a fake CLI and has done its first turn.
    fn team_with_lead(
        tweak: impl FnOnce(&mut crate::runtime::RuntimeConfig),
    ) -> (Sim, tempfile::TempDir, orch_store::TeamRun, Cli) {
        let sim = Sim::new(tweak);
        sim.short_bounds();
        let (rd, run) = sim.team();
        let cli = sim.attach(LEAD);
        sim.until("the lead's brief done", 30, || {
            sim.state(LEAD) == AgentState::Idle
                && cli.saw("Fix the X bug.")
                && sim.rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == LEAD).all(|t| t.status == T::Done))
        });
        (sim, rd, run, cli)
    }

    /// F9: a panic in a step of the team tick is an event, and delivery goes on.
    #[test]
    fn f9_a_panic_in_the_team_tick_does_not_stop_delivery() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        sim.rt.inject_tick_fault(Some("dispatch"));
        sim.rt.user_message(&run.id, "lead", "still delivered").unwrap();
        sim.until("the message", 30, || cli.saw("still delivered"));
        let panics = sim.run_events(&run.id, "tick_panic");
        assert!(panics.iter().any(|e| e["step"] == "dispatch" && e["run"] == run.id.as_str()), "{panics:?}");
        sim.rt.inject_tick_fault(None);
        sim.rt.user_message(&run.id, "lead", "and after the fault").unwrap();
        sim.until("the next message", 30, || cli.saw("and after the fault"));
    }

    /// F7: a message too long for the paste is written to a file and shown cut.
    /// It is read only when the model read the file; one that did not is handed
    /// back once, then marked read with `truncated: true`, never in a loop.
    #[test]
    fn f7_a_long_message_is_read_only_when_the_model_saw_all_of_it() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        let long = format!("Q{}", "x".repeat(5_000));
        cli.set(|s| s.read_full = true);
        sim.rt.user_message(&run.id, "lead", &long).unwrap();
        sim.until("the cut message", 30, || cli.saw("full text:"));
        sim.until("the message read", 30, || sim.unread(&run.id, "lead", LEAD) == 0);
        assert!(sim.events(LEAD, "read").iter().all(|e| e["truncated"] != true), "read in full");
        // The model does not read the file: handed back once, then read as truncated.
        cli.set(|s| s.read_full = false);
        sim.rt.user_message(&run.id, "lead", &long).unwrap();
        sim.until("read as truncated", 40, || {
            sim.unread(&run.id, "lead", LEAD) == 0 && sim.events(LEAD, "read").iter().any(|e| e["truncated"] == true)
        });
        sim.until("idle, nothing looping", 10, || sim.state(LEAD) == AgentState::Idle);
        sim.rt.user_message(&run.id, "lead", "short follow-up").unwrap();
        sim.until("the follow-up", 30, || cli.saw("short follow-up"));
    }

    /// F8: a message that arrives during a turn is injected at the next tool call
    /// and is seen once the reply was written; the model gets it once. A tool
    /// call of a subagent gets nothing: the message waits and arrives as a turn.
    #[test]
    fn f8_an_injected_message_is_seen_once_and_never_goes_into_a_subagent() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        let busy = |note: &str| {
            cli.set(|s| s.delay_post = Some(Duration::from_secs(2)));
            let t = sim.say(LEAD, "work on it");
            sim.until("the lead working", 30, || sim.state(LEAD) == AgentState::Working);
            sim.rt.user_message(&run.id, "lead", note).unwrap();
            t
        };
        let t = busy("mid-turn note");
        sim.until("the note in the tool reply", 30, || {
            cli.lines().iter().any(|l| l.starts_with("POST[") && l.contains("mid-turn note"))
        });
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        sim.until("the note read", 30, || sim.unread(&run.id, "lead", LEAD) == 0);
        sim.until("idle", 10, || sim.state(LEAD) == AgentState::Idle);
        assert_eq!(
            cli.lines().iter().filter(|l| l.contains("mid-turn note")).count(),
            1,
            "shown once, not pasted again"
        );
        cli.set(|s| s.subagent = true);
        let t = busy("subagent note");
        sim.until("the turn done", 30, || sim.status(&t) == T::Done);
        sim.until("the note as its own turn", 40, || {
            cli.lines().iter().any(|l| l.starts_with("DONE[") && l.contains("subagent note"))
        });
        assert!(!cli.lines().iter().any(|l| l.starts_with("POST[") && l.contains("subagent note")));
        sim.until("the note read", 30, || sim.unread(&run.id, "lead", LEAD) == 0);
    }

    /// Exit criterion 2: a message unread over the limit (10 min in production,
    /// `delivery.invariant_secs`) while its target is not working raises one
    /// `delivery` decision naming the cause. When the message is finally seen
    /// the decision closes by itself.
    #[test]
    fn the_delivery_invariant_names_the_cause_of_a_message_left_unread() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        // The paste watchdog stays away until the test lets it act.
        sim.rt.db.set_setting(ACK_KEY, &json!(3600)).unwrap();
        cli.set(|s| (s.modal, s.modal_always) = (true, true));
        sim.rt.user_message(&run.id, "lead", "are you there?").unwrap();
        sim.until("a delivery decision", 30, || !sim.items("delivery").is_empty());
        let item = sim.items("delivery").remove(0);
        assert_eq!(item.payload["blocker"], "no_ack", "{}", item.payload);
        assert_eq!(item.agent_id.as_deref(), Some(LEAD));
        for _ in 0..5 {
            sim.step();
        }
        assert_eq!(sim.items("delivery").len(), 1, "one decision per episode");
        // The cause goes away: the watchdog acts, the message arrives, the decision closes.
        cli.set(|s| (s.modal, s.modal_always) = (false, false));
        sim.rt.db.set_setting(ACK_KEY, &json!(1)).unwrap();
        sim.until("the message seen and the decision closed", 40, || {
            cli.saw("are you there?") && sim.unread(&run.id, "lead", LEAD) == 0 && sim.items("delivery").is_empty()
        });
    }

    /// F6: the live agents fill the cap while a member with unread messages has
    /// no process. After the wait (2 min in production, `delivery.slot_wait_secs`)
    /// the core suspends an idle member of the run that has nothing unread, and
    /// the waiting member starts and gets its message.
    #[test]
    fn f6_a_member_waiting_for_a_slot_gets_one_from_an_idle_colleague() {
        let (sim, _rd, run, _lead) = team_with_lead(|c| c.base_cap = Some(1));
        crate::team::tests::ok(&sim.rt, LEAD, "create_task", json!({"title": "UI", "node": "front", "body": "ui"}));
        sim.until("front's first turn done", 60, || {
            sim.rt.store.read(|s| {
                s.agents.get("fx-front-1").is_some_and(|a| a.tracker.state == AgentState::Idle && a.spawns > 0)
            })
        });
        crate::team::tests::ok(&sim.rt, LEAD, "create_task", json!({"title": "API", "node": "back", "body": "api"}));
        sim.rt.user_message(&run.id, "back", "please look at this").unwrap();
        let back = sim.cli("fx-back-1");
        sim.until("back's message", 60, || back.saw("please look at this"));
        assert!(
            !sim.events("fx-front-1", "slot_freed").is_empty() || !sim.run_events(&run.id, "slot_freed").is_empty()
        );
        sim.until("read", 30, || sim.unread(&run.id, "back", "fx-back-1") == 0);
    }

    /// Retry on an invariant decision takes the stuck paste back and sends it
    /// again; if that fails too the user hears of it again (a new episode, after
    /// another invariant window), and when it works the decision stays closed.
    #[test]
    fn retry_on_an_invariant_decision_resends_and_a_second_failure_is_raised_again() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        sim.rt.db.set_setting(ACK_KEY, &json!(3600)).unwrap();
        cli.set(|s| (s.modal, s.modal_always) = (true, true));
        sim.rt.user_message(&run.id, "lead", "are you there?").unwrap();
        sim.until("a delivery decision", 30, || !sim.items("delivery").is_empty());
        let first = sim.items("delivery").remove(0);
        assert_eq!(first.payload["episode"], "stuck_lead", "the lead's own episode");
        let stuck: Vec<String> = sim.rt.store.read(|s| {
            s.turns.iter().filter(|t| t.agent_id == LEAD && t.status == T::Delivering).map(|t| t.id.clone()).collect()
        });
        assert_eq!(stuck.len(), 1);
        sim.rt.retry_delivery(first.id).unwrap();
        assert!(sim.items("delivery").is_empty(), "the decision is closed");
        assert_eq!(sim.status(&stuck[0]), T::Queued, "the stuck paste is taken back");
        // Still stuck: a second decision, after another window.
        sim.until("a second decision", 30, || !sim.items("delivery").is_empty());
        assert_ne!(sim.items("delivery")[0].id, first.id);
        // It works this time, through a slow CLI that takes the paste after the 1 s
        // invariant window: the window is lifted (and `ACK_KEY` stays 3600), so
        // nothing is raised again or re-pressed while the retry goes through.
        cli.set(|s| {
            (s.modal, s.modal_always) = (false, false);
            s.delay_submit = Some(Duration::from_millis(1500));
        });
        sim.rt.db.set_setting(INVARIANT_KEY, &json!(3600)).unwrap();
        let item = sim.items("delivery").remove(0);
        sim.rt.retry_delivery(item.id).unwrap();
        sim.until("the message seen", 40, || cli.saw("are you there?") && sim.unread(&run.id, "lead", LEAD) == 0);
        let left: Vec<_> = sim.items("delivery").into_iter().map(|i| i.payload).collect();
        assert!(left.is_empty(), "no decision raised again: {left:?}");
    }

    /// The same failing agent keeps failing: one open decision, not one per failed
    /// Continue; after the user is done with it, a new failure is a new decision.
    #[test]
    fn a_repeated_failure_keeps_one_open_decision() {
        let (sim, cli) = sim_with_agent();
        cli.set(|s| s.stop_failure_sticky = Some("server_error".into()));
        sim.say("a1", "one");
        sim.until("a decision", 30, || !sim.items("delivery").is_empty());
        sim.until("idle", 10, || sim.state("a1") == AgentState::Idle);
        sim.say("a1", "two");
        sim.until("the second failure chain", 30, || sim.events("a1", "continue_failed").len() >= 2);
        sim.until("idle", 10, || sim.state("a1") == AgentState::Idle);
        assert_eq!(sim.items("delivery").len(), 1, "one open decision");
    }

    /// Messages reach the model in the order they were sent, a long one keeps its
    /// place (cut, with its file), and nothing is dropped.
    #[test]
    fn messages_are_rendered_in_send_order_and_a_long_one_keeps_its_place() {
        let (sim, _rd, run, cli) = team_with_lead(|_| {});
        let long = format!("LONG{}", "y".repeat(4_000));
        for body in ["first note", long.as_str(), "third note"] {
            sim.rt.user_message(&run.id, "lead", body).unwrap();
        }
        sim.until("all three", 30, || cli.saw("third note"));
        let line =
            cli.lines().into_iter().find(|l| l.starts_with("DONE[") && l.contains("first note")).expect("one turn");
        let at = |n: &str| line.find(n).unwrap_or_else(|| panic!("{n} in {line}"));
        assert!(at("first note") < at("LONG") && at("LONG") < at("full text:") && at("full text:") < at("third note"));
        assert!(!line.contains(&"y".repeat(1_000)), "the long one is cut");
    }

    /// The plumbing of the messages (handed, seen, read…) is kept per agent and
    /// never fills the run timeline's window: a real message is still there
    /// after 300 of them.
    #[test]
    fn plumbing_events_do_not_push_real_messages_off_the_timeline() {
        let (sim, _rd, run, _cli) = team_with_lead(|_| {});
        sim.rt.user_message(&run.id, "lead", "the one that matters").unwrap();
        for i in 0..300 {
            let kind = ["handed", "seen", "read", "continue_failed"][i % 4];
            sim.rt.delivery_event(LEAD, kind, json!({"messages": [i]}));
        }
        sim.rt.db.flush().unwrap();
        let v = sim.rt.run_json(&run.id).unwrap();
        let kinds: Vec<&str> = v["timeline"].as_array().unwrap().iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(kinds.contains(&"message"), "the message is in the timeline: {kinds:?}");
        assert!(kinds.iter().all(|k| !["handed", "seen", "read", "continue_failed"].contains(k)));
        // They are still the agent's events, for the report.
        assert!(!sim.events(LEAD, "seen").is_empty());
    }
}
