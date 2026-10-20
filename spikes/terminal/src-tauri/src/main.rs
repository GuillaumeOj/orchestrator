//! Terminal spike: the Rust core owns PTYs and streams their output to xterm.js
//! over Tauri 2 Channels as raw bytes. A Unix control socket ($SPIKE_CTL) lets
//! scripts drive the UI; the UI reports results back through `report`.

mod kitty;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, State};

/// Flow control: the reader stops once this many bytes are sent but not yet
/// parsed by xterm.js, and resumes below LOW.
const HIGH: u64 = 2 << 20;
const LOW: u64 = 512 << 10;

struct Flow {
    unacked: Mutex<u64>,
    cv: Condvar,
    /// Pull mode: output waiting for the UI's next `pty_read`.
    pending: Mutex<(Vec<u8>, bool)>,
    data_cv: Condvar,
}

struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    flow: Arc<Flow>,
}

#[derive(Default)]
struct Ptys {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Pty>>,
}

fn log_line(s: &str) {
    if let Ok(p) = std::env::var("SPIKE_LOG") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{s}");
        }
    }
    eprintln!("{s}");
}

/// Minimal env for the login shell, as if the app was launched from Finder:
/// the shell's own profile builds the rest (PATH from nvm, bob, …).
fn base_env(cmd: &mut CommandBuilder) {
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
    cmd.env("TERM_PROGRAM", "OrchTerminalSpike");
}

#[tauri::command]
fn pty_spawn(
    state: State<'_, Arc<Ptys>>,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<i32>,
    pull: Option<bool>,
) -> Result<u32, String> {
    let pull = pull.unwrap_or(false);
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;
    // new_default_prog runs $SHELL as a login shell (argv0 "-zsh").
    let mut cmd = CommandBuilder::new_default_prog();
    base_env(&mut cmd);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    cmd.cwd(cwd.unwrap_or(home));
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let flow = Arc::new(Flow {
        unacked: Mutex::new(0),
        cv: Condvar::new(),
        pending: Mutex::new((Vec::new(), false)),
        data_cv: Condvar::new(),
    });
    let f2 = flow.clone();
    let coalesce_ms: u64 = std::env::var("SPIKE_COALESCE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let fd = pair.master.as_raw_fd().ok_or("no fd")?;
    std::thread::Builder::new()
        .name(format!("pty-{id}"))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut total: u64 = 0;
            let start = Instant::now();
            loop {
                {
                    let mut u = f2.unacked.lock().unwrap();
                    if *u > HIGH {
                        while *u > LOW {
                            u = f2.cv.wait(u).unwrap();
                        }
                    }
                }
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        total += n as u64;
                        let mut chunk = buf[..n].to_vec();
                        // Optional coalescing: keep reading for a few ms so the UI
                        // gets fewer, larger messages under heavy output.
                        if coalesce_ms > 0 && n >= 1024 {
                            let deadline = Instant::now() + std::time::Duration::from_millis(coalesce_ms);
                            while chunk.len() < 512 * 1024 && Instant::now() < deadline {
                                if !readable(fd, deadline) { break; }
                                match reader.read(&mut buf) {
                                    Ok(0) | Err(_) => break,
                                    Ok(m) => { total += m as u64; chunk.extend_from_slice(&buf[..m]); }
                                }
                            }
                        }
                        if pull {
                            let mut p = f2.pending.lock().unwrap();
                            while p.0.len() > HIGH as usize {
                                p = f2.data_cv.wait(p).unwrap();
                            }
                            p.0.extend_from_slice(&chunk);
                            f2.data_cv.notify_all();
                            continue;
                        }
                        *f2.unacked.lock().unwrap() += chunk.len() as u64;
                        if on_data.send(InvokeResponseBody::Raw(chunk)).is_err() {
                            break;
                        }
                    }
                }
            }
            f2.pending.lock().unwrap().1 = true;
            f2.data_cv.notify_all();
            log_line(&format!("pty {id} closed after {total} bytes, {:.1}s", start.elapsed().as_secs_f64()));
            let _ = on_exit.send(0);
        })
        .map_err(|e| e.to_string())?;
    state.map.lock().unwrap().insert(id, Pty { master: pair.master, writer, child, flow });
    Ok(id)
}

#[tauri::command]
fn pty_write(state: State<'_, Arc<Ptys>>, id: u32, data: String) -> Result<(), String> {
    if std::env::var("SPIKE_KEYLOG").is_ok() {
        log_line(&format!("keylog pty={id} bytes={:?}", data.escape_debug().to_string()));
    }
    let mut map = state.map.lock().unwrap();
    let p = map.get_mut(&id).ok_or("no pty")?;
    p.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())
}

/// Pull mode: wait for output and return all of it as raw bytes (one IPC
/// request per batch, no eval, natural backpressure).
#[tauri::command]
async fn pty_read(state: State<'_, Arc<Ptys>>, id: u32) -> Result<tauri::ipc::Response, String> {
    let flow = state.map.lock().unwrap().get(&id).map(|p| p.flow.clone()).ok_or("no pty")?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut p = flow.pending.lock().unwrap();
        while p.0.is_empty() && !p.1 {
            p = flow.data_cv.wait(p).unwrap();
        }
        if p.0.is_empty() {
            return Err("eof".to_string());
        }
        let data = std::mem::take(&mut p.0);
        flow.data_cv.notify_all();
        Ok(tauri::ipc::Response::new(data))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn pty_ack(state: State<'_, Arc<Ptys>>, id: u32, n: u64) {
    if let Some(p) = state.map.lock().unwrap().get(&id) {
        let mut u = p.flow.unacked.lock().unwrap();
        *u = u.saturating_sub(n);
        p.flow.cv.notify_all();
    }
}

#[tauri::command]
fn pty_resize(state: State<'_, Arc<Ptys>>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let map = state.map.lock().unwrap();
    let p = map.get(&id).ok_or("no pty")?;
    p.master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_kill(state: State<'_, Arc<Ptys>>, id: u32) {
    if let Some(mut p) = state.map.lock().unwrap().remove(&id) {
        // Kill the whole process group, not only the shell (P0: orphans survive).
        if let Some(pid) = p.child.process_id() {
            unsafe_killpg(pid);
        }
        let _ = p.child.kill();
        p.flow.cv.notify_all();
    }
}

/// poll(2) the PTY master until it is readable or the deadline passes.
fn readable(fd: i32, deadline: Instant) -> bool {
    let left = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
    let mut p = PollFd { fd, events: 1 /* POLLIN */, revents: 0 };
    unsafe { poll(&mut p, 1, left) > 0 }
}

#[repr(C)]
struct PollFd { fd: i32, events: i16, revents: i16 }
extern "C" { fn poll(fds: *mut PollFd, n: u32, timeout: i32) -> i32; }

fn unsafe_killpg(pid: u32) {
    let _ = std::process::Command::new("/bin/kill").args(["-HUP", &format!("-{pid}")]).status();
}

#[tauri::command]
fn kitty_config() -> kitty::KittyImport {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("SPIKE_KITTY_CONF").unwrap_or(format!("{home}/.config/kitty/kitty.conf"));
    kitty::import(std::path::Path::new(&path))
}

/// WKWebView hides user-installed fonts (~/Library/Fonts) from web content, so
/// the core finds the family's files with CoreText and the UI loads them with
/// the FontFace API.
#[tauri::command]
fn font_files(family: String) -> Vec<(String, String)> {
    let Some(coll) = core_text::font_collection::create_for_family(&family) else { return vec![] };
    let Some(descs) = coll.get_descriptors() else { return vec![] };
    descs
        .iter()
        .filter_map(|d| Some((d.font_path()?.display().to_string(), d.style_name())))
        .collect()
}

#[tauri::command]
fn read_font(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
}

#[tauri::command]
fn report(msg: String) {
    log_line(&format!("report {msg}"));
}

fn control_socket(app: tauri::AppHandle) {
    let Ok(path) = std::env::var("SPIKE_CTL") else { return };
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return log_line(&format!("ctl bind failed: {e}")),
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(v) => {
                        let _ = app.emit("ctl", v);
                    }
                    Err(e) => log_line(&format!("ctl bad json: {e}")),
                }
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
        .manage(Arc::new(Ptys::default()))
        .setup(|app| {
            control_socket(app.handle().clone());
            log_line(&format!("app pid {}", std::process::id()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pty_spawn, pty_write, pty_read, pty_ack, pty_resize, pty_kill, kitty_config, font_files, read_font, report
        ])
        .run(tauri::generate_context!())
        .expect("tauri run");
}
