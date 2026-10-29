//! Memory self-check: once a minute the WebKit helpers' footprints (WebContent,
//! GPU, Networking) and the core's are appended to `<home>/logs/memory.log`,
//! and a helper above the threshold raises a warning in the status bar. Kept
//! for the real week after one WebContent runaway (2.8 GB after a resize with
//! agent, nvim and shell panes; docs/app/README.md).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::LockOk;
use crate::memory;

pub const EVERY: Duration = Duration::from_secs(60);
/// A helper above this is far outside what was measured (WebContent 65–135 MB,
/// GPU 40–95 MB with 0–3 agent panes). `ORCH_WEBKIT_WARN_MB` overrides it.
pub const DEFAULT_WARN_MB: f64 = 600.0;
/// The log is rotated once (to `memory.log.1`) above this size.
const LOG_MAX: u64 = 4 << 20;

/// What the status bar reads; the figures themselves go to the log.
#[derive(Clone, Debug, Default, Serialize)]
pub struct WatchView {
    pub warn_mb: f64,
    /// Set while a helper is above the threshold, e.g. "WebContent 2800 MB".
    pub alert: Option<String>,
    pub log: PathBuf,
}

#[derive(Default)]
pub struct MemoryWatch {
    view: Mutex<WatchView>,
}

impl MemoryWatch {
    pub fn view(&self) -> WatchView {
        self.view.lock_ok().clone()
    }

    /// Records one reading; returns the log line, whether the alert just
    /// started, and whether it started or ended.
    pub fn record(
        &self,
        at_ms: u64,
        core_mb: f64,
        helpers: &[(String, f64)],
        warn_mb: f64,
    ) -> (serde_json::Value, bool, bool) {
        let mut v = self.view.lock_ok();
        let was = v.alert.is_some();
        v.warn_mb = warn_mb;
        v.alert = helpers
            .iter()
            .filter(|(_, mb)| *mb > warn_mb)
            .map(|(n, mb)| format!("{} {mb:.0} MB", short(n)))
            .reduce(|a, b| format!("{a}, {b}"));
        let h: serde_json::Map<String, serde_json::Value> =
            helpers.iter().map(|(n, mb)| (short(n).to_string(), json!(round(*mb)))).collect();
        let now = v.alert.is_some();
        (json!({"ts_ms": at_ms, "core_mb": round(core_mb), "helpers": h, "alert": v.alert}), !was && now, was != now)
    }
}

use orch_protocol::round1 as round;

/// `com.apple.WebKit.WebContent` → `WebContent`.
fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn warn_mb() -> f64 {
    std::env::var("ORCH_WEBKIT_WARN_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_WARN_MB)
}

fn append(log: &Path, line: &serde_json::Value) {
    if std::fs::metadata(log).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::rename(log, log.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
        let _ = writeln!(f, "{line}");
    }
}

/// The watch thread: a reading now (once WebKit has started) and every minute.
/// `changed` is called when the alert starts or ends (the `memory` topic).
pub fn start(watch: Arc<MemoryWatch>, log_dir: PathBuf, stop: Arc<AtomicBool>, changed: impl Fn() + Send + 'static) {
    let _ = std::fs::create_dir_all(&log_dir);
    let log = log_dir.join("memory.log");
    watch.view.lock_ok().log = log.clone();
    let _ = std::thread::Builder::new().name("orch-memwatch".into()).spawn(move || {
        let mut next = std::time::Instant::now() + Duration::from_secs(10);
        while !stop.load(Ordering::SeqCst) {
            if std::time::Instant::now() >= next {
                let core = agent_runtime::procs::footprint_mb(std::process::id() as i32).unwrap_or(0.0);
                let warn = warn_mb();
                let (line, started, flipped) = watch.record(orch_protocol::now_ms(), core, &memory::webkit_now(), warn);
                append(&log, &line);
                if flipped {
                    changed();
                }
                if started {
                    let alert = line["alert"].as_str().unwrap_or("");
                    eprintln!("orchestrator: memory watch: {alert} above {warn} MB (see {})", log.display());
                }
                next = std::time::Instant::now() + EVERY;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_above_the_threshold_until_back_under() {
        let w = MemoryWatch::default();
        let wc = "com.apple.WebKit.WebContent".to_string();
        let gpu = "com.apple.WebKit.GPU".to_string();
        let (line, started, _) = w.record(1, 30.0, &[(wc.clone(), 120.0), (gpu.clone(), 80.0)], 600.0);
        assert_eq!(line["helpers"]["WebContent"], 120.0);
        assert!(!started && w.view().alert.is_none());
        let (_, started, _) = w.record(2, 30.0, &[(wc.clone(), 2800.0), (gpu.clone(), 90.0)], 600.0);
        assert!(started);
        assert_eq!(w.view().alert.as_deref(), Some("WebContent 2800 MB"));
        let (_, started, flipped) = w.record(3, 30.0, &[(wc.clone(), 2900.0), (gpu.clone(), 90.0)], 600.0);
        assert!(!started && !flipped, "reported once");
        assert!(w.record(4, 30.0, &[(wc, 130.0), (gpu, 85.0)], 600.0).2, "ended");
        assert!(w.view().alert.is_none(), "cleared once back under");
    }

    #[test]
    fn the_log_is_appended_and_rotated() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("memory.log");
        append(&log, &json!({"a": 1}));
        append(&log, &json!({"a": 2}));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
        std::fs::write(&log, vec![b'x'; (LOG_MAX + 1) as usize]).unwrap();
        append(&log, &json!({"a": 3}));
        assert!(log.with_extension("log.1").exists());
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), r#"{"a":3}"#);
    }
}
