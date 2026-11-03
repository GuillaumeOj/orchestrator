//! The app updates itself from the GitHub releases CI publishes for every green
//! `main` commit (spec §3.3, api-contract §3–§4): `gh` finds and downloads the
//! newest release into `<app data>/updates/`, its minisign signature is checked
//! against the key compiled in, the bundle is extracted (staged), and it replaces
//! the running bundle on quit (CI builds) or on "Restart to update".
//!
//! One `update` thread does all the work (gh, download, verify, extract) with no
//! lock held; the state mutex is only held to read or change fields, and the
//! `update` event is emitted after it is released. The updater never touches
//! the core.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BuildInfo, LockOk};

pub(crate) const REPO: &str = "GuillaumeOj/orchestrator";
/// The public half of the update signing key: base64 of the minisign public key
/// text, exactly as `tauri signer generate` writes it (spec §4). CI signs with
/// the private half (the `ORCH_UPDATE_KEY` secret).
pub(crate) const PUB_KEY: &str = include_str!("../update.pub");

const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(30 * 60);
pub(crate) const VIEW_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub(crate) const RELEASE_FIELDS: &str = "tagName,assets,publishedAt,targetCommitish,body";
/// Written when a swap at quit failed: the next start shows it as the error, once.
const LAST_ERROR: &str = "last_error";
const DOWNLOAD: &str = "download";
/// The PR build's own download dir, so it never meets a main download. Every PR dir under
/// `updates/` starts with `pr-` (`pr-download`, `pr-<n>.partial`, `pr-<n>`): the main staging
/// code leaves them alone.
const PR_DOWNLOAD: &str = "pr-download";

fn is_pr_dir(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("pr-"))
}

// ---------------------------------------------------------------- versions

pub(crate) fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.strip_prefix('v').unwrap_or(s).split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

/// Whether release `version` is newer than the running build. A local build
/// (`make app`, channel `local`) or a PR build (channel `pr`, whose version comes from a run
/// number shared with `main`) is older than any `main` release.
fn is_newer(version: &str, current: &BuildInfo) -> bool {
    let Some(v) = parse_version(version) else { return false };
    current.channel != "main" || parse_version(&current.version).is_none_or(|c| v > c)
}

// ---------------------------------------------------------------- releases

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    target_commitish: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
}

/// The newer release, as the UI sees it (api-contract §3 `available`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Available {
    version: String,
    sha: String,
    published_at: Option<i64>,
    subject: String,
}

/// The latest release and its two assets (api-contract §1).
#[derive(Debug, Clone)]
struct Release {
    info: Available,
    tag: String,
    tarball: String,
    tarball_size: u64,
    sig: String,
}

/// Parses `gh release view --json tagName,assets,publishedAt,targetCommitish,body`.
fn parse_release(json: &str) -> Result<Release> {
    let r: GhRelease = serde_json::from_str(json).context("release JSON")?;
    let version = r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name).to_string();
    if parse_version(&version).is_none() {
        bail!("release tag {:?} is not v<major>.<minor>.<patch>", r.tag_name);
    }
    let tarball = format!("Orchestrator_{version}_aarch64.app.tar.gz");
    let sig = format!("{tarball}.sig");
    let asset = |name: &str| r.assets.iter().find(|a| a.name == name);
    let Some(tar) = asset(&tarball) else { bail!("release {} has no {tarball}", r.tag_name) };
    if asset(&sig).is_none() {
        bail!("release {} has no {sig}", r.tag_name);
    }
    let published_at = r.published_at.and_then(|p| p.parse::<jiff::Timestamp>().ok()).map(|t| t.as_second());
    Ok(Release {
        info: Available {
            version,
            sha: r.target_commitish,
            published_at,
            subject: r.body.lines().next().unwrap_or("").trim().to_string(),
        },
        tag: r.tag_name.clone(),
        tarball_size: tar.size,
        tarball,
        sig,
    })
}

// ---------------------------------------------------------------- signatures

fn decode_b64_text(b64: &str) -> Result<String> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim()).context("not base64")?;
    String::from_utf8(raw).context("not text")
}

/// The minisign public key from `update.pub`'s format (base64 of the key text).
pub(crate) fn decode_public_key(b64: &str) -> Result<minisign_verify::PublicKey> {
    minisign_verify::PublicKey::decode(&decode_b64_text(b64)?).map_err(|e| anyhow!("public key: {e}"))
}

/// Checks `file` against a Tauri `.sig` (base64 of the minisign signature text),
/// streaming the file so a 100 MB tarball never sits in memory. The signed
/// trusted comment must name this file, so an older signed tarball re-published
/// under a newer name is refused (no downgrade).
pub(crate) fn verify_file(key: &minisign_verify::PublicKey, sig_b64: &str, file: &Path) -> Result<()> {
    let sig = decode_b64_text(sig_b64)
        .ok()
        .and_then(|t| minisign_verify::Signature::decode(&t).ok())
        .ok_or_else(|| anyhow!("signature unreadable"))?;
    let mut verifier = key.verify_stream(&sig).map_err(|e| anyhow!("signature: {e}"))?;
    let mut f = std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier.finalize().map_err(|_| anyhow!("signature does not match"))?;
    let name = file.file_name().map(|n| format!("file:{}", n.to_string_lossy())).unwrap_or_default();
    if !sig.trusted_comment().split('\t').any(|f| f == name) {
        bail!("signature is for another file");
    }
    Ok(())
}

// ---------------------------------------------------------------- when it is off

/// The `.app` that `exe` runs from (`…/X.app/Contents/MacOS/exe`).
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

/// Under a cargo `target/` (a `target` dir next to a `Cargo.toml`).
fn under_target(exe: &Path) -> bool {
    exe.ancestors().any(|a| {
        a.file_name().is_some_and(|n| n == "target") && a.parent().is_some_and(|p| p.join("Cargo.toml").is_file())
    })
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Why the updater never checks nor applies (api-contract §3 `disabled_why`),
/// for the reasons that do not change while the app runs.
fn disabled_reason(debug: bool, no_update: bool, key_ok: bool, exe: &Path) -> Option<&'static str> {
    if debug {
        return Some("debug build");
    }
    if no_update {
        return Some("ORCH_NO_UPDATE");
    }
    if !key_ok {
        return Some("no update key");
    }
    if under_target(exe) {
        return Some("build in target/");
    }
    match bundle_of(exe) {
        Some(app) if writable(&app) && app.parent().is_some_and(writable) => None,
        _ => Some("bundle not writable"),
    }
}

// ---------------------------------------------------------------- swap

/// `<parent>/.<name>.<suffix>`: next to the bundle, so on the same volume.
fn sibling(bundle: &Path, suffix: &str) -> PathBuf {
    let name = bundle.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    bundle.with_file_name(format!(".{name}.{suffix}"))
}

pub(crate) fn is_app(p: &Path) -> bool {
    p.join("Contents/Info.plist").is_file() && p.join("Contents/MacOS").is_dir()
}

/// The pre-flight: moves (or, across volumes, copies) the staged bundle next to
/// `bundle` as `.<name>.update`, so the swap itself is two renames.
fn prepare(bundle: &Path, staged: &Path) -> Result<PathBuf> {
    if !is_app(staged) {
        bail!("staged update is incomplete");
    }
    let parent = bundle.parent().ok_or_else(|| anyhow!("bundle has no parent"))?;
    if !writable(parent) {
        bail!("{} is not writable", parent.display());
    }
    let dest = sibling(bundle, "update");
    remove_any(&dest)?;
    if std::fs::rename(staged, &dest).is_err() {
        let ok = Command::new("/usr/bin/ditto").arg(staged).arg(&dest).status().is_ok_and(|s| s.success());
        if !ok {
            let _ = remove_any(&dest);
            bail!("copying the update next to {} failed", bundle.display());
        }
    }
    Ok(dest)
}

/// Swaps `prepared` in for `bundle`: the current one moves aside, the new one
/// moves in, the old one is removed. Any failure moves the old one back.
fn swap(bundle: &Path, prepared: &Path) -> Result<()> {
    let old = sibling(bundle, "old");
    remove_any(&old)?;
    std::fs::rename(bundle, &old).with_context(|| format!("moving {} aside", bundle.display()))?;
    if let Err(err) = std::fs::rename(prepared, bundle) {
        std::fs::rename(&old, bundle).with_context(|| format!("putting {} back", bundle.display()))?;
        return Err(err).context("moving the update in");
    }
    if let Err(err) = std::fs::remove_dir_all(&old) {
        // Only disk space: the next start removes it.
        eprintln!("update: removing {}: {err}", old.display());
    }
    Ok(())
}

pub(crate) fn remove_any(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(_) => Ok(()),
    }
}

/// Start-up cleanup of a run that stopped half-way: the bundle's `.old` and
/// `.update` siblings, extractions and the download dir. Only at start, so a
/// `.update` prepared by this process is never removed before its swap.
fn cleanup(bundle: Option<&Path>, updates: &Path) {
    if let Some(b) = bundle {
        let _ = remove_any(&sibling(b, "old"));
        let _ = remove_any(&sibling(b, "update"));
    }
    let _ = remove_any(&updates.join(DOWNLOAD));
    let _ = remove_any(&updates.join(PR_DOWNLOAD));
    for e in std::fs::read_dir(updates).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().ends_with(".partial") {
            let _ = remove_any(&e.path());
        }
    }
}

/// What quitting does with the update.
#[derive(Debug, PartialEq, Eq)]
enum ExitAction {
    Nothing,
    /// A plain quit installs a staged update, CI builds only.
    Apply,
    /// "Restart to update": swap the prepared bundle in and relaunch.
    Restart(PathBuf),
}

fn exit_action(state: Phase, channel: &str, restart: Option<PathBuf>) -> ExitAction {
    match restart {
        Some(p) => ExitAction::Restart(p),
        None if state == Phase::Ready && channel == "main" => ExitAction::Apply,
        None => ExitAction::Nothing,
    }
}

// ---------------------------------------------------------------- gh

pub(crate) enum GhError {
    Missing,
    Offline,
    Failed(String),
}

impl GhError {
    /// The text of `error` (api-contract §3).
    pub(crate) fn msg(&self) -> String {
        match self {
            GhError::Offline => "offline".into(),
            GhError::Missing => "gh: not installed".into(),
            GhError::Failed(m) => format!("gh: {m}"),
        }
    }
}

fn offline(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    [
        "error connecting to",
        "dial tcp",
        "no such host",
        "could not resolve",
        "network is unreachable",
        "i/o timeout",
        "connection refused",
        "timed out",
    ]
    .iter()
    .any(|p| s.contains(p))
}

/// Runs `gh`, killed after `timeout` so a hung network call never wedges the thread.
pub(crate) fn gh(args: &[&str], timeout: Duration) -> Result<String, GhError> {
    let mut cmd = Command::new("gh");
    cmd.args(args).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    let out = match orch_git::git::run_with_deadline(cmd, timeout) {
        Ok(o) => o,
        Err(e) => {
            return Err(match e.downcast_ref::<std::io::Error>() {
                Some(io) if io.kind() == std::io::ErrorKind::NotFound => GhError::Missing,
                Some(io) => GhError::Failed(io.to_string()),
                // The deadline.
                None => GhError::Offline,
            });
        }
    };
    let err = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else if offline(&err) {
        Err(GhError::Offline)
    } else {
        Err(GhError::Failed(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string()))
    }
}

// ---------------------------------------------------------------- the updater

/// api-contract §3 `state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    /// Up to date, or never checked.
    Idle,
    Checking,
    Downloading,
    /// Verified and staged.
    Ready,
    Error,
}

/// A verified, extracted update (`updates/<version>/`).
struct Staged {
    app: PathBuf,
    info: Available,
}

/// The PR build being installed or staged (api-contract §3 `pr_install`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrInstall {
    pub pr: u64,
    pub title: String,
    pub version: String,
    pub sha: String,
    pub state: PrPhase,
    pub progress: Option<f64>,
    pub error: Option<String>,
}

impl PrInstall {
    /// A download that has not found its release yet.
    fn pending(pr: u64) -> Self {
        PrInstall {
            pr,
            title: String::new(),
            version: String::new(),
            sha: String::new(),
            state: PrPhase::Downloading,
            progress: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrPhase {
    Downloading,
    /// Verified and staged.
    Ready,
    Error,
}

/// What `updates/pr-<n>/release.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PrStaged {
    version: String,
    sha: String,
    pr: u64,
    title: String,
}

/// Which staged build `update_restart` swaps in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartTarget {
    Main,
    Pr,
}

/// api-contract §3, field for field.
#[derive(Debug, Clone, Serialize)]
struct UpdateState {
    current: BuildInfo,
    enabled: bool,
    disabled_why: Option<String>,
    state: Phase,
    available: Option<Available>,
    progress: Option<f64>,
    error: Option<String>,
    checked_at: Option<i64>,
    apply_on_quit: bool,
    pr_install: Option<PrInstall>,
}

struct Inner {
    s: UpdateState,
    staged: Option<Staged>,
    /// The verified PR build under `updates/pr-<n>/` (`pr_install.state = ready`).
    staged_pr: Option<PathBuf>,
    /// `update_check` asked for a check now.
    poke: bool,
    /// "Restart to update" prepared this bundle: the swap runs at exit.
    restart: Option<PathBuf>,
    /// A check is under way (set with `poke` taken, under the same lock).
    running: bool,
}

impl Inner {
    /// The staged PR bundle, when it is verified and no restart is under way.
    fn ready_pr(&self) -> Option<PathBuf> {
        match (&self.staged_pr, self.s.pr_install.as_ref().map(|p| p.state)) {
            (Some(app), Some(PrPhase::Ready)) if self.restart.is_none() => Some(app.clone()),
            _ => None,
        }
    }
}

type Sink = Arc<dyn Fn(Value) + Send + Sync>;

pub struct Updater {
    inner: Mutex<Inner>,
    wake: Condvar,
    sink: Mutex<Option<Sink>>,
    /// `<app data>/updates`.
    dir: PathBuf,
    bundle: Option<PathBuf>,
    /// Off for good (debug, ORCH_NO_UPDATE, key, bundle): no thread, no apply.
    off: bool,
}

impl Updater {
    pub fn new(current: BuildInfo, dir: PathBuf) -> Arc<Updater> {
        let exe = std::env::current_exe().unwrap_or_default();
        let key_ok = decode_public_key(PUB_KEY).is_ok();
        let no_update = std::env::var("ORCH_NO_UPDATE").is_ok_and(|v| v == "1");
        let why = disabled_reason(cfg!(debug_assertions), no_update, key_ok, &exe);
        // Unused when off (a `target/` build is off).
        let bundle = bundle_of(&exe);
        Updater::with(current, dir, bundle, why)
    }

    fn with(current: BuildInfo, dir: PathBuf, bundle: Option<PathBuf>, why: Option<&'static str>) -> Arc<Updater> {
        let mut s = UpdateState {
            current,
            enabled: why.is_none(),
            disabled_why: why.map(String::from),
            state: Phase::Idle,
            available: None,
            progress: None,
            error: None,
            checked_at: None,
            apply_on_quit: false,
            pr_install: None,
        };
        let mut staged = None;
        let mut staged_pr = None;
        if why.is_none() {
            cleanup(bundle.as_deref(), &dir);
            if let Ok(err) = std::fs::read_to_string(dir.join(LAST_ERROR)) {
                let _ = std::fs::remove_file(dir.join(LAST_ERROR));
                s.state = Phase::Error;
                s.error = Some(err.trim().to_string());
            }
            if let Some(st) = find_staged(&dir, &s.current) {
                s.available = Some(st.info.clone());
                if s.state != Phase::Error {
                    s.state = Phase::Ready;
                }
                staged = Some(st);
            }
            if let Some((app, info)) = find_staged_pr(&dir, &s.current) {
                s.pr_install = Some(PrInstall {
                    pr: info.pr,
                    title: info.title,
                    version: info.version,
                    sha: info.sha,
                    state: PrPhase::Ready,
                    progress: None,
                    error: None,
                });
                staged_pr = Some(app);
            }
        }
        Arc::new(Updater {
            inner: Mutex::new(Inner { s, staged, staged_pr, poke: false, restart: None, running: false }),
            wake: Condvar::new(),
            sink: Mutex::new(None),
            dir,
            bundle,
            off: why.is_some(),
        })
    }

    /// Emits every change to `sink` and starts the update thread (unless off).
    pub fn start(self: &Arc<Self>, sink: Sink) {
        *self.sink.lock_ok() = Some(sink);
        if self.off {
            return;
        }
        let me = self.clone();
        let _ = std::thread::Builder::new().name("update".into()).spawn(move || {
            me.wait(FIRST_CHECK);
            loop {
                let manual = {
                    let mut i = me.inner.lock_ok();
                    i.running = true;
                    std::mem::take(&mut i.poke)
                };
                me.check(manual);
                me.inner.lock_ok().running = false;
                me.wait(EVERY);
            }
        });
    }

    fn wait(&self, d: Duration) {
        let g = self.inner.lock_ok();
        let _ = self.wake.wait_timeout_while(g, d, |i| !i.poke);
    }

    /// The state object (api-contract §3).
    pub fn json(&self) -> Value {
        serde_json::to_value(self.snapshot()).unwrap_or(Value::Null)
    }

    fn snapshot(&self) -> UpdateState {
        let mut s = self.inner.lock_ok().s.clone();
        s.enabled = s.disabled_why.is_none();
        s.apply_on_quit = exit_action(s.state, &s.current.channel, None) == ExitAction::Apply;
        s
    }

    /// Changes the state, then emits it with no lock held.
    fn set(&self, f: impl FnOnce(&mut Inner)) {
        f(&mut self.inner.lock_ok());
        let sink = self.sink.lock_ok().clone();
        if let Some(sink) = sink {
            sink(self.json());
        }
    }

    /// `update_check`: refused while off; a running check is not doubled.
    pub fn check_now(&self) -> Result<Value> {
        if self.off {
            bail!("updates are off ({})", self.inner.lock_ok().s.disabled_why.clone().unwrap_or_default());
        }
        self.set(|i| {
            if !i.running {
                i.poke = true;
                if i.s.state != Phase::Ready {
                    i.s.state = Phase::Checking;
                }
            }
        });
        self.wake.notify_all();
        Ok(self.json())
    }

    /// A failed check or download. A verified update already staged stays ready
    /// (the failure is only logged): it can still be installed.
    fn fail(&self, msg: String) {
        eprintln!("update: {msg}");
        self.set(|i| {
            i.s.progress = None;
            match &i.staged {
                Some(st) => {
                    i.s.state = Phase::Ready;
                    i.s.available = Some(st.info.clone());
                    i.s.error = None;
                }
                None => {
                    i.s.state = Phase::Error;
                    i.s.error = Some(msg);
                }
            }
        });
    }

    fn check(&self, manual: bool) {
        let before = {
            let i = self.inner.lock_ok();
            if i.restart.is_some() {
                return;
            }
            i.s.state
        };
        // Back to where it was: an offline scheduled check says nothing.
        let quiet = |me: &Self| {
            me.set(|i| {
                if i.s.state == Phase::Checking {
                    i.s.state = if before == Phase::Checking { Phase::Idle } else { before };
                }
                if i.s.state != Phase::Error {
                    i.s.error = None;
                }
            })
        };
        let offline = |me: &Self| if manual { me.fail(GhError::Offline.msg()) } else { quiet(me) };
        if before != Phase::Ready {
            self.set(|i| i.s.state = Phase::Checking);
        }
        let gh_off = match gh(&["auth", "status", "--hostname", "github.com"], VIEW_TIMEOUT) {
            Ok(_) => None,
            Err(GhError::Missing) => Some("gh not installed"),
            Err(GhError::Failed(_)) => Some("gh not logged in"),
            Err(GhError::Offline) => return offline(self),
        };
        self.set(|i| i.s.disabled_why = gh_off.map(String::from));
        if gh_off.is_some() {
            return quiet(self);
        }
        let json = match gh(&["release", "view", "--repo", REPO, "--json", RELEASE_FIELDS], VIEW_TIMEOUT) {
            Ok(j) => j,
            Err(GhError::Offline) => return offline(self),
            Err(e) => return self.fail(e.msg()),
        };
        let rel = match parse_release(&json) {
            Ok(r) => r,
            Err(e) => return self.fail(format!("{e:#}")),
        };
        let now = jiff::Timestamp::now().as_second();
        let (newer, staged) = {
            let i = self.inner.lock_ok();
            // The staged bundle's own version: `available` may name a newer one whose download failed.
            let staged = i.staged.as_ref().is_some_and(|st| st.info.version == rel.info.version);
            (is_newer(&rel.info.version, &i.s.current), staged)
        };
        if !newer || staged {
            return self.set(|i| {
                i.s.state = if staged { Phase::Ready } else { Phase::Idle };
                i.s.available = staged.then(|| rel.info.clone());
                i.s.error = None;
                i.s.checked_at = Some(now);
            });
        }
        if let Err(e) = self.download(&rel) {
            let _ = remove_any(&self.dir.join(DOWNLOAD));
            self.fail(format!("{e:#}"));
        }
        self.set(|i| i.s.checked_at = Some(now));
    }

    /// Downloads, verifies and stages `rel`.
    fn download(&self, rel: &Release) -> Result<()> {
        self.set(|i| {
            i.s.state = Phase::Downloading;
            i.s.available = Some(rel.info.clone());
            i.s.progress = (rel.tarball_size > 0).then_some(0.0);
            i.s.error = None;
        });
        let partial = self.dir.join(format!("{}.partial", rel.info.version));
        let progress = |p| self.set(|i| i.s.progress = Some(p));
        self.fetch_verified(&rel.tag, &rel.tarball, &rel.sig, rel.tarball_size, DOWNLOAD, &partial, &progress)?;
        std::fs::write(partial.join("release.json"), serde_json::to_vec(&rel.info)?)?;
        let dest = self.dir.join(&rel.info.version);
        {
            let mut i = self.inner.lock_ok();
            if i.restart.is_some() {
                // A restart is under way with the previous staged build: keep that one.
                drop(i);
                let _ = remove_any(&partial);
                return Ok(());
            }
            // The previous staged build goes now; while "downloading", nothing else uses it.
            i.staged = None;
        }
        // One staged version at most (removed with no lock held).
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if e.path() != partial && !is_pr_dir(&e.path()) && e.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = remove_any(&e.path());
            }
        }
        if let Err(e) = std::fs::rename(&partial, &dest) {
            let _ = remove_any(&partial);
            return Err(e).context("staging the update");
        }
        eprintln!("update: {} verified and staged", rel.info.version);
        self.set(|i| {
            i.staged = Some(Staged { app: dest.join("Orchestrator.app"), info: rel.info.clone() });
            i.s.state = Phase::Ready;
            i.s.progress = None;
            i.s.error = None;
        });
        Ok(())
    }

    /// Downloads release `tag`'s tarball and signature into `updates/<dl>`, verifies the signature
    /// (nothing unverified is ever extracted) and extracts the bundle to `partial`, which holds
    /// `Orchestrator.app` on success. The download dir is removed whatever the outcome.
    #[allow(clippy::too_many_arguments)]
    fn fetch_verified(
        &self,
        tag: &str,
        tarball_name: &str,
        sig_name: &str,
        size: u64,
        dl: &str,
        partial: &Path,
        progress: &(impl Fn(f64) + Sync),
    ) -> Result<()> {
        let dl = self.dir.join(dl);
        remove_any(&dl)?;
        std::fs::create_dir_all(&dl)?;
        let tarball = dl.join(tarball_name);
        let done = AtomicBool::new(false);
        let res = std::thread::scope(|scope| {
            if size > 0 {
                scope.spawn(|| poll_progress(&tarball, size, &done, progress));
            }
            let res = gh(
                &[
                    "release",
                    "download",
                    tag,
                    "--repo",
                    REPO,
                    "--pattern",
                    tarball_name,
                    "--pattern",
                    sig_name,
                    "--dir",
                    &dl.to_string_lossy(),
                    "--clobber",
                ],
                DOWNLOAD_TIMEOUT,
            );
            done.store(true, Ordering::SeqCst);
            res
        });
        let verified = res.map_err(|e| anyhow!(e.msg())).and_then(|_| {
            let key = decode_public_key(PUB_KEY)?;
            let sig = std::fs::read_to_string(dl.join(sig_name)).map_err(|_| anyhow!("signature missing"))?;
            verify_file(&key, &sig, &tarball)
        });
        let extracted = verified.and_then(|()| {
            remove_any(partial)?;
            std::fs::create_dir_all(partial)?;
            let ok = Command::new("/usr/bin/tar").arg("-xzf").arg(&tarball).arg("-C").arg(partial).status()?.success();
            if !ok || !is_app(&partial.join("Orchestrator.app")) {
                bail!("extracting the update failed");
            }
            Ok(())
        });
        let _ = remove_any(&dl);
        if extracted.is_err() {
            let _ = remove_any(partial);
        }
        extracted
    }

    /// `update_restart`'s pre-flight, with the app still up: the staged bundle of `target` is
    /// moved next to the running one. Errors leave the app as it was.
    pub fn prepare_restart(&self) -> Result<()> {
        self.prepare_restart_to(RestartTarget::Main)
    }

    /// `prepare_restart` for either staged build.
    pub fn prepare_restart_to(&self, target: RestartTarget) -> Result<()> {
        let mut i = self.inner.lock_ok();
        let staged = match target {
            RestartTarget::Main => match (&i.staged, i.s.state) {
                (Some(st), Phase::Ready) if i.restart.is_none() => st.app.clone(),
                _ => bail!("no update ready"),
            },
            RestartTarget::Pr => i.ready_pr().ok_or_else(|| anyhow!("no PR build ready"))?,
        };
        let bundle = self.bundle.clone().ok_or_else(|| anyhow!("not running from an app bundle"))?;
        match prepare(&bundle, &staged) {
            Ok(p) => {
                i.restart = Some(p);
                Ok(())
            }
            Err(e) => {
                match target {
                    RestartTarget::Main => {
                        i.staged = None;
                        drop(i);
                        self.fail(format!("{e:#}"));
                    }
                    RestartTarget::Pr => {
                        drop(i);
                        self.fail_pr(format!("{e:#}"));
                    }
                }
                Err(e)
            }
        }
    }

    /// The running build.
    pub fn current(&self) -> BuildInfo {
        self.inner.lock_ok().s.current.clone()
    }

    /// Whether `update_restart` with target `pr` has a verified build to swap in.
    pub fn pr_ready(&self) -> Result<()> {
        self.inner.lock_ok().ready_pr().map(|_| ()).ok_or_else(|| anyhow!("no PR build ready"))
    }

    /// `Err` when the updater is off for good (`disabled_why` names the reason).
    pub(crate) fn require_on(&self) -> Result<()> {
        if self.off {
            bail!("updates are off ({})", self.inner.lock_ok().s.disabled_why.clone().unwrap_or_default());
        }
        Ok(())
    }

    /// `pr_builds_list` (api-contract §4). Runs gh: call it off the main thread.
    pub fn pr_builds_list(&self) -> Result<Value> {
        self.require_on()?;
        Ok(crate::pr_builds::list())
    }

    fn fail_pr(&self, msg: String) {
        eprintln!("update: PR build: {msg}");
        self.set(|i| {
            i.staged_pr = None;
            if let Some(p) = i.s.pr_install.as_mut() {
                p.state = PrPhase::Error;
                p.progress = None;
                p.error = Some(msg);
            }
        });
    }

    /// `pr_build_install`: refuses at once when it cannot start; otherwise fetches the PR's
    /// release (gh trouble ends as `pr_install.state = error`), then downloads in the background.
    pub fn install_pr(self: &Arc<Self>, number: u64) -> Result<Value> {
        self.require_on()?;
        {
            let mut i = self.inner.lock_ok();
            if i.s.current.pr == Some(number) {
                bail!("already running");
            }
            if i.s.pr_install.as_ref().is_some_and(|p| p.state == PrPhase::Downloading) {
                bail!("an install is already downloading");
            }
            if i.restart.is_some() {
                bail!("a restart is under way");
            }
            // One PR build staged at most: the new install replaces it, whatever the lookup
            // finds (state and disk are cleared together, the disk below with no lock held).
            i.staged_pr = None;
            i.s.pr_install = Some(PrInstall::pending(number));
        }
        self.set(|_| {}); // emit "downloading" now: the release lookup below can take a while
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("pr-") && name != PR_DOWNLOAD && e.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = remove_any(&e.path());
            }
        }
        let rel = match crate::pr_builds::fetch(number) {
            Ok(r) => r,
            Err(crate::pr_builds::FetchError::NoBuild) => {
                self.set(|i| i.s.pr_install = None);
                bail!("no build for PR #{number}");
            }
            Err(crate::pr_builds::FetchError::Gh(msg)) => {
                self.set(|i| {
                    i.s.pr_install =
                        Some(PrInstall { state: PrPhase::Error, error: Some(msg), ..PrInstall::pending(number) });
                });
                return Ok(self.json());
            }
        };
        self.set(|i| {
            i.s.pr_install = Some(PrInstall {
                pr: number,
                title: rel.title.clone(),
                version: rel.version.clone(),
                sha: rel.sha.clone(),
                state: PrPhase::Downloading,
                progress: (rel.tarball_size > 0).then_some(0.0),
                error: None,
            });
        });
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("pr-install".into()).spawn(move || {
            if let Err(e) = me.download_pr(number, &rel) {
                let _ = remove_any(&me.dir.join(PR_DOWNLOAD));
                me.fail_pr(format!("{e:#}"));
            }
        });
        if let Err(e) = spawned {
            self.fail_pr(format!("starting the download: {e}"));
        }
        Ok(self.json())
    }

    /// Downloads, verifies and stages a PR build (`updates/pr-<n>/`).
    fn download_pr(&self, number: u64, rel: &crate::pr_builds::PrRelease) -> Result<()> {
        let partial = self.dir.join(format!("pr-{number}.partial"));
        let progress = |p| {
            self.set(|i| {
                if let Some(pi) = i.s.pr_install.as_mut() {
                    pi.progress = Some(p);
                }
            })
        };
        self.fetch_verified(&rel.tag, &rel.tarball, &rel.sig, rel.tarball_size, PR_DOWNLOAD, &partial, &progress)?;
        let staged =
            PrStaged { version: rel.version.clone(), sha: rel.sha.clone(), pr: number, title: rel.title.clone() };
        std::fs::write(partial.join("release.json"), serde_json::to_vec(&staged)?)?;
        if self.inner.lock_ok().restart.is_some() {
            // A restart is under way with another staged build: keep that one.
            let _ = remove_any(&partial);
            return Ok(());
        }
        let dest = self.dir.join(format!("pr-{number}"));
        remove_any(&dest)?;
        if let Err(e) = std::fs::rename(&partial, &dest) {
            let _ = remove_any(&partial);
            return Err(e).context("staging the PR build");
        }
        eprintln!("update: PR #{number} build {} verified and staged", rel.version);
        self.set(|i| {
            i.staged_pr = Some(dest.join("Orchestrator.app"));
            if let Some(p) = i.s.pr_install.as_mut() {
                p.state = PrPhase::Ready;
                p.progress = None;
                p.error = None;
            }
        });
        Ok(())
    }

    /// At `RunEvent::Exit`, after the core has suspended the agents.
    pub fn on_exit(&self) {
        let (action, staged) = {
            let mut i = self.inner.lock_ok();
            let restart = i.restart.take();
            let action =
                if self.off { ExitAction::Nothing } else { exit_action(i.s.state, &i.s.current.channel, restart) };
            (action, i.staged.as_ref().map(|st| st.app.clone()))
        };
        let Some(bundle) = &self.bundle else { return };
        match action {
            ExitAction::Nothing => {}
            ExitAction::Apply => {
                let Some(staged) = staged else { return };
                match prepare(bundle, &staged).and_then(|p| swap(bundle, &p)) {
                    Ok(()) => eprintln!("update: installed at quit"),
                    Err(e) => self.write_last_error(&e),
                }
            }
            ExitAction::Restart(prepared) => {
                match swap(bundle, &prepared) {
                    Ok(()) => eprintln!("update: installed, relaunching"),
                    Err(e) => self.write_last_error(&e),
                }
                relaunch(bundle);
            }
        }
    }

    fn write_last_error(&self, e: &anyhow::Error) {
        eprintln!("update: installing failed: {e:#}");
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join(LAST_ERROR), format!("install failed: {e:#}"));
    }
}

/// Reports the partial tarball's size against the asset's while gh downloads.
fn poll_progress(file: &Path, size: u64, done: &AtomicBool, progress: &impl Fn(f64)) {
    let mut last = 0.0;
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(500));
        let got = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
        let p = (got as f64 / size as f64).min(1.0);
        if p - last >= 0.05 {
            last = p;
            progress((p * 100.0).round() / 100.0);
        }
    }
}

/// The staged update of an earlier run, if it is still newer than this build;
/// every other directory in `dir` goes (one staged version at most).
fn find_staged(dir: &Path, current: &BuildInfo) -> Option<Staged> {
    let mut found: Option<Staged> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if is_pr_dir(&p) {
            continue;
        }
        let app = p.join("Orchestrator.app");
        let info =
            std::fs::read(p.join("release.json")).ok().and_then(|b| serde_json::from_slice::<Available>(&b).ok());
        match info {
            Some(info) if found.is_none() && is_newer(&info.version, current) && is_app(&app) => {
                found = Some(Staged { app, info });
            }
            _ if e.file_type().is_ok_and(|t| t.is_dir()) => {
                let _ = remove_any(&p);
            }
            _ => {}
        }
    }
    found
}

/// The staged PR build of an earlier run (`updates/pr-<n>/`), unless it is the build now
/// running or incomplete; those directories, and any second one, go.
fn find_staged_pr(dir: &Path, current: &BuildInfo) -> Option<(PathBuf, PrStaged)> {
    let mut found: Option<(PathBuf, PrStaged)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !e.file_type().is_ok_and(|t| t.is_dir()) || name == PR_DOWNLOAD || !is_pr_dir(&p) {
            continue;
        }
        let app = p.join("Orchestrator.app");
        let info = std::fs::read(p.join("release.json")).ok().and_then(|b| serde_json::from_slice::<PrStaged>(&b).ok());
        match info {
            Some(info)
                if found.is_none()
                    && name == format!("pr-{}", info.pr)
                    && current.pr != Some(info.pr)
                    && is_app(&app) =>
            {
                found = Some((app, info));
            }
            _ => {
                let _ = remove_any(&p);
            }
        }
    }
    found
}

/// Opens `bundle` once this process has exited (it owns the core socket until then).
fn relaunch(bundle: &Path) {
    use std::os::unix::process::CommandExt;
    let res = Command::new("/bin/sh")
        .args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open -n \"$2\"", "sh"])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(e) = res {
        eprintln!("update: relaunching {}: {e}", bundle.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update");

    fn build(version: &str, channel: &str) -> BuildInfo {
        BuildInfo { version: version.into(), sha: "abc".into(), channel: channel.into(), pr: None }
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(FIX).join(name)).unwrap()
    }

    fn test_key() -> minisign_verify::PublicKey {
        decode_public_key(&fixture("test.key.pub")).unwrap()
    }

    /// A minimal `.app` with a marker file.
    fn fake_app(at: &Path, marker: &str) {
        std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
        std::fs::write(at.join("Contents/Info.plist"), "<plist/>").unwrap();
        std::fs::write(at.join("Contents/MacOS/orchestrator"), marker).unwrap();
    }

    fn marker(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/MacOS/orchestrator")).unwrap()
    }

    #[test]
    fn versions_compare_numerically() {
        let cur = build("0.1.9", "main");
        assert!(is_newer("0.1.10", &cur));
        assert!(is_newer("v0.1.10", &cur));
        assert!(!is_newer("0.1.9", &cur));
        assert!(!is_newer("0.1.8", &cur));
        assert!(!is_newer("0.1.x", &cur));
        assert!(!is_newer("0.1.10.1", &cur));
        assert!(is_newer("0.2.0", &build("0.1.99", "main")));
    }

    #[test]
    fn a_local_build_is_older_than_any_ci_build() {
        let local = build("0.1.0", "local");
        assert!(is_newer("0.1.1", &local));
        assert!(is_newer("0.1.0", &local));
    }

    #[test]
    fn a_pr_build_is_older_than_any_main_release() {
        // A PR run number can be higher than main's latest release.
        let pr = build("0.1.99", "pr");
        assert!(is_newer("0.1.50", &pr));
    }

    #[test]
    fn parses_the_release_gh_returns() {
        let r = parse_release(&fixture("release.json")).unwrap();
        assert_eq!(r.tag, "v0.1.42");
        assert_eq!(r.info.version, "0.1.42");
        assert_eq!(r.info.sha, "f9b1e20c0ffeef9b1e20c0ffeef9b1e20c0ffee0");
        assert_eq!(r.info.published_at, Some(1793527200));
        assert_eq!(r.info.subject, "Merge pull request #23 from GuillaumeOj/orch/some-feature/integration");
        assert_eq!(r.tarball, "Orchestrator_0.1.42_aarch64.app.tar.gz");
        assert_eq!(r.tarball_size, 104857600);
        assert_eq!(r.sig, "Orchestrator_0.1.42_aarch64.app.tar.gz.sig");
    }

    #[test]
    fn a_release_without_its_assets_or_with_a_bad_tag_is_refused() {
        let mut v: Value = serde_json::from_str(&fixture("release.json")).unwrap();
        v["assets"].as_array_mut().unwrap().pop();
        assert!(parse_release(&v.to_string()).unwrap_err().to_string().contains(".sig"));
        v["assets"] = serde_json::json!([]);
        assert!(parse_release(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&fixture("release.json")).unwrap();
        v["tagName"] = "nightly".into();
        assert!(parse_release(&v.to_string()).is_err());
    }

    #[test]
    fn the_committed_update_key_decodes() {
        decode_public_key(PUB_KEY).unwrap();
    }

    #[test]
    fn a_good_signature_is_accepted() {
        verify_file(&test_key(), &fixture("payload.bin.sig"), &Path::new(FIX).join("payload.bin")).unwrap();
    }

    #[test]
    fn a_tampered_file_or_bad_signature_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("payload.bin");
        std::fs::write(&file, fixture("payload.bin") + "x").unwrap();
        let err = verify_file(&test_key(), &fixture("payload.bin.sig"), &file).unwrap_err();
        assert_eq!(err.to_string(), "signature does not match");
        let good = Path::new(FIX).join("payload.bin");
        assert!(verify_file(&test_key(), "bm9wZQ==", &good).is_err());
        assert!(verify_file(&test_key(), "%%%", &good).is_err());
        // A valid signature, but for another file name.
        let renamed = tmp.path().join("Orchestrator_0.1.99_aarch64.app.tar.gz");
        std::fs::copy(&good, &renamed).unwrap();
        let err = verify_file(&test_key(), &fixture("payload.bin.sig"), &renamed).unwrap_err();
        assert_eq!(err.to_string(), "signature is for another file");
        // Signed by another key (the real one is not the test one).
        assert!(verify_file(&decode_public_key(PUB_KEY).unwrap(), &fixture("payload.bin.sig"), &good).is_err());
    }

    #[test]
    fn swap_puts_the_new_bundle_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Applications/Orchestrator.app");
        let staged = tmp.path().join("updates/0.1.42/Orchestrator.app");
        fake_app(&bundle, "old");
        fake_app(&staged, "new");
        let prepared = prepare(&bundle, &staged).unwrap();
        assert_eq!(prepared, tmp.path().join("Applications/.Orchestrator.app.update"));
        swap(&bundle, &prepared).unwrap();
        assert_eq!(marker(&bundle), "new");
        assert!(!prepared.exists());
        assert!(!tmp.path().join("Applications/.Orchestrator.app.old").exists());
    }

    #[test]
    fn a_failed_swap_puts_the_old_bundle_back() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        fake_app(&bundle, "old");
        assert!(swap(&bundle, &tmp.path().join("missing.app")).is_err());
        assert_eq!(marker(&bundle), "old");
        assert!(!tmp.path().join(".Orchestrator.app.old").exists());
    }

    #[test]
    fn an_incomplete_staged_bundle_is_never_prepared() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        fake_app(&bundle, "old");
        let staged = tmp.path().join("staged.app");
        std::fs::create_dir_all(&staged).unwrap();
        assert!(prepare(&bundle, &staged).is_err());
        assert_eq!(marker(&bundle), "old");
    }

    #[test]
    fn a_plain_quit_never_applies_a_local_build() {
        assert_eq!(exit_action(Phase::Ready, "local", None), ExitAction::Nothing);
        assert_eq!(exit_action(Phase::Ready, "main", None), ExitAction::Apply);
        assert_eq!(exit_action(Phase::Downloading, "main", None), ExitAction::Nothing);
        assert_eq!(exit_action(Phase::Error, "main", None), ExitAction::Nothing);
        let p = PathBuf::from("/x/.Orchestrator.app.update");
        assert_eq!(exit_action(Phase::Ready, "local", Some(p.clone())), ExitAction::Restart(p));
    }

    #[test]
    fn on_exit_leaves_a_local_bundle_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        let updates = tmp.path().join("updates");
        fake_app(&bundle, "old");
        fake_app(&updates.join("0.1.42/Orchestrator.app"), "new");
        let info = Available { version: "0.1.42".into(), sha: "f".into(), published_at: None, subject: "s".into() };
        std::fs::write(updates.join("0.1.42/release.json"), serde_json::to_vec(&info).unwrap()).unwrap();
        let u = Updater::with(build("0.1.0", "local"), updates.clone(), Some(bundle.clone()), None);
        let s = u.snapshot();
        assert_eq!((s.state, s.apply_on_quit), (Phase::Ready, false));
        u.on_exit();
        assert_eq!(marker(&bundle), "old");

        let u = Updater::with(build("0.1.41", "main"), updates, Some(bundle.clone()), None);
        assert!(u.snapshot().apply_on_quit);
        u.on_exit();
        assert_eq!(marker(&bundle), "new");
    }

    #[test]
    fn disabled_cases() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = |app: &Path| app.join("Contents/MacOS/orchestrator");
        let app = tmp.path().join("Applications/Orchestrator.app");
        fake_app(&app, "x");
        assert_eq!(disabled_reason(false, false, true, &exe(&app)), None);
        assert_eq!(disabled_reason(true, false, true, &exe(&app)), Some("debug build"));
        assert_eq!(disabled_reason(false, true, true, &exe(&app)), Some("ORCH_NO_UPDATE"));
        assert_eq!(disabled_reason(false, false, false, &exe(&app)), Some("no update key"));
        assert_eq!(
            disabled_reason(false, false, true, Path::new("/usr/local/bin/orchestrator")),
            Some("bundle not writable")
        );

        let repo = tmp.path().join("repo");
        let built = repo.join("target/release/bundle/macos/Orchestrator.app");
        fake_app(&built, "x");
        std::fs::write(repo.join("Cargo.toml"), "").unwrap();
        assert_eq!(disabled_reason(false, false, true, &exe(&built)), Some("build in target/"));
        assert_eq!(
            disabled_reason(false, false, true, &repo.join("target/release/orchestrator")),
            Some("build in target/")
        );

        use std::os::unix::fs::PermissionsExt;
        let parent = tmp.path().join("Applications");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = disabled_reason(false, false, true, &exe(&app));
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(r, Some("bundle not writable"));
    }

    #[test]
    fn a_disabled_updater_refuses_checks_and_never_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        fake_app(&bundle, "old");
        let u = Updater::with(
            build("0.1.41", "main"),
            tmp.path().join("updates"),
            Some(bundle.clone()),
            Some("ORCH_NO_UPDATE"),
        );
        let s = u.snapshot();
        assert!(!s.enabled);
        assert_eq!(s.disabled_why.as_deref(), Some("ORCH_NO_UPDATE"));
        assert!(u.check_now().is_err());
        assert!(u.prepare_restart().is_err());
        u.on_exit();
        assert_eq!(marker(&bundle), "old");
    }

    #[test]
    fn a_failed_check_keeps_a_staged_update_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let updates = tmp.path().join("updates");
        fake_app(&updates.join("0.1.42/Orchestrator.app"), "new");
        let info = Available { version: "0.1.42".into(), sha: "f".into(), published_at: None, subject: "s".into() };
        std::fs::write(updates.join("0.1.42/release.json"), serde_json::to_vec(&info).unwrap()).unwrap();
        let u = Updater::with(build("0.1.41", "main"), updates, None, None);
        u.fail("offline".into());
        let s = u.snapshot();
        assert_eq!((s.state, s.error, s.available), (Phase::Ready, None, Some(info)));

        let u = Updater::with(build("0.1.41", "main"), tmp.path().join("none"), None, None);
        u.fail("offline".into());
        let s = u.snapshot();
        assert_eq!((s.state, s.error.as_deref()), (Phase::Error, Some("offline")));
    }

    #[test]
    fn restart_needs_a_ready_update() {
        let tmp = tempfile::tempdir().unwrap();
        let u = Updater::with(build("0.1.41", "main"), tmp.path().join("updates"), None, None);
        assert_eq!(u.prepare_restart().unwrap_err().to_string(), "no update ready");
    }

    #[test]
    fn start_reports_the_last_install_error_once_and_cleans_leftovers() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        let updates = tmp.path().join("updates");
        fake_app(&bundle, "old");
        fake_app(&tmp.path().join(".Orchestrator.app.old"), "x");
        fake_app(&tmp.path().join(".Orchestrator.app.update"), "x");
        std::fs::create_dir_all(updates.join("download")).unwrap();
        std::fs::create_dir_all(updates.join("0.1.42.partial")).unwrap();
        std::fs::write(updates.join(LAST_ERROR), "install failed: boom\n").unwrap();
        let u = Updater::with(build("0.1.41", "main"), updates.clone(), Some(bundle.clone()), None);
        let s = u.snapshot();
        assert_eq!((s.state, s.error.as_deref()), (Phase::Error, Some("install failed: boom")));
        assert!(!updates.join(LAST_ERROR).exists());
        for gone in [".Orchestrator.app.old", ".Orchestrator.app.update", "updates/download", "updates/0.1.42.partial"]
        {
            assert!(!tmp.path().join(gone).exists(), "{gone}");
        }
        let u = Updater::with(build("0.1.41", "main"), updates, Some(bundle), None);
        assert_eq!(u.snapshot().state, Phase::Idle);
    }

    #[test]
    fn the_state_object_has_the_contract_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let u = Updater::with(build("0.1.41", "main"), tmp.path().join("updates"), None, None);
        let v = u.json();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in [
            "current",
            "enabled",
            "disabled_why",
            "state",
            "available",
            "progress",
            "error",
            "checked_at",
            "apply_on_quit",
        ] {
            assert!(keys.contains(&k), "{k}");
        }
        assert_eq!(v["current"]["channel"], "main");
        assert_eq!(v["state"], "idle");
    }

    // ------------------------------------------------------------ PR builds

    fn pr_build(pr: u64) -> BuildInfo {
        BuildInfo { version: "0.1.57".into(), sha: "abc".into(), channel: "pr".into(), pr: Some(pr) }
    }

    /// `updates/pr-<n>/` as a finished install leaves it.
    fn stage_pr(updates: &Path, pr: u64, marker: &str) {
        let dir = updates.join(format!("pr-{pr}"));
        fake_app(&dir.join("Orchestrator.app"), marker);
        let info = PrStaged { version: "0.1.61".into(), sha: "a17c3d0".into(), pr, title: "Fix the graph".into() };
        std::fs::write(dir.join("release.json"), serde_json::to_vec(&info).unwrap()).unwrap();
    }

    fn pr_state(u: &Updater) -> Option<PrInstall> {
        u.snapshot().pr_install
    }

    #[test]
    fn pr_install_and_list_are_refused_while_updates_are_off() {
        let tmp = tempfile::tempdir().unwrap();
        let u = Updater::with(build("0.1.5", "main"), tmp.path().into(), None, Some("debug build"));
        assert!(u.install_pr(12).unwrap_err().to_string().contains("updates are off (debug build)"));
        assert!(u.pr_builds_list().unwrap_err().to_string().contains("updates are off"));
        assert_eq!(pr_state(&u), None);
    }

    #[test]
    fn the_running_pr_build_cannot_be_installed_and_one_download_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let u = Updater::with(pr_build(12), tmp.path().into(), None, None);
        assert_eq!(u.install_pr(12).unwrap_err().to_string(), "already running");
        u.set(|i| {
            i.s.pr_install = Some(PrInstall {
                pr: 14,
                title: "t".into(),
                version: "0.1.61".into(),
                sha: "s".into(),
                state: PrPhase::Downloading,
                progress: Some(0.2),
                error: None,
            })
        });
        assert!(u.install_pr(15).unwrap_err().to_string().contains("already downloading"));
        // The refused call changed nothing.
        assert_eq!(pr_state(&u).unwrap().pr, 14);
    }

    #[test]
    fn a_staged_pr_build_is_restored_at_start_and_shown_as_ready() {
        let tmp = tempfile::tempdir().unwrap();
        stage_pr(tmp.path(), 14, "pr");
        let u = Updater::with(build("0.1.50", "main"), tmp.path().into(), None, None);
        let p = pr_state(&u).unwrap();
        assert_eq!((p.pr, p.state, p.version.as_str(), p.sha.as_str()), (14, PrPhase::Ready, "0.1.61", "a17c3d0"));
        assert_eq!((p.title.as_str(), p.progress, p.error), ("Fix the graph", None, None));
        let v = u.json();
        assert_eq!(v["pr_install"]["state"], "ready");
        assert_eq!(v["current"]["pr"], Value::Null);
        assert!(u.pr_ready().is_ok());
        // Idle: no object at all.
        let idle = Updater::with(build("0.1.50", "main"), tempfile::tempdir().unwrap().path().into(), None, None);
        assert!(idle.json()["pr_install"].is_null());
    }

    #[test]
    fn the_running_or_an_incomplete_pr_build_is_not_kept_and_main_staging_is_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        stage_pr(tmp.path(), 12, "pr");
        let u = Updater::with(pr_build(12), tmp.path().into(), None, None);
        assert_eq!(pr_state(&u), None);
        assert!(!tmp.path().join("pr-12").exists());

        std::fs::create_dir_all(tmp.path().join("pr-13/Orchestrator.app")).unwrap();
        std::fs::create_dir_all(tmp.path().join("pr-14.partial")).unwrap();
        std::fs::create_dir_all(tmp.path().join("pr-download")).unwrap();
        stage_pr(tmp.path(), 15, "pr");
        fake_app(&tmp.path().join("0.1.70/Orchestrator.app"), "main");
        let info = Available { version: "0.1.70".into(), sha: "f".into(), published_at: None, subject: "s".into() };
        std::fs::write(tmp.path().join("0.1.70/release.json"), serde_json::to_vec(&info).unwrap()).unwrap();
        let u = Updater::with(build("0.1.50", "main"), tmp.path().into(), None, None);
        let s = u.snapshot();
        assert_eq!((s.state, s.available.unwrap().version.as_str()), (Phase::Ready, "0.1.70"));
        assert_eq!(pr_state(&u).unwrap().pr, 15);
        for gone in ["pr-13", "pr-14.partial", "pr-download"] {
            assert!(!tmp.path().join(gone).exists(), "{gone}");
        }
        assert!(tmp.path().join("pr-15/Orchestrator.app").exists());
        assert!(tmp.path().join("0.1.70/Orchestrator.app").exists());
    }

    #[test]
    fn restarting_into_a_pr_build_needs_one_staged_and_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Applications/Orchestrator.app");
        fake_app(&bundle, "old");
        let u = Updater::with(build("0.1.50", "main"), tmp.path().join("updates"), Some(bundle.clone()), None);
        assert_eq!(u.prepare_restart_to(RestartTarget::Pr).unwrap_err().to_string(), "no PR build ready");
        assert_eq!(u.pr_ready().unwrap_err().to_string(), "no PR build ready");
        assert_eq!(marker(&bundle), "old");

        stage_pr(&tmp.path().join("updates"), 14, "pr build");
        let u = Updater::with(build("0.1.50", "main"), tmp.path().join("updates"), Some(bundle.clone()), None);
        u.prepare_restart_to(RestartTarget::Pr).unwrap();
        // Prepared once; a second restart (any target) is refused.
        assert!(u.prepare_restart_to(RestartTarget::Pr).is_err());
        assert!(u.prepare_restart().is_err());
        u.on_exit();
        assert_eq!(marker(&bundle), "pr build");
    }

    #[test]
    fn a_plain_quit_never_applies_a_pr_build() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        fake_app(&bundle, "old");
        stage_pr(&tmp.path().join("updates"), 14, "pr build");
        for current in [build("0.1.50", "main"), build("0.1.0", "local"), pr_build(9)] {
            let u = Updater::with(current, tmp.path().join("updates"), Some(bundle.clone()), None);
            assert_eq!(pr_state(&u).unwrap().state, PrPhase::Ready);
            u.on_exit();
            assert_eq!(marker(&bundle), "old");
        }
    }

    #[test]
    fn a_pr_build_runs_beside_main_updates_as_a_local_build_does() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Orchestrator.app");
        let updates = tmp.path().join("updates");
        fake_app(&bundle, "pr build");
        // Version 0.1.99 (a PR run number) still sees main's 0.1.50 as newer, stages it, and a
        // plain quit does not apply it; Restart does.
        fake_app(&updates.join("0.1.50/Orchestrator.app"), "main");
        let info = Available { version: "0.1.50".into(), sha: "f".into(), published_at: None, subject: "s".into() };
        std::fs::write(updates.join("0.1.50/release.json"), serde_json::to_vec(&info).unwrap()).unwrap();
        let mut running = pr_build(9);
        running.version = "0.1.99".into();
        let u = Updater::with(running.clone(), updates.clone(), Some(bundle.clone()), None);
        let s = u.snapshot();
        assert_eq!((s.state, s.apply_on_quit), (Phase::Ready, false));
        u.on_exit();
        assert_eq!(marker(&bundle), "pr build");
        let u = Updater::with(running, updates, Some(bundle.clone()), None);
        u.prepare_restart().unwrap();
        u.on_exit();
        assert_eq!(marker(&bundle), "main");
    }

    #[test]
    fn a_failed_pr_pre_flight_keeps_the_app_up_with_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let updates = tmp.path().join("updates");
        stage_pr(&updates, 14, "pr build");
        // The bundle's parent does not exist: the pre-flight cannot work.
        let bundle = tmp.path().join("nowhere/Orchestrator.app");
        let u = Updater::with(build("0.1.50", "main"), updates, Some(bundle), None);
        assert!(u.prepare_restart_to(RestartTarget::Pr).is_err());
        let p = pr_state(&u).unwrap();
        assert_eq!(p.state, PrPhase::Error);
        assert!(p.error.is_some());
        assert!(u.pr_ready().is_err());
    }
}
