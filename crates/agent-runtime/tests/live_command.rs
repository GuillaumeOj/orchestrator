//! Opt-in: checks with a real `claude --model haiku` (uses plan quota) that a
//! slash command the core types (ADR-010) runs as a command and is recognised
//! as the app's turn, not the user's.
//!
//! ```bash
//! ORCH_LIVE_CLAUDE=1 cargo test -p agent-runtime --test live_command -- --ignored --nocapture
//! ```
//!
//! Uses the scratch repo `/tmp/orch-live/demo` (created if missing) with a
//! project command `/orchping`. The workspace trust dialog is answered only
//! with `ORCH_LIVE_ANSWER_TRUST=1`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{self, request};
use orch_protocol::{AgentSpec, CtlRequest, bus};

mod common;
use common::{agent, helper_bins};

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
}

fn screen(sock: &Path, id: &str) -> String {
    let s = request(sock, CtlRequest::Read { agent_id: id.into() }).map(|r| r.data).unwrap_or_default();
    agent_runtime::pty::strip_ansi(s.as_str().unwrap_or(""))
}

fn wait_turns(sock: &Path, id: &str, n: u64, secs: u64, answer_trust: bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut answered = false;
    while Instant::now() < end {
        let a = agent(sock, id);
        if a["trust_dialog"] == true && !answered {
            assert!(answer_trust, "trust the repo once yourself, or set ORCH_LIVE_ANSWER_TRUST=1");
            for keys in [&b"\x1b[B"[..], b"\r"] {
                let (s, _) = server::attach_client(sock, id, 50, 160).unwrap();
                (&s).write_all(keys).unwrap();
                std::thread::sleep(Duration::from_millis(300));
            }
            answered = true;
        }
        if a["turns"].as_u64() >= Some(n) && a["state"] == "idle" {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

#[test]
#[ignore = "starts a real claude (uses plan quota): ORCH_LIVE_CLAUDE=1 … -- --ignored"]
fn a_typed_slash_command_is_the_apps_turn() {
    if std::env::var("ORCH_LIVE_CLAUDE").as_deref() != Ok("1") {
        eprintln!("skipped: set ORCH_LIVE_CLAUDE=1");
        return;
    }
    let answer_trust = std::env::var("ORCH_LIVE_ANSWER_TRUST").as_deref() == Ok("1");
    let repo = PathBuf::from("/tmp/orch-live/demo");
    if !repo.join(".git").exists() {
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "# live demo\n").unwrap();
    }
    let cmd = repo.join(".claude/commands/orchping.md");
    if !cmd.exists() {
        std::fs::create_dir_all(cmd.parent().unwrap()).unwrap();
        std::fs::write(&cmd, "Reply with just the word ORCHPONG. Do not use any tools.\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-qm", "orchping command"]);
    }
    let repo = std::fs::canonicalize(&repo).unwrap();
    let root = tempfile::Builder::new().prefix("orchcmd").tempdir_in("/tmp").unwrap();
    let paths = AppPaths::new(root.path().join("app"));
    agent_runtime::preflight::install_helpers(&helper_bins(), &paths.bin_dir()).unwrap();
    let rt = Runtime::new(RuntimeConfig::new(paths.clone())).unwrap();
    let listener = bus::bind_listener(&paths.socket()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let srv = {
        let (rt, stop) = (rt.clone(), stop.clone());
        std::thread::spawn(move || server::serve(rt, listener, stop).unwrap())
    };
    let sock = paths.socket();
    let id = format!("cmd{}", std::process::id() % 10000);
    struct Cleanup {
        sock: PathBuf,
        repo: PathBuf,
        id: String,
        wt: Option<PathBuf>,
        srv: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = request(&self.sock, CtlRequest::Kill { agent_id: self.id.clone() });
            let _ = request(&self.sock, CtlRequest::Shutdown);
            if let Some(s) = self.srv.take() {
                let _ = s.join();
            }
            if let Some(wt) = &self.wt {
                git(&self.repo, &["worktree", "unlock", &wt.display().to_string()]);
                git(&self.repo, &["worktree", "remove", "--force", &wt.display().to_string()]);
                git(&self.repo, &["branch", "-D", &format!("orch/solo/{}", self.id)]);
            }
        }
    }
    let mut guard = Cleanup { sock: sock.clone(), repo: repo.clone(), id: id.clone(), wt: None, srv: Some(srv) };
    let r = request(
        &sock,
        CtlRequest::Start {
            spec: Box::new(AgentSpec {
                agent_id: id.clone(),
                repo: repo.display().to_string(),
                model: Some("haiku".into()),
                strict_mcp: Some(true),
                first_message: Some("Reply with just the word PONG. Do not use any tools.".into()),
                ..Default::default()
            }),
        },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");
    guard.wt = Some(PathBuf::from(r.data["worktree"].as_str().unwrap()));
    assert!(wait_turns(&sock, &id, 1, 120, answer_trust), "first turn: {}\n{}", agent(&sock, &id), screen(&sock, &id));

    let tid = rt.deliver_command(&id, "/orchping").unwrap();
    let ran = wait_turns(&sock, &id, 2, 120, false);
    let out = screen(&sock, &id);
    let turn = rt.store.read(|s| s.turns.iter().find(|t| t.id == tid).cloned()).unwrap();
    let g = request(&sock, CtlRequest::Governor).unwrap().data;
    let user_turns = g["decisions"].as_array().map_or(0, |d| {
        d.iter().filter(|x| x["request"]["agent_id"] == id && x["request"]["interactive"] == true).count()
    });
    eprintln!("turn: {turn:?}\nuser-origin admissions: {user_turns}\nscreen:\n{out}");
    assert!(ran, "the command turn did not finish: {}", agent(&sock, &id));
    assert!(out.contains("ORCHPONG"), "the command ran as a command");
    assert_eq!(format!("{:?}", turn.status), "Done");
    assert_eq!(user_turns, 0, "the command was the app's turn, not the user's");
}
