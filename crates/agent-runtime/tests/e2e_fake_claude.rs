//! End to end without quota: the real bus server, real PTYs, the real
//! orch-hook and orch-status binaries, and a shell script standing in for
//! `claude` (tests/fake_claude.sh).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use agent_runtime::paths::AppPaths;
use agent_runtime::resources::PressureLevel;
use agent_runtime::runtime::{Runtime, RuntimeConfig, test_plan};
use agent_runtime::server::{self, request};
use orch_protocol::{AgentSpec, CtlRequest, bus};
use serde_json::Value;

mod common;
use common::{Core, Screens, agent, git, helper_bins, short_tempdir, wait_for};

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
    cfg.default_plan = test_plan;
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

#[test]
fn memory_pressure_lowers_the_live_cap_and_queued_work_waits_for_a_slot() {
    let mem = Arc::new(agent_runtime::resources::FixedMemory::new(PressureLevel::Normal));
    let m2 = mem.clone();
    let core = Core::start("orchmp", move |c| {
        c.base_cap = Some(2);
        c.memory = m2;
        c.pressure_filter = agent_runtime::resources::PressureFilter::new(0, 0);
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
        c.base_cap = Some(2);
        c.memory = m2;
        c.pressure_filter = agent_runtime::resources::PressureFilter::new(0, 0);
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

/// FR-705/709 (acceptance 2, 3): a rule answers at once; a prompt nobody
/// answers parks the agent (suspended, its card intact); Allow resumes it and
/// the retried call runs without a second prompt. The audit names each decider.
#[test]
fn an_unanswered_permission_parks_the_agent_and_allow_runs_the_retry() {
    use agent_runtime::store::InboxStatus;
    use orch_protocol::InboxDecision;
    let core = Core::start("orchpk", |c| c.permission_hold = Duration::from_secs(2));
    core.start_agent("a");
    let audit = || {
        let a = request(&core.sock, CtlRequest::Audit { agent_id: Some("a".into()), limit: None }).unwrap().data;
        a.as_array().unwrap().iter().rev().map(|e| e["actor"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
    };
    let screens = Screens::default();
    let screen = || screens.read(&core.sock, "a");

    // A rule answers at once.
    let rule = core
        .rt
        .add_rule(
            orch_store::RuleScope::Agent,
            "a",
            None,
            "Bash(cargo test:*)",
            agent_runtime::policy::Effect::Allow,
            None,
        )
        .unwrap();
    let rule_id = rule["rule"]["id"].as_str().unwrap().to_string();
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT cargo test -q".into() }).unwrap();
    wait_for("rule turn", 15, || agent(&core.sock, "a")["turns"] == 2 && core.state_of("a") == "idle");
    let perm2 = screen().lines().find(|l| l.contains("PERM[2]:")).map(str::to_string).unwrap_or_default();
    assert!(perm2.contains(r#""behavior":"allow""#), "{}", screen());
    assert!(core.rt.inbox().is_empty());

    // Nobody answers within the hold: denied, parked, suspended.
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT pnpm add zod".into() }).unwrap();
    wait_for("parked and suspended", 30, || core.state_of("a") == "suspended");
    let item = core.rt.inbox().into_iter().find(|i| i.kind == "permission").expect("the card stays");
    assert_eq!(item.status, InboxStatus::Parked);
    assert_eq!(item.payload["tool_input"]["command"], "pnpm add zod");
    assert!(item.payload["cwd"].as_str().is_some_and(|c| c.ends_with("solo/a")), "{}", item.payload);
    assert_eq!(item.payload["tool_use_id"], "tp3");
    // Queued work does not wake a parked agent.
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "other work".into() }).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(core.state_of("a"), "suspended");

    // Allow: resumed with the retry, which runs without a second prompt.
    let r = request(
        &core.sock,
        CtlRequest::Decide { item_id: item.id, decision: InboxDecision::Approve, scope: None, message: None },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");
    wait_for("retried", 30, || screen().contains("retry the call Bash pnpm add zod") && core.state_of("a") == "idle");
    wait_for("other work too", 30, || agent(&core.sock, "a")["queued_turns"] == 0 && core.state_of("a") == "idle");
    // The pasted retry shows (and the agent reads idle) before its turn
    // submits: wait for the retried call's own permission answer.
    wait_for("retry answered", 30, || audit().len() >= 4);
    let open: Vec<_> = core.rt.inbox().into_iter().map(|i| (i.kind, i.status, i.body, i.payload)).collect();
    assert!(open.is_empty(), "no second prompt: {open:?}");
    // The rule, the expiry, the user's Allow, and the retry on their one-shot allow.
    assert_eq!(audit(), [format!("rule:{rule_id}"), "expired".into(), "user".into(), "user".into()]);
    let one_shot = request(&core.sock, CtlRequest::Audit { agent_id: Some("a".into()), limit: Some(1) }).unwrap().data;
    assert_eq!(one_shot[0]["detail"]["one_shot"], true, "{one_shot}");
}

/// FR-415 (acceptance 4): no SessionStart → a `startup` item with the PTY
/// tail; a resume that dies at once with no transcript restarts fresh once;
/// a fresh start that dies too is the user's (agent in error, no loop).
#[test]
fn starts_that_never_happen_reach_the_inbox_or_restart_fresh_once() {
    use agent_runtime::store::InboxStatus;
    let core = Core::start("orchsu", |c| c.startup_timeout = Duration::from_secs(2));
    let startup = || core.rt.inbox().into_iter().find(|i| i.kind == "startup");
    let spawns = |id: &str| {
        core.rt.db.flush().unwrap();
        core.rt.db.events(0, Some(id), Some("spawned"), 50).unwrap().len()
    };

    // Stuck before SessionStart: one item, with what the pane shows.
    git(&core.repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "x"]);
    std::fs::write(core.repo.join(".fake-no-session-start"), "").unwrap();
    git(&core.repo, &["add", ".fake-no-session-start"]);
    git(&core.repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "-m", "stuck"]);
    let spec = AgentSpec { agent_id: "stuck".into(), repo: core.repo.display().to_string(), ..Default::default() };
    assert!(request(&core.sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap().ok);
    wait_for("startup item", 30, || startup().is_some());
    let item = startup().unwrap();
    assert_eq!((item.agent_id.as_deref(), item.payload["mode"].as_str()), (Some("stuck"), Some("new")));
    assert!(item.payload["tail"].as_str().unwrap().contains("a dialog nobody answers"), "{}", item.payload);
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(core.rt.inbox().iter().filter(|i| i.kind == "startup").count(), 1, "one item, not one per pass");
    assert_eq!(core.state_of("stuck"), "starting", "not suspended while the card is open");
    assert!(request(&core.sock, CtlRequest::Kill { agent_id: "stuck".into() }).unwrap().ok);
    core.rt.decide(item.id, orch_protocol::InboxDecision::Dismiss, None, None).unwrap();
    git(&core.repo, &["rm", "-q", ".fake-no-session-start"]);
    git(&core.repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "-m", "unstuck"]);

    // A resume with no transcript that exits at once: a fresh session, once.
    core.start_agent("a");
    let wt = PathBuf::from(agent(&core.sock, "a")["worktree"].as_str().unwrap());
    let sid = agent(&core.sock, "a")["session_id"].as_str().unwrap().to_string();
    assert!(request(&core.sock, CtlRequest::Suspend { agent_id: "a".into() }).unwrap().ok);
    wait_for("suspended", 10, || core.state_of("a") == "suspended");
    std::fs::write(wt.join(".fake-resume-dies"), "").unwrap();
    // No transcript: the fake keeps one where the core looks (beside the socket).
    let no_transcript = |sid: &str| {
        let t = core.sock.parent().unwrap().join(format!("fake-projects/-fake/{sid}.jsonl"));
        std::fs::remove_file(&t).unwrap_or_else(|e| panic!("{}: {e}", t.display()));
    };
    no_transcript(&sid);
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "after the resume".into() }).unwrap();
    let did_turn = || agent(&core.sock, "a")["turns"].as_u64() == Some(2) && core.state_of("a") == "idle";
    let end = std::time::Instant::now() + Duration::from_secs(30);
    while !did_turn() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !did_turn() {
        core.rt.db.flush().unwrap();
        let ev: Vec<_> =
            core.rt.db.events(0, Some("a"), None, 60).unwrap().into_iter().map(|e| (e.kind, e.detail)).collect();
        let turns = request(&core.sock, CtlRequest::State).unwrap().data["turns"].clone();
        panic!("fresh session did the turn: {} turns: {turns} events: {ev:?}", agent(&core.sock, "a"));
    }
    assert_ne!(agent(&core.sock, "a")["session_id"].as_str().unwrap(), sid);
    core.rt.db.flush().unwrap();
    let fresh = core.rt.db.events(0, Some("a"), Some("restarted_fresh"), 5).unwrap();
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].detail["old_session"], sid.as_str());
    assert!(startup().is_none());

    // A fresh start that dies too: the user's, with no third attempt.
    std::fs::write(wt.join(".fake-start-dies"), "").unwrap();
    assert!(request(&core.sock, CtlRequest::Suspend { agent_id: "a".into() }).unwrap().ok);
    wait_for("suspended again", 10, || core.state_of("a") == "suspended");
    no_transcript(agent(&core.sock, "a")["session_id"].as_str().unwrap());
    let before = spawns("a");
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "again".into() }).unwrap();
    wait_for("failed start item", 30, || startup().is_some_and(|i| i.payload.get("failed").is_some()));
    let failed = startup().unwrap();
    assert_eq!((failed.payload["failed"].as_str(), failed.status), (Some("fresh"), InboxStatus::Pending));
    assert!(failed.payload["tail"].as_str().unwrap().contains("no conversation found"));
    assert_eq!(core.state_of("a"), "error");
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(spawns("a"), before + 2, "the resume and one fresh start, no loop");
}

/// FR-203 (acceptance 5): a probe flapping warn↔normal leaves the cap alone,
/// and an agent with a pasted, not started turn is never suspended.
#[test]
fn a_flapping_probe_keeps_the_cap_and_a_pasted_turn_is_not_suspended_until_requeued_or_failed() {
    let mem = Arc::new(agent_runtime::resources::FixedMemory::new(PressureLevel::Normal));
    let m2 = mem.clone();
    let core = Core::start("orchfl", move |c| {
        c.base_cap = Some(2);
        c.memory = m2;
        c.idle_suspend = Some(Duration::from_secs(1));
    });
    core.start_agent("a");
    // A pasted turn the fake never starts (a SLOW one hangs before its Stop,
    // so mark one Delivering by hand): neither idleness nor pressure stops it.
    // Marked first: idle for 1 s, the agent would be suspended during the flapping.
    core.rt
        .store
        .write(|s| {
            let mut t = agent_runtime::store::TurnRow::queued(
                "a",
                "pasted",
                governor::TurnKind::Message,
                agent_runtime::store::Admission::Admitted,
            );
            t.status = agent_runtime::store::TurnStatus::Delivering;
            t.delivered_at_ms = Some(orch_protocol::now_ms());
            // Its last attempt: the watchdog fails it rather than paste it again.
            t.attempts = 2;
            s.turns.push(t);
        })
        .unwrap();
    for i in 0..8 {
        mem.set(if i % 2 == 0 { PressureLevel::Warn } else { PressureLevel::Normal });
        std::thread::sleep(Duration::from_millis(1_500));
        assert_eq!(core.rt.live_cap(), 2, "the cap holds while the probe flaps");
    }
    assert_ne!(core.state_of("a"), "suspended");
    // fix-team-getting-stucked §3.1: the paste watchdog ends the paste (no
    // ack within `delivery.ack_secs`), and the agent is suspendable again.
    core.rt.db.set_setting(agent_runtime::handover::ACK_KEY, &serde_json::json!(1)).unwrap();
    let failed = || {
        core.rt
            .store
            .read(|s| s.turns.iter().any(|t| t.agent_id == "a" && t.status == agent_runtime::store::TurnStatus::Failed))
    };
    wait_for("the watchdog to fail the paste", 20, failed);
    wait_for("an idle suspend once nothing is pasted", 20, || core.state_of("a") == "suspended");
}

/// FR-1311 (acceptance 7): typing into a waiting agent's pane records one
/// `pane_intervention` with its cause; focus changes record nothing.
#[test]
fn typing_into_a_waiting_pane_is_one_intervention_and_focus_is_none() {
    let core = Core::start("orchpi", |c| c.permission_hold = Duration::from_secs(30));
    core.start_agent("a");
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT pnpm add zod".into() }).unwrap();
    wait_for("held prompt", 15, || core.rt.inbox().iter().any(|i| i.kind == "permission"));
    let events = || {
        core.rt.db.flush().unwrap();
        core.rt.db.events(0, Some("a"), Some("pane_intervention"), 10).unwrap()
    };
    core.rt.pane_input("a", b"\x1b[I\x1b[O").unwrap();
    assert!(events().is_empty(), "focus in/out is not typing");
    core.rt.pane_input("a", b"y").unwrap();
    core.rt.pane_input("a", b"y").unwrap();
    let ev = events();
    assert_eq!(ev.len(), 1, "one per wait episode");
    assert_eq!(
        (ev[0].detail["waited_on"].as_str(), ev[0].detail["cause"].as_str()),
        (Some("user"), Some("permission"))
    );
    let item = core.rt.inbox().into_iter().find(|i| i.kind == "permission").unwrap();
    assert_eq!(ev[0].detail["item"], item.id);
    core.rt.decide(item.id, orch_protocol::InboxDecision::Approve, None, None).unwrap();
}

/// P4.1: a permission prompt moves the agent's record to needs_you, the
/// answer back to running, one `now` event each, with a gapless rev.
#[test]
fn a_permission_prompt_goes_needs_you_then_running_one_now_event_each() {
    let core = Core::start("orchnow", |c| c.permission_hold = Duration::from_secs(30));
    core.start_agent("a");
    let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    {
        let g = got.clone();
        core.rt.subscribe_now(Arc::new(move |v| g.lock().unwrap().push(v.clone())));
    }
    // A record of `a` in an event: (rev, level, status).
    let mine = || {
        got.lock()
            .unwrap()
            .iter()
            .filter_map(|e| {
                let r = e["attention"]["agents"].as_array()?.iter().find(|r| r["agent"] == "a")?.clone();
                Some((e["rev"].as_u64().unwrap(), r))
            })
            .collect::<Vec<_>>()
    };
    let level_is = |l: &str| mine().last().is_some_and(|(_, r)| r["level"] == l);
    // SLOW keeps the turn running after the answer (the fake sleeps 30 s).
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT pnpm add zod SLOW".into() }).unwrap();
    wait_for("needs you", 15, || level_is("needs_you"));
    let item = core.rt.inbox().into_iter().find(|i| i.kind == "permission").unwrap();
    let (_, rec) = mine().into_iter().find(|(_, r)| r["level"] == "needs_you").unwrap();
    assert_eq!(rec["status"], "waiting_for_you");
    assert_eq!(rec["waiting_on"]["kind"], "user");
    assert_eq!(rec["waiting_on"]["ref"], format!("inbox-{}", item.id));
    assert_eq!(rec["reason"], "permission: Bash pnpm add zod SLOW");
    assert_eq!(rec["waiting_on"]["since_ms"], item.created_at_ms);
    core.rt.decide(item.id, orch_protocol::InboxDecision::Approve, None, None).unwrap();
    wait_for("running", 15, || level_is("running"));
    std::thread::sleep(Duration::from_millis(600));
    let levels: Vec<String> = mine().iter().map(|(_, r)| r["level"].as_str().unwrap().to_string()).collect();
    let at = levels.iter().position(|l| l == "needs_you").unwrap();
    assert_eq!(levels[at..], ["needs_you", "running"], "one event each: {levels:?}");
    assert!(levels[..at].iter().all(|l| l == "running" || l == "idle"), "{levels:?}");
    // Every event's rev is the previous one + 1.
    let revs: Vec<u64> = got.lock().unwrap().iter().map(|e| e["rev"].as_u64().unwrap()).collect();
    assert!(revs.windows(2).all(|w| w[1] == w[0] + 1), "{revs:?}");
    // The snapshot agrees with the last event.
    let snap = request(&core.sock, CtlRequest::Now).unwrap().data;
    assert_eq!(snap["rev"], *revs.last().unwrap());
    let a = snap["agents"].as_array().unwrap().iter().find(|r| r["agent"] == "a").unwrap().clone();
    assert_eq!(a["level"], "running");
    assert_eq!(snap["counts"]["global"]["needs_you"], 0);
    let _ = core.rt.interrupt("a");
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
    let screens = Screens::default();
    let screen = |agent_id: &str| screens.read(&sock, agent_id);
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
    // The team reviews itself first: the core types the review commands as is,
    // each its own turn, then asks for the report again.
    call(back, r#"ORCHTOOL report_done {"summary":"api.rs added"}"#);
    let s = request(&sock, CtlRequest::State).unwrap().data;
    let commands: Vec<&Value> =
        s["turns"].as_array().unwrap().iter().filter(|t| t["agent_id"] == back && t["command"] == true).collect();
    assert_eq!(commands.len(), 2, "{s}");
    assert!(commands.iter().all(|t| t["status"] == "done"), "each command ran as its own turn: {commands:?}");
    let out = screen(back);
    assert!(out.contains("DONE") && out.contains("/simplify"), "{out}");
    assert!(out.contains("your self-review is done"), "{out}");
    call(back, r#"ORCHTOOL report_done {"summary":"api.rs added","review":{"fixed":["typo"],"unfixed":[]}}"#);
    let tasks = request(&sock, CtlRequest::Tasks { project_id: None, run_id: Some(run.clone()) }).unwrap().data;
    let task = tasks[0]["id"].as_u64().unwrap();
    assert_eq!(tasks[0]["status"], "review", "{}", screen(back));
    call(lead, &format!(r#"ORCHTOOL merge {{"branch":"orch/fx/back-1","task":{task}}}"#));
    assert!(lead_wt.join("api.rs").is_file(), "{}", screen(lead));
    // A second task for back: the same agent, started in a fresh session.
    let old_session = agent(&sock, back)["session_id"].as_str().unwrap().to_string();
    call(lead, r#"ORCHTOOL create_task {"title":"Docs","node":"back","body":"add docs.md"}"#);
    wait_for("back's second task", 30, || {
        let a = agent(&sock, back);
        a["session_id"] != old_session.as_str() && settled(back) && screen(back).contains("Earlier on this branch")
    });
    let new_session = agent(&sock, back)["session_id"].as_str().unwrap().to_string();
    let spawns = request(
        &sock,
        CtlRequest::Events { agent_id: Some(back.into()), kind: Some("spawned".into()), since_ms: None, limit: None },
    )
    .unwrap()
    .data;
    let args: Vec<&str> = spawns[0]["detail"]["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    let at = args.iter().position(|a| *a == "--session-id").expect("a new session");
    assert_eq!(args[at + 1], new_session, "{args:?}");
    assert!(!args.contains(&"--resume"), "{args:?}");
    std::fs::write(back_wt.join("docs.md"), "# Docs\n").unwrap();
    call(back, r#"ORCHTOOL commit {"message":"add docs.md"}"#);
    call(back, r#"ORCHTOOL report_done {"summary":"docs added"}"#);
    call(back, r#"ORCHTOOL report_done {"summary":"docs added","review":{"fixed":[],"unfixed":[]}}"#);
    let tasks = request(&sock, CtlRequest::Tasks { project_id: None, run_id: Some(run.clone()) }).unwrap().data;
    let docs = tasks.as_array().unwrap().iter().find(|t| t["title"] == "Docs").unwrap()["id"].as_u64().unwrap();
    call(lead, &format!(r#"ORCHTOOL merge {{"branch":"orch/fx/back-1","task":{docs}}}"#));
    wait_for("integrating", 20, || run_state(&run) == "integrating");
    call(lead, r#"ORCHTOOL request_pr {"title":"Fix X"}"#);
    assert_eq!(run_state(&run), "pr_gate", "{}", screen(lead));
    let tl = request(&sock, CtlRequest::Timeline { run_id: run.clone(), limit: None }).unwrap().data;
    let kinds: Vec<&str> = tl.as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    for k in ["run.launched", "agent.added", "task.assigned", "message", "git.merge", "task.done"] {
        assert!(kinds.contains(&k), "timeline has {k}: {kinds:?}");
    }
}

/// P4.4 (spec §5 "Nothing waits on the Overwatch", acceptance 6): a
/// permission with effect `overwatch` starts the Overwatch lazily (its day
/// folder, briefing and first turn), the card waits on it while its turn
/// runs; its process killed mid-turn puts it `down`, the card goes to the
/// user at once and still acts, and the next permission goes straight to
/// the user. No pane intervention anywhere.
#[test]
fn killing_the_overwatch_mid_turn_leaves_every_card_actionable() {
    use orch_protocol::InboxDecision;
    let core = Core::start("orchow", |c| c.permission_hold = Duration::from_secs(40));
    // Solo sessions default to the user: send this one's calls to the Overwatch.
    core.rt.set_policy_defaults("overwatch", "overwatch").unwrap();
    let mut s = core.rt.overwatch_settings();
    s.permission_secs = 30;
    core.rt.set_overwatch_settings(s).unwrap();
    core.start_agent("a");
    assert!(agent(&core.sock, agent_runtime::overwatch::ID).is_null(), "lazy: no session before an event");

    // The fake sleeps 30 s in a turn whose text says SLOW: the Overwatch's
    // turn carries this command in its event line.
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT pnpm add SLOW".into() }).unwrap();
    let ow = agent_runtime::overwatch::ID;
    wait_for("the Overwatch's first turn", 20, || agent(&core.sock, ow)["state"] == "working");
    let day = core.rt.store.read(|s| s.agents[ow].worktree.clone());
    assert!(day.join("briefing.md").is_file(), "{day:?}");
    let screens = Screens::default();
    wait_for("its turn text", 10, || screens.read(&core.sock, ow).contains("permission inbox-"));
    assert!(screens.read(&core.sock, ow).contains("Read briefing.md"));
    let open_card = || {
        let snap = request(&core.sock, CtlRequest::Now).unwrap().data;
        snap["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["kind"] == "permission" && d["lifecycle"]["state"] == "open")
            .cloned()
            .unwrap_or(Value::Null)
    };
    let card = open_card();
    assert_eq!(card["waits_on"], "overwatch", "{card}");
    assert!(card["deciding_until_ms"].is_u64());
    let id: u64 = card["id"].as_str().unwrap().trim_start_matches("inbox-").parse().unwrap();

    // Killed from outside, mid-turn.
    let pid = agent(&core.sock, ow)["pid"].as_i64().unwrap();
    assert!(Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap().success());
    wait_for("down", 10, || core.rt.overwatch_status().status == agent_runtime::overwatch::Status::Down);
    assert_eq!(core.rt.overwatch_status().why.as_deref(), Some("stopped unexpectedly"));
    // `down` is set first, the hand-over follows in the same call: at once.
    wait_for("handed over at once", 2, || open_card()["waits_on"] == "user");
    let card = open_card();
    assert!(card.get("deciding_until_ms").is_none(), "{card}");
    assert_eq!(card["payload"]["asks_because"]["kind"], "overwatch_unsure");
    // The card still acts.
    let r = request(
        &core.sock,
        CtlRequest::Decide { item_id: id, decision: InboxDecision::Approve, scope: None, message: None },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");
    wait_for("the call allowed", 10, || screens.read(&core.sock, "a").contains("\"allow\""));

    // While down, the next permission goes straight to the user.
    wait_for("a idle", 40, || core.state_of("a") == "idle");
    request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "PERMIT pnpm add zx".into() }).unwrap();
    wait_for("second card", 10, || open_card()["waits_on"] == "user");
    assert!(open_card().get("deciding_until_ms").is_none());
    let id2: u64 = open_card()["id"].as_str().unwrap().trim_start_matches("inbox-").parse().unwrap();
    let r = request(
        &core.sock,
        CtlRequest::Decide { item_id: id2, decision: InboxDecision::Reject, scope: None, message: Some("no".into()) },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");

    // Restart: a fresh session with a new briefing.
    assert!(request(&core.sock, CtlRequest::OverwatchRestart).unwrap().ok);
    wait_for("back", 20, || core.rt.overwatch_status().status != agent_runtime::overwatch::Status::Down);
    core.rt.db.flush().unwrap();
    assert!(core.rt.db.events(0, None, Some("pane_intervention"), 10).unwrap().is_empty());
    let report = request(&core.sock, CtlRequest::Overwatch).unwrap().data;
    assert!(report["turns"].as_array().is_some(), "{report}");
}

/// P4.4 exit criterion (spec §1 (a)): a team run where the user only uses
/// the cards. A routine permission is settled by the policy, an unmatched
/// one by the Overwatch, a question by the Overwatch with a source, a
/// specialist that never started is restarted by it, the spec gate is
/// approved from its card (annotated by the Overwatch), and no
/// `pane_intervention` is written. The test plays the model of the Overwatch:
/// it delivers `ORCHTOOL …` lines to the fake `overwatch` session, which calls
/// the real orch-mcp tools.
#[test]
fn a_team_run_where_the_user_only_uses_the_cards_has_no_pane_intervention() {
    use agent_runtime::overwatch::{ID as OW, Status};
    use agent_runtime::policy::Effect;
    // Long enough that a slow CI runner never hands the permission over to
    // the user before the Overwatch's turn decides it.
    let core = Core::start("orchx", |c| c.permission_hold = Duration::from_secs(150));
    let sock = core.sock.clone();
    let mut s = core.rt.overwatch_settings();
    (s.batch_secs, s.permission_secs) = (1, 120);
    core.rt.set_overwatch_settings(s).unwrap();
    let settled = |id: &str| {
        let a = agent(&sock, id);
        a["state"] == "idle" && a["queued_turns"] == 0
    };
    let call = |agent_id: &str, line: &str| {
        let r = request(&sock, CtlRequest::Deliver { agent_id: agent_id.into(), text: line.into() }).unwrap();
        let turn = r.data["turn"].as_str().unwrap().to_string();
        wait_for(line, 40, || {
            let s = request(&sock, CtlRequest::State).unwrap().data;
            let done = s["turns"].as_array().unwrap().iter().any(|t| t["id"] == turn.as_str() && t["status"] == "done");
            done && settled(agent_id)
        });
    };
    // As the Overwatch's model would: one tool call, from its own session.
    let ow_calls = |line: &str| {
        request(&sock, CtlRequest::Deliver { agent_id: OW.into(), text: line.into() }).unwrap();
    };
    let screens = Screens::default();
    let screen = |agent_id: &str| screens.read(&sock, agent_id);
    // On a timeout, what the lead and the Overwatch saw (CI has no other trace).
    fn wait_dump(screens: &Screens, sock: &Path, what: &str, secs: u64, mut f: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        while std::time::Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let (ow, lead) = (screens.read(sock, "overwatch"), screens.read(sock, "fx-lead-1"));
        panic!("timed out waiting for {what}\n--- the Overwatch's screen ---\n{ow}\n--- the lead's screen ---\n{lead}");
    }
    let open = |kind: &str| -> Option<Value> {
        let snap = request(&sock, CtlRequest::Now).unwrap().data;
        snap["decisions"]
            .as_array()?
            .iter()
            .find(|d| d["kind"] == kind && matches!(d["lifecycle"]["state"].as_str(), Some("open" | "parked")))
            .cloned()
    };

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
    let lead = "fx-lead-1";
    wait_dump(&screens, &sock, "lead's first turn", 30, || {
        agent(&sock, lead)["turns"] == 1 && agent(&sock, lead)["state"] == "idle"
    });
    let lead_wt = PathBuf::from(agent(&sock, lead)["worktree"].as_str().unwrap());
    std::fs::create_dir_all(lead_wt.join(".orchestrator/features/fx")).unwrap();
    std::fs::write(lead_wt.join(".orchestrator/features/fx/spec.md"), "# Fix X\n\nUse cursor pagination.\n").unwrap();
    call(lead, r#"ORCHTOOL commit {"message":"spec for fx"}"#);

    // 1. A routine permission: the policy's rule settles it, no card.
    core.rt.add_rule(orch_store::RuleScope::Project, &pid, None, "Bash(cargo add:*)", Effect::Allow, None).unwrap();
    call(lead, "PERMIT cargo add serde_json");
    assert!(open("permission").is_none(), "settled by the policy, no card");
    assert!(screen(lead).contains("\"allow\""), "{}", screen(lead));
    assert!(core.rt.db.audit_log(Some(lead), 20).unwrap().iter().any(|e| e.actor.starts_with("rule:")));

    // 2. An unmatched one: team roles default to the Overwatch, which decides
    // it inside the turn that carries it (OWALLOW: the fake calls
    // decide_permission there; a later turn would be too late, F1).
    request(&sock, CtlRequest::Deliver { agent_id: lead.into(), text: "PERMIT pnpm add zod OWALLOW".into() }).unwrap();
    // Read from the store: a warm Overwatch can settle it before a `Now` poll
    // ever sees the card open.
    let zod = || {
        core.rt.store.read(|s| {
            s.inbox
                .iter()
                .rev()
                .find(|i| i.kind == "permission" && i.payload.to_string().contains("pnpm add zod"))
                .cloned()
        })
    };
    wait_dump(&screens, &sock, "the card", 20, || zod().is_some());
    let item = zod().unwrap();
    assert_eq!(item.payload["effect"].as_str(), Some("overwatch"), "{}", item.payload);
    let n = item.id;
    wait_dump(&screens, &sock, "the Overwatch's turn", 40, || screen(OW).contains(&format!("permission inbox-{n}")));
    let allowed = || zod().is_some_and(|i| i.decided_by.is_some()) && open("permission").is_none() && settled(lead);
    let end = std::time::Instant::now() + Duration::from_secs(60);
    while !allowed() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(allowed(), "not allowed: {:?}\n--- the Overwatch's screen ---\n{}", zod(), screen(OW));
    let item = zod().unwrap();
    assert_eq!(item.decided_by.as_deref(), Some("overwatch"));

    // 3. A question: answered by the Overwatch, citing the spec.
    call(lead, r#"ORCHTOOL ask_user {"question":"Which pagination should the API use?"}"#);
    wait_dump(&screens, &sock, "the question", 20, || open("question").is_some());
    let q = open("question").unwrap()["id"].as_str().unwrap().to_string();
    wait_dump(&screens, &sock, "the Overwatch hears of it", 40, || screen(OW).contains(&format!("question {q}")));
    ow_calls(&format!(
        r#"ORCHTOOL answer_question {{"decision":"{q}","answers":[{{"text":"Cursor based."}}],"sources":["spec.md#1"]}}"#
    ));
    wait_dump(&screens, &sock, "the lead hears the answer", 30, || {
        screen(lead).contains("the Overwatch answered your question (source: spec.md#1)")
    });

    // 4. A specialist-like session that never starts: the Overwatch restarts it fresh.
    let stuck_repo = core.root_path().join("proj/stuck");
    std::fs::create_dir_all(&stuck_repo).unwrap();
    git(&stuck_repo, &["init", "-q", "-b", "main"]);
    std::fs::write(stuck_repo.join(".fake-no-session-start"), "").unwrap();
    git(&stuck_repo, &["add", "."]);
    git(&stuck_repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "-m", "init"]);
    let spec = AgentSpec { agent_id: "stuck".into(), repo: stuck_repo.display().to_string(), ..Default::default() };
    assert!(request(&sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap().ok);
    wait_dump(&screens, &sock, "its startup card", 40, || open("startup").is_some());
    let stuck_wt = PathBuf::from(agent(&sock, "stuck")["worktree"].as_str().unwrap());
    std::fs::remove_file(stuck_wt.join(".fake-no-session-start")).unwrap();
    core.rt.overwatch_tick();
    wait_dump(&screens, &sock, "the Overwatch hears of it", 40, || screen(OW).contains("startup-"));
    ow_calls(r#"ORCHTOOL restart_fresh {"agent":"stuck","why":"never started"}"#);
    wait_dump(&screens, &sock, "restarted", 40, || open("startup").is_none() && core.state_of("stuck") == "idle");
    let startup = core.rt.store.read(|s| s.inbox.iter().find(|i| i.kind == "startup").cloned().unwrap());
    assert_eq!(startup.decided_by.as_deref(), Some("overwatch"));

    // 5. The spec gate, annotated by the Overwatch and approved from its card.
    call(lead, r#"ORCHTOOL request_spec_approval {"summary":"one task for back"}"#);
    wait_dump(&screens, &sock, "the gate", 20, || open("gate").is_some());
    let gate = open("gate").unwrap()["id"].as_str().unwrap().to_string();
    core.rt.overwatch_pump();
    wait_dump(&screens, &sock, "the Overwatch hears of it", 40, || screen(OW).contains(&gate));
    ow_calls(&format!(
        r#"ORCHTOOL annotate {{"decision":"{gate}","note":"Scope matches the brief.","recommend":"approve"}}"#
    ));
    wait_dump(&screens, &sock, "annotated", 30, || {
        open("gate").is_some_and(|g| g["annotation"]["recommend"] == "approve")
    });
    let n: u64 = gate.trim_start_matches("gate-").parse().unwrap();
    let d = request(
        &sock,
        CtlRequest::Decide { item_id: n, decision: orch_protocol::InboxDecision::Approve, scope: None, message: None },
    )
    .unwrap();
    assert!(d.ok, "{d:?}");

    // The Overwatch did its work with the user in the cards and the chat only.
    assert_ne!(core.rt.overwatch_status().status, Status::Down, "{:?}", core.rt.overwatch_status());
    core.rt.db.flush().unwrap();
    let who: Vec<String> =
        core.rt.db.audit_log(None, 200).unwrap().into_iter().filter(|e| e.actor == OW).map(|e| e.action).collect();
    for a in ["permission", "question", "restart_fresh", "annotate"] {
        assert!(who.iter().any(|w| w == a), "the Overwatch's {a}: {who:?}");
    }
    let interventions = core.rt.db.events_of_kinds_in(0, u64::MAX, &["pane_intervention"]).unwrap();
    assert!(interventions.is_empty(), "no pane intervention: {interventions:?}");
}
