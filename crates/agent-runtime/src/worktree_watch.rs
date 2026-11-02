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
    watch(root, move |_| dirty.store(true, Ordering::SeqCst))
}

/// Watches `root` (canonical): `on_change` gets what raised a change, the
/// path that [`counts`] or the error (events dropped, say).
fn watch(root: &Path, on_change: impl Fn(&dyn std::fmt::Debug) + Send + 'static) -> Result<notify::RecommendedWatcher> {
    // A linked worktree's git dir (its `.git` is a `gitdir:` file), else its own `.git`.
    let linked = orch_git::repair::check_links(root).gitdir;
    let git_dir = linked.clone().unwrap_or_else(|| root.join(".git"));
    let r = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(ev) => {
            if let Some(p) = ev.paths.iter().find(|p| counts(&r, &git_dir, p)) {
                on_change(p);
            }
        }
        // An error may hide a change: refetch to be sure.
        Err(e) => on_change(&e),
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

    /// What the test sink recorded: when each event arrived, and what it was.
    type Events = Mutex<Vec<(Instant, String)>>;

    /// The pump, every 50 ms for `d`: when `t` raised its topics.
    fn pump_for(t: &mut Throttle, d: Duration) -> Vec<Instant> {
        let (end, mut topics) = (Instant::now() + d, vec![]);
        while Instant::now() < end {
            let now = Instant::now();
            if t.due(now, DEBOUNCE) {
                topics.push(now);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        topics
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

    /// Waits until the stream has been quiet for 500 ms (bounded), clearing
    /// `dirty` and the recorded `fired`: events of the setup or of the phase
    /// before must not count for the next one.
    fn settle(t: &mut Throttle, fired: &Events) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            t.dirty.store(false, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(500));
            if !t.dirty.load(Ordering::SeqCst) {
                break;
            }
            assert!(Instant::now() < end, "the stream never went quiet: {:?}", fired.lock().unwrap());
        }
        fired.lock().unwrap().clear();
        t.last = None;
    }

    /// Checks the throttle contract on what a phase really did, whenever the
    /// OS delivered its events (FSEvents may split one write over batches
    /// more than a second apart, and then a trailing topic is right):
    ///
    /// 1. a topic is raised;
    /// 2. the first one at once after the first event (or after the pump
    ///    started, if the events came first: the burst writes before it pumps);
    /// 3. a later one only after an event newer than the previous topic;
    /// 4. two are at least `DEBOUNCE` apart;
    /// 5. every event is the phase's own (`own`; one elsewhere is shown, not
    ///    failed).
    ///
    /// `t0` is the write, `pumped` when the pump started.
    fn assert_throttled(
        phase: &str,
        t0: Instant,
        pumped: Instant,
        events: &[(Instant, String)],
        topics: &[Instant],
        own: impl Fn(&str) -> bool,
    ) {
        // An event is recorded just before it sets `dirty`: let one that was
        // recorded a hair before a topic still count as after it.
        const SLACK: Duration = Duration::from_millis(10);
        let ms = |i: Instant| i.saturating_duration_since(t0).as_millis();
        let timeline = || {
            let ev: Vec<_> = events.iter().map(|(i, p)| format!("{}ms {p}", ms(*i))).collect();
            let tp: Vec<_> = topics.iter().map(|i| format!("{}ms", ms(*i))).collect();
            format!("{phase}: events [{}], topics [{}]", ev.join(", "), tp.join(", "))
        };
        println!("{}", timeline());
        let (Some(first), Some(&raised)) = (events.first(), topics.first()) else {
            panic!("1. a topic is raised: {}", timeline());
        };
        assert!(
            raised.saturating_duration_since(first.0.max(pumped)) <= Duration::from_millis(250),
            "2. at once: {}",
            timeline()
        );
        for w in topics.windows(2) {
            assert!(w[1].duration_since(w[0]) >= DEBOUNCE, "4. at most once a second: {}", timeline());
            assert!(
                events.iter().any(|(i, _)| *i + SLACK > w[0] && *i <= w[1] + SLACK),
                "3. not without a change: {}",
                timeline()
            );
        }
        let elsewhere: Vec<_> = events.iter().filter(|(_, p)| !own(p)).collect();
        if !elsewhere.is_empty() {
            println!("{phase}: events elsewhere (tolerated): {elsewhere:?}");
        }
    }

    /// Real FSEvents (macOS CI): a sandbox without the FSEvents service gets
    /// no events, so this one runs outside it. Topics follow the throttle
    /// contract for a write and for a burst, and none for an ignored path.
    #[test]
    fn topics_follow_the_throttle_contract_none_for_ignored() {
        let (_d, root) = tmp();
        std::fs::create_dir_all(root.join("target")).unwrap();
        let mut t = Throttle::default();
        let fired = Arc::new(Events::default());
        let (dirty, sink) = (t.dirty.clone(), fired.clone());
        let _w = watch(&root, move |what| {
            // Recorded before `dirty`, so every event `due` sees is on the timeline.
            sink.lock().unwrap().push((Instant::now(), format!("{what:?}")));
            dirty.store(true, Ordering::SeqCst);
        })
        .unwrap();
        let events = || fired.lock().unwrap().clone();
        // Late events of the setup (the temp dir itself, a rescan) are not the test's.
        settle(&mut t, &fired);
        // An ignored path: it raises nothing (a stray event elsewhere is tolerated).
        std::fs::write(root.join("target/out.o"), "x").unwrap();
        pump_for(&mut t, Duration::from_millis(1600));
        let ignored = format!("{}/target", root.display());
        assert!(!events().iter().any(|(_, p)| p.contains(&ignored)), "ignored paths raise nothing: {:?}", events());
        // One write: topics follow the contract, whatever the batches.
        settle(&mut t, &fired);
        let t0 = Instant::now();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let pumped = Instant::now();
        let one = pump_for(&mut t, Duration::from_millis(1500));
        assert_throttled("one write", t0, pumped, &events(), &one, |p| p.contains("/a.txt"));
        settle(&mut t, &fired);
        // 50 writes in 200 ms: a leading topic, then at most one a second.
        let t0 = Instant::now();
        for i in 0..50 {
            std::fs::write(root.join(format!("b{i}.txt")), "b").unwrap();
            std::thread::sleep(Duration::from_millis(4));
        }
        let pumped = Instant::now();
        let burst = pump_for(&mut t, Duration::from_millis(2500));
        println!("burst: {} topics", burst.len());
        assert_throttled("burst", t0, pumped, &events(), &burst, |p| p.contains("/b"));
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
