//! `orch-status`: the statusLine command of every agent (SPEC §6.3).
//!
//! The per-agent `--settings` statusLine replaces the user's own, so this
//! forwards the JSON to the core (best effort, ≤ 1 s, never waits for an
//! answer) and then runs the user's statusline command with the same input
//! and prints its output. Without one it prints a short usage line.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use orch_protocol::bus;
use orch_protocol::{CONNECT_TIMEOUT_MS, Envelope, Msg, StatusMsg, StatusSample, now_ms};

#[derive(Clone, Debug, Default)]
pub struct StatusEnv {
    pub agent_id: Option<String>,
    pub socket: Option<PathBuf>,
    /// The user's own `statusLine.command`, run through `sh -c`.
    pub user_command: Option<String>,
}

impl StatusEnv {
    pub fn from_env() -> StatusEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        StatusEnv {
            agent_id: var(orch_protocol::env::AGENT_ID),
            socket: var(orch_protocol::env::SOCKET).map(PathBuf::from),
            user_command: var(orch_protocol::env::USER_STATUSLINE),
        }
    }
}

/// Forwards, then returns the bytes to print.
pub fn run(input: &[u8], env: &StatusEnv) -> Vec<u8> {
    let payload: serde_json::Value = serde_json::from_slice(input).unwrap_or(serde_json::Value::Null);
    let _ = forward(env, &payload);
    if let Some(cmd) = &env.user_command {
        if let Some(out) = run_user_command(cmd, input) {
            return out;
        }
    }
    default_line(&payload).into_bytes()
}

fn forward(env: &StatusEnv, payload: &serde_json::Value) -> Result<(), String> {
    let (Some(sock), Some(agent_id)) = (&env.socket, &env.agent_id) else {
        return Err("not configured".into());
    };
    let s = bus::connect(sock, Duration::from_millis(CONNECT_TIMEOUT_MS)).map_err(|e| e.tag())?;
    let msg =
        Envelope::new(Msg::Status(StatusMsg { agent_id: agent_id.clone(), ts_ms: now_ms(), payload: payload.clone() }));
    bus::write_line(&mut &s, &msg).map_err(|e| e.tag())
}

fn run_user_command(cmd: &str, input: &[u8]) -> Option<Vec<u8>> {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let out = child.wait_with_output().ok()?;
    Some(out.stdout)
}

/// `ORCH 5h 7% · 7d 83% · ctx 7%`; unknown windows show `?`.
pub fn default_line(payload: &serde_json::Value) -> String {
    let s = StatusSample::parse(payload);
    let rl = s.rate_limits.unwrap_or_default();
    let pct = |w: Option<orch_protocol::WindowSample>| w.map_or("?".to_string(), |w| format!("{}%", w.used_percentage));
    let ctx = s.context_used_pct.map_or("?".to_string(), |c| format!("{c}%"));
    format!("ORCH 5h {} · 7d {} · ctx {}", pct(rl.five_hour), pct(rl.seven_day), ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn default_line_marks_unknown_windows() {
        let v = serde_json::json!({"rate_limits":{"five_hour":{"used_percentage":7,"resets_at":1}},"context_window":{"used_percentage":12}});
        assert_eq!(default_line(&v), "ORCH 5h 7% · 7d ? · ctx 12%");
        assert_eq!(default_line(&serde_json::Value::Null), "ORCH 5h ? · 7d ? · ctx ?");
    }

    #[test]
    fn forwards_then_chains_to_the_user_command() {
        let d = tempfile::Builder::new().prefix("os").tempdir_in("/tmp").unwrap();
        let sock = d.path().join("c.sock");
        let l = bus::bind_listener(&sock).unwrap();
        let core = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let e: Envelope = bus::read_line(&mut BufReader::new(&s)).unwrap();
            e
        });
        let env =
            StatusEnv { agent_id: Some("a1".into()), socket: Some(sock), user_command: Some("tr a-z A-Z".into()) };
        let out = run(br#"{"session_id":"abc"}"#, &env);
        assert_eq!(String::from_utf8(out).unwrap(), r#"{"SESSION_ID":"ABC"}"#);
        let Msg::Status(m) = core.join().unwrap().msg else { panic!() };
        assert_eq!(m.agent_id, "a1");
        assert_eq!(m.payload["session_id"], "abc");
    }

    #[test]
    fn core_down_still_prints() {
        let env =
            StatusEnv { agent_id: Some("a".into()), socket: Some("/tmp/no-such-orch.sock".into()), user_command: None };
        assert!(String::from_utf8(run(b"{}", &env)).unwrap().starts_with("ORCH"));
    }
}
