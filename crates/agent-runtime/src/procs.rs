//! Finding and killing an agent's processes (FR-207, J10 step 2.5).
//!
//! The process group is not enough: Claude's Bash tool runs each command in
//! its own session with no tty, and `cmd &` children outlive `claude`. They
//! are found by their cwd inside the agent's worktree. The environment can't
//! be used (macOS hides it for platform binaries).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub stat: String,
    /// `??` when the process has no controlling terminal.
    pub tty: String,
    pub cmd: String,
    pub cwd: Option<PathBuf>,
}

impl ProcInfo {
    pub fn has_tty(&self) -> bool {
        !self.tty.is_empty() && self.tty != "??" && self.tty != "?"
    }
}

/// A snapshot of the process table (faked in tests).
pub trait ProcessTable {
    fn snapshot(&self) -> Vec<ProcInfo>;
}

/// `ps` + `lsof -d cwd`.
pub struct SystemProcs;

impl ProcessTable for SystemProcs {
    fn snapshot(&self) -> Vec<ProcInfo> {
        let cwds = cwd_map();
        let out = Command::new("ps").args(["-axo", "pid=,ppid=,pgid=,stat=,tty=,command="]).output();
        let Ok(out) = out else { return vec![] };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() < 6 {
                    return None;
                }
                let pid: i32 = f[0].parse().ok()?;
                Some(ProcInfo {
                    pid,
                    ppid: f[1].parse().ok()?,
                    pgid: f[2].parse().ok()?,
                    stat: f[3].into(),
                    tty: f[4].into(),
                    cmd: f[5..].join(" "),
                    cwd: cwds.get(&pid).cloned(),
                })
            })
            .collect()
    }
}

fn cwd_map() -> BTreeMap<i32, PathBuf> {
    let mut m = BTreeMap::new();
    let Ok(out) = Command::new("lsof").args(["-nP", "-d", "cwd", "-Fpn"]).output() else { return m };
    let mut pid = None;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(n), Some(p)) = (l.strip_prefix('n'), pid) {
            m.insert(p, PathBuf::from(n));
        }
    }
    m
}

/// Why a process was attributed to the agent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub proc: ProcInfo,
    pub how: Vec<&'static str>,
}

/// What identifies an agent's processes.
#[derive(Clone, Debug)]
pub struct AgentMatch<'a> {
    pub pgid: Option<i32>,
    /// The per-agent folder appears in `claude`'s argv (`--settings <dir>/settings.json`).
    pub agent_dir: &'a Path,
    pub worktree: &'a Path,
    /// Never matched (the core itself and its client).
    pub exclude: &'a [i32],
}

/// Processes of the agent: its argv, and **tty-less** processes
/// whose cwd is inside the worktree. A process with its own terminal and
/// another group (the user's shell `cd`'d into the worktree) is left alone.
pub fn find_agent_procs(procs: &[ProcInfo], m: &AgentMatch) -> Vec<Found> {
    let dir = m.agent_dir.display().to_string();
    procs
        .iter()
        .filter(|p| !m.exclude.contains(&p.pid) && p.pid > 1)
        .filter_map(|p| {
            let mut how = vec![];
            if m.pgid.is_some_and(|g| g > 1 && g == p.pgid) {
                how.push("pgid");
            }
            if p.cmd.contains(&dir) {
                how.push("argv");
            }
            if let Some(c) = &p.cwd {
                if c.starts_with(m.worktree) && (!p.has_tty() || how.contains(&"pgid")) {
                    how.push("cwd");
                }
            }
            // A stored pgid alone is not proof: after a reboot it may belong to
            // another program. It only adds weight to an argv or cwd match.
            (how.contains(&"argv") || how.contains(&"cwd")).then(|| Found { proc: p.clone(), how })
        })
        .collect()
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks for existence.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn group_alive(pgid: i32) -> bool {
    // SAFETY: as above, for a process group.
    let r = unsafe { libc::killpg(pgid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// SIGTERM to each group and pid, then SIGKILL whatever is left after `grace`.
pub fn kill_tree(pgids: &[i32], pids: &[i32], grace: Duration) -> Vec<String> {
    let mut log = vec![];
    for sig in [libc::SIGTERM, libc::SIGKILL] {
        let name = if sig == libc::SIGTERM { "SIGTERM" } else { "SIGKILL" };
        for &g in pgids.iter().filter(|g| **g > 1) {
            // SAFETY: plain signal delivery to a group we identified.
            if unsafe { libc::killpg(g, sig) } == 0 {
                log.push(format!("killpg({g},{name})"));
            }
        }
        for &p in pids.iter().filter(|p| **p > 1) {
            // SAFETY: plain signal delivery to a pid we identified.
            if unsafe { libc::kill(p, sig) } == 0 {
                log.push(format!("kill({p},{name})"));
            }
        }
        if sig == libc::SIGTERM {
            let end = Instant::now() + grace;
            while Instant::now() < end {
                if !pids.iter().any(|p| alive(*p)) && !pgids.iter().any(|g| *g > 1 && group_alive(*g)) {
                    return log;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    log
}

/// phys_footprint in MB (NFR-1b: not RSS), from `proc_pid_rusage`, the
/// figure `footprint -p` reports, without spawning a process.
pub fn footprint_mb(pid: i32) -> Option<f64> {
    // SAFETY: `info` is a plain C struct the kernel fills for RUSAGE_INFO_V4.
    let bytes = unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let r = libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, (&mut info as *mut libc::rusage_info_v4).cast());
        if r != 0 {
            return None;
        }
        info.ri_phys_footprint
    };
    Some((bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: i32, pgid: i32, tty: &str, cmd: &str, cwd: Option<&str>) -> ProcInfo {
        ProcInfo { pid, ppid: 1, pgid, stat: "S".into(), tty: tty.into(), cmd: cmd.into(), cwd: cwd.map(PathBuf::from) }
    }

    #[test]
    fn finds_group_argv_and_detached_children_but_not_user_shells() {
        let procs = vec![
            p(100, 100, "ttys003", "claude --settings /app/agents/a1/settings.json", Some("/w/solo/a1")),
            p(101, 101, "??", "/bin/zsh -c eval 'sleep 300 &'", Some("/w/solo/a1/sub")),
            p(102, 101, "??", "sleep 300", Some("/w/solo/a1")),
            p(200, 200, "ttys001", "-zsh", Some("/w/solo/a1")),
            p(300, 300, "??", "sleep 1", Some("/w/solo/a10")),
            p(400, 400, "??", "orchd serve", Some("/w/solo/a1")),
            // Same group as the stored pgid, but nothing else ties it to the agent.
            p(500, 100, "ttys009", "vim notes.txt", Some("/Users/u")),
        ];
        let m = AgentMatch {
            pgid: Some(100),
            agent_dir: Path::new("/app/agents/a1"),
            worktree: Path::new("/w/solo/a1"),
            exclude: &[400],
        };
        let f = find_agent_procs(&procs, &m);
        let pids: Vec<i32> = f.iter().map(|f| f.proc.pid).collect();
        assert_eq!(pids, [100, 101, 102]);
        assert_eq!(f[0].how, ["pgid", "argv", "cwd"]);
        assert_eq!(f[2].how, ["cwd"]);
    }

    #[test]
    fn kills_a_detached_child_found_by_cwd() {
        let d = tempfile::tempdir().unwrap();
        let wt = crate::paths::real(d.path());
        // A detached, tty-less sleeper in its own session, like a Bash tool `cmd &`.
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg("exec sleep 30").current_dir(&wt);
        // SAFETY: setsid in the child before exec, as Claude's Bash tool does.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        let pid = child.id() as i32;
        std::thread::sleep(Duration::from_millis(200));
        let snap = SystemProcs.snapshot();
        let me = std::process::id() as i32;
        let f = find_agent_procs(
            &snap,
            &AgentMatch { pgid: None, agent_dir: Path::new("/nonexistent"), worktree: &wt, exclude: &[me] },
        );
        assert!(f.iter().any(|f| f.proc.pid == pid), "{f:?}");
        let log = kill_tree(&[], &[pid], Duration::from_secs(2));
        assert!(log.iter().any(|l| l.contains("SIGTERM")));
        let mut child = child;
        let _ = child.wait();
        assert!(!alive(pid));
    }

    #[test]
    fn footprint_of_self() {
        let mb = footprint_mb(std::process::id() as i32).expect("own footprint");
        assert!(mb > 0.0);
        assert_eq!(footprint_mb(i32::MAX), None);
    }
}
