//! Worktree file watchers (P4.1 acceptance 6): an open Diff pane refreshes
//! when files change in its worktree, whoever changed them (an agent, the
//! user's editor or shell). One recursive FSEvents watcher per watched
//! worktree, ref-counted, raising the `worktree:<path>` topic at most once a
//! second, with a trailing one. Build output, dependencies and git's object
//! store are ignored; the worktree's HEAD and index count.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use notify::{RecursiveMode, Watcher};

use crate::runtime::Runtime;

/// At most one topic per worktree this often (a trailing one follows).
pub const DEBOUNCE: Duration = Duration::from_secs(1);

/// Directories whose changes never refresh a diff.
const IGNORED_DIRS: [&str; 2] = ["target", "node_modules"];

/// Whether a change at `path` (inside the watched worktree `root`, or in its
/// git dir `git_dir`) refreshes the diff.
pub fn counts(root: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    if let Some(g) = git_dir
        && let Ok(rel) = path.strip_prefix(g)
    {
        // The git dir: only what moves the diff (a checkout, a commit, staging).
        return matches!(rel.to_str(), Some("HEAD" | "index"));
    }
    let Ok(rel) = path.strip_prefix(root) else { return false };
    let mut parts = rel.components().filter_map(|c| match c {
        Component::Normal(s) => s.to_str(),
        _ => None,
    });
    let Some(first) = parts.next() else { return true };
    if first == ".git" {
        // A main checkout's own git dir.
        return matches!(parts.next(), Some("HEAD" | "index")) && parts.next().is_none();
    }
    let all = std::iter::once(first).chain(parts);
    !all.into_iter().any(|p| IGNORED_DIRS.contains(&p))
}

/// The git dir of a linked worktree (`.git` is a file: `gitdir: <path>`).
fn linked_git_dir(root: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(root.join(".git")).ok()?;
    let dir = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
    Some(if dir.is_absolute() { dir } else { root.join(dir) })
}

/// One live watcher: dropping it stops the FS watch and its thread.
pub struct WorktreeWatch {
    _watcher: notify::RecommendedWatcher,
}

impl WorktreeWatch {
    /// Watches `root` (canonical) and calls `emit` at most once per
    /// `debounce`, then once more after the last change of a burst.
    pub fn start(root: PathBuf, debounce: Duration, emit: impl Fn() + Send + 'static) -> Result<WorktreeWatch> {
        let git_dir = linked_git_dir(&root).and_then(|d| d.canonicalize().ok());
        let (tx, rx) = channel::<()>();
        let (r, g) = (root.clone(), git_dir.clone());
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| counts(&r, g.as_deref(), p))
            {
                let _ = tx.send(());
            }
        })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;
        if let Some(g) = &git_dir {
            watcher.watch(g, RecursiveMode::NonRecursive)?;
        }
        // The sender lives in the watcher: dropping the watcher ends the thread.
        std::thread::Builder::new().name("orch-wt-watch".into()).spawn(move || debounce_loop(&rx, debounce, emit))?;
        Ok(WorktreeWatch { _watcher: watcher })
    }
}

/// Calls `emit` for the changes `rx` receives: at once for the first, then
/// at most once per `debounce`, with a trailing call after the last change
/// of a burst. Returns when the sender is gone.
fn debounce_loop(rx: &Receiver<()>, debounce: Duration, emit: impl Fn()) {
    let mut last: Option<Instant> = None;
    let mut pending = false;
    loop {
        let wait = match (pending, last) {
            (true, Some(t)) => debounce.saturating_sub(t.elapsed()),
            _ => Duration::from_secs(3600),
        };
        match rx.recv_timeout(wait) {
            Ok(()) => pending = true,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if pending && last.is_none_or(|t| t.elapsed() >= debounce) {
            emit();
            last = Some(Instant::now());
            pending = false;
        }
    }
}

/// The watched worktrees and how many panes watch each.
#[derive(Default)]
pub struct Watches(Mutex<BTreeMap<PathBuf, (usize, WorktreeWatch)>>);

impl Runtime {
    /// A worktree the app knows: a registered project's main checkout, or a
    /// worktree of one. Anything else is refused (no arbitrary watching).
    fn known_worktree(&self, path: &Path) -> Result<PathBuf> {
        let p = path.canonicalize().map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let canon = |q: &Path| q.canonicalize().ok();
        let projects = self.db.projects(false)?;
        let registered = |repo: &Path| {
            projects.iter().any(|pr| canon(&pr.path).is_some_and(|c| c == canon(repo).unwrap_or_default()))
        };
        let known = projects.iter().any(|pr| canon(&pr.path).as_deref() == Some(&p))
            || self
                .store
                .read(|s| s.worktrees.values().any(|w| canon(&w.path).as_deref() == Some(&p) && registered(&w.repo)));
        if !known {
            bail!("{} is not a worktree of a registered project", path.display());
        }
        Ok(p)
    }

    /// `watch_worktree`: a Diff pane opened on `path`. The first one starts
    /// the watcher; the topic is `worktree:<path>` as the pane asked it.
    pub fn watch_worktree(&self, path: &Path) -> Result<()> {
        let canon = self.known_worktree(path)?;
        let mut w = self.watches.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((n, _)) = w.get_mut(&canon) {
            *n += 1;
            return Ok(());
        }
        let me = self.me.clone();
        let shown = path.to_path_buf();
        let watch = WorktreeWatch::start(canon.clone(), DEBOUNCE, move || {
            if let Some(rt) = me.upgrade() {
                rt.touch_worktree(&shown);
            }
        })?;
        w.insert(canon, (1, watch));
        Ok(())
    }

    /// `unwatch_worktree`: a Diff pane closed; the last one stops the watcher.
    pub fn unwatch_worktree(&self, path: &Path) {
        let Ok(canon) = path.canonicalize() else { return };
        let mut w = self.watches.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((n, _)) = w.get_mut(&canon) {
            *n -= 1;
            if *n == 0 {
                w.remove(&canon);
            }
        }
    }

    /// How many panes watch `path` (tests, diagnostics).
    pub fn watchers_of(&self, path: &Path) -> usize {
        let Ok(canon) = path.canonicalize() else { return 0 };
        self.watches.0.lock().unwrap_or_else(|e| e.into_inner()).get(&canon).map_or(0, |(n, _)| *n)
    }

    /// Stops every watcher (shutdown).
    pub(crate) fn unwatch_all(&self) {
        self.watches.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Counts emits, for tests.
#[cfg(test)]
fn counter() -> (std::sync::Arc<std::sync::atomic::AtomicUsize>, impl Fn() + Send + 'static) {
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let m = n.clone();
    (n, move || {
        m.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::Builder::new().prefix("orchwt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let p = d.path().canonicalize().unwrap();
        (d, p)
    }

    /// FSEvents delivers with some latency; settle before counting.
    fn settle() {
        std::thread::sleep(Duration::from_millis(1600));
    }

    #[test]
    fn the_debounce_sends_one_then_at_most_one_more_per_second() {
        let (n, emit) = counter();
        let (tx, rx) = channel::<()>();
        let debounce = Duration::from_millis(300);
        let t = std::thread::spawn(move || debounce_loop(&rx, debounce, emit));
        tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(n.load(Ordering::SeqCst), 1, "the first change at once");
        // A burst inside the window: one trailing call after it.
        for _ in 0..50 {
            tx.send(()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(n.load(Ordering::SeqCst), 1, "held back");
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(n.load(Ordering::SeqCst), 2, "the trailing one");
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(n.load(Ordering::SeqCst), 2, "nothing more without changes");
        drop(tx);
        t.join().unwrap();
    }

    #[test]
    fn what_counts() {
        let root = Path::new("/w/back-1");
        let git = Path::new("/repo/.git/worktrees/back-1");
        let c = |p: &str| counts(root, Some(git), Path::new(p));
        assert!(c("/w/back-1/src/a.rs"));
        assert!(c("/w/back-1"));
        assert!(!c("/w/back-1/target/debug/x"));
        assert!(!c("/w/back-1/ui/node_modules/a/b.js"));
        assert!(!c("/elsewhere/a.rs"));
        assert!(c("/repo/.git/worktrees/back-1/HEAD"));
        assert!(c("/repo/.git/worktrees/back-1/index"));
        assert!(!c("/repo/.git/worktrees/back-1/logs/HEAD"));
        let main = |p: &str| counts(Path::new("/repo"), None, Path::new(p));
        assert!(main("/repo/.git/HEAD") && main("/repo/.git/index"));
        assert!(!main("/repo/.git/objects/ab/cdef") && !main("/repo/.git/index.lock"));
    }

    /// Real FSEvents (macOS CI): a sandbox without the FSEvents service gets
    /// no events, so this one runs outside it.
    #[test]
    fn one_topic_per_change_two_per_burst_none_for_ignored() {
        let (_d, root) = tmp();
        std::fs::create_dir_all(root.join("target")).unwrap();
        let (n, emit) = counter();
        let _w = WorktreeWatch::start(root.clone(), DEBOUNCE, emit).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // An ignored path: nothing.
        std::fs::write(root.join("target/out.o"), "x").unwrap();
        settle();
        assert_eq!(n.load(Ordering::SeqCst), 0, "ignored paths raise nothing");
        // One write: one topic within ~1.5 s.
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let t = Instant::now();
        while n.load(Ordering::SeqCst) == 0 && t.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
        settle();
        // 50 writes in 200 ms: at most 2 (the leading one and the trailing one).
        let before = n.load(Ordering::SeqCst);
        for i in 0..50 {
            std::fs::write(root.join(format!("b{i}.txt")), "b").unwrap();
            std::thread::sleep(Duration::from_millis(4));
        }
        settle();
        let burst = n.load(Ordering::SeqCst) - before;
        assert!((1..=2).contains(&burst), "{burst}");
    }

    #[test]
    fn only_registered_worktrees_and_ref_counted() {
        let (_d, rt) = crate::runtime::tests::test_runtime(true);
        let (_r, other) = tmp();
        let err = rt.watch_worktree(&other).unwrap_err().to_string();
        assert!(err.contains("not a worktree of a registered project"), "{err}");
        let (_s, repo) = orch_git::fixtures::scratch_repo();
        orch_git::project::register(&rt.db, &repo).unwrap();
        rt.watch_worktree(&repo).unwrap();
        rt.watch_worktree(&repo).unwrap();
        assert_eq!(rt.watchers_of(&repo), 2);
        rt.unwatch_worktree(&repo);
        assert_eq!(rt.watchers_of(&repo), 1);
        rt.unwatch_worktree(&repo);
        assert_eq!(rt.watchers_of(&repo), 0, "the last pane stops it");
        rt.unwatch_worktree(&repo); // one too many: harmless
    }

    /// Through the runtime (real FSEvents, like the test above): a write in a
    /// watched worktree is a `worktree:<path>` topic in the `now` push.
    #[test]
    fn a_write_in_a_watched_worktree_is_a_now_topic() {
        let (_d, rt) = crate::runtime::tests::test_runtime(true);
        let got = std::sync::Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        rt.subscribe_now(std::sync::Arc::new(move |v| g.lock().unwrap().push(v.clone())));
        let (_s, repo) = orch_git::fixtures::scratch_repo();
        orch_git::project::register(&rt.db, &repo).unwrap();
        rt.now_pump(false);
        rt.watch_worktree(&repo).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        let topic = format!("worktree:{}", repo.display());
        let t = Instant::now();
        let seen = || {
            got.lock().unwrap().iter().any(|e| e["topics"].as_array().is_some_and(|a| a.iter().any(|x| x == &topic)))
        };
        while !seen() && t.elapsed() < Duration::from_secs(3) {
            rt.now_pump(false);
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen(), "{topic}");
        rt.shutdown();
        assert_eq!(rt.watchers_of(&repo), 0, "nothing left after shutdown");
    }
}
