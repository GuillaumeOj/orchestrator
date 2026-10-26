//! Development harness, off unless `ORCH_DEV_CTL=<socket>` is set (as in the
//! P0 terminal spike): each JSON line on that socket is emitted to the UI as a
//! `dev` event (run a command, evaluate a snippet), and the UI's `dev_report`
//! lines are appended to `ORCH_DEV_LOG`. Used for screenshots and measurements
//! without driving the real keyboard; it bypasses AppKit, like P0's synthetic keys.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

use tauri::{AppHandle, Emitter, Runtime};

pub fn start<R: Runtime>(app: AppHandle<R>) {
    let Ok(path) = std::env::var("ORCH_DEV_CTL") else { return };
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else { return };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    let _ = app.emit("dev", v);
                }
            }
        }
    });
}

#[tauri::command]
pub fn dev_report(msg: String) {
    if let Ok(p) = std::env::var("ORCH_DEV_LOG") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

pub fn enabled() -> bool {
    std::env::var("ORCH_DEV_CTL").is_ok()
}
