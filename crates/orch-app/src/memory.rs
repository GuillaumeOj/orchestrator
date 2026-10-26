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

/// WebKit helper processes this app is responsible for.
pub fn webkit_helpers(app_pid: i32) -> Vec<(i32, String)> {
    all_pids()
        .into_iter()
        .filter(|&p| p > 0 && p != app_pid)
        .filter_map(|p| {
            let name = proc_name(p);
            // SAFETY: plain query on a pid.
            let owner = unsafe { responsibility_get_pid_responsible_for_pid(p) };
            (name.starts_with("com.apple.WebKit") && owner == app_pid).then_some((p, name))
        })
        .collect()
}

pub fn system_memory_mb() -> f64 {
    let mut size: u64 = 0;
    let mut len = size_of::<u64>();
    // SAFETY: sysctlbyname writes a u64 into `size`.
    unsafe {
        libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut size as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0);
    }
    size as f64 / (1024.0 * 1024.0)
}

pub fn measure(shell_pids: &[i32], agent_mb: f64) -> AppMemory {
    let me = std::process::id() as i32;
    let core_mb = footprint_mb(me).unwrap_or(0.0);
    let webkit: Vec<(String, f64)> =
        webkit_helpers(me).into_iter().map(|(p, n)| (n, footprint_mb(p).unwrap_or(0.0))).collect();
    let app_mb = core_mb + webkit.iter().map(|w| w.1).sum::<f64>();
    let shells_mb = shell_pids.iter().filter_map(|p| footprint_mb(*p)).sum();
    let r = |x: f64| (x * 10.0).round() / 10.0;
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
