//! Claude Code's latest version and `claude update`, from the app
//! (add-update-claude-code spec §4.1, api-contract §1–2). Agents still run
//! with the auto-updater off (FR-1201): updating stays the user's call.
//! Claude Code's own files are only read (FR-1206).
//!
//! [`ClaudeUpdater`] owns the [`ClaudeCheck`] (`core.claude`): an update
//! replaces it with a fresh check, so a read-only state caused by the version
//! clears without a restart.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use agent_runtime::preflight::{self, Version};
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;

use crate::LockOk;
use crate::setup::{self, ClaudeCheck, ProblemKind};

/// The latest-version check repeats this often while the app runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// `curl --max-time` of one version request.
const FETCH_SECS: u32 = 15;
/// `claude update` is killed after this.
pub const UPDATE_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// Lines of `claude update` output kept.
const LOG_LINES: usize = 40;
/// The Setup topic is touched at most this often while the update prints.
const TOUCH_EVERY: Duration = Duration::from_millis(500);

pub const ERR_RUNNING: &str = "an update is running";
pub const ERR_NOT_FOUND: &str = "Claude Code not found";

/// The native installer's release endpoint (what `claude update` reads for a
/// native install): `<base>/latest` and `<base>/stable` hold a bare version.
const RELEASES: &str = "https://downloads.claude.ai/claude-code-releases";
/// npm's dist-tags (`latest`, `stable`, `next`): what an npm install updates to.
const NPM_DIST_TAGS: &str = "https://registry.npmjs.org/-/package/@anthropic-ai/claude-code/dist-tags";

// ---------------------------------------------------------------- install, channel

/// How Claude Code was installed (shown, and hints only: `claude update`
/// itself picks the way to update).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Install {
    Native,
    Homebrew,
    Npm,
    Unknown,
}

/// From the `claude` path, symlinks resolved.
pub fn install_method(claude: &Path, home: &Path) -> Install {
    install_of(&std::fs::canonicalize(claude).unwrap_or_else(|_| claude.to_path_buf()), home)
}

fn install_of(real: &Path, home: &Path) -> Install {
    let s = real.to_string_lossy();
    if s.contains("/node_modules/@anthropic-ai/claude-code/") {
        Install::Npm
    } else if real.starts_with(home.join(".local/share/claude/versions")) {
        Install::Native
    } else if ["/opt/homebrew/", "/usr/local/Cellar/", "/usr/local/Caskroom/", "/home/linuxbrew/.linuxbrew/"]
        .iter()
        .any(|p| s.starts_with(p))
        || s.contains("/Caskroom/")
        || s.contains("/Cellar/")
    {
        Install::Homebrew
    } else {
        Install::Unknown
    }
}

/// Claude Code's release channel (`autoUpdatesChannel` in `~/.claude/settings.json`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Latest,
    Stable,
}

impl Channel {
    fn tag(self) -> &'static str {
        match self {
            Channel::Latest => "latest",
            Channel::Stable => "stable",
        }
    }
}

/// `latest` unless the file says `stable` (missing file, key or bad JSON: `latest`).
pub fn channel_in(settings: &Path) -> Channel {
    let v: Option<Value> = std::fs::read(settings).ok().and_then(|b| serde_json::from_slice(&b).ok());
    match v.as_ref().and_then(|v| v["autoUpdatesChannel"].as_str()) {
        Some("stable") => Channel::Stable,
        _ => Channel::Latest,
    }
}

// ---------------------------------------------------------------- latest version

/// The version under `channel` in npm's dist-tags JSON.
fn parse_dist_tags(json: &str, channel: Channel) -> Result<Version> {
    let v: Value = serde_json::from_str(json).context("dist-tags: not JSON")?;
    let tag = v[channel.tag()].as_str().ok_or_else(|| anyhow!("dist-tags: no {:?} tag", channel.tag()))?;
    exact_version(tag)
}

/// A bare `x.y.z` (the release endpoint's body, a dist-tag), nothing else.
fn exact_version(text: &str) -> Result<Version> {
    let t = text.trim();
    let v = Version::find(t).filter(|v| v.to_string() == t);
    v.ok_or_else(|| anyhow!("not a version: {:?}", t.chars().take(40).collect::<String>()))
}

fn curl(url: &str) -> Result<String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsS", "--max-time", &FETCH_SECS.to_string(), url]);
    let out = orch_git::git::run_with_deadline(cmd, Duration::from_secs(FETCH_SECS as u64 + 5))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.trim().trim_start_matches("curl: "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The newest version on `channel`, from the source `claude update` uses for
/// this install (the native release endpoint, or npm for an npm install),
/// the other one if it fails.
pub fn fetch_latest(install: Install, channel: Channel) -> Result<Version> {
    let native = || exact_version(&curl(&format!("{RELEASES}/{}", channel.tag()))?);
    let npm = || parse_dist_tags(&curl(NPM_DIST_TAGS)?, channel);
    let (first, second): (&dyn Fn() -> Result<Version>, &dyn Fn() -> Result<Version>) =
        if install == Install::Npm { (&npm, &native) } else { (&native, &npm) };
    first().or_else(|e| second().map_err(|_| e))
}

/// `latest` is newer than `installed` (false when either is unknown).
fn is_available(installed: Option<&str>, latest: Option<&str>) -> bool {
    match (installed.and_then(Version::find), latest.and_then(Version::find)) {
        (Some(i), Some(l)) => l > i,
        _ => false,
    }
}

// ---------------------------------------------------------------- the view

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Updated {
    pub from: String,
    pub to: String,
    pub at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ErrorFrom {
    Check,
    Update,
}

#[derive(Default)]
struct Inner {
    checking: bool,
    updating: bool,
    latest: Option<String>,
    checked_at: Option<u64>,
    error: Option<(ErrorFrom, String)>,
    started_at: Option<u64>,
    updated: Option<Updated>,
    log: VecDeque<String>,
}

/// `setup.claude` (api-contract §1).
#[derive(Serialize)]
struct View<'a> {
    #[serde(flatten)]
    check: &'a ClaudeCheck,
    install: Install,
    channel: Channel,
    supported: Supported,
    update: UpdateView<'a>,
}

#[derive(Serialize)]
struct Supported {
    min: String,
    tested_max: String,
}

#[derive(Serialize)]
struct UpdateView<'a> {
    state: &'static str,
    latest: Option<&'a str>,
    available: bool,
    checked_at: Option<u64>,
    error: Option<&'a str>,
    error_from: Option<ErrorFrom>,
    started_at: Option<u64>,
    updated: Option<&'a Updated>,
    log: &'a VecDeque<String>,
}

fn now_secs() -> u64 {
    orch_protocol::now_ms() / 1000
}

// ---------------------------------------------------------------- the updater

type Fetch = Box<dyn Fn(Install, Channel) -> Result<Version> + Send + Sync>;

/// The Claude Code check, the latest version, and the one `claude update` at a time.
pub struct ClaudeUpdater {
    check: Mutex<Option<ClaudeCheck>>,
    inner: Mutex<Inner>,
    /// Notified when a check or an update ends.
    settled: Condvar,
    /// `~/.claude/settings.json` lives here; `claude update` runs here.
    home: PathBuf,
    touch: Box<dyn Fn() + Send + Sync>,
    fetch: Fetch,
    /// PATH of `claude update` (the login shell's).
    path: Box<dyn Fn() -> String + Send + Sync>,
    deadline: Duration,
}

impl ClaudeUpdater {
    /// For the app: the network, the user's login-shell PATH. `touch` touches the Setup topic.
    pub fn new(touch: impl Fn() + Send + Sync + 'static) -> ClaudeUpdater {
        let home = agent_runtime::paths::home_dir();
        let h = home.clone();
        let path = move || {
            let shell = std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| "/bin/zsh".into());
            agent_runtime::env::login_path(&shell, &h)
        };
        ClaudeUpdater::with(home, Box::new(touch), Box::new(fetch_latest), Box::new(path), UPDATE_DEADLINE)
    }

    fn with(
        home: PathBuf,
        touch: Box<dyn Fn() + Send + Sync>,
        fetch: Fetch,
        path: Box<dyn Fn() -> String + Send + Sync>,
        deadline: Duration,
    ) -> ClaudeUpdater {
        ClaudeUpdater {
            check: Mutex::new(None),
            inner: Mutex::new(Inner::default()),
            settled: Condvar::new(),
            home,
            touch,
            fetch,
            path,
            deadline,
        }
    }

    /// The Claude Code check, once one has run.
    pub fn check(&self) -> Option<ClaudeCheck> {
        self.check.lock_ok().clone()
    }

    /// Touches the Setup topic.
    pub fn touch(&self) {
        (self.touch)();
    }

    pub fn set_check(&self, c: ClaudeCheck) {
        *self.check.lock_ok() = Some(c);
    }

    /// `setup.claude`: null until the start-up check landed.
    pub fn view(&self) -> Value {
        let Some(check) = self.check() else { return Value::Null };
        let i = self.inner.lock_ok();
        let state = if i.updating {
            "updating"
        } else if i.checking {
            "checking"
        } else if i.error.is_some() {
            "error"
        } else {
            "idle"
        };
        let r = preflight::SUPPORTED_CLAUDE;
        let v = View {
            check: &check,
            install: install_method(&check.path, &self.home),
            channel: self.channel(),
            supported: Supported { min: r.min.to_string(), tested_max: r.tested_max.to_string() },
            update: UpdateView {
                state,
                latest: i.latest.as_deref(),
                available: is_available(check.version.as_deref(), i.latest.as_deref()),
                checked_at: i.checked_at,
                error: i.error.as_ref().map(|(_, e)| e.as_str()),
                error_from: i.error.as_ref().map(|(f, _)| *f),
                started_at: i.started_at,
                updated: i.updated.as_ref(),
                log: &i.log,
            },
        };
        serde_json::to_value(v).unwrap_or(Value::Null)
    }

    fn channel(&self) -> Channel {
        channel_in(&self.home.join(".claude/settings.json"))
    }

    /// The `claude` to update; refused when it was not found (or not checked yet).
    fn claude_path(&self) -> Result<(PathBuf, Install)> {
        match self.check() {
            Some(c) if c.problem_kind != Some(ProblemKind::NotFound) => {
                let install = install_method(&c.path, &self.home);
                Ok((c.path, install))
            }
            _ => bail!(ERR_NOT_FOUND),
        }
    }

    /// Asks for the latest version now (`claude_update_check`); a check
    /// already running is waited for instead. Refused while an update runs.
    pub fn check_now(&self) -> Result<()> {
        let (_, install) = self.claude_path()?;
        let mut i = self.inner.lock_ok();
        if i.updating {
            bail!(ERR_RUNNING);
        }
        if i.checking {
            let _g = self.settled.wait_while(i, |i| i.checking).unwrap_or_else(|e| e.into_inner());
            return Ok(());
        }
        i.checking = true;
        i.error = None;
        drop(i);
        (self.touch)();
        self.fetch_and_store(install, false);
        Ok(())
    }

    /// The check proper (`checking` already set). After an update, a failed
    /// check does not replace the update's error.
    fn fetch_and_store(&self, install: Install, keep_error: bool) {
        let res = (self.fetch)(install, self.channel());
        let mut i = self.inner.lock_ok();
        match res {
            Ok(v) => {
                i.latest = Some(v.to_string());
                i.checked_at = Some(now_secs());
            }
            Err(e) if !(keep_error && i.error.is_some()) => {
                i.error = Some((ErrorFrom::Check, format!("could not check the latest version: {e:#}")))
            }
            Err(_) => {}
        }
        i.checking = false;
        drop(i);
        self.settled.notify_all();
        (self.touch)();
    }

    /// The check at start, then every [`CHECK_EVERY`] (skipped while an
    /// update runs, or with no Claude Code). Runs on the caller's thread.
    pub fn check_periodically(&self, stop: &std::sync::atomic::AtomicBool) {
        use std::sync::atomic::Ordering;
        while !stop.load(Ordering::SeqCst) {
            if let Err(e) = self.check_now()
                && e.to_string() != ERR_NOT_FOUND
                && e.to_string() != ERR_RUNNING
            {
                eprintln!("orchestrator: claude update check: {e:#}");
            }
            let next = Instant::now() + CHECK_EVERY;
            let mut i = self.inner.lock_ok();
            while Instant::now() < next && !stop.load(Ordering::SeqCst) {
                let left = next.saturating_duration_since(Instant::now()).min(Duration::from_secs(60));
                i = self.settled.wait_timeout(i, left).unwrap_or_else(|e| e.into_inner()).0;
            }
        }
    }

    /// Starts `claude update` (`claude_update_run`) on its own thread; waits
    /// for a running check first. Progress and the outcome land in [`view`](Self::view).
    pub fn start_update(self: &Arc<Self>) -> Result<()> {
        let (claude, install) = self.claude_path()?;
        let mut i = self.inner.lock_ok();
        if i.updating {
            bail!(ERR_RUNNING);
        }
        i = self.settled.wait_while(i, |i| i.checking).unwrap_or_else(|e| e.into_inner());
        if i.updating {
            bail!(ERR_RUNNING);
        }
        i.updating = true;
        i.started_at = Some(now_secs());
        i.error = None;
        i.log.clear();
        drop(i);
        (self.touch)();
        let me = self.clone();
        std::thread::spawn(move || me.run_update(&claude, install));
        Ok(())
    }

    fn run_update(&self, claude: &Path, install: Install) {
        let before = self.check().and_then(|c| c.version);
        let res = self.run_claude_update(claude);
        // A fresh check: the new version, untested or not, the login.
        let after = setup::check_claude(claude);
        let now = after.version.clone();
        self.set_check(after);
        let mut i = self.inner.lock_ok();
        let last = i.log.back().cloned();
        let with_last = |s: String| match &last {
            Some(l) => format!("{s}: {l}"),
            None => s,
        };
        match res {
            Ok(st) if st.success() => match (&before, &now) {
                (Some(from), Some(to)) if from != to => {
                    i.updated = Some(Updated { from: from.clone(), to: to.clone(), at: now_secs() })
                }
                _ => {
                    let v = now.as_deref().unwrap_or("not found");
                    i.error = Some((ErrorFrom::Update, format!("claude update finished but Claude Code is still {v}")))
                }
            },
            Ok(st) => {
                let how = st.code().map_or_else(|| "was killed".to_string(), |c| format!("exited with {c}"));
                i.error = Some((ErrorFrom::Update, with_last(format!("claude update {how}"))));
            }
            Err(e) => i.error = Some((ErrorFrom::Update, with_last(format!("claude update: {e:#}")))),
        }
        i.updating = false;
        i.started_at = None;
        // The latest version again, with `checking` set before `updating` is
        // cleared so no other check or update slips in between.
        i.checking = true;
        drop(i);
        self.settled.notify_all();
        (self.touch)();
        self.fetch_and_store(install, true);
    }

    /// The environment of `claude update`: the user's, with the auto-updater
    /// left on and no API key (nothing else inherited).
    fn update_env(&self) -> BTreeMap<String, String> {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
        BTreeMap::from([
            ("PATH".into(), (self.path)()),
            ("HOME".into(), self.home.display().to_string()),
            ("USER".into(), var("USER")),
            ("TERM".into(), "dumb".into()),
            ("SHELL".into(), shell),
        ])
    }

    /// `<claude> update`, output merged into the log, killed (with its
    /// process group) after the deadline.
    fn run_claude_update(&self, claude: &Path) -> Result<ExitStatus> {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new(claude)
            .arg("update")
            .env_clear()
            .envs(self.update_env())
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .with_context(|| format!("running {} update", claude.display()))?;
        let (tx, rx) = mpsc::channel::<String>();
        for pipe in [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as _),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            std::thread::spawn(move || read_lines(pipe, &tx));
        }
        drop(tx);
        let end = Instant::now() + self.deadline;
        let mut last_touch: Option<Instant> = None;
        let mut pending = false;
        let mut exited: Option<(ExitStatus, Instant)> = None;
        let mut open = true;
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => {
                    let mut i = self.inner.lock_ok();
                    i.log.push_back(line);
                    while i.log.len() > LOG_LINES {
                        i.log.pop_front();
                    }
                    pending = true;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => open = false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if pending && last_touch.is_none_or(|t| t.elapsed() >= TOUCH_EVERY) {
                pending = false;
                last_touch = Some(Instant::now());
                (self.touch)();
            }
            if exited.is_none()
                && let Some(st) = child.try_wait()?
            {
                exited = Some((st, Instant::now()));
            }
            match exited {
                // A child left behind may hold the pipes: stop reading soon after the exit.
                Some((st, at)) if !open || at.elapsed() > Duration::from_secs(2) => return Ok(st),
                _ => {}
            }
            if exited.is_none() && Instant::now() >= end {
                // SAFETY: the child leads its own process group (`process_group(0)`).
                unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
                let _ = child.kill();
                let _ = child.wait();
                bail!("timed out after {}", fmt_duration(self.deadline));
            }
        }
    }

    /// Test helper: waits until no check or update runs.
    #[cfg(test)]
    fn wait_settled(&self, timeout: Duration) {
        let i = self.inner.lock_ok();
        let (_g, t) = self.settled.wait_timeout_while(i, timeout, |i| i.checking || i.updating).unwrap();
        assert!(!t.timed_out(), "still running after {timeout:?}");
    }
}

fn fmt_duration(d: Duration) -> String {
    match d.as_secs() {
        s if s >= 60 && s % 60 == 0 => format!("{} min", s / 60),
        s if s >= 1 => format!("{s} s"),
        _ => format!("{} ms", d.as_millis()),
    }
}

/// Each line of `pipe`, ANSI stripped, blank ones dropped. A `\r`-rewritten
/// line keeps its last state (what a terminal would show).
fn read_lines(pipe: Box<dyn Read + Send>, tx: &mpsc::Sender<String>) {
    let mut r = BufReader::new(pipe);
    let mut buf = vec![];
    while matches!(r.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
        let text = String::from_utf8_lossy(&buf);
        let shown = text.trim_end_matches(['\n', '\r']).rsplit('\r').map(strip_ansi).find(|s| !s.trim().is_empty());
        if let Some(l) = shown
            && tx.send(l.trim_end().to_string()).is_err()
        {
            return;
        }
        buf.clear();
    }
}

/// Text without terminal escapes (CSI, OSC, two-byte escapes) or control characters.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.next() {
                Some('[') => {
                    // Parameters, then one final byte in @..~.
                    for c in it.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // Up to BEL or ESC \.
                    while let Some(c) = it.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && it.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn install_method_from_the_resolved_path() {
        let home = Path::new("/Users/u");
        let i = |p: &str| install_of(Path::new(p), home);
        assert_eq!(i("/Users/u/.local/share/claude/versions/2.1.317"), Install::Native);
        assert_eq!(i("/opt/homebrew/Caskroom/claude-code/2.1.317/claude"), Install::Homebrew);
        assert_eq!(i("/usr/local/Caskroom/claude-code/2.1.317/claude"), Install::Homebrew);
        assert_eq!(i("/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js"), Install::Npm);
        assert_eq!(
            i("/Users/u/.nvm/versions/node/v22/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
            Install::Npm
        );
        assert_eq!(i("/Users/other/.local/share/claude/versions/2.1.317"), Install::Unknown);
        assert_eq!(i("/usr/bin/claude"), Install::Unknown);
        // Through the symlink `~/.local/bin/claude`.
        let d = tempfile::tempdir().unwrap();
        let v = d.path().join(".local/share/claude/versions");
        std::fs::create_dir_all(&v).unwrap();
        std::fs::create_dir_all(d.path().join(".local/bin")).unwrap();
        std::fs::write(v.join("2.1.317"), "").unwrap();
        let link = d.path().join(".local/bin/claude");
        symlink(v.join("2.1.317"), &link).unwrap();
        let home = std::fs::canonicalize(d.path()).unwrap();
        assert_eq!(install_method(&link, &home), Install::Native);
    }

    #[test]
    fn channel_defaults_to_latest() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        assert_eq!(channel_in(&f), Channel::Latest, "missing file");
        std::fs::write(&f, r#"{"model": "opus"}"#).unwrap();
        assert_eq!(channel_in(&f), Channel::Latest, "missing key");
        std::fs::write(&f, "{not json").unwrap();
        assert_eq!(channel_in(&f), Channel::Latest, "bad JSON");
        std::fs::write(&f, r#"{"autoUpdatesChannel": "stable"}"#).unwrap();
        assert_eq!(channel_in(&f), Channel::Stable);
        std::fs::write(&f, r#"{"autoUpdatesChannel": "latest"}"#).unwrap();
        assert_eq!(channel_in(&f), Channel::Latest);
    }

    #[test]
    fn dist_tags_and_release_versions() {
        let j = r#"{"stable":"2.1.311","next":"2.1.319","latest":"2.1.318"}"#;
        assert_eq!(parse_dist_tags(j, Channel::Latest).unwrap(), Version(2, 1, 318));
        assert_eq!(parse_dist_tags(j, Channel::Stable).unwrap(), Version(2, 1, 311));
        assert!(parse_dist_tags(r#"{"next":"2.1.319"}"#, Channel::Latest).is_err());
        assert!(parse_dist_tags("<html>", Channel::Latest).is_err());
        assert_eq!(exact_version("2.1.318\n").unwrap(), Version(2, 1, 318));
        assert!(exact_version("<html>2.1.318</html>").is_err());
        assert!(exact_version("").is_err());
    }

    #[test]
    fn available_when_latest_is_newer() {
        assert!(is_available(Some("2.1.317"), Some("2.1.330")));
        assert!(is_available(Some("2.1.99"), Some("2.1.100")), "numeric, not text");
        assert!(!is_available(Some("2.1.330"), Some("2.1.330")));
        assert!(!is_available(Some("2.1.331"), Some("2.1.330")));
        assert!(!is_available(None, Some("2.1.330")));
        assert!(!is_available(Some("2.1.317"), None));
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[32m✔\u{1b}[0m Updated\u{1b}[K"), "✔ Updated");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}x\u{1b}]8;;u\u{1b}\\y"), "xy");
        assert_eq!(strip_ansi("a\tb\u{8}"), "a b");
    }

    /// A fake native install under `home`: `~/.local/bin/claude` → `versions/<v>`.
    /// `update` runs `body` (sh), with `$V` the versions dir and `$L` the link.
    struct Fake {
        _dir: tempfile::TempDir,
        home: PathBuf,
        link: PathBuf,
    }

    fn fake(version: &str, body: &str) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(dir.path()).unwrap();
        let versions = home.join(".local/share/claude/versions");
        std::fs::create_dir_all(&versions).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let link = home.join(".local/bin/claude");
        write_version(&versions, version, body);
        symlink(versions.join(version), &link).unwrap();
        Fake { _dir: dir, home, link }
    }

    /// One version's script: `--version`, a logged-in `auth status`, and `update`.
    fn write_version(versions: &Path, version: &str, body: &str) {
        let script = format!(
            r#"#!/bin/sh
V='{v}'; L='{l}'
case "$1" in
  --version) echo '{version} (Claude Code)' ;;
  auth) echo '{{"loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max"}}' ;;
  update) {body} ;;
esac
"#,
            v = versions.display(),
            l = versions.parent().unwrap().parent().unwrap().parent().unwrap().join("bin/claude").display(),
        );
        let p = versions.join(version);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    struct Rig {
        up: Arc<ClaudeUpdater>,
        touches: Arc<AtomicUsize>,
        fake: Fake,
    }

    fn rig(fake: Fake, latest: &'static str, deadline: Duration) -> Rig {
        let touches = Arc::new(AtomicUsize::new(0));
        let t = touches.clone();
        let up = ClaudeUpdater::with(
            fake.home.clone(),
            Box::new(move || {
                t.fetch_add(1, Ordering::SeqCst);
            }),
            Box::new(move |_, _| exact_version(latest)),
            Box::new(|| "/usr/bin:/bin".into()),
            deadline,
        );
        up.set_check(setup::check_claude(&fake.link));
        Rig { up: Arc::new(up), touches, fake }
    }

    fn update(r: &Rig) -> Value {
        r.up.start_update().unwrap();
        r.up.wait_settled(Duration::from_secs(20));
        r.up.view()
    }

    /// `update` installs 2.1.330 and swaps the link, like the native installer.
    const SWAP: &str = r#"echo 'Checking for updates...'; printf '\033[32mDownloading\033[0m\n'; echo "DAU=${DISABLE_AUTOUPDATER-unset} KEY=${ANTHROPIC_API_KEY-unset} TERM=$TERM"; sed "s/^  --version) echo '2.1.317/  --version) echo '2.1.330/" "$V/2.1.317" > "$V/2.1.330" && chmod +x "$V/2.1.330" && ln -sfn "$V/2.1.330" "$L.new" && mv -f "$L.new" "$L"; echo 'Successfully updated from 2.1.317 to 2.1.330'"#;

    #[test]
    fn update_swaps_the_version_and_rechecks() {
        let r = rig(fake("2.1.317", SWAP), "2.1.330", Duration::from_secs(20));
        r.up.check_now().unwrap();
        let v = r.up.view();
        assert_eq!(
            (v["version"].as_str(), v["install"].as_str(), v["channel"].as_str()),
            (Some("2.1.317"), Some("native"), Some("latest"))
        );
        assert_eq!((v["update"]["latest"].as_str(), v["update"]["available"].as_bool()), (Some("2.1.330"), Some(true)));
        assert_eq!(v["supported"]["min"], preflight::SUPPORTED_CLAUDE.min.to_string());
        let v = update(&r);
        let u = &v["update"];
        assert_eq!(v["version"], "2.1.330", "{v:#}");
        assert_eq!((u["state"].as_str(), u["available"].as_bool()), (Some("idle"), Some(false)), "{u:#}");
        assert_eq!((u["updated"]["from"].as_str(), u["updated"]["to"].as_str()), (Some("2.1.317"), Some("2.1.330")));
        assert!(u["error"].is_null() && u["started_at"].is_null());
        let log: Vec<&str> = u["log"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(log[1], "Downloading", "ANSI stripped: {log:?}");
        assert!(log.contains(&"DAU=unset KEY=unset TERM=dumb"), "auto-updater on, no API key: {log:?}");
        assert_eq!(log.last(), Some(&"Successfully updated from 2.1.317 to 2.1.330"));
        assert!(r.touches.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn a_too_old_version_clears_after_the_update() {
        let body = SWAP.replace("2.1.317", "2.1.200");
        let r = rig(fake("2.1.200", &body), "2.1.330", Duration::from_secs(20));
        assert_eq!(r.up.check().unwrap().problem_kind, Some(ProblemKind::Version));
        let v = update(&r);
        assert!(v["problem"].is_null() && v["problem_kind"].is_null(), "{v:#}");
        assert!(r.up.check().unwrap().ready().is_ok());
    }

    #[test]
    fn an_update_that_changes_nothing_is_an_error() {
        let r = rig(fake("2.1.317", "echo 'Claude Code is up to date (2.1.317)'"), "2.1.330", Duration::from_secs(20));
        let u = update(&r)["update"].clone();
        assert_eq!((u["state"].as_str(), u["error_from"].as_str()), (Some("error"), Some("update")));
        assert_eq!(u["error"], "claude update finished but Claude Code is still 2.1.317");
        assert_eq!(u["log"][0], "Claude Code is up to date (2.1.317)");
        assert!(u["updated"].is_null());
        // The latest check after it did not replace the update's error.
        assert_eq!(u["latest"], "2.1.330");
    }

    #[test]
    fn a_failing_update_keeps_the_version() {
        let r = rig(
            fake("2.1.317", "echo 'Downloading...'; echo 'error: EACCES' >&2; exit 1"),
            "2.1.330",
            Duration::from_secs(20),
        );
        let v = update(&r);
        assert_eq!(v["version"], "2.1.317");
        assert_eq!(v["update"]["error"], "claude update exited with 1: error: EACCES");
        assert_eq!(v["update"]["state"], "error");
        // The next check starts clean.
        r.up.check_now().unwrap();
        assert_eq!(r.up.view()["update"]["state"], "idle");
    }

    #[test]
    fn a_hung_update_is_killed_at_the_deadline() {
        let r = rig(fake("2.1.317", "echo 'Checking...'; sleep 30"), "2.1.330", Duration::from_millis(500));
        let t = Instant::now();
        let u = update(&r)["update"].clone();
        assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
        assert_eq!(u["error"], "claude update: timed out after 500 ms: Checking...");
    }

    #[test]
    fn one_update_at_a_time() {
        let r = rig(fake("2.1.317", "sleep 1"), "2.1.330", Duration::from_secs(20));
        r.up.start_update().unwrap();
        assert_eq!(r.up.view()["update"]["state"], "updating");
        assert!(r.up.view()["update"]["started_at"].is_u64());
        assert_eq!(r.up.start_update().unwrap_err().to_string(), ERR_RUNNING);
        assert_eq!(r.up.check_now().unwrap_err().to_string(), ERR_RUNNING);
        r.up.wait_settled(Duration::from_secs(20));
        let _ = &r.fake;
    }

    #[test]
    fn refused_without_claude() {
        let up = ClaudeUpdater::with(
            "/nonexistent".into(),
            Box::new(|| {}),
            Box::new(|_, _| bail!("no network in this test")),
            Box::new(String::new),
            UPDATE_DEADLINE,
        );
        let up = Arc::new(up);
        assert_eq!(up.view(), Value::Null, "null until the start-up check landed");
        assert_eq!(up.check_now().unwrap_err().to_string(), ERR_NOT_FOUND);
        up.set_check(setup::check_claude(Path::new("/no/such/claude")));
        assert_eq!(up.start_update().unwrap_err().to_string(), ERR_NOT_FOUND);
        assert_eq!(up.check_now().unwrap_err().to_string(), ERR_NOT_FOUND);
        assert_eq!(up.view()["problem_kind"], "not_found");
        assert_eq!(up.view()["install"], "unknown");
    }

    #[test]
    fn a_failed_check_keeps_the_last_latest() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let f = fake("2.1.317", "true");
        let up = ClaudeUpdater::with(
            f.home.clone(),
            Box::new(|| {}),
            Box::new(move |_, _| match c.fetch_add(1, Ordering::SeqCst) {
                0 => exact_version("2.1.330"),
                _ => bail!("Could not resolve host"),
            }),
            Box::new(String::new),
            UPDATE_DEADLINE,
        );
        up.set_check(setup::check_claude(&f.link));
        up.check_now().unwrap();
        let at = up.view()["update"]["checked_at"].clone();
        up.check_now().unwrap();
        let u = up.view()["update"].clone();
        assert_eq!((u["state"].as_str(), u["error_from"].as_str()), (Some("error"), Some("check")));
        assert_eq!(u["error"], "could not check the latest version: Could not resolve host");
        assert_eq!((&u["latest"], &u["checked_at"]), (&Value::from("2.1.330"), &at));
        assert_eq!(u["available"], true);
    }
}
