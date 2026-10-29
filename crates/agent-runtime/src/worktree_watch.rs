//! Worktree file watchers (P4.1 acceptance 6): an open Diff pane refreshes
//! when files change in its worktree, whoever changed them (an agent, the
//! user's editor or shell). One recursive FSEvents watcher per watched
//! worktree, ref-counted; a change marks it dirty and the 200 ms pump raises
//! the `worktree:<path>` topic, at once and then at most once a second (a
//! burst ends with a trailing one). Build output, dependencies and git's
//! internals are ignored; the worktree's HEAD and index count.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use notify::{RecursiveMode, Watcher};

use crate::runtime::Runtime;

/// At most one topic per worktree this often (a trailing one follows).
const DEBOUNCE: Duration = Duration::from_secs(1);

/// Directories whose changes never refresh a diff.
const IGNORED_DIRS: [&str; 2] = ["target", "node_modules"];

/// Whether a change at `path` refreshes the diff of the worktree `root`
/// whose git dir is `git_dir` (`<root>/.git`, or a linked worktree's own).
fn counts(root: &Path, git_dir: &Path, path: &Path) -> bool {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        // The git dir: only what moves the diff (a checkout, a commit, staging).
        return matches!(rel.to_str(), Some("HEAD" | "index"));
    }
    let Ok(rel) = path.strip_prefix(root) else { return false };
    !rel.components()
        .any(|c| matches!(c, Component::Normal(s) if s.to_str().is_some_and(|s| IGNORED_DIRS.contains(&s))))
}

/// Raises a topic at once, then at most once per `every`: the pump asks.
#[derive(Default)]
struct Throttle {
    dirty: Arc<AtomicBool>,
    last: Option<Instant>,
}

impl Throttle {
    /// A change is waiting and the last topic is old enough: raise one now.
    fn due(&mut self, now: Instant, every: Duration) -> bool {
        if self.last.is_some_and(|t| now.duration_since(t) < every) || !self.dirty.swap(false, Ordering::SeqCst) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// One live watcher: dropping it stops the FS watch.
struct Watch {
    panes: usize,
    throttle: Throttle,
    _watcher: notify::RecommendedWatcher,
}

/// Watches `root` (canonical): every change that [`counts`] sets `dirty`.
fn start(root: &Path, dirty: Arc<AtomicBool>) -> Result<notify::RecommendedWatcher> {
    // A linked worktree's git dir (its `.git` is a `gitdir:` file), else its own `.git`.
    let linked = orch_git::repair::check_links(root).gitdir;
    let git_dir = linked.clone().unwrap_or_else(|| root.join(".git"));
    let r = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        // An error (events dropped, say) may hide a change: refetch to be sure.
        if res.map_or(true, |ev| ev.paths.iter().any(|p| counts(&r, &git_dir, p))) {
            dirty.store(true, Ordering::SeqCst);
        }
    })?;
    // One batch: FSEvents restarts its stream once, not once per path.
    let mut paths = watcher.paths_mut();
    paths.add(root, RecursiveMode::Recursive)?;
    if let Some(g) = &linked {
        paths.add(g, RecursiveMode::NonRecursive)?;
    }
    paths.commit()?;
    Ok(watcher)
}

/// The watched worktrees, keyed by the path the panes passed (their topic,
/// and what `unwatch_worktree` gets back even once the folder is gone).
#[derive(Default)]
pub struct Watches(Mutex<BTreeMap<PathBuf, Watch>>);

impl Runtime {
    fn watches(&self) -> std::sync::MutexGuard<'_, BTreeMap<PathBuf, Watch>> {
        self.watches.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A worktree the app knows: a registered project's main checkout, or a
    /// worktree of one (git's, whoever made it). Anything else is refused.
    fn known_worktree(&self, path: &Path) -> Result<PathBuf> {
        let p = path.canonicalize().map_err(|e| anyhow!("{}: {e}", path.display()))?;
        // Projects are stored at their real main checkout (`project::register`).
        let main = orch_git::worktree::main_checkout(&p).unwrap_or_else(|_| p.clone());
        if self.db.project_by_path(&main)?.is_none() {
            bail!("{} is not a worktree of a registered project", path.display());
        }
        Ok(p)
    }

    /// `watch_worktree`: a Diff pane opened on `path`. The first one starts
    /// the watcher; the topic is `worktree:<path>` as the pane asked it.
    pub fn watch_worktree(&self, path: &Path) -> Result<()> {
        // The check (a git call, a DB read) needs no lock. From the lookup to
        // the insert the lock is held (an FSEvents recursive watch starts at
        // once): a pane closed meanwhile finds this one counted.
        let root = self.known_worktree(path)?;
        let mut w = self.watches();
        if let Some(e) = w.get_mut(path) {
            e.panes += 1;
            return Ok(());
        }
        let throttle = Throttle::default();
        let watcher = start(&root, throttle.dirty.clone())?;
        w.insert(path.to_path_buf(), Watch { panes: 1, throttle, _watcher: watcher });
        Ok(())
    }

    /// `unwatch_worktree`: a Diff pane closed; the last one stops the watcher.
    pub fn unwatch_worktree(&self, path: &Path) {
        let gone = {
            let mut w = self.watches();
            let Some(e) = w.get_mut(path) else { return };
            e.panes -= 1;
            if e.panes > 0 {
                return;
            }
            w.remove(path)
        };
        drop(gone); // stops the FS watch outside the lock
    }

    /// On the pump: the topics of the watched worktrees that changed.
    pub(crate) fn flush_watches(&self) {
        let now = Instant::now();
        let due: Vec<PathBuf> =
            self.watches().iter_mut().filter_map(|(p, w)| w.throttle.due(now, DEBOUNCE).then(|| p.clone())).collect();
        for p in due {
            self.touch_worktree(&p);
        }
    }

    /// Stops every watcher (shutdown).
    pub(crate) fn unwatch_all(&self) {
        let all = std::mem::take(&mut *self.watches());
        drop(all); // stops the FS watches outside the lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::Builder::new().prefix("orchwt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let p = d.path().canonicalize().unwrap();
        (d, p)
    }

    fn panes(rt: &Runtime, path: &Path) -> usize {
        rt.watches().get(path).map_or(0, |w| w.panes)
    }

    /// The pump, every 50 ms for `d`: how many topics `t` raised.
    fn pump_for(t: &mut Throttle, d: Duration) -> usize {
        let (end, mut n) = (Instant::now() + d, 0);
        while Instant::now() < end {
            n += usize::from(t.due(Instant::now(), DEBOUNCE));
            std::thread::sleep(Duration::from_millis(50));
        }
        n
    }

    #[test]
    fn a_topic_at_once_then_at_most_one_per_second() {
        let mut t = Throttle::default();
        let t0 = Instant::now();
        assert!(!t.due(t0, DEBOUNCE), "nothing changed");
        t.dirty.store(true, Ordering::SeqCst);
        assert!(t.due(t0, DEBOUNCE), "the first change at once");
        t.dirty.store(true, Ordering::SeqCst);
        assert!(!t.due(t0 + Duration::from_millis(200), DEBOUNCE), "held back inside the second");
        assert!(t.due(t0 + Duration::from_millis(1000), DEBOUNCE), "the trailing one");
        assert!(!t.due(t0 + Duration::from_millis(3000), DEBOUNCE), "nothing more without changes");
    }

    #[test]
    fn what_counts() {
        let root = Path::new("/w/back-1");
        let git = Path::new("/repo/.git/worktrees/back-1");
        let c = |p: &str| counts(root, git, Path::new(p));
        assert!(c("/w/back-1/src/a.rs"));
        assert!(c("/w/back-1"));
        assert!(!c("/w/back-1/target/debug/x"));
        assert!(!c("/w/back-1/ui/node_modules/a/b.js"));
        assert!(!c("/elsewhere/a.rs"));
        assert!(c("/repo/.git/worktrees/back-1/HEAD"));
        assert!(c("/repo/.git/worktrees/back-1/index"));
        assert!(!c("/repo/.git/worktrees/back-1/logs/HEAD"));
        let main = |p: &str| counts(Path::new("/repo"), Path::new("/repo/.git"), Path::new(p));
        assert!(main("/repo/.git/HEAD") && main("/repo/.git/index"));
        assert!(!main("/repo/.git/objects/ab/cdef") && !main("/repo/.git/index.lock"));
    }

    /// Real FSEvents (macOS CI): a sandbox without the FSEvents service gets
    /// no events, so this one runs outside it.
    #[test]
    fn one_topic_per_change_two_per_burst_none_for_ignored() {
        let (_d, root) = tmp();
        std::fs::create_dir_all(root.join("target")).unwrap();
        let mut t = Throttle::default();
        let _w = start(&root, t.dirty.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // An ignored path: nothing.
        std::fs::write(root.join("target/out.o"), "x").unwrap();
        assert_eq!(pump_for(&mut t, Duration::from_millis(1600)), 0, "ignored paths raise nothing");
        // One write: one topic within ~1.5 s.
        std::fs::write(root.join("a.txt"), "a").unwrap();
        assert_eq!(pump_for(&mut t, Duration::from_millis(1500)), 1);
        std::thread::sleep(Duration::from_millis(1100));
        // 50 writes in 200 ms: at most 2 (the leading one and the trailing one).
        for i in 0..50 {
            std::fs::write(root.join(format!("b{i}.txt")), "b").unwrap();
            std::thread::sleep(Duration::from_millis(4));
        }
        let burst = pump_for(&mut t, Duration::from_millis(2500));
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
        assert_eq!(panes(&rt, &repo), 2);
        rt.unwatch_worktree(&repo);
        assert_eq!(panes(&rt, &repo), 1);
        rt.unwatch_worktree(&repo);
        assert_eq!(panes(&rt, &repo), 0, "the last pane stops it");
        rt.unwatch_worktree(&repo); // one too many: harmless
        // A worktree git made outside the app is still the project's.
        let wt = repo.parent().unwrap().join("by-hand");
        orch_git::git(&repo, &["worktree", "add", "-q", "-b", "by-hand", wt.to_str().unwrap()]).unwrap();
        rt.watch_worktree(&wt).unwrap();
        assert_eq!(panes(&rt, &wt), 1);
        // A pane still open on a worktree that was removed: closing it stops the watcher.
        std::fs::remove_dir_all(&wt).unwrap();
        rt.unwatch_worktree(&wt);
        assert_eq!(panes(&rt, &wt), 0, "no watcher leaks on a removed worktree");
    }

    /// Through the runtime (real FSEvents, like the test above): a write in a
    /// watched worktree is a `worktree:<path>` topic in the `now` push.
    #[test]
    fn a_write_in_a_watched_worktree_is_a_now_topic() {
        let (_d, rt) = crate::runtime::tests::test_runtime(true);
        let got = Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        rt.subscribe_now(Arc::new(move |v| g.lock().unwrap().push(v.clone())));
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
            rt.pump();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen(), "{topic}");
        rt.shutdown();
        assert_eq!(panes(&rt, &repo), 0, "nothing left after shutdown");
    }
}
