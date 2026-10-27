//! NFR-2 / J10 on the real binary: `kill -9` of `orchd serve` while an agent
//! is mid-turn, then a restart. The state comes back from SQLite, the
//! reconciler resumes the agent with `--resume`, and the turn whose marker is
//! in the transcript is not delivered twice. Uses the fake claude (no quota).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use agent_runtime::server::request;
use orch_protocol::{AgentSpec, CtlRequest, bus};
use serde_json::Value;

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

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
    assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn orchd() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_orchd"))
}

fn serve(home: &Path, fake: &Path, log: &Path) -> Child {
    Command::new(orchd())
        .arg("--home")
        .arg(home)
        .args(["serve", "--no-otel", "--idle-min", "0", "--claude"])
        .arg(fake)
        .arg("--projects-root")
        .arg(home.join("fake-projects"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap())
        .spawn()
        .unwrap()
}

/// Stops the core and its agents even when an assertion fails.
struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = request(&self.0.join("core.sock"), CtlRequest::Shutdown);
        let _ = Command::new("pkill").arg("-f").arg(self.0.display().to_string()).status();
    }
}

fn agent(sock: &Path, id: &str) -> Value {
    let s = request(sock, CtlRequest::State).map(|r| r.data).unwrap_or(Value::Null);
    s["agents"].as_array().and_then(|a| a.iter().find(|a| a["id"] == id).cloned()).unwrap_or(Value::Null)
}

#[test]
fn kill_9_mid_turn_then_restart_resumes_without_double_delivery() {
    // Helpers next to orchd, then into the home's bin/.
    let st = Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "orch-hook", "-p", "orch-status", "-p", "orch-mcp"])
        .status()
        .unwrap();
    assert!(st.success());
    let root = tempfile::Builder::new().prefix("orchkr").tempdir_in("/tmp").unwrap();
    let home = root.path().join("h");
    let _guard = Guard(root.path().to_path_buf());
    let st = Command::new(orchd()).arg("--home").arg(&home).arg("install-helpers").status().unwrap();
    assert!(st.success());
    let fake = root.path().join("fake-claude");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../agent-runtime/tests/fake_claude.sh"), &fake).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = root.path().join("proj/demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);
    let sock = home.join("core.sock");

    // Run 1: an agent, a finished turn, then a slow turn in progress.
    let mut core = serve(&home, &fake, &root.path().join("serve1.log"));
    wait_for("core up", 15, || bus::is_listening(&sock));
    let spec = AgentSpec { agent_id: "a1".into(), repo: repo.display().to_string(), ..Default::default() };
    assert!(request(&sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap().ok);
    request(&sock, CtlRequest::Deliver { agent_id: "a1".into(), text: "first".into() }).unwrap();
    wait_for("first turn", 15, || agent(&sock, "a1")["turns"] == 1 && agent(&sock, "a1")["state"] == "idle");
    let slow = request(&sock, CtlRequest::Deliver { agent_id: "a1".into(), text: "SLOW work".into() }).unwrap();
    let slow_turn = slow.data["turn"].as_str().unwrap().to_string();
    wait_for("slow turn running", 15, || agent(&sock, "a1")["state"] == "working");
    let pid1 = agent(&sock, "a1")["pid"].as_i64().unwrap() as i32;
    let sid = agent(&sock, "a1")["session_id"].as_str().unwrap().to_string();

    // Crash.
    unsafe { libc::kill(core.id() as i32, libc::SIGKILL) };
    let _ = core.wait();
    // The PTY closed: the agent ends (SIGHUP) within a few seconds.
    wait_for("agent ended", 15, || unsafe { libc::kill(pid1, 0) } != 0);

    // Run 2: reconcile from SQLite, resume.
    let mut core = serve(&home, &fake, &root.path().join("serve2.log"));
    wait_for("core up again", 15, || bus::is_listening(&sock));
    // The resume message is delivered (the pull buffer is drained by each read).
    let mut screen = String::new();
    wait_for("resumed and done", 30, || {
        if let Ok(r) = request(&sock, CtlRequest::Read { agent_id: "a1".into() }) {
            screen.push_str(r.data.as_str().unwrap_or(""));
        }
        screen.contains("DONE[")
            && screen.contains("the orchestrator restarted")
            && agent(&sock, "a1")["state"] == "idle"
    });
    let a = agent(&sock, "a1");
    assert_eq!(a["session_id"], sid.as_str(), "same session, resumed by id");
    assert_ne!(a["pid"].as_i64().unwrap() as i32, pid1);
    let events = request(&sock, CtlRequest::Events { agent_id: None, kind: None, since_ms: None, limit: Some(500) })
        .unwrap()
        .data;
    let evs = events.as_array().unwrap();
    assert!(evs.iter().any(|e| e["kind"] == "reconciled"), "reconciler ran");
    let spawns: Vec<&Value> = evs.iter().filter(|e| e["kind"] == "spawned" && e["agent_id"] == "a1").collect();
    assert!(spawns[0]["detail"]["args"].to_string().contains("--resume"), "{:?}", spawns[0]);
    // The slow turn reached the transcript before the crash: delivered once only.
    let deliveries =
        evs.iter().filter(|e| e["kind"] == "delivered" && e["detail"]["turn"] == slow_turn.as_str()).count();
    assert_eq!(deliveries, 1, "turn {slow_turn} delivered {deliveries} times");
    assert!(!screen.contains("SLOW work"), "the slow turn was not pasted again: {screen}");

    assert!(request(&sock, CtlRequest::Shutdown).unwrap().ok);
    let _ = core.wait();
    // No agent process left behind.
    assert_ne!(unsafe { libc::kill(agent(&sock, "a1")["pid"].as_i64().unwrap_or(1) as i32, 0) }, 0);
}
