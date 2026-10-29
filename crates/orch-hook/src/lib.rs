//! `orch-hook`: the command every hook of every agent runs (SPEC §6.3, FR-806).
//!
//! It reads the hook JSON on stdin, sends it to the core and prints the core's
//! answer. It **always answers itself**, because Claude Code fails open on hook
//! timeouts, crashes and missing hook binaries:
//! - PreToolUse and PermissionRequest wait for the core within orch-hook's own
//!   deadline and answer **deny** ("ORCH: orchestrator unavailable") if the
//!   socket is missing, refused, closes mid-hold, or misses the deadline.
//! - Every other event waits at most 1 s, exits 0 with no output when the core
//!   is unreachable, and is spooled to the agent folder for replay.
//! - It never exits 2 (that blocks and erases the user's prompt) and never 1.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orch_protocol::bus::{self, BusError};
use orch_protocol::spool::{self, SpoolRecord};
use orch_protocol::{
    CONNECT_TIMEOUT_MS, DEFAULT_HOOK_DEADLINE_MS, Envelope, HookEvent, HookMsg, HookOutput, Msg, PROTOCOL_VERSION,
    Reply, ReplyEnvelope, now_ms,
};

/// What orch-hook needs from its environment (set by the core at spawn).
#[derive(Clone, Debug, Default)]
pub struct HookEnv {
    pub agent_id: Option<String>,
    pub socket: Option<PathBuf>,
    pub agent_dir: Option<PathBuf>,
    pub deadline: Duration,
}

impl HookEnv {
    pub fn from_env() -> HookEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        HookEnv {
            agent_id: var(orch_protocol::env::AGENT_ID),
            socket: var(orch_protocol::env::SOCKET).map(PathBuf::from),
            agent_dir: var(orch_protocol::env::AGENT_DIR).map(PathBuf::from),
            deadline: Duration::from_millis(
                var(orch_protocol::env::HOOK_DEADLINE_MS)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_HOOK_DEADLINE_MS),
            ),
        }
    }
}

/// What the process does: print `stdout` (if any), then exit with `exit`.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub stdout: Option<String>,
    pub exit: i32,
    /// Set when the core could not answer (for tests and debugging).
    pub fallback: Option<String>,
}

/// Non-tool events never wait longer than this for the reply.
const NON_TOOL_REPLY_DEADLINE: Duration = Duration::from_millis(1_000);

pub fn run(input: &[u8], env: &HookEnv) -> Outcome {
    let started = Instant::now();
    let ts_ms = now_ms();
    let payload: serde_json::Value =
        serde_json::from_slice(input).unwrap_or_else(|_| serde_json::json!({ "_raw": String::from_utf8_lossy(input) }));
    let event = HookEvent::from(payload.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or("?").to_string());
    let waits = event.waits_for_core();
    let reply_deadline = if waits { env.deadline } else { NON_TOOL_REPLY_DEADLINE };

    let result = ask_core(env, &event, &payload, ts_ms, waits, reply_deadline, started);
    match result {
        Ok(output) => Outcome { stdout: output.map(|o| o.to_json_string()), exit: 0, fallback: None },
        Err(why) => {
            if let (Some(dir), Some(agent)) = (&env.agent_dir, &env.agent_id) {
                let rec = SpoolRecord {
                    v: PROTOCOL_VERSION,
                    ts_ms,
                    agent_id: agent.clone(),
                    event: event.clone(),
                    why: why.clone(),
                    payload,
                };
                // Best effort: a hook must never fail because of its spool.
                let _ = spool::append(dir, &rec);
            }
            Outcome {
                stdout: HookOutput::fail_closed(&event).map(|o| o.to_json_string()),
                exit: 0,
                fallback: Some(why),
            }
        }
    }
}

fn ask_core(
    env: &HookEnv,
    event: &HookEvent,
    payload: &serde_json::Value,
    ts_ms: u64,
    waits: bool,
    reply_deadline: Duration,
    started: Instant,
) -> Result<Option<HookOutput>, String> {
    let socket: &Path = env.socket.as_deref().ok_or("ORCH_SOCK not set")?;
    let agent_id = env.agent_id.clone().ok_or("ORCH_AGENT_ID not set")?;
    let stream =
        bus::connect(socket, Duration::from_millis(CONNECT_TIMEOUT_MS).min(reply_deadline)).map_err(|e| e.tag())?;
    let msg = Envelope::new(Msg::Hook(HookMsg {
        agent_id,
        event: event.clone(),
        ts_ms,
        deadline_ms: if waits { reply_deadline.as_millis() as u64 } else { 0 },
        payload: payload.clone(),
    }));
    bus::write_line(&mut &stream, &msg).map_err(|e| e.tag())?;
    let left = reply_deadline.saturating_sub(started.elapsed());
    if left.is_zero() {
        return Err(BusError::Timeout.tag());
    }
    stream.set_read_timeout(Some(left)).map_err(|e| e.to_string())?;
    let reply: ReplyEnvelope = bus::read_line(&mut BufReader::new(&stream)).map_err(|e| e.tag())?;
    if reply.v != PROTOCOL_VERSION {
        return Err(format!("protocol version {}", reply.v));
    }
    match reply.reply {
        Reply::Hook { output } => Ok(output),
        Reply::Ack => Ok(None),
        Reply::Error { message } => Err(format!("core error: {message}")),
        Reply::Ctl(_) | Reply::Mcp { .. } => Err("unexpected reply".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_protocol::bus::bind_listener;
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixListener;

    fn short_dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("oh").tempdir_in(orch_protocol::test_tmp()).unwrap()
    }

    fn env(dir: &Path, deadline_ms: u64) -> HookEnv {
        HookEnv {
            agent_id: Some("a1".into()),
            socket: Some(dir.join("core.sock")),
            agent_dir: Some(dir.to_path_buf()),
            deadline: Duration::from_millis(deadline_ms),
        }
    }

    fn payload(event: &str) -> Vec<u8> {
        serde_json::json!({"hook_event_name": event, "session_id": "s", "tool_name": "Bash"}).to_string().into_bytes()
    }

    fn spooled(dir: &Path) -> Vec<SpoolRecord> {
        spool::read(&spool::spool_path(dir)).unwrap().0
    }

    fn assert_denied(o: &Outcome) {
        assert_eq!(o.exit, 0);
        let v: serde_json::Value = serde_json::from_str(o.stdout.as_ref().expect("deny printed")).unwrap();
        let hso = &v["hookSpecificOutput"];
        let deny = hso["permissionDecision"] == "deny" || hso["decision"]["behavior"] == "deny";
        assert!(deny, "{v}");
    }

    #[test]
    fn core_absent_tools_deny_others_exit_0_and_spool() {
        let d = short_dir();
        let e = env(d.path(), 5_000);
        for ev in ["PreToolUse", "PermissionRequest"] {
            let o = run(&payload(ev), &e);
            assert_denied(&o);
            assert_eq!(o.fallback.as_deref(), Some("ENOENT"));
        }
        for ev in HookEvent::REGISTERED.iter().filter(|e| !e.waits_for_core()) {
            let o = run(&payload(ev.as_str()), &e);
            assert_eq!(o, Outcome { stdout: None, exit: 0, fallback: Some("ENOENT".into()) }, "{ev}");
        }
        let recs = spooled(d.path());
        assert_eq!(recs.len(), HookEvent::REGISTERED.len());
        assert!(recs.iter().all(|r| r.agent_id == "a1" && r.why == "ENOENT"));
    }

    #[test]
    fn stale_socket_is_refused_at_once() {
        let d = short_dir();
        drop(UnixListener::bind(d.path().join("core.sock")).unwrap());
        let t = Instant::now();
        let o = run(&payload("PreToolUse"), &env(d.path(), 5_000));
        assert_denied(&o);
        assert_eq!(o.fallback.as_deref(), Some("ECONNREFUSED"));
        assert!(t.elapsed() < Duration::from_millis(500));
        let o = run(&payload("UserPromptSubmit"), &env(d.path(), 5_000));
        assert_eq!(o.stdout, None);
        assert_eq!(o.exit, 0);
    }

    #[test]
    fn missing_env_fails_closed() {
        let o = run(&payload("PreToolUse"), &HookEnv { deadline: Duration::from_secs(1), ..Default::default() });
        assert_denied(&o);
        let o = run(&payload("Stop"), &HookEnv::default());
        assert_eq!((o.stdout, o.exit), (None, 0));
    }

    /// A core that reads the request then does `f` with the connection.
    fn fake_core(dir: &Path, n: usize, f: fn(std::os::unix::net::UnixStream)) -> std::thread::JoinHandle<()> {
        let l = bind_listener(&dir.join("core.sock")).unwrap();
        std::thread::spawn(move || {
            for _ in 0..n {
                let (s, _) = l.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&s).read_line(&mut line).unwrap();
                f(s);
            }
        })
    }

    #[test]
    fn eof_mid_hold_denies() {
        let d = short_dir();
        let core = fake_core(d.path(), 2, |s| {
            std::thread::sleep(Duration::from_millis(100));
            drop(s); // the core dies while holding the hook
        });
        let o = run(&payload("PreToolUse"), &env(d.path(), 5_000));
        assert_denied(&o);
        assert_eq!(o.fallback.as_deref(), Some("core closed the connection"));
        let o = run(&payload("PermissionRequest"), &env(d.path(), 5_000));
        assert_denied(&o);
        core.join().unwrap();
        assert_eq!(spooled(d.path()).len(), 2);
    }

    #[test]
    fn deadline_denies_before_the_hook_timeout() {
        let d = short_dir();
        let core = fake_core(d.path(), 2, |s| {
            std::thread::sleep(Duration::from_millis(1_500)); // hung core
            drop(s);
        });
        let t = Instant::now();
        let o = run(&payload("PreToolUse"), &env(d.path(), 300));
        assert_denied(&o);
        assert_eq!(o.fallback.as_deref(), Some("deadline"));
        assert!(t.elapsed() < Duration::from_millis(1_000), "{:?}", t.elapsed());
        // A non-tool event never waits more than ~1 s on a hung core.
        let t = Instant::now();
        let o = run(&payload("UserPromptSubmit"), &env(d.path(), 300_000));
        assert_eq!((o.stdout.clone(), o.exit), (None, 0));
        assert!(t.elapsed() < Duration::from_millis(1_400), "{:?}", t.elapsed());
        core.join().unwrap();
    }

    #[test]
    fn prints_the_core_answer() {
        let d = short_dir();
        let core = fake_core(d.path(), 2, |mut s| {
            let r = serde_json::json!({"v": 1, "reply": "hook", "output":
                {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "ORCH: paused by governor"}}});
            writeln!(s, "{r}").unwrap();
        });
        let o = run(&payload("PreToolUse"), &env(d.path(), 5_000));
        assert!(o.stdout.unwrap().contains("paused by governor"));
        assert_eq!(o.fallback, None);
        // An answer with no output prints nothing (the CLI then decides as usual).
        drop(core);
        let d2 = short_dir();
        let core2 = fake_core(d2.path(), 1, |mut s| {
            writeln!(s, r#"{{"v":1,"reply":"hook","output":null}}"#).unwrap();
        });
        let o = run(&payload("PreToolUse"), &env(d2.path(), 5_000));
        assert_eq!(o, Outcome { stdout: None, exit: 0, fallback: None }, "{o:?}");
        core2.join().unwrap();
    }

    #[test]
    fn version_mismatch_falls_back() {
        let d = short_dir();
        let core = fake_core(d.path(), 1, |mut s| {
            writeln!(s, r#"{{"v":99,"reply":"ack"}}"#).unwrap();
        });
        assert_denied(&run(&payload("PreToolUse"), &env(d.path(), 5_000)));
        core.join().unwrap();
    }

    #[test]
    fn garbage_input_never_blocks() {
        let d = short_dir();
        let o = run(b"not json", &env(d.path(), 1_000));
        assert_eq!((o.stdout, o.exit), (None, 0));
    }
}
