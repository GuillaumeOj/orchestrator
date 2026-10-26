//! Shell and editor panes (FR-1001, FR-1102): a PTY running the user's login
//! shell, owned by the core. Output reaches the UI through the pull transport
//! (ADR-001 §4): the reader appends to a per-pane buffer and blocks above a
//! high watermark (backpressure); `read_wait` hands everything buffered to one
//! IPC response.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const HIGH_WATERMARK: usize = 2 << 20;

#[derive(Default)]
struct Buf {
    bytes: Vec<u8>,
    closed: bool,
}

#[derive(Default)]
pub struct Output {
    buf: Mutex<Buf>,
    cv: Condvar,
}

impl Output {
    fn lock(&self) -> std::sync::MutexGuard<'_, Buf> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, data: &[u8]) {
        let mut b = self.lock();
        while b.bytes.len() >= HIGH_WATERMARK && !b.closed {
            b = self.cv.wait(b).unwrap_or_else(|e| e.into_inner());
        }
        b.bytes.extend_from_slice(data);
        self.cv.notify_all();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
    }

    /// Everything buffered, waiting up to `timeout` for the first byte.
    /// `None` once the PTY is closed and drained.
    pub fn read_wait(&self, timeout: Duration) -> Option<Vec<u8>> {
        let b = self.lock();
        let (mut b, _) =
            self.cv.wait_timeout_while(b, timeout, |b| b.bytes.is_empty() && !b.closed).unwrap_or_else(|e| e.into_inner());
        if b.bytes.is_empty() && b.closed {
            return None;
        }
        let out = std::mem::take(&mut b.bytes);
        self.cv.notify_all();
        Some(out)
    }
}

pub struct Term {
    pub id: u32,
    pub kind: TermKind,
    pub cwd: PathBuf,
    pub pid: Option<u32>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    pub output: Arc<Output>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    Shell,
    Editor,
}

/// The environment a login shell gets, as if the app was launched from the
/// Finder: the shell's own profile builds the rest (PATH from nvm, bob, …).
pub fn base_env(cmd: &mut CommandBuilder) {
    cmd.env_clear();
    for k in ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    if std::env::var("LANG").is_err() {
        cmd.env("LANG", "en_US.UTF-8");
    }
    cmd.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "Orchestrator");
}

pub fn login_shell() -> PathBuf {
    std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| "/bin/zsh".into())
}

/// What a pane runs: the login shell itself, or a command through it
/// (`$SHELL -l -i -c 'exec …'`, so PATH and the user's env are set up).
pub fn command(run: Option<&str>) -> CommandBuilder {
    match run {
        None => CommandBuilder::new_default_prog(),
        Some(c) => {
            let mut cmd = CommandBuilder::new(login_shell());
            cmd.args(["-l", "-i", "-c", &format!("exec {c}")]);
            cmd
        }
    }
}

#[derive(Default)]
pub struct Terms {
    next: AtomicU32,
    map: Mutex<BTreeMap<u32, Arc<Term>>>,
}

impl Terms {
    pub fn spawn(&self, kind: TermKind, cwd: &Path, run: Option<&str>, rows: u16, cols: u16) -> Result<Arc<Term>> {
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| anyhow!("openpty: {e}"))?;
        let mut cmd = command(run);
        base_env(&mut cmd);
        let cwd = if cwd.is_dir() { cwd.to_path_buf() } else { agent_runtime::paths::home_dir() };
        cmd.cwd(&cwd);
        let child = pair.slave.spawn_command(cmd).map_err(|e| anyhow!("spawn: {e}"))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
        let writer = pair.master.take_writer().map_err(|e| anyhow!("{e}"))?;
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let output = Arc::new(Output::default());
        let out = output.clone();
        std::thread::Builder::new()
            .name(format!("term-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => out.push(&buf[..n]),
                    }
                }
                out.close();
            })
            .context("reader thread")?;
        let pid = child.process_id();
        let t = Arc::new(Term {
            id,
            kind,
            cwd,
            pid,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            output,
        });
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(id, t.clone());
        Ok(t)
    }

    pub fn get(&self, id: u32) -> Result<Arc<Term>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned().ok_or_else(|| anyhow!("no terminal {id}"))
    }

    pub fn ids(&self) -> Vec<u32> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).keys().copied().collect()
    }

    /// SIGHUP to the pane's process group (the shell is a session leader), as
    /// closing a terminal window does; the child is reaped.
    pub fn kill(&self, id: u32) {
        let t = self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(t) = t {
            if let Some(pid) = t.pid {
                // SAFETY: plain syscall on a pid we spawned.
                unsafe { libc::killpg(pid as i32, libc::SIGHUP) };
            }
            let mut c = t.child.lock().unwrap_or_else(|e| e.into_inner());
            let _ = c.kill();
            let _ = c.wait();
            t.output.close();
        }
    }

    pub fn kill_all(&self) {
        for id in self.ids() {
            self.kill(id);
        }
    }
}

impl Term {
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| anyhow!("resize: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(t: &Term, until: &str) -> String {
        let mut s = String::new();
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < end && !s.contains(until) {
            match t.output.read_wait(Duration::from_millis(200)) {
                Some(b) => s.push_str(&String::from_utf8_lossy(&b)),
                None => break,
            }
        }
        s
    }

    #[test]
    fn runs_a_command_in_a_pty_and_pulls_its_output() {
        let terms = Terms::default();
        let d = tempfile::tempdir().unwrap();
        // The user's login shell, as a pane runs it.
        let t = terms.spawn(TermKind::Shell, d.path(), None, 24, 80).unwrap();
        t.write(b"/bin/echo orch-$((40+2)); exit\r").unwrap();
        let out = drain(&t, "orch-42");
        assert!(out.contains("orch-42"), "{out:?}");
        terms.kill(t.id);
        assert!(terms.get(t.id).is_err());
        // Closed and drained: the pull loop sees the end.
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while t.output.read_wait(Duration::from_millis(100)).is_some() && std::time::Instant::now() < end {}
        assert!(t.output.read_wait(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn backpressure_blocks_the_reader_until_pulled() {
        let o = Arc::new(Output::default());
        let o2 = o.clone();
        let big = vec![b'x'; HIGH_WATERMARK];
        let w = std::thread::spawn(move || {
            o2.push(&big);
            o2.push(b"more"); // blocks: the buffer is at the watermark
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!w.is_finished());
        assert_eq!(o.read_wait(Duration::from_millis(10)).unwrap().len(), HIGH_WATERMARK);
        w.join().unwrap();
        assert_eq!(o.read_wait(Duration::from_millis(10)).unwrap(), b"more");
    }
}
