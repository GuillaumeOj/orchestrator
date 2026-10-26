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

#[derive(Clone, Debug, Default, Serialize)]
pub struct WatchView {
    pub warn_mb: f64,
    pub at_ms: u64,
    pub core_mb: f64,
    /// `(helper, MB)` at the last check.
    pub helpers: Vec<(String, f64)>,
    /// The largest footprint seen per helper since start.
    pub peak: Vec<(String, f64)>,
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

    /// Records one reading; returns the log line.
    pub fn record(&self, at_ms: u64, core_mb: f64, helpers: Vec<(String, f64)>, warn_mb: f64) -> serde_json::Value {
        let mut v = self.view.lock_ok();
        v.warn_mb = warn_mb;
        v.at_ms = at_ms;
        v.core_mb = core_mb;
        for (name, mb) in &helpers {
            match v.peak.iter_mut().find(|(n, _)| n == name) {
                Some(p) => p.1 = p.1.max(*mb),
                None => v.peak.push((name.clone(), *mb)),
            }
        }
        v.alert = helpers
            .iter()
            .filter(|(_, mb)| *mb > warn_mb)
            .map(|(n, mb)| format!("{} {mb:.0} MB", short(n)))
            .reduce(|a, b| format!("{a}, {b}"));
        v.helpers = helpers;
        let h: serde_json::Map<String, serde_json::Value> =
            v.helpers.iter().map(|(n, mb)| (short(n).to_string(), json!(round(*mb)))).collect();
        json!({"ts_ms": at_ms, "core_mb": round(core_mb), "helpers": h, "alert": v.alert})
    }
}

fn round(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

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
pub fn start(watch: Arc<MemoryWatch>, log_dir: PathBuf, stop: Arc<AtomicBool>) {
    let _ = std::fs::create_dir_all(&log_dir);
    let log = log_dir.join("memory.log");
    watch.view.lock_ok().log = log.clone();
    let _ = std::thread::Builder::new().name("orch-memwatch".into()).spawn(move || {
        let mut next = std::time::Instant::now() + Duration::from_secs(10);
        while !stop.load(Ordering::SeqCst) {
            if std::time::Instant::now() >= next {
                let core = agent_runtime::procs::footprint_mb(std::process::id() as i32).unwrap_or(0.0);
                let was = watch.view().alert;
                let line = watch.record(orch_protocol::now_ms(), core, memory::webkit_now(), warn_mb());
                append(&log, &line);
                let now = watch.view().alert;
                if now.is_some() && was.is_none() {
                    eprintln!(
                        "orchestrator: memory watch: {} above {} MB (see {})",
                        now.as_deref().unwrap_or(""),
                        warn_mb(),
                        log.display()
                    );
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
    fn alerts_above_the_threshold_and_keeps_peaks() {
        let w = MemoryWatch::default();
        let wc = "com.apple.WebKit.WebContent".to_string();
        let gpu = "com.apple.WebKit.GPU".to_string();
        let line = w.record(1, 30.0, vec![(wc.clone(), 120.0), (gpu.clone(), 80.0)], 600.0);
        assert_eq!(line["helpers"]["WebContent"], 120.0);
        assert!(w.view().alert.is_none());
        w.record(2, 30.0, vec![(wc.clone(), 2800.0), (gpu.clone(), 90.0)], 600.0);
        assert_eq!(w.view().alert.as_deref(), Some("WebContent 2800 MB"));
        w.record(3, 30.0, vec![(wc.clone(), 130.0), (gpu, 85.0)], 600.0);
        let v = w.view();
        assert!(v.alert.is_none(), "cleared once back under");
        assert_eq!(v.peak.iter().find(|p| p.0 == wc).unwrap().1, 2800.0);
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
