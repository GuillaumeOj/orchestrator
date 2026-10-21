//! `orchd`: the headless core for development. See README.md.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig};
use agent_runtime::server::{self, request};
use anyhow::{Context, Result, anyhow, bail};
use orch_protocol::{AgentSpec, CtlRequest, WorktreeKindSpec, bus};

const USAGE: &str = "\
orchd: headless Orchestrator core (dev)

usage: orchd [--home DIR] <command> [args]

  serve [--enforce] [--no-recover] [--cap N] [--hold-secs N] [--otel URL] [--claude PATH]
                                run the core in the foreground (Ctrl-C suspends every agent)
  install-helpers               copy orch-hook and orch-status next to this binary into <home>/bin
  reconcile                     dry run of the startup reconciler (core not running)
  worktree <repo> solo <slug> | integration <feature> | team <feature> <role> <n>  [--base B]
  start <agent> --repo <path> [--slug S] [--base B] [--model M] [--effort E] [--mode manual|acceptEdits|auto|dontAsk|plan]
                [--prompt TEXT] [--strict-mcp] [--allow-host H]... [--allow RULE]... [--tier low|normal|high|critical] [--message TEXT]
  send <agent> <text...>        queue a message (admitted by the Governor, delivered when idle)
  attach <agent>                connect this terminal to the agent's PTY (detach: Ctrl-])
  interrupt <agent>             Esc
  suspend <agent>               stop the process, keep the session
  resume <agent> [--message TEXT]
  kill <agent>                  process group + processes whose cwd is in the worktree
  pause-all | resume-all        Governor Pause all (FR-811): autonomous work holds at its next tool call
  state | usage | governor      print JSON
  stop                          shut the core down (suspends agents)

The home defaults to $ORCH_HOME or ~/Library/Application Support/Orchestrator.";

fn main() {
    if let Err(e) = run() {
        eprintln!("orchd: {e:#}");
        std::process::exit(1);
    }
}

struct Args(Vec<String>);

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.0.iter().position(|a| a == name) {
            self.0.remove(i);
            true
        } else {
            false
        }
    }

    fn opt(&mut self, name: &str) -> Result<Option<String>> {
        match self.0.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.0.len() => {
                let v = self.0.remove(i + 1);
                self.0.remove(i);
                Ok(Some(v))
            }
            Some(_) => bail!("{name} needs a value"),
            None => Ok(None),
        }
    }

    fn opts(&mut self, name: &str) -> Result<Vec<String>> {
        let mut v = vec![];
        while let Some(x) = self.opt(name)? {
            v.push(x);
        }
        Ok(v)
    }

    fn next(&mut self, what: &str) -> Result<String> {
        if self.0.is_empty() {
            bail!("missing {what}\n\n{USAGE}");
        }
        Ok(self.0.remove(0))
    }

    fn done(&self) -> Result<()> {
        if let Some(a) = self.0.first() {
            bail!("unexpected argument {a:?}");
        }
        Ok(())
    }
}

fn run() -> Result<()> {
    let mut a = Args(std::env::args().skip(1).collect());
    let home = a
        .opt("--home")?
        .or_else(|| std::env::var("ORCH_HOME").ok())
        .map(PathBuf::from)
        .unwrap_or_else(AppPaths::default_root);
    let paths = AppPaths::new(home);
    if a.0.is_empty() || a.flag("--help") || a.flag("-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let cmd = a.next("command")?;
    let sock = paths.socket();
    let print = |r: orch_protocol::CtlReply| -> Result<()> {
        println!("{}", serde_json::to_string_pretty(&r.data)?);
        if r.ok { Ok(()) } else { Err(anyhow!("request failed")) }
    };
    match cmd.as_str() {
        "serve" => serve(paths, a),
        "install-helpers" => {
            a.done()?;
            let exe_dir = std::env::current_exe()?.parent().map(PathBuf::from).context("no exe dir")?;
            let h = agent_runtime::preflight::install_helpers(&exe_dir, &paths.bin_dir())?;
            println!("installed {} and {}", h.hook.display(), h.status.display());
            Ok(())
        }
        "reconcile" => {
            a.done()?;
            if bus::is_listening(&sock) {
                bail!("the core is running: the reconciler runs at its start");
            }
            let rt = Runtime::new(RuntimeConfig::new(paths))?;
            println!("{}", serde_json::to_string_pretty(&rt.recover(false)?)?);
            Ok(())
        }
        "worktree" => {
            let repo = a.next("repo")?;
            let base = a.opt("--base")?;
            let kind = match a.next("kind")?.as_str() {
                "solo" => WorktreeKindSpec::Solo { slug: a.next("slug")? },
                "integration" => WorktreeKindSpec::Integration { feature: a.next("feature")? },
                "team" => WorktreeKindSpec::Team {
                    feature: a.next("feature")?,
                    role: a.next("role")?,
                    n: a.next("n")?.parse()?,
                },
                k => bail!("unknown worktree kind {k}"),
            };
            a.done()?;
            print(request(&sock, CtlRequest::CreateWorktree { repo: abs(&repo)?, kind, base })?)
        }
        "start" => {
            let agent_id = a.next("agent")?;
            let spec = AgentSpec {
                agent_id,
                repo: abs(&a.opt("--repo")?.context("--repo is required")?)?,
                slug: a.opt("--slug")?,
                base: a.opt("--base")?,
                model: a.opt("--model")?,
                effort: a.opt("--effort")?,
                permission_mode: a.opt("--mode")?,
                prompt: a.opt("--prompt")?,
                strict_mcp: Some(a.flag("--strict-mcp")),
                network_allow: a.opts("--allow-host")?,
                allow_rules: a.opts("--allow")?,
                allocation: a.opt("--allocation")?,
                tier: a.opt("--tier")?,
                first_message: a.opt("--message")?,
            };
            a.done()?;
            print(request(&sock, CtlRequest::Start { spec: Box::new(spec) })?)
        }
        "send" => {
            let agent_id = a.next("agent")?;
            if a.0.is_empty() {
                bail!("missing text");
            }
            let text = a.0.join(" ");
            print(request(&sock, CtlRequest::Deliver { agent_id, text })?)
        }
        "resume" => {
            let agent_id = a.next("agent")?;
            let message = a.opt("--message")?;
            a.done()?;
            print(request(&sock, CtlRequest::Resume { agent_id, message })?)
        }
        "interrupt" | "suspend" | "kill" => {
            let agent_id = a.next("agent")?;
            a.done()?;
            let req = match cmd.as_str() {
                "interrupt" => CtlRequest::Interrupt { agent_id },
                "suspend" => CtlRequest::Suspend { agent_id },
                _ => CtlRequest::Kill { agent_id },
            };
            print(request(&sock, req)?)
        }
        "state" | "usage" | "governor" => {
            a.done()?;
            let req = match cmd.as_str() {
                "state" => CtlRequest::State,
                "usage" => CtlRequest::Usage,
                _ => CtlRequest::Governor,
            };
            print(request(&sock, req)?)
        }
        "pause-all" | "resume-all" => {
            a.done()?;
            print(request(&sock, if cmd == "pause-all" { CtlRequest::PauseAll } else { CtlRequest::ResumeAll })?)
        }
        "stop" => {
            a.done()?;
            print(request(&sock, CtlRequest::Shutdown)?)
        }
        "attach" => {
            let agent_id = a.next("agent")?;
            a.done()?;
            attach(&sock, &agent_id)
        }
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn abs(p: &str) -> Result<String> {
    Ok(std::fs::canonicalize(p).with_context(|| format!("no such path {p}"))?.display().to_string())
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn serve(paths: AppPaths, mut a: Args) -> Result<()> {
    let mut cfg = RuntimeConfig::new(paths.clone());
    cfg.shadow = !a.flag("--enforce");
    let recover = !a.flag("--no-recover");
    if let Some(c) = a.opt("--cap")? {
        cfg.base_cap = c.parse()?;
    }
    if let Some(h) = a.opt("--hold-secs")? {
        cfg.hold = Duration::from_secs(h.parse()?);
    }
    cfg.otel_endpoint = a.opt("--otel")?;
    if let Some(c) = a.opt("--claude")? {
        cfg.claude = PathBuf::from(c);
    }
    a.done()?;
    // Refuse early if another core is up (before touching any agent).
    if bus::is_listening(&paths.socket()) {
        bail!("another core is listening on {}", paths.socket().display());
    }
    if agent_runtime::preflight::check_helpers(&cfg.helpers).is_err() {
        eprintln!(
            "orchd: helpers missing in {}: run `orchd install-helpers` (agents can't start without them)",
            paths.bin_dir().display()
        );
    }
    let rt = Runtime::new(cfg)?;
    for w in &rt.warnings {
        eprintln!("orchd: warning: {w}");
    }
    if recover {
        let r = rt.recover(true)?;
        eprintln!(
            "orchd: reconciled: {} spooled event(s), {} worktree(s), resumed {:?}",
            r.spool.iter().map(|s| s.events.len()).sum::<usize>(),
            r.worktrees.len(),
            r.to_resume()
        );
    }
    let listener = bus::bind_listener(&paths.socket())?;
    // SAFETY: installing a handler that only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (stop, rt) = (stop.clone(), rt.clone());
        std::thread::spawn(move || {
            while !STOP.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if STOP.load(Ordering::SeqCst) {
                eprintln!("orchd: stopping: suspending agents");
                rt.shutdown();
                stop.store(true, Ordering::SeqCst);
            }
        });
    }
    eprintln!(
        "orchd: core {} listening on {} (governor {})",
        std::process::id(),
        paths.socket().display(),
        if rt.cfg.shadow { "in shadow mode" } else { "enforced" }
    );
    server::serve(rt, listener, stop)?;
    let _ = std::fs::remove_file(paths.socket());
    eprintln!("orchd: stopped");
    Ok(())
}

// ------------------------------------------------------------ attach

fn winsize() -> (u16, u16) {
    // SAFETY: TIOCGWINSZ fills a winsize struct we own.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
            (ws.ws_row, ws.ws_col)
        } else {
            (50, 160)
        }
    }
}

struct RawMode(Option<libc::termios>);

impl RawMode {
    fn enable() -> RawMode {
        // SAFETY: termios calls on our own stdin.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
                return RawMode(None);
            }
            let saved = t;
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            RawMode(Some(saved))
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(t) = self.0 {
            // SAFETY: restoring the saved attributes.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
}

const DETACH: u8 = 0x1d; // Ctrl-]

fn attach(sock: &std::path::Path, agent_id: &str) -> Result<()> {
    let (rows, cols) = winsize();
    let (s, leftover) = server::attach_client(sock, agent_id, rows, cols)?;
    eprint!("[attached to {agent_id}; detach with Ctrl-]]\r\n");
    let raw = RawMode::enable();
    let done = Arc::new(AtomicBool::new(false));
    // Core → terminal.
    let out_thread = {
        let done = done.clone();
        let mut sock_r = s.try_clone()?;
        std::thread::spawn(move || {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(&leftover);
            let _ = stdout.flush();
            let mut buf = [0u8; 65536];
            loop {
                match sock_r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() {
                            break;
                        }
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // Window size changes.
    {
        let (done, sock, id) = (done.clone(), sock.to_path_buf(), agent_id.to_string());
        std::thread::spawn(move || {
            let mut last = (rows, cols);
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(300));
                let now = winsize();
                if now != last {
                    last = now;
                    let _ = request(&sock, CtlRequest::Resize { agent_id: id.clone(), rows: now.0, cols: now.1 });
                }
            }
        });
    }
    // Terminal → core. stdin is read on a thread; the main thread watches for the end.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut w = s;
    while !done.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                if let Some(i) = bytes.iter().position(|b| *b == DETACH) {
                    let _ = w.write_all(&bytes[..i]);
                    break;
                }
                if w.write_all(&bytes).is_err() {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    let _ = w.shutdown(std::net::Shutdown::Both);
    let _ = out_thread.join();
    drop(raw);
    eprint!("\r\n[detached from {agent_id}]\r\n");
    Ok(())
}
