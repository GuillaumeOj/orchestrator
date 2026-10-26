//! `claude` in a PTY (portable-pty): its own session and process group, a
//! close-on-exec master, and the **pull** transport of ADR-001 §4 (P0 terminal
//! spike): the reader appends to a per-agent buffer and a consumer takes
//! everything buffered with [`PtyProcess::read`].

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::compile::Compiled;

/// Above this, the reader waits for an attached consumer to pull (backpressure).
pub const HIGH_WATERMARK: usize = 2 * 1024 * 1024;
/// With no consumer attached, only the tail is kept (replayed on attach).
pub const DETACHED_KEEP: usize = 256 * 1024;
/// Plain-text tail kept for screen checks (trust dialog).
const TAIL_TEXT: usize = 8 * 1024;

/// The CLI's workspace trust dialog (J0): the app never answers it.
pub const TRUST_DIALOG_TEXT: &str = "Quick safety check";

#[derive(Default)]
struct Buf {
    bytes: VecDeque<u8>,
    attached: bool,
    total: u64,
    tail_text: String,
    closed: bool,
}

pub struct Output {
    buf: Mutex<Buf>,
    cv: Condvar,
    pub trust_dialog: AtomicBool,
    /// Off once SessionStart has fired: the dialog only comes before it.
    watch_trust: AtomicBool,
}

impl Output {
    fn new() -> Self {
        Output {
            buf: Mutex::new(Buf::default()),
            cv: Condvar::new(),
            trust_dialog: AtomicBool::new(false),
            watch_trust: AtomicBool::new(true),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Buf> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called by the reader thread. Returns the terminal queries to answer when
    /// nobody is attached (a real terminal would answer them).
    fn push(&self, data: &[u8]) -> Vec<&'static [u8]> {
        let mut b = self.lock();
        while b.attached && b.bytes.len() >= HIGH_WATERMARK && !b.closed {
            b = self.cv.wait(b).unwrap_or_else(|e| e.into_inner());
        }
        b.bytes.extend(data);
        b.total += data.len() as u64;
        if !b.attached && b.bytes.len() > DETACHED_KEEP {
            let drop = b.bytes.len() - DETACHED_KEEP;
            b.bytes.drain(..drop);
        }
        b.tail_text.push_str(&strip_ansi(&String::from_utf8_lossy(data)));
        if b.tail_text.len() > TAIL_TEXT {
            let mut cut = b.tail_text.len() - TAIL_TEXT;
            while !b.tail_text.is_char_boundary(cut) {
                cut += 1;
            }
            b.tail_text.drain(..cut);
        }
        if self.watch_trust.load(Ordering::Relaxed) {
            let squashed: String = b.tail_text.split_whitespace().collect::<Vec<_>>().join(" ");
            self.trust_dialog.store(squashed.contains(TRUST_DIALOG_TEXT), Ordering::Relaxed);
        }
        self.cv.notify_all();
        let mut answers = vec![];
        if !b.attached {
            if contains(data, b"\x1b[6n") {
                answers.push(&b"\x1b[1;1R"[..]);
            }
            if contains(data, b"\x1b[c") {
                answers.push(&b"\x1b[?62;22c"[..]);
            }
        }
        answers
    }

    /// Like [`read`](Self::read), but waits up to `timeout` for output.
    pub fn read_wait(&self, timeout: std::time::Duration) -> Vec<u8> {
        let b = self.lock();
        let (mut b, _) = self
            .cv
            .wait_timeout_while(b, timeout, |b| b.bytes.is_empty() && !b.closed)
            .unwrap_or_else(|e| e.into_inner());
        let out: Vec<u8> = b.bytes.drain(..).collect();
        self.cv.notify_all();
        out
    }

    /// Pull: everything buffered since the last read.
    pub fn read(&self) -> Vec<u8> {
        let mut b = self.lock();
        let out: Vec<u8> = b.bytes.drain(..).collect();
        self.cv.notify_all();
        out
    }

    pub fn is_attached(&self) -> bool {
        self.lock().attached
    }

    pub fn set_attached(&self, on: bool) {
        self.lock().attached = on;
        self.cv.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    pub fn total(&self) -> u64 {
        self.lock().total
    }

    /// The last few KB of output as plain text.
    pub fn tail_text(&self) -> String {
        self.lock().tail_text.clone()
    }

    /// SessionStart fired, so the trust dialog is gone (no hook fires before
    /// it is answered): forget the screen text that showed it.
    pub fn clear_trust_dialog(&self) {
        self.watch_trust.store(false, Ordering::Relaxed);
        self.lock().tail_text.clear();
        self.trust_dialog.store(false, Ordering::Relaxed);
    }

    fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Removes CSI/OSC/charset escape sequences (each replaced by a space).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            if !c.is_control() || c == '\n' {
                out.push(c);
            }
            continue;
        }
        out.push(' ');
        match it.next() {
            Some('[') => {
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = it.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && it.peek() == Some(&'\\') {
                        it.next();
                        break;
                    }
                }
            }
            Some('(' | ')') => {
                it.next();
            }
            _ => {}
        }
    }
    out
}

/// A live `claude` process.
pub struct PtyProcess {
    pub pid: i32,
    pub pgid: i32,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    pub output: Arc<Output>,
    exited: Arc<AtomicBool>,
    /// When the process was spawned (ms since the epoch).
    pub spawned_at_ms: u64,
}

impl PtyProcess {
    /// Spawns the compiled command. `on_exit` runs on a background thread
    /// once the process has exited and its output is drained.
    pub fn spawn(
        cfg: &Compiled,
        rows: u16,
        cols: u16,
        raw_log: Option<&Path>,
        on_exit: Box<dyn FnOnce(Option<u32>) + Send>,
    ) -> Result<PtyProcess> {
        let pair =
            native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).context("openpty")?;
        if let Some(fd) = pair.master.as_raw_fd() {
            // SAFETY: setting FD_CLOEXEC on a descriptor we own. A leaked master
            // would keep a headless `claude` alive (FR-207).
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
        let mut cmd = CommandBuilder::new(&cfg.program);
        cmd.args(&cfg.args);
        cmd.env_clear();
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        cmd.cwd(&cfg.cwd);
        // portable-pty runs setsid + TIOCSCTTY in the child: own session and group.
        let mut child = pair.slave.spawn_command(cmd).context("spawning claude")?;
        drop(pair.slave);
        let pid = child.process_id().context("no pid")? as i32;
        // SAFETY: getpgid on our own child.
        let pgid = unsafe { libc::getpgid(pid) };
        let pgid = if pgid > 0 { pgid } else { pid };
        let mut reader = pair.master.try_clone_reader().context("pty reader")?;
        let writer = pair.master.take_writer().context("pty writer")?;
        let output = Arc::new(Output::new());
        let exited = Arc::new(AtomicBool::new(false));
        let proc = PtyProcess {
            pid,
            pgid,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            output: output.clone(),
            exited: exited.clone(),
            spawned_at_ms: orch_protocol::now_ms(),
        };
        let mut log = raw_log.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
        let out2 = output.clone();
        let (answers_tx, answers_rx) = std::sync::mpsc::channel::<&'static [u8]>();
        let reader_thread = std::thread::Builder::new().name(format!("pty-read-{pid}")).spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Some(f) = log.as_mut() {
                            let _ = f.write_all(&buf[..n]);
                        }
                        for a in out2.push(&buf[..n]) {
                            let _ = answers_tx.send(a);
                        }
                    }
                }
            }
            out2.close();
        })?;
        std::thread::Builder::new().name(format!("pty-wait-{pid}")).spawn(move || {
            let status = child.wait().ok().map(|s| s.exit_code());
            let _ = reader_thread.join();
            exited.store(true, Ordering::SeqCst);
            on_exit(status);
        })?;
        // Terminal queries answered while detached (the recovery harness did the same).
        let p = proc.writer_handle();
        std::thread::Builder::new().name(format!("pty-answer-{pid}")).spawn(move || {
            while let Ok(a) = answers_rx.recv() {
                p(a);
            }
        })?;
        Ok(proc)
    }

    fn writer_handle(&self) -> impl Fn(&[u8]) + Send + 'static {
        // A duplicate of the master fd, so answers don't contend with the main writer.
        let fd = self.master.lock().ok().and_then(|m| m.as_raw_fd());
        let dup = fd.map(|fd| {
            // SAFETY: duplicating our own master fd, with close-on-exec.
            unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) }
        });
        let file = dup.filter(|d| *d >= 0).map(|d| {
            // SAFETY: `d` is a fresh descriptor we own.
            unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(d) }
        });
        let file = Mutex::new(file);
        move |bytes: &[u8]| {
            if let Ok(mut f) = file.lock() {
                if let Some(f) = f.as_mut() {
                    let _ = f.write_all(bytes);
                }
            }
        }
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let m = self.master.lock().unwrap_or_else(|e| e.into_inner());
        m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn read(&self) -> Vec<u8> {
        self.output.read()
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn master_fd(&self) -> Option<i32> {
        self.master.lock().ok().and_then(|m| m.as_raw_fd())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn cmd(program: &str, args: &[&str], cwd: &Path) -> Compiled {
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), "/usr/bin:/bin".into());
        env.insert("ORCH_PROBE".into(), "yes".into());
        Compiled {
            agent_dir: cwd.into(),
            settings_path: PathBuf::new(),
            mcp_path: PathBuf::new(),
            gitconfig_path: PathBuf::new(),
            settings: serde_json::Value::Null,
            mcp: serde_json::Value::Null,
            gitconfig: String::new(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env,
            cwd: cwd.into(),
        }
    }

    fn read_until(p: &PtyProcess, needle: &str, secs: u64) -> String {
        let end = Instant::now() + Duration::from_secs(secs);
        let mut all = Vec::new();
        while Instant::now() < end {
            all.extend(p.read());
            if String::from_utf8_lossy(&all).contains(needle) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        String::from_utf8_lossy(&all).into_owned()
    }

    #[test]
    fn own_session_env_from_scratch_cloexec_and_exit() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let script = "echo pid=$$ sid=$(ps -o sess= -p $$) probe=$ORCH_PROBE home=${HOME:-none} cwd=$(pwd); read line; echo got=$line";
        let p = PtyProcess::spawn(
            &cmd("/bin/sh", &["-c", script], d.path()),
            24,
            80,
            None,
            Box::new(move |s| tx.send(s).unwrap()),
        )
        .unwrap();
        p.output.set_attached(true);
        let out = read_until(&p, "cwd=", 5);
        assert!(out.contains("probe=yes"), "{out}");
        assert!(out.contains("home=none"), "env is from scratch: {out}");
        assert_eq!(p.pgid, p.pid, "own process group");
        // SAFETY: reading our child's session id.
        let sid = unsafe { libc::getsid(p.pid) };
        assert_eq!(sid, p.pid, "own session");
        let fd = p.master_fd().unwrap();
        // SAFETY: reading descriptor flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC != 0);
        p.write(b"hello\r").unwrap();
        let out = read_until(&p, "got=hello", 5);
        assert!(out.contains("got=hello"), "{out}");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Some(0));
        assert!(p.has_exited());
    }

    #[test]
    fn detached_buffer_keeps_only_the_tail_and_detects_trust_dialog() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let script = "head -c 600000 /dev/zero | tr '\\0' x; printf '\\033[1mQuick\\033[0m safety  check\\n'";
        let p = PtyProcess::spawn(
            &cmd("/bin/sh", &["-c", script], d.path()),
            24,
            80,
            None,
            Box::new(move |s| tx.send(s).unwrap()),
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let got = p.read();
        assert!(got.len() <= DETACHED_KEEP, "{}", got.len());
        assert!(p.output.total() >= 600_000);
        assert!(p.output.trust_dialog.load(Ordering::Relaxed));
    }

    #[test]
    fn ansi_stripping() {
        assert_eq!(strip_ansi("\x1b[1mA\x1b[0m\x1b]0;title\x07B\x1b(BC"), " A  B C");
    }
}
