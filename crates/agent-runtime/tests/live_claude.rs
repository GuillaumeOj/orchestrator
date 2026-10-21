//! Opt-in: starts a real `claude --model haiku` (uses plan quota).
//!
//! ```bash
//! ORCH_LIVE_CLAUDE=1 cargo test -p agent-runtime --test live_claude -- --ignored --nocapture
//! ```
//!
//! - `ORCH_LIVE_REPO=<path>`: a git repo you already trusted in Claude Code.
//!   Default: a scratch repo at `/tmp/orch-live/demo` (stable, so trust is asked once).
//! - The workspace trust dialog is never answered unless `ORCH_LIVE_ANSWER_TRUST=1`
//!   (then the test answers it the way the user would, and Claude Code records it).
//! - After the first trivial response the weekly window is checked: above 90 % the
//!   test stops there.

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
use serde_json::Value;

mod common;
use common::{agent, helper_bins};

/// git that tolerates failure (cleanup of a branch that may not exist).
fn git_try(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
}

/// Writes raw keys to the agent's PTY through an attach connection.
fn type_keys(sock: &Path, id: &str, keys: &[u8]) {
    let (s, _) = server::attach_client(sock, id, 50, 160).unwrap();
    (&s).write_all(keys).unwrap();
    std::thread::sleep(Duration::from_millis(300));
}

fn wait_turns(sock: &Path, id: &str, n: u64, secs: u64, answer_trust: bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut answered = false;
    while Instant::now() < end {
        let a = agent(sock, id);
        if a["trust_dialog"] == true && !answered {
            assert!(
                answer_trust,
                "Claude Code asks to trust the repo: trust it once yourself, or set ORCH_LIVE_ANSWER_TRUST=1"
            );
            let screen = request(sock, CtlRequest::Read { agent_id: id.into() }).map(|r| r.data).unwrap_or_default();
            eprintln!("trust dialog:\n{}", agent_runtime::pty::strip_ansi(screen.as_str().unwrap_or("")));
            // Answered as the user would. The default is "No, exit": move down to
            // "Yes, I trust this folder", then Enter.
            type_keys(sock, id, b"\x1b[B");
            type_keys(sock, id, b"\r");
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
fn live_haiku_end_to_end() {
    if std::env::var("ORCH_LIVE_CLAUDE").as_deref() != Ok("1") {
        eprintln!("skipped: set ORCH_LIVE_CLAUDE=1");
        return;
    }
    let answer_trust = std::env::var("ORCH_LIVE_ANSWER_TRUST").as_deref() == Ok("1");
    let repo = match std::env::var("ORCH_LIVE_REPO") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let r = PathBuf::from("/tmp/orch-live/demo");
            if !r.join(".git").exists() {
                std::fs::create_dir_all(&r).unwrap();
                git_try(&r, &["init", "-q", "-b", "main"]);
                std::fs::write(r.join("README.md"), "# live demo\nsecret-main-checkout\n").unwrap();
                git_try(&r, &["add", "."]);
                git_try(&r, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-qm", "init"]);
            }
            std::fs::canonicalize(&r).unwrap()
        }
    };
    let root = tempfile::Builder::new().prefix("orchlive").tempdir_in("/tmp").unwrap();
    let paths = AppPaths::new(root.path().join("app"));
    agent_runtime::preflight::install_helpers(&helper_bins(), &paths.bin_dir()).unwrap();
    let mut cfg = RuntimeConfig::new(paths.clone());
    if let Ok(c) = std::env::var("ORCH_LIVE_CLAUDE_BIN") {
        cfg.claude = PathBuf::from(c);
    }
    let rt = Runtime::new(cfg).unwrap();
    for w in &rt.warnings {
        eprintln!("warning: {w}");
    }
    let listener = bus::bind_listener(&paths.socket()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let srv = {
        let (rt, stop) = (rt, stop.clone());
        std::thread::spawn(move || server::serve(rt, listener, stop).unwrap())
    };
    let sock = paths.socket();
    let id = format!("live{}", std::process::id() % 10000);
    // Always stop the agent and remove its worktree, even when an assert fails.
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
                git_try(&self.repo, &["worktree", "unlock", &wt.display().to_string()]);
                git_try(&self.repo, &["worktree", "remove", "--force", &wt.display().to_string()]);
                git_try(&self.repo, &["branch", "-D", &format!("orch/solo/{}", self.id)]);
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
                // Inside the sandbox, Bash runs without a prompt (as in the recovery harness).
                allow_rules: vec!["Bash".into()],
                first_message: Some("Reply with just the word PONG. Do not use any tools.".into()),
                ..Default::default()
            }),
        },
    )
    .unwrap();
    assert!(r.ok, "{r:?}");
    let wt = PathBuf::from(r.data["worktree"].as_str().unwrap());
    guard.wt = Some(wt.clone());
    let ok = wait_turns(&sock, &id, 1, 120, answer_trust);
    if !ok {
        let screen = request(&sock, CtlRequest::Read { agent_id: id.clone() }).map(|r| r.data).unwrap_or_default();
        panic!(
            "first turn did not finish: {}\n{}",
            agent(&sock, &id),
            agent_runtime::pty::strip_ansi(screen.as_str().unwrap_or(""))
        );
    }

    // Quota guard: the weekly window from the first response.
    // The statusline refreshes every 5 s; the first response's sample may lag the Stop.
    let end = Instant::now() + Duration::from_secs(20);
    let mut u = request(&sock, CtlRequest::Usage).unwrap().data;
    while u["weekly"].is_null() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
        u = request(&sock, CtlRequest::Usage).unwrap().data;
    }
    let weekly = u["weekly"]["used_pct"].as_f64();
    eprintln!("statusline: 5h {:?} · weekly {:?}", u["five_hour"]["used_pct"].as_f64(), weekly);
    assert!(weekly.is_some(), "no weekly reading after the first response: {u}");
    let idle_mb = agent(&sock, &id)["memory_mb"].as_f64();
    if weekly.unwrap() > 90.0 {
        eprintln!("weekly window above 90 %: stopping here");
        return;
    }

    // A tool turn: Bash inside the sandbox, a commit in the worktree (git common dir writable),
    // and a read of the main checkout that must fail.
    let main_readme = repo.join("README.md");
    let msg = format!(
        "Use the Bash tool once to run exactly this command, then reply DONE: \
         echo orch-live > live.txt; (git add live.txt && git -c user.name=A -c user.email=a@x commit -qm live) > git.txt 2>&1; \
         (cat {} > denied.txt 2>&1; echo rc=$? >> denied.txt)",
        main_readme.display()
    );
    request(&sock, CtlRequest::Deliver { agent_id: id.clone(), text: msg }).unwrap();
    // Sample memory while it works.
    let mut working_mb: Option<f64> = None;
    let end = Instant::now() + Duration::from_secs(180);
    let mut done = false;
    while Instant::now() < end {
        let a = agent(&sock, &id);
        if a["state"] == "working" {
            if let Some(m) = a["memory_mb"].as_f64() {
                working_mb = Some(working_mb.map_or(m, |w: f64| w.max(m)));
            }
        }
        if a["turns"].as_u64() >= Some(2) && a["state"] == "idle" {
            done = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(done, "tool turn did not finish: {}", agent(&sock, &id));
    let committed = git_try(&wt, &["log", "--oneline", "-1"]);
    let denied = std::fs::read_to_string(wt.join("denied.txt")).unwrap_or_default();
    let git_out = std::fs::read_to_string(wt.join("git.txt")).unwrap_or_default();
    eprintln!("commit: {committed}git: {git_out}\ndenied.txt: {denied}");
    eprintln!(
        "settings: {}",
        std::fs::read_to_string(paths.agent_dir(&id).join("settings.json"))
            .map(|s| {
                let v: Value = serde_json::from_str(&s).unwrap();
                v["sandbox"].to_string()
            })
            .unwrap_or_default()
    );

    // Suspend, then resume by session id.
    let sid = agent(&sock, &id)["session_id"].clone();
    assert!(request(&sock, CtlRequest::Suspend { agent_id: id.clone() }).unwrap().ok);
    assert_eq!(agent(&sock, &id)["state"], "suspended");
    assert!(
        request(
            &sock,
            CtlRequest::Resume { agent_id: id.clone(), message: Some("Reply with just the word AGAIN.".into()) }
        )
        .unwrap()
        .ok
    );
    let resumed = wait_turns(&sock, &id, 3, 120, false);
    let after_mb = agent(&sock, &id)["memory_mb"].as_f64();
    assert_eq!(agent(&sock, &id)["session_id"], sid);
    let g = request(&sock, CtlRequest::Governor).unwrap().data;
    eprintln!(
        "memory (phys_footprint MB): idle {idle_mb:?}, working {working_mb:?}, after resume {after_mb:?}\ngovernor decisions: {}",
        g["decisions"].as_array().map_or(0, |d| d.len())
    );
    drop(guard);
    assert!(resumed, "resumed turn did not finish");
    // Known gap (README): with Claude Code 2.1.302 sandboxed Bash cannot write the main
    // repo's .git from a linked worktree, so this commit fails. Report, don't fail.
    if !committed.contains("live") {
        eprintln!("KNOWN GAP: commit from the sandboxed worktree failed: {git_out}");
    }
    assert!(!denied.contains("secret-main-checkout"), "main checkout was readable: {denied}");
}
