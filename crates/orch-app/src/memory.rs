//! The app's own memory (FR-904, NFR-1b), as phys_footprint: the app process
//! (Rust core + Tauri) plus the WebKit helpers (WebContent, GPU, Networking)
//! that macOS attributes to it, and the shells and agents it runs.

use agent_runtime::procs::footprint_mb;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct AppMemory {
    pub core_mb: f64,
    /// `(process name, MB)` of each WebKit helper responsible to this app.
    pub webkit: Vec<(String, f64)>,
    /// Core + WebKit helpers: the figure NFR-1b budgets (≤ 250 MB idle).
    pub app_mb: f64,
    pub shells_mb: f64,
    pub agents_mb: f64,
    pub total_mb: f64,
    pub system_mb: f64,
}

unsafe extern "C" {
    /// libquarantine (in libSystem): the "responsible" process of `pid`, the
    /// one Activity Monitor groups XPC helpers under.
    fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
}

fn all_pids() -> Vec<i32> {
    // SAFETY: proc_listallpids fills at most `cap` pids.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return vec![];
        }
        let cap = n as usize + 64;
        let mut v = vec![0i32; cap];
        let got = libc::proc_listallpids(v.as_mut_ptr().cast(), (cap * size_of::<i32>()) as i32);
        v.truncate(got.max(0) as usize);
        v
    }
}

fn proc_name(pid: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its size is passed.
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    String::from_utf8_lossy(&buf[..n.max(0) as usize]).to_string()
}

/// Start time of a process, in seconds since the epoch.
fn start_time(pid: i32) -> Option<u64> {
    // SAFETY: `info` is a plain C struct the kernel fills for PROC_PIDTBSDINFO.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        let n = libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size);
        (n == size).then_some(info.pbi_start_tvsec)
    }
}

/// How long after the app WebKit starts its helpers.
const HELPER_START_WINDOW_S: u64 = 30;

/// WebKit helper processes of this app. Launched from the Finder, the app is
/// their "responsible" process. Launched from a terminal, they share the app's
/// responsible process (the terminal) with everything else it started, so
/// only helpers that started with the app's webview count (a helper WebKit
/// restarts later is then missed: the figure errs low, not high).
pub fn webkit_helpers(app_pid: i32) -> Vec<(i32, String)> {
    // SAFETY: plain queries on pids.
    let resp = |p: i32| unsafe { responsibility_get_pid_responsible_for_pid(p) };
    let me = resp(app_pid);
    let since = start_time(app_pid).unwrap_or(0);
    all_pids()
        .into_iter()
        .filter(|&p| p > 0 && p != app_pid)
        .filter_map(|p| {
            let name = proc_name(p);
            if !name.starts_with("com.apple.WebKit") {
                return None;
            }
            let r = resp(p);
            let started = start_time(p).unwrap_or(0);
            let ours = r == app_pid
                || (r == me && me != app_pid && (since..=since + HELPER_START_WINDOW_S).contains(&started));
            ours.then_some((p, name))
        })
        .collect()
}

pub fn system_memory_mb() -> f64 {
    static MB: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *MB.get_or_init(|| {
        let mut size: u64 = 0;
        let mut len = size_of::<u64>();
        // SAFETY: sysctlbyname writes a u64 into `size`.
        unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut size as *mut u64).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            );
        }
        size as f64 / (1024.0 * 1024.0)
    })
}

/// How long a scan of all processes for the WebKit helpers is reused.
const HELPER_RESCAN: std::time::Duration = std::time::Duration::from_secs(60);

/// WebKit runs these three helpers per app; fewer means some have not started yet.
const HELPERS: usize = 3;

/// The app's WebKit helpers with their footprints. The pid scan (hundreds of
/// syscalls) is redone every minute, when a known helper is gone, or while
/// not all three have been seen yet (the GPU helper starts lazily).
fn helpers_with_footprint(app_pid: i32) -> Vec<(String, f64)> {
    use crate::LockOk;
    type Scan = Option<(std::time::Instant, Vec<(i32, String)>)>;
    static CACHE: std::sync::Mutex<Scan> = std::sync::Mutex::new(None);
    let mut c = CACHE.lock_ok();
    let measure = |hs: &[(i32, String)]| {
        hs.iter().map(|(p, n)| footprint_mb(*p).map(|mb| (n.clone(), mb))).collect::<Option<Vec<_>>>()
    };
    if let Some((at, hs)) = c.as_ref() {
        if at.elapsed() < HELPER_RESCAN && hs.len() >= HELPERS {
            if let Some(m) = measure(hs) {
                return m;
            }
        }
    }
    let hs = webkit_helpers(app_pid);
    let m = hs.iter().map(|(p, n)| (n.clone(), footprint_mb(*p).unwrap_or(0.0))).collect();
    *c = Some((std::time::Instant::now(), hs));
    m
}

/// This app's WebKit helpers and their footprints now (`(name, MB)`).
pub fn webkit_now() -> Vec<(String, f64)> {
    helpers_with_footprint(std::process::id() as i32)
}

pub fn measure(shell_pids: &[i32], agent_mb: f64) -> AppMemory {
    let me = std::process::id() as i32;
    let core_mb = footprint_mb(me).unwrap_or(0.0);
    let webkit = helpers_with_footprint(me);
    let app_mb = core_mb + webkit.iter().map(|w| w.1).sum::<f64>();
    let shells_mb = shell_pids.iter().filter_map(|p| footprint_mb(*p)).sum();
    let r = orch_protocol::round1;
    AppMemory {
        core_mb: r(core_mb),
        webkit,
        app_mb: r(app_mb),
        shells_mb: r(shells_mb),
        agents_mb: r(agent_mb),
        total_mb: r(app_mb + shells_mb + agent_mb),
        system_mb: system_memory_mb(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_this_process() {
        let m = measure(&[std::process::id() as i32], 0.0);
        assert!(m.core_mb > 0.0 && m.shells_mb > 0.0);
        assert!(m.system_mb > 1024.0);
        assert!(m.webkit.is_empty(), "a test binary has no webview");
        assert!(!all_pids().is_empty());
        assert!(!proc_name(std::process::id() as i32).is_empty());
    }
}
