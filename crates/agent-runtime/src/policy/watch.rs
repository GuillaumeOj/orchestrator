//! The policy files' watcher: one non-recursive watch per directory that
//! holds a known `policy.toml`, debounced (500 ms of quiet) before the
//! changed files are read again. Our own writes reload at once and are seen
//! here a second time (harmless).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::runtime::Runtime;

pub const DEBOUNCE: Duration = Duration::from_millis(500);

pub(super) struct Watch {
    watcher: notify::RecommendedWatcher,
    dirs: BTreeSet<PathBuf>,
}

fn is_policy_file(p: &std::path::Path) -> bool {
    p.file_name().is_some_and(|n| n == "policy.toml")
}

impl Runtime {
    /// Watches the directories of the known files (those that exist).
    pub(super) fn policy_watch(&self) {
        let want: BTreeSet<PathBuf> = self
            .policy
            .files()
            .iter()
            .filter_map(|f| f.path.parent().map(PathBuf::from))
            .filter(|d| d.is_dir())
            .collect();
        let mut slot = self.policy.watch.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|w| w.dirs == want) {
            return;
        }
        if slot.is_none() {
            let (tx, rx) = mpsc::channel::<PathBuf>();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    for p in ev.paths.into_iter().filter(|p| is_policy_file(p)) {
                        let _ = tx.send(p);
                    }
                }
            });
            let watcher = match watcher {
                Ok(w) => w,
                Err(e) => return eprintln!("warn: policy files not watched: {e}"),
            };
            let me = self.me.clone();
            std::thread::Builder::new()
                .name("policy-watch".into())
                .spawn(move || {
                    // Ends when the watcher (and its sender) is dropped with the Runtime.
                    while let Ok(first) = rx.recv() {
                        let mut changed = BTreeSet::from([first]);
                        while let Ok(p) = rx.recv_timeout(DEBOUNCE) {
                            changed.insert(p);
                        }
                        let Some(rt) = me.upgrade() else { break };
                        for p in changed {
                            rt.policy_reload_path(&p);
                        }
                    }
                })
                .ok();
            *slot = Some(Watch { watcher, dirs: BTreeSet::new() });
        }
        let Some(w) = slot.as_mut() else { return };
        for gone in w.dirs.difference(&want).cloned().collect::<Vec<_>>() {
            let _ = w.watcher.unwatch(&gone);
            w.dirs.remove(&gone);
        }
        for d in want {
            if !w.dirs.contains(&d) {
                match w.watcher.watch(&d, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        w.dirs.insert(d);
                    }
                    Err(e) => eprintln!("warn: {} not watched: {e}", d.display()),
                }
            }
        }
    }
}
