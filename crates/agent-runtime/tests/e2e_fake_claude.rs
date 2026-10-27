//! End to end without quota: the real bus server, real PTYs, the real
//! orch-hook and orch-status binaries, and a shell script standing in for
//! `claude` (tests/fake_claude.sh).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use agent_runtime::paths::AppPaths;
use agent_runtime::resources::PressureLevel;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{self, request};
use orch_protocol::{AgentSpec, CtlRequest, bus};
use serde_json::Value;

mod common;
use common::{agent, git, helper_bins, short_tempdir};

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn start_deliver_suspend_resume_kill_with_fake_claude() {
    // Short root: the socket path must stay under 104 bytes.
    let root = short_tempdir("orche2e");
    let paths = AppPaths::new(root.path().join("app"));
    agent_runtime::preflight::install_helpers(&helper_bins(), &paths.bin_dir()).unwrap();
    let fake = root.path().join("fake-claude");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_claude.sh"), &fake).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = root.path().join("proj/demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);

    let mut cfg = RuntimeConfig::new(paths.clone());
    cfg.claude = fake;
    cfg.projects_root = root.path().join("projects");
    cfg.hold = Duration::from_secs(1);
    let rt = Runtime::new(cfg).unwrap();
    let listener = bus::bind_listener(&paths.socket()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let srv = {
        let (rt, stop) = (rt.clone(), stop.clone());
        std::thread::spawn(move || server::serve(rt, listener, stop).unwrap())
    };
    let sock = paths.socket();
    let state = || request(&sock, CtlRequest::State).unwrap().data;

    // Start: worktree, compile, spawn, first message through the Governor.
    let spec = AgentSpec {
        agent_id: "e2e".into(),
        repo: repo.display().to_string(),
        model: Some("haiku".into()),
        first_message: Some("/hello there".into()),
        ..Default::default()
    };
    let r = request(&sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap();
    assert!(r.ok, "{r:?}");
    let wt = PathBuf::from(r.data["worktree"].as_str().unwrap());
    assert!(wt.ends_with("demo.worktrees/solo/e2e"), "{wt:?}");
    wait_for("first turn done", 15, || agent(&sock, "e2e")["turns"] == 1 && agent(&sock, "e2e")["state"] == "idle");

    // The pasted turn carried its marker and the leading-`/` escape, and came back Done.
    let s = state();
    let t = s["turns"].as_array().unwrap().iter().find(|t| t["agent_id"] == "e2e").unwrap().clone();
    assert_eq!(t["status"], "done", "{t}");
    let screen = String::from_utf8(
        request(&sock, CtlRequest::Read { agent_id: "e2e".into() }).unwrap().data.as_str().unwrap().as_bytes().to_vec(),
    )
    .unwrap();
    assert!(screen.contains("env-api-key=unset autoupdate=1"), "{screen}");
    assert!(screen.contains(&format!("DONE[1] [t:{}]  /hello there", t["id"].as_str().unwrap())), "{screen}");
    // Shadow mode: the tool call was admitted and nothing was printed for it.
    assert!(screen.contains("PRE[1]:\r\n") || screen.contains("PRE[1]:\n"), "{screen}");

    // Usage: the statusline sample reached the collector and the Governor.
    let u = request(&sock, CtlRequest::Usage).unwrap().data;
    assert_eq!(u["five_hour"]["used_pct"], 11.0, "{u}");
    assert_eq!(u["weekly"]["used_pct"], 34.0);
    let g = request(&sock, CtlRequest::Governor).unwrap().data;
    let points: Vec<&str> =
        g["decisions"].as_array().unwrap().iter().map(|d| d["request"]["point"].as_str().unwrap()).collect();
    assert!(points.contains(&"turn_start") && points.contains(&"tool_call"), "{g}");

    // A second message: delivered only once the agent is idle again. It asks
    // the fake to commit through orch-mcp: the core commits in the worktree.
    request(&sock, CtlRequest::Deliver { agent_id: "e2e".into(), text: "second COMMIT".into() }).unwrap();
    wait_for("second turn", 10, || agent(&sock, "e2e")["turns"] == 2 && agent(&sock, "e2e")["state"] == "idle");
    assert_eq!(git(&wt, &["log", "-1", "--format=%s"]).trim(), "fake commit 2");
    assert_eq!(git(&wt, &["show", "--name-only", "--format=", "HEAD"]).trim(), "work-2.txt");
    let audit = request(&sock, CtlRequest::Audit { agent_id: Some("e2e".into()), limit: None }).unwrap().data;
    assert_eq!(audit[0]["action"], "commit", "{audit}");
    assert_eq!(audit[0]["decision"], "ok");

    // Generated files.
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(paths.agent_dir("e2e").join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 600);
    assert!(std::fs::read_to_string(paths.agent_dir("e2e").join("gitconfig")).unwrap().contains("helper ="));

    // Suspend keeps the session; resume brings it back with --resume.
    let sid = agent(&sock, "e2e")["session_id"].as_str().unwrap().to_string();
    let pid = agent(&sock, "e2e")["pid"].as_i64().unwrap() as i32;
    assert!(request(&sock, CtlRequest::Suspend { agent_id: "e2e".into() }).unwrap().ok);
    wait_for("suspended", 10, || agent(&sock, "e2e")["state"] == "suspended");
    assert!(!agent_runtime::procs::alive(pid));
    assert!(
        request(&sock, CtlRequest::Resume { agent_id: "e2e".into(), message: Some("after resume".into()) }).unwrap().ok
    );
    wait_for("resumed turn", 15, || agent(&sock, "e2e")["turns"].as_u64() == Some(3));
    assert_eq!(agent(&sock, "e2e")["session_id"], sid.as_str());
    rt.db.flush().unwrap();
    let spawns = rt.db.events(0, Some("e2e"), Some("spawned"), 10).unwrap();
    assert!(spawns.iter().any(|e| e.detail.to_string().contains("--resume")), "resume used --resume");

    // Kill: process gone, state killed.
    let pid = agent(&sock, "e2e")["pid"].as_i64().unwrap() as i32;
    assert!(request(&sock, CtlRequest::Kill { agent_id: "e2e".into() }).unwrap().ok);
    wait_for("killed", 10, || agent(&sock, "e2e")["state"] == "killed");
    assert!(!agent_runtime::procs::alive(pid));

    assert!(request(&sock, CtlRequest::Shutdown).unwrap().ok);
    srv.join().unwrap();
}

#[test]
fn hook_with_core_down_denies_tools_and_spools() {
    // The real orch-hook binary, no core: PreToolUse is denied, Stop exits 0 and is spooled.
    let bins = helper_bins();
    let d = short_tempdir("orchhk");
    let run = |event: &str| {
        let mut c = Command::new(bins.join("orch-hook"))
            .env_clear()
            .env("ORCH_AGENT_ID", "a1")
            .env("ORCH_SOCK", d.path().join("core.sock"))
            .env("ORCH_AGENT_DIR", d.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        c.stdin
            .take()
            .unwrap()
            .write_all(format!(r#"{{"hook_event_name":"{event}","tool_name":"Bash"}}"#).as_bytes())
            .unwrap();
        c.wait_with_output().unwrap()
    };
    let o = run("PreToolUse");
    assert_eq!(o.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    let o = run("Stop");
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stdout.is_empty());
    let spool = std::fs::read_to_string(d.path().join("spool.jsonl")).unwrap();
    assert_eq!(spool.lines().count(), 2);
}

/// A core on a fake claude with its own config tweaks; returns what tests need.
struct Core {
    _root: tempfile::TempDir,
    rt: Arc<Runtime>,
    sock: PathBuf,
    repo: PathBuf,
    stop: Arc<AtomicBool>,
    srv: Option<std::thread::JoinHandle<()>>,
}

impl Core {
    fn start(prefix: &str, tweak: impl FnOnce(&mut RuntimeConfig)) -> Core {
        let root = short_tempdir(prefix);
        let paths = AppPaths::new(root.path().join("app"));
        agent_runtime::preflight::install_helpers(&helper_bins(), &paths.bin_dir()).unwrap();
        let fake = root.path().join("fake-claude");
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_claude.sh"), &fake).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repo = root.path().join("proj/demo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);
        let mut cfg = RuntimeConfig::new(paths.clone());
        cfg.claude = fake;
        cfg.projects_root = root.path().join("projects");
        cfg.hold = Duration::from_secs(1);
        cfg.idle_suspend = None;
        tweak(&mut cfg);
        let rt = Runtime::new(cfg).unwrap();
        let listener = bus::bind_listener(&paths.socket()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let srv = {
            let (rt, stop) = (rt.clone(), stop.clone());
            std::thread::spawn(move || server::serve(rt, listener, stop).unwrap())
        };
        Core { _root: root, rt, sock: paths.socket(), repo, stop, srv: Some(srv) }
    }

    fn start_agent(&self, id: &str) {
        let spec = AgentSpec { agent_id: id.into(), repo: self.repo.display().to_string(), ..Default::default() };
        let r = request(&self.sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap();
        assert!(r.ok, "{r:?}");
        request(&self.sock, CtlRequest::Deliver { agent_id: id.into(), text: "hello".into() }).unwrap();
        wait_for("first turn", 15, || agent(&self.sock, id)["turns"] == 1 && agent(&self.sock, id)["state"] == "idle");
    }

    fn state_of(&self, id: &str) -> String {
        agent(&self.sock, id)["state"].as_str().unwrap_or("").to_string()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = request(&self.sock, CtlRequest::Shutdown);
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.srv.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn memory_pressure_lowers_the_live_cap_and_queued_work_waits_for_a_slot() {
    let mem = Arc::new(agent_runtime::resources::FixedMemory::new(PressureLevel::Normal));
    let m2 = mem.clone();
    let core = Core::start("orchmp", move |c| {
        c.base_cap = 2;
        c.memory = m2;
    });
    core.start_agent("old");
    std::thread::sleep(Duration::from_millis(300));
    core.start_agent("new");
    assert_eq!(core.rt.live_cap(), 2);
    // Critical pressure: cap 1, the least recently active idle agent is suspended.
    mem.set(PressureLevel::Critical);
    wait_for("one suspended", 15, || core.state_of("old") == "suspended");
    assert_eq!(core.state_of("new"), "idle");
    assert_eq!(core.rt.live_cap(), 1);
    // Work for the suspended agent waits in the Governor's queue (shadow mode
    // too: memory is enforced), with its reason.
    request(&core.sock, CtlRequest::Deliver { agent_id: "old".into(), text: "more".into() }).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(core.state_of("old"), "suspended");
    let reason = core.rt.with_governor(|g| g.queue().iter().find(|e| e.agent.0 == "old").map(|e| e.reason.clone()));
    assert_eq!(reason, Some(governor::QueueReason::MemoryPressure { cap: 1 }));
    // Pressure ends: the cap recovers and the agent is resumed through the Governor.
    mem.set(PressureLevel::Normal);
    wait_for("resumed and done", 20, || agent(&core.sock, "old")["turns"].as_u64() == Some(2));
}

#[test]
fn the_memory_cap_is_checked_at_spawn_and_run_now_overrides_it() {
    use agent_runtime::store::{Admission, TurnRow};
    let mem = Arc::new(agent_runtime::resources::FixedMemory::new(PressureLevel::Normal));
    let m2 = mem.clone();
    let core = Core::start("orchsp", move |c| {
        c.base_cap = 2;
        c.memory = m2;
    });
    core.start_agent("old");
    std::thread::sleep(Duration::from_millis(300));
    core.start_agent("new");
    mem.set(PressureLevel::Critical);
    wait_for("one suspended", 15, || core.state_of("old") == "suspended");
    // A turn already admitted (a release, a restored turn) for the suspended
    // agent: the spawn finds no slot, so it goes back to the queue.
    core.rt
        .store
        .write(|s| s.turns.push(TurnRow::queued("old", "more", governor::TurnKind::Message, Admission::Admitted)))
        .unwrap();
    wait_for("spawn held", 10, || !core.rt.db.events(0, Some("old"), Some("spawn_held"), 5).unwrap().is_empty());
    assert_eq!(core.state_of("old"), "suspended");
    let entry = core.rt.with_governor(|g| g.queue().iter().find(|e| e.agent.0 == "old").cloned()).expect("queued");
    assert_eq!(entry.reason, governor::QueueReason::MemoryPressure { cap: 1 });
    // Run now: the user may exceed the cap (FR-203).
    core.rt.run_now(entry.id.0).unwrap();
    // (Housekeeping may then suspend the other, idle agent to get back under the cap.)
    wait_for("resumed past the cap", 20, || core.state_of("old") != "suspended");
    assert_eq!(core.rt.live_cap(), 1);
}

#[test]
fn idle_agents_are_suspended_and_resume_on_new_work() {
    let core = Core::start("orchid", |c| c.idle_suspend = Some(Duration::from_secs(2)));
    core.start_agent("a");
    wait_for("idle suspend", 15, || core.state_of("a") == "suspended");
    let ev = core.rt.db.events(0, Some("a"), Some("auto_suspend"), 5).unwrap();
    assert_eq!(ev[0].detail["why"], "idle");
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "wake up".into() }).unwrap();
    wait_for("resumed turn", 20, || agent(&core.sock, "a")["turns"].as_u64() == Some(2));
}

/// J3 end to end with the fake claude: launch, the lead starts when the
/// Governor admits it, spec gate, a task dispatched to a new specialist in its
/// own worktree, a message, report, merge, then the PR gate.
#[test]
fn a_team_run_end_to_end_with_fake_claude() {
    let core = Core::start("orchtm", |_| {});
    let sock = core.sock.clone();
    // Settled: idle with nothing queued (the app's own turns included).
    let settled = |agent_id: &str| {
        let a = agent(&sock, agent_id);
        a["state"] == "idle" && a["queued_turns"] == 0
    };
    // Until that very turn is done and the agent settled.
    let call = |agent_id: &str, line: &str| {
        let r = request(&sock, CtlRequest::Deliver { agent_id: agent_id.into(), text: line.into() }).unwrap();
        let turn = r.data["turn"].as_str().unwrap().to_string();
        wait_for(line, 30, || {
            let s = request(&sock, CtlRequest::State).unwrap().data;
            let done = s["turns"].as_array().unwrap().iter().any(|t| t["id"] == turn.as_str() && t["status"] == "done");
            done && settled(agent_id)
        });
    };
    let screen = |agent_id: &str| {
        request(&sock, CtlRequest::Read { agent_id: agent_id.into() }).unwrap().data.as_str().unwrap_or("").to_string()
    };
    let run_state =
        |run: &str| request(&sock, CtlRequest::Run { run_id: run.into() }).unwrap().data["run"]["status"].clone();
    let p = request(&sock, CtlRequest::ProjectAdd { path: core.repo.display().to_string() }).unwrap();
    let pid = p.data["id"].as_str().unwrap().to_string();
    assert!(request(&sock, CtlRequest::InstallPresets).unwrap().ok);
    let r = request(
        &sock,
        CtlRequest::Launch {
            project_id: pid.clone(),
            team_id: "bugfix-squad".into(),
            slug: "fx".into(),
            title: Some("Fix X".into()),
            brief: Some("Fix the X bug.".into()),
            base: None,
        },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");
    let run = r.data["id"].as_str().unwrap().to_string();
    let lead = "fx-lead-1";
    // The lead starts with its brief once admitted.
    wait_for("lead's first turn", 20, || agent(&sock, lead)["turns"] == 1 && agent(&sock, lead)["state"] == "idle");
    assert!(screen(lead).contains("Fix the X bug."));
    let lead_wt = PathBuf::from(agent(&sock, lead)["worktree"].as_str().unwrap());
    assert!(lead_wt.ends_with("demo.worktrees/fx/integration"), "{lead_wt:?}");
    // The spec, committed through orch-mcp, then the gate.
    std::fs::create_dir_all(lead_wt.join(".orchestrator/features/fx")).unwrap();
    std::fs::write(lead_wt.join(".orchestrator/features/fx/spec.md"), "# Fix X\n").unwrap();
    call(lead, r#"ORCHTOOL commit {"message":"spec for fx"}"#);
    call(lead, r#"ORCHTOOL request_spec_approval {"summary":"one task for back"}"#);
    assert_eq!(run_state(&run), "spec_gate", "{}", screen(lead));
    let inbox = request(&sock, CtlRequest::Inbox).unwrap().data;
    let gate = inbox.as_array().unwrap().iter().find(|i| i["payload"]["action"] == "spec_approval").unwrap().clone();
    let d = request(
        &sock,
        CtlRequest::Decide {
            item_id: gate["id"].as_u64().unwrap(),
            decision: orch_protocol::InboxDecision::Approve,
            scope: None,
            message: None,
        },
    )
    .unwrap();
    assert!(d.ok, "{d:?}");
    assert_eq!(run_state(&run), "planning");
    wait_for("approval delivered", 20, || agent(&sock, lead)["turns"].as_u64() >= Some(4));
    wait_for("approval in the pane", 10, || screen(lead).contains("approved: proceed"));
    call(lead, r#"ORCHTOOL create_task {"title":"API","node":"back","body":"add api.rs"}"#);
    // Dispatch: a new specialist in its own worktree, branched from integration.
    let back = "fx-back-1";
    wait_for("back's first turn", 30, || agent(&sock, back)["turns"] == 1 && agent(&sock, back)["state"] == "idle");
    assert!(screen(back).contains("ORCH: task T-"));
    assert_eq!(run_state(&run), "executing");
    let back_wt = PathBuf::from(agent(&sock, back)["worktree"].as_str().unwrap());
    assert_eq!(git(&back_wt, &["symbolic-ref", "--short", "HEAD"]).trim(), "orch/fx/back-1");
    assert!(back_wt.join(".orchestrator/features/fx/spec.md").is_file());
    // A tool the role lacks is not even listed.
    call(back, r#"ORCHTOOL merge {"branch":"orch/fx/back-1"}"#);
    assert!(screen(back).contains("unknown tool"), "{}", screen(back));
    // A message to the idle lead arrives as a turn.
    call(back, r#"ORCHTOOL send_message {"to":"lead","body":"which file?"}"#);
    wait_for("message delivered", 20, || screen(lead).contains("which file?"));
    // Work, report, merge.
    std::fs::write(back_wt.join("api.rs"), "fn api() {}\n").unwrap();
    call(back, r#"ORCHTOOL commit {"message":"add api.rs"}"#);
    call(back, r#"ORCHTOOL report_done {"summary":"api.rs added"}"#);
    let tasks = request(&sock, CtlRequest::Tasks { project_id: None, run_id: Some(run.clone()) }).unwrap().data;
    let task = tasks[0]["id"].as_u64().unwrap();
    assert_eq!(tasks[0]["status"], "review", "{}", screen(back));
    call(lead, &format!(r#"ORCHTOOL merge {{"branch":"orch/fx/back-1","task":{task}}}"#));
    assert!(lead_wt.join("api.rs").is_file(), "{}", screen(lead));
    wait_for("integrating", 20, || run_state(&run) == "integrating");
    call(lead, r#"ORCHTOOL request_pr {"title":"Fix X"}"#);
    assert_eq!(run_state(&run), "pr_gate", "{}", screen(lead));
    let tl = request(&sock, CtlRequest::Timeline { run_id: run.clone(), limit: None }).unwrap().data;
    let kinds: Vec<&str> = tl.as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    for k in ["run.launched", "agent.added", "task.assigned", "message", "git.merge", "task.done"] {
        assert!(kinds.contains(&k), "timeline has {k}: {kinds:?}");
    }
}
