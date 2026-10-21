//! The agent environment, built from scratch (SPEC §6.3, FR-703, FR-1201).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Variables that must never reach an agent. The environment is built from
/// scratch, so this list only serves the tests and the parent-env warning.
pub const FORBIDDEN: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "SSH_AUTH_SOCK",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITLAB_TOKEN",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "NPM_TOKEN",
    "CLAUDE_CONFIG_DIR",
];

/// Fallback when the login shell cannot be asked.
pub const FALLBACK_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// The PATH of the user's login shell (user plugin hooks need tools like
/// `node` from nvm). Runs `$SHELL -l -i -c` with a marker so dotfile noise is
/// ignored, with a 5 s deadline. `~/.local/bin` (where `claude` lives) is
/// appended if missing.
pub fn login_path(shell: &Path, home: &Path) -> String {
    let probe = (|| {
        let mut child = Command::new(shell)
            .args(["-l", "-i", "-c", "printf '__ORCH_PATH__%s__ORCH_END__' \"$PATH\""])
            .env_clear()
            .env("HOME", home)
            .env("USER", std::env::var("USER").unwrap_or_default())
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let out = child.wait_with_output().ok()?;
        parse_marked_path(&String::from_utf8_lossy(&out.stdout))
    })();
    let mut path = probe.unwrap_or_else(|| FALLBACK_PATH.to_string());
    let local_bin = home.join(".local/bin");
    if !path.split(':').any(|p| Path::new(p) == local_bin) {
        path = format!("{}:{path}", local_bin.display());
    }
    path
}

fn parse_marked_path(out: &str) -> Option<String> {
    let start = out.find("__ORCH_PATH__")? + "__ORCH_PATH__".len();
    let end = out[start..].find("__ORCH_END__")? + start;
    let p = out[start..end].trim();
    (!p.is_empty()).then(|| p.to_string())
}

/// Warns about variables in the app's own environment that would change
/// billing or leak credentials if they reached an agent (FR-1201).
pub fn parent_env_warnings() -> Vec<String> {
    let mut w = vec![];
    if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
        w.push("ANTHROPIC_API_KEY is set in the app's environment: it is not passed to agents (it would bill the API instead of the plan)".into());
    }
    w
}

/// Inputs for [`agent_env`].
#[derive(Clone, Debug)]
pub struct EnvInputs {
    pub home: PathBuf,
    pub user: String,
    pub shell: PathBuf,
    pub lang: String,
    pub path: String,
    pub tmpdir: Option<String>,
    pub agent_id: String,
    pub socket: PathBuf,
    pub agent_dir: PathBuf,
    pub gitconfig: PathBuf,
    pub hook_deadline_ms: u64,
    pub user_statusline: Option<String>,
    /// OTLP http/json endpoint of the local receiver (`http://127.0.0.1:<port>`).
    pub otel_endpoint: Option<String>,
}

impl EnvInputs {
    /// Fills the user-level fields from the app's environment.
    pub fn from_process(path: String) -> EnvInputs {
        let home = crate::paths::home_dir();
        EnvInputs {
            user: std::env::var("USER").unwrap_or_default(),
            shell: std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| "/bin/zsh".into()),
            lang: std::env::var("LANG").ok().filter(|l| !l.is_empty()).unwrap_or_else(|| "en_US.UTF-8".into()),
            tmpdir: std::env::var("TMPDIR").ok(),
            home,
            path,
            agent_id: String::new(),
            socket: PathBuf::new(),
            agent_dir: PathBuf::new(),
            gitconfig: PathBuf::new(),
            hook_deadline_ms: orch_protocol::DEFAULT_HOOK_DEADLINE_MS,
            user_statusline: None,
            otel_endpoint: None,
        }
    }
}

/// The whole environment of a `claude` agent. Nothing is inherited.
pub fn agent_env(i: &EnvInputs) -> BTreeMap<String, String> {
    use orch_protocol::env as e;
    let mut env = BTreeMap::new();
    let mut set = |k: &str, v: String| {
        env.insert(k.to_string(), v);
    };
    set("HOME", i.home.display().to_string());
    set("USER", i.user.clone());
    set("LOGNAME", i.user.clone());
    set("SHELL", i.shell.display().to_string());
    set("LANG", i.lang.clone());
    set("TERM", "xterm-256color".into());
    set("COLORTERM", "truecolor".into());
    set("PATH", i.path.clone());
    if let Some(t) = &i.tmpdir {
        set("TMPDIR", t.clone());
    }
    set(e::AGENT_ID, i.agent_id.clone());
    set(e::SOCKET, i.socket.display().to_string());
    set(e::AGENT_DIR, i.agent_dir.display().to_string());
    set(e::HOOK_DEADLINE_MS, i.hook_deadline_ms.to_string());
    if let Some(s) = &i.user_statusline {
        set(e::USER_STATUSLINE, s.clone());
    }
    // The CLI replaced its own binary mid-session in P0.
    set("DISABLE_AUTOUPDATER", "1".into());
    // No credentials: generated global git config, no prompts.
    set("GIT_CONFIG_GLOBAL", i.gitconfig.display().to_string());
    set("GIT_TERMINAL_PROMPT", "0".into());
    if let Some(endpoint) = &i.otel_endpoint {
        set("CLAUDE_CODE_ENABLE_TELEMETRY", "1".into());
        set("OTEL_METRICS_EXPORTER", "otlp".into());
        set("OTEL_LOGS_EXPORTER", "otlp".into());
        set("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json".into());
        set("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.clone());
        set("OTEL_METRIC_EXPORT_INTERVAL", "5000".into());
        set("OTEL_LOGS_EXPORT_INTERVAL", "2000".into());
        set("OTEL_RESOURCE_ATTRIBUTES", format!("orch.agent_id={}", i.agent_id));
    }
    env
}

/// The generated global git config: the user's identity, no credential helper.
pub fn gitconfig(name: &str, email: &str) -> String {
    format!("[user]\n\tname = {name}\n\temail = {email}\n[credential]\n\thelper =\n")
}

/// The user's git identity, read (never written) from their own config.
pub fn user_git_identity() -> (String, String) {
    let out = Command::new("git")
        .args(["config", "--global", "--get-regexp", r"^user\.(name|email)$"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let get = |k: &str| {
        out.lines().filter_map(|l| l.split_once(' ')).find(|(key, _)| *key == k).map(|(_, v)| v.trim().to_string())
    };
    (
        get("user.name").unwrap_or_else(|| "Orchestrator Agent".into()),
        get("user.email").unwrap_or_else(|| "agent@orchestrator.invalid".into()),
    )
}

/// The user's own `statusLine.command` from `~/.claude/settings.json` (read
/// only: the app never writes there, FR-1206).
pub fn user_statusline_command(home: &Path) -> Option<String> {
    let s = std::fs::read_to_string(home.join(".claude/settings.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    let sl = v.get("statusLine")?;
    if sl.get("type").and_then(|t| t.as_str()).unwrap_or("command") != "command" {
        return None;
    }
    sl.get("command").and_then(|c| c.as_str()).map(str::to_string).filter(|c| !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> EnvInputs {
        EnvInputs {
            home: "/Users/u".into(),
            user: "u".into(),
            shell: "/bin/zsh".into(),
            lang: "en_US.UTF-8".into(),
            path: "/a:/b".into(),
            tmpdir: Some("/var/t/".into()),
            agent_id: "a1".into(),
            socket: "/s/core.sock".into(),
            agent_dir: "/s/agents/a1".into(),
            gitconfig: "/s/agents/a1/gitconfig".into(),
            hook_deadline_ms: 540_000,
            user_statusline: Some("~/.claude/sl.sh".into()),
            otel_endpoint: Some("http://127.0.0.1:4318".into()),
        }
    }

    #[test]
    fn env_is_from_scratch_and_complete() {
        // Even with secrets in the app's own env, nothing leaks: nothing is inherited.
        let env = agent_env(&inputs());
        for k in FORBIDDEN {
            assert!(!env.contains_key(*k), "{k}");
        }
        assert_eq!(env["DISABLE_AUTOUPDATER"], "1");
        assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
        assert_eq!(env["GIT_CONFIG_GLOBAL"], "/s/agents/a1/gitconfig");
        assert_eq!(env["PATH"], "/a:/b");
        assert_eq!(env["ORCH_AGENT_ID"], "a1");
        assert_eq!(env["ORCH_SOCK"], "/s/core.sock");
        assert_eq!(env["ORCH_USER_STATUSLINE"], "~/.claude/sl.sh");
        assert_eq!(env["OTEL_RESOURCE_ATTRIBUTES"], "orch.agent_id=a1");
        assert_eq!(env["OTEL_EXPORTER_OTLP_ENDPOINT"], "http://127.0.0.1:4318");
        assert_eq!(env["OTEL_EXPORTER_OTLP_PROTOCOL"], "http/json");
        let no_otel = agent_env(&EnvInputs { otel_endpoint: None, ..inputs() });
        assert!(!no_otel.keys().any(|k| k.starts_with("OTEL_")));
    }

    #[test]
    fn gitconfig_has_no_credential_helper() {
        let g = gitconfig("N", "e@x");
        assert!(g.contains("[credential]\n\thelper =\n"));
        assert!(g.contains("name = N"));
    }

    #[test]
    fn marked_path_parsing_ignores_noise() {
        assert_eq!(parse_marked_path("motd\n__ORCH_PATH__/x:/y__ORCH_END__\nbye").as_deref(), Some("/x:/y"));
        assert_eq!(parse_marked_path("nothing"), None);
    }

    #[test]
    fn login_path_falls_back_and_adds_local_bin() {
        let p = login_path(Path::new("/nonexistent/shell"), Path::new("/Users/u"));
        assert!(p.starts_with("/Users/u/.local/bin:"), "{p}");
        assert!(p.contains("/usr/bin"));
    }

    #[test]
    fn statusline_command_is_read_only_from_user_settings() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".claude")).unwrap();
        std::fs::write(
            d.path().join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"sl.sh"}}"#,
        )
        .unwrap();
        assert_eq!(user_statusline_command(d.path()).as_deref(), Some("sl.sh"));
        assert_eq!(user_statusline_command(Path::new("/nonexistent")), None);
    }
}
