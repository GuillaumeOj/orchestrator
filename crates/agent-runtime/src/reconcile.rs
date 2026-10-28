//! Startup reconciler (J10 step 2, NFR-2). A core crash ends every agent
//! (SIGHUP on the PTYs), so recovery is "reconcile, then `--resume`", in this
//! order:
//! 1. replay the orch-hook spool;
//! 2. reconcile worktrees (finish `creating` rows, report unknown or missing ones);
//! 3. detect half-done git operations → flag and suspend, Inbox item;
//! 4. remove a stale `index.lock` only when no git process runs in that worktree;
//! 5. kill leftover processes by process group, argv and cwd inside the worktree;
//! 6. read each transcript by session id and decide resume / suspend / flag;
//!    app-delivered turns are redelivered only when their marker is missing;
//! 7. open gates come back as pending Inbox items; governor holds are requeued.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orch_protocol::{HookPayload, spool};
use serde::Serialize;

use crate::paths::{AppPaths, real};
use crate::procs::{AgentMatch, Found, ProcessTable, find_agent_procs};
use crate::state::{AgentState, apply as apply_event};
use crate::store::{Admission, HoldKind, HoldStatus, StateFile, Store, TurnRow, TurnStatus, WorktreeRowState};
use crate::transcript::{self, Summary};
use crate::worktree::{self, Inspection};

/// The resume message (P0 recovery: the agent checked before redoing, every time).
pub const RESUME_MESSAGE: &str = "ORCH: the orchestrator restarted while you were working. Check what your last step actually did before redoing anything, then finish the task.";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "do", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    /// Nothing to do (archived, killed, never started).
    None,
    /// `--resume <id>` with [`RESUME_MESSAGE`].
    Resume(String),
    /// Leave suspended; resumes on demand.
    Suspend(String),
    /// Needs the user (merge in progress…): suspended with an Inbox item.
    Flag(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct SpoolReplay {
    pub agent_id: String,
    pub events: Vec<(String, String)>,
    pub malformed: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorktreeReport {
    pub path: PathBuf,
    pub verdict: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentReport {
    pub id: String,
    pub state_before: AgentState,
    pub procs: Vec<Found>,
    pub kill_log: Vec<String>,
    pub index_lock: Option<String>,
    pub transcript_found: bool,
    pub pending_tool_uses: usize,
    pub in_progress: Vec<String>,
    pub decision: Decision,
}

#[derive(Clone, Debug, Serialize)]
pub struct TurnReport {
    pub id: String,
    pub agent_id: String,
    pub was: TurnStatus,
    pub now: TurnStatus,
    pub in_transcript: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub applied: bool,
    pub spool: Vec<SpoolReplay>,
    pub worktrees: Vec<WorktreeReport>,
    pub agents: Vec<AgentReport>,
    pub turns: Vec<TurnReport>,
    pub holds: Vec<(String, String)>,
    pub inbox: Vec<String>,
}

impl Report {
    pub fn to_resume(&self) -> Vec<String> {
        self.agents.iter().filter(|a| matches!(a.decision, Decision::Resume(_))).map(|a| a.id.clone()).collect()
    }
}

/// SIGTERM groups and pids, SIGKILL after a grace period; returns a log.
pub type KillFn<'a> = &'a dyn Fn(&[i32], &[i32]) -> Vec<String>;

pub struct ReconcileEnv<'a> {
    pub store: &'a Store,
    pub paths: &'a AppPaths,
    /// `~/.claude/projects` (fixture folder in tests).
    pub projects_root: &'a Path,
    pub procs: &'a dyn ProcessTable,
    pub kill: KillFn<'a>,
    /// Never matched (the core itself).
    pub exclude_pids: &'a [i32],
    /// `false` = dry run: report only.
    pub apply: bool,
}

pub fn reconcile(env: &ReconcileEnv) -> anyhow::Result<Report> {
    let mut st: StateFile = env.store.read(|s| s.clone());
    let mut report = Report { applied: env.apply, ..Default::default() };

    // 1. Spool: events hooks could not deliver while the core was down.
    let mut spool_files = vec![];
    for (id, rec) in st.agents.iter_mut() {
        let path = spool::spool_path(&env.paths.agent_dir(id));
        let (records, malformed) = spool::read(&path)?;
        if records.is_empty() && malformed == 0 {
            continue;
        }
        let mut events = vec![];
        for r in &records {
            // SessionEnd changes nothing (its reason can't tell a crash from a suspend).
            apply_event(&mut rec.tracker, &r.event, &HookPayload::parse(&r.payload));
            events.push((r.event.to_string(), r.why.clone()));
        }
        report.spool.push(SpoolReplay { agent_id: id.clone(), events, malformed });
        spool_files.push(path);
    }

    // 2. Worktrees: git vs DB.
    let repos: BTreeSet<PathBuf> =
        st.worktrees.values().map(|w| w.repo.clone()).chain(st.agents.values().map(|a| a.repo.clone())).collect();
    for repo in &repos {
        let git_wts = worktree::list(repo).unwrap_or_default();
        let in_git = |p: &Path| git_wts.iter().any(|w| real(&w.path) == real(p) && !w.prunable);
        let rows: Vec<_> = st.worktrees.values().filter(|w| &w.repo == repo).cloned().collect();
        for row in &rows {
            let verdict = match row.state {
                WorktreeRowState::Creating => {
                    if env.apply {
                        match worktree::complete(env.store, row) {
                            Ok(done) => {
                                st.worktrees.insert(done.path.clone(), done);
                                "creation finished (add + lock + row)".to_string()
                            }
                            Err(e) => format!("could not finish creation: {e}"),
                        }
                    } else {
                        "finish creation".to_string()
                    }
                }
                WorktreeRowState::Ready if !repo.join(".git").exists() => {
                    // The whole project moved: the user points the app at it (relocate).
                    let msg = format!(
                        "Project {} is missing (moved or deleted?): relocate it to repair its worktrees",
                        repo.display()
                    );
                    if !st.inbox.iter().any(|i| i.pending() && i.body == msg) {
                        st.add_inbox(None, "alert", msg.clone());
                        report.inbox.push(msg);
                    }
                    "project missing".to_string()
                }
                WorktreeRowState::Ready
                    if row.path.is_dir() && (!in_git(&row.path) || !orch_git::repair::check_links(&row.path).ok) =>
                {
                    // Links broken (one of the repo / sibling folder moved): targeted repair.
                    if env.apply {
                        match orch_git::repair::repair(repo, &[(row.path.clone(), row.agent_id.clone())]) {
                            Ok(r) if r.repaired.len() + r.already_ok.len() == 1 => {
                                "repaired (targeted worktree repair + re-lock)".to_string()
                            }
                            Ok(r) => {
                                let msg =
                                    format!("Worktree {} could not be repaired: {:?}", row.path.display(), r.failed);
                                st.add_inbox(row.agent_id.as_deref(), "alert", msg.clone());
                                report.inbox.push(msg);
                                "repair failed".to_string()
                            }
                            Err(e) => format!("repair failed: {e}"),
                        }
                    } else {
                        "repair (targeted worktree repair + re-lock)".to_string()
                    }
                }
                WorktreeRowState::Ready if !in_git(&row.path) || !row.path.is_dir() => {
                    let msg = format!(
                        "Worktree {} is missing from git (moved or removed outside the app)",
                        row.path.display()
                    );
                    st.add_inbox(row.agent_id.as_deref(), "alert", msg.clone());
                    report.inbox.push(msg);
                    "missing".to_string()
                }
                WorktreeRowState::Ready => "ok".to_string(),
                WorktreeRowState::Removed => continue,
            };
            report.worktrees.push(WorktreeReport { path: row.path.clone(), verdict });
        }
        let root = worktree::worktree_root(repo);
        for w in &git_wts {
            let p = real(&w.path);
            if p.starts_with(&root) && !rows.iter().any(|r| real(&r.path) == p) {
                report.worktrees.push(WorktreeReport { path: p, verdict: "unknown to the app".into() });
            }
        }
    }

    // 3–7 per agent.
    let snapshot = env.procs.snapshot();
    let ids: Vec<String> = st.agents.keys().cloned().collect();
    for id in ids {
        let rec = st.agents[&id].clone();
        let state_before = rec.tracker.state;
        if state_before == AgentState::Archived {
            continue;
        }
        let wt = real(&rec.worktree);
        let insp: Inspection = worktree::inspect(&wt);

        // 4. Stale index.lock: no git process with its cwd in this worktree.
        let git_holders: Vec<i32> = snapshot
            .iter()
            .filter(|p| p.cwd.as_ref().is_some_and(|c| c.starts_with(&wt)))
            .filter(|p| p.cmd.split_whitespace().next().is_some_and(|c| c == "git" || c.ends_with("/git")))
            .map(|p| p.pid)
            .collect();
        let mut index_lock = None;
        if insp.index_lock {
            if git_holders.is_empty() {
                if env.apply {
                    if let Some(gd) = &insp.gitdir {
                        let _ = std::fs::remove_file(gd.join("index.lock"));
                    }
                }
                index_lock = Some("stale: removed".to_string());
            } else {
                index_lock = Some(format!("held by git {git_holders:?}"));
            }
        }

        // 5. Leftover processes.
        let agent_dir = env.paths.agent_dir(&id);
        let found = find_agent_procs(
            &snapshot,
            &AgentMatch { pgid: rec.pgid, agent_dir: &agent_dir, worktree: &wt, exclude: env.exclude_pids },
        );
        let mut kill_log = vec![];
        if env.apply && !found.is_empty() {
            let pgids: Vec<i32> =
                found.iter().map(|f| f.proc.pgid).filter(|g| *g > 1).collect::<BTreeSet<_>>().into_iter().collect();
            let pids: Vec<i32> = found.iter().map(|f| f.proc.pid).collect();
            kill_log = (env.kill)(&pgids, &pids);
            // A git we just killed may have left its lock behind.
            if insp.index_lock && !git_holders.is_empty() {
                if let Some(gd) = &insp.gitdir {
                    let _ = std::fs::remove_file(gd.join("index.lock"));
                    index_lock = Some("removed after killing its git".into());
                }
            }
        }

        // 6. Transcript.
        let tr: Summary = transcript::summarize_session(env.projects_root, &rec.session_id);
        let open_gate =
            st.holds.iter().any(|h| h.agent_id == id && h.status == HoldStatus::Held && h.kind == HoldKind::Gate);
        let busy = matches!(
            state_before,
            AgentState::Working
                | AgentState::Queued
                | AgentState::WaitingGate
                | AgentState::WaitingForYou
                | AgentState::Starting
        );
        let decision = if state_before == AgentState::Killed || (rec.spawns == 0 && state_before == AgentState::Created)
        {
            Decision::None
        } else if !insp.exists {
            Decision::Flag(format!("worktree {} is missing (moved?)", wt.display()))
        } else if state_before == AgentState::Suspended {
            // Stopped on purpose before the crash (a pending resume is a queued turn).
            Decision::Suspend("was suspended".into())
        } else if !insp.in_progress.is_empty() {
            Decision::Flag(format!("{} in worktree, {} unmerged path(s)", insp.in_progress.join("/"), insp.unmerged))
        } else if open_gate {
            Decision::Suspend("gate still open: back in the Inbox".into())
        } else if rec.tracker.background_tasks > 0 {
            Decision::Resume(format!(
                "{} background task(s) were running at the last Stop",
                rec.tracker.background_tasks
            ))
        } else if busy || !tr.pending_tool_uses.is_empty() {
            Decision::Resume(format!(
                "was {:?}{}",
                state_before,
                if tr.pending_tool_uses.is_empty() {
                    String::new()
                } else {
                    format!(", {} unfinished tool use(s)", tr.pending_tool_uses.len())
                }
            ))
        } else {
            Decision::Suspend(format!("was {state_before:?}"))
        };

        // Apply to the copy of the state.
        {
            let r = st.agents.get_mut(&id).expect("agent");
            // The process is gone (a core crash ends every agent); the decision sets the state.
            let state = r.tracker.state;
            r.on_process_gone();
            r.tracker.state = state;
            r.tracker.background_tasks = 0;
            match &decision {
                Decision::None => {}
                Decision::Flag(why) => {
                    r.tracker.state = AgentState::Error;
                    r.flag = Some(why.clone());
                }
                Decision::Resume(_) | Decision::Suspend(_) => r.tracker.state = AgentState::Suspended,
            }
        }
        if matches!(decision, Decision::Resume(_)) {
            // Durable: if the core dies again before the resume, the queued
            // turn still brings the agent back (the runtime resumes suspended
            // agents that have admitted work).
            st.turns.push(TurnRow::queued(&id, RESUME_MESSAGE, governor::TurnKind::Continue, Admission::Pending));
        }
        if let Decision::Flag(why) = &decision {
            let msg = format!("{id}: work in progress after a restart: {why}");
            st.add_inbox(Some(&id), "alert", msg.clone());
            report.inbox.push(msg);
        }

        // 7. Holds open when the core died were answered by orch-hook (deny).
        // A hold waiting on its own Inbox item (a permission prompt) closes
        // that item as interrupted: the agent already got the deny.
        let mut closed_items = vec![];
        let mut fresh_gates = vec![];
        for h in st.holds.iter_mut().filter(|h| h.agent_id == id && h.status == HoldStatus::Held) {
            h.status = HoldStatus::Interrupted;
            let now = match (h.kind, h.inbox_id) {
                (HoldKind::Gate, Some(item)) => {
                    closed_items.push(item);
                    format!("Inbox item {item} closed: the agent was denied while the core was down")
                }
                (HoldKind::Gate, None) => {
                    fresh_gates.push(h.summary.clone());
                    "gate pending in the Inbox".to_string()
                }
                (HoldKind::Governor, _) => "requeued".to_string(),
            };
            report.holds.push((h.id.clone(), now));
        }
        for item in closed_items {
            let _ = st.close_inbox(
                item,
                orch_store::InboxStatus::Interrupted,
                Some("core restarted: denied by orch-hook".into()),
            );
        }
        let gates = fresh_gates;
        for g in gates {
            let msg = format!("{id}: gate still pending after a restart: {g}");
            if !st.inbox.iter().any(|i| i.pending() && i.body == msg) {
                st.add_inbox(Some(&id), "gate", msg.clone());
                report.inbox.push(msg);
            }
        }

        // 6b. Turns: the transcript is the source of truth for delivery.
        for t in st.turns.iter_mut().filter(|t| t.agent_id == id) {
            let seen = tr.markers.contains(&t.id);
            let now = match t.status {
                TurnStatus::Delivering | TurnStatus::Delivered | TurnStatus::Queued if seen => TurnStatus::Delivered,
                // A slash command has no marker in the transcript: one its hook
                // saw start ran, and is not typed again.
                TurnStatus::Delivered if t.command => TurnStatus::Delivered,
                TurnStatus::Delivering | TurnStatus::Delivered => TurnStatus::Queued,
                other => other,
            };
            // A turn still waiting in the Governor's queue keeps its entry: the
            // core restored it with the carry-over state (or asks again if not).
            if now != t.status {
                report.turns.push(TurnReport {
                    id: t.id.clone(),
                    agent_id: id.clone(),
                    was: t.status,
                    now,
                    in_transcript: seen,
                });
                if now == TurnStatus::Queued {
                    t.admission = Admission::Pending;
                }
                t.status = now;
            }
        }

        report.agents.push(AgentReport {
            id: id.clone(),
            state_before,
            procs: found,
            kill_log,
            index_lock,
            transcript_found: tr.found,
            pending_tool_uses: tr.pending_tool_uses.len(),
            in_progress: insp.in_progress.clone(),
            decision,
        });
    }

    if env.apply {
        env.store.write(|s| *s = st)?;
        let stamp = orch_protocol::now_ms();
        for p in spool_files {
            let _ = std::fs::rename(&p, p.with_extension(format!("jsonl.replayed.{stamp}")));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::ProcInfo;
    use crate::role::Role;
    use crate::state::Tracker;
    use crate::store::{AgentRecord, HoldRow};
    use orch_git::fixtures::scratch_repo;
    use orch_protocol::WorktreeKindSpec;
    use std::cell::RefCell;

    struct FakeProcs(Vec<ProcInfo>);
    impl ProcessTable for FakeProcs {
        fn snapshot(&self) -> Vec<ProcInfo> {
            self.0.clone()
        }
    }

    fn agent(id: &str, repo: &Path, wt: &Path, state: AgentState) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            repo: repo.into(),
            worktree: wt.into(),
            role: Role::solo(),
            session_id: format!("sid-{id}"),
            tracker: Tracker { state, ..Default::default() },
            pid: Some(4242),
            pgid: Some(4242),
            allocation: "default".into(),
            tier: governor::Tier::Normal,
            created_at_ms: 0,
            last_event: None,
            last_event_at_ms: None,
            spawns: 1,
            flag: None,
            trust_dialog: false,
            team: None,
        }
    }

    fn turn(id: &str, agent: &str, status: TurnStatus) -> TurnRow {
        let mut t = TurnRow::queued(agent, "b", governor::TurnKind::Message, Admission::Admitted);
        t.id = id.into();
        t.status = status;
        t
    }

    /// A fixture world: a repo with worktrees, agents in every crash situation,
    /// transcripts and a spool, as in the P0 recovery rounds.
    struct World {
        _d: tempfile::TempDir,
        _app: tempfile::TempDir,
        paths: AppPaths,
        projects: PathBuf,
        store: Store,
        wts: std::collections::BTreeMap<&'static str, PathBuf>,
    }

    fn world() -> World {
        let (d, repo) = scratch_repo();
        let app = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(app.path());
        let store = Store::open(orch_store::Store::open(&paths.db_file()).unwrap(), None).unwrap();
        let mut wts = std::collections::BTreeMap::new();
        for slug in ["idle", "tool", "gate", "merge", "orphan", "bg"] {
            let row = worktree::ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: slug.into() }, None, Some(slug))
                .unwrap();
            wts.insert(slug, row.path);
        }
        let projects = d.path().join("claude-projects");
        let tdir = projects.join("-old-slug");
        std::fs::create_dir_all(&tdir).unwrap();
        let write_tr =
            |id: &str, lines: &[&str]| std::fs::write(tdir.join(format!("sid-{id}.jsonl")), lines.join("\n")).unwrap();
        write_tr(
            "idle",
            &[
                r#"{"type":"user","message":{"content":"[t:aaaa0001] say hi"}}"#,
                r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"hi"}]}}"#,
            ],
        );
        write_tr(
            "tool",
            &[
                r#"{"type":"user","message":{"content":"[t:bbbb0001] run sleep"}}"#,
                r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{}}]}}"#,
            ],
        );
        write_tr("gate", &[r#"{"type":"user","message":{"content":"[t:cccc0001] git status"}}"#]);
        write_tr("orphan", &[r#"{"type":"user","message":{"content":"hello"}}"#]);
        // bbbb0002 was pasted but never reached the CLI: redeliver.
        let st = &store;
        st.write(|s| {
            s.agents.insert("idle".into(), agent("idle", &repo, &wts["idle"], AgentState::Idle));
            s.agents.insert("tool".into(), agent("tool", &repo, &wts["tool"], AgentState::Working));
            s.agents.insert("gate".into(), agent("gate", &repo, &wts["gate"], AgentState::WaitingGate));
            s.agents.insert("merge".into(), agent("merge", &repo, &wts["merge"], AgentState::Idle));
            s.agents.insert("orphan".into(), agent("orphan", &repo, &wts["orphan"], AgentState::Idle));
            s.agents.insert("bg".into(), agent("bg", &repo, &wts["bg"], AgentState::Working));
            s.turns.push(turn("aaaa0001", "idle", TurnStatus::Delivering));
            s.turns.push(turn("bbbb0001", "tool", TurnStatus::Delivered));
            s.turns.push(turn("bbbb0002", "tool", TurnStatus::Delivering));
            s.holds.push(HoldRow {
                id: "h1".into(),
                agent_id: "gate".into(),
                tool_use_id: None,
                kind: HoldKind::Gate,
                status: HoldStatus::Held,
                summary: "git status".into(),
                created_at_ms: 0,
                inbox_id: None,
            });
        })
        .unwrap();
        // Spool: "bg" got a Stop with a running subagent while the core was down.
        let bgdir = paths.agent_dir("bg");
        std::fs::create_dir_all(&bgdir).unwrap();
        spool::append(&bgdir, &spool::SpoolRecord {
            v: 1,
            ts_ms: 1,
            agent_id: "bg".into(),
            event: orch_protocol::HookEvent::Stop,
            why: "ECONNREFUSED".into(),
            payload: serde_json::json!({"hook_event_name":"Stop","background_tasks":[{"id":"x","type":"subagent","status":"running"}]}),
        })
        .unwrap();
        spool::append(
            &bgdir,
            &spool::SpoolRecord {
                v: 1,
                ts_ms: 2,
                agent_id: "bg".into(),
                event: orch_protocol::HookEvent::SessionEnd,
                why: "ECONNREFUSED".into(),
                payload: serde_json::json!({"hook_event_name":"SessionEnd","reason":"other"}),
            },
        )
        .unwrap();
        // Merge in progress in "merge"; stale index.lock in "idle".
        let gd = worktree::inspect(&wts["merge"]).gitdir.unwrap();
        std::fs::write(gd.join("MERGE_HEAD"), "0\n").unwrap();
        let gd = worktree::inspect(&wts["idle"]).gitdir.unwrap();
        std::fs::write(gd.join("index.lock"), "").unwrap();
        World { _d: d, _app: app, paths, projects, store, wts }
    }

    #[test]
    fn decisions_match_the_p0_rounds() {
        let w = world();
        let procs = FakeProcs(vec![ProcInfo {
            pid: 777,
            ppid: 1,
            pgid: 777,
            stat: "SNs".into(),
            tty: "??".into(),
            cmd: "sleep 300".into(),
            cwd: Some(w.wts["orphan"].clone()),
        }]);
        let killed = RefCell::new(vec![]);
        let kill = |g: &[i32], p: &[i32]| {
            killed.borrow_mut().push((g.to_vec(), p.to_vec()));
            vec!["killed".to_string()]
        };

        // Dry run first: reports, changes nothing.
        let dry = reconcile(&ReconcileEnv {
            store: &w.store,
            paths: &w.paths,
            projects_root: &w.projects,
            procs: &procs,
            kill: &kill,
            exclude_pids: &[],
            apply: false,
        })
        .unwrap();
        assert!(killed.borrow().is_empty());
        assert_eq!(w.store.read(|s| s.agents["tool"].tracker.state), AgentState::Working);
        assert!(spool::spool_path(&w.paths.agent_dir("bg")).exists());
        assert_eq!(dry.agents.len(), 6);

        let r = reconcile(&ReconcileEnv {
            store: &w.store,
            paths: &w.paths,
            projects_root: &w.projects,
            procs: &procs,
            kill: &kill,
            exclude_pids: &[],
            apply: true,
        })
        .unwrap();
        let d = |id: &str| r.agents.iter().find(|a| a.id == id).unwrap().decision.clone();
        assert!(matches!(d("idle"), Decision::Suspend(_)), "{:?}", d("idle"));
        assert!(matches!(d("tool"), Decision::Resume(_)), "{:?}", d("tool"));
        assert!(matches!(d("gate"), Decision::Suspend(ref s) if s.contains("gate")), "{:?}", d("gate"));
        assert!(matches!(d("merge"), Decision::Flag(ref s) if s.contains("MERGE_HEAD")), "{:?}", d("merge"));
        assert!(matches!(d("orphan"), Decision::Suspend(_)), "{:?}", d("orphan"));
        assert!(matches!(d("bg"), Decision::Resume(ref s) if s.contains("background")), "{:?}", d("bg"));
        let mut resume = r.to_resume();
        resume.sort();
        assert_eq!(resume, ["bg", "tool"]);

        // 1. Spool replayed and renamed.
        assert_eq!(r.spool.len(), 1);
        assert_eq!(r.spool[0].events.len(), 2);
        assert!(!spool::spool_path(&w.paths.agent_dir("bg")).exists());
        // 4. Stale index.lock removed.
        let idle = r.agents.iter().find(|a| a.id == "idle").unwrap();
        assert_eq!(idle.index_lock.as_deref(), Some("stale: removed"));
        assert!(!worktree::inspect(&w.wts["idle"]).index_lock);
        // 5. The detached child found by cwd was killed.
        assert_eq!(killed.borrow().as_slice(), [(vec![777], vec![777])]);
        // 6b. Turns: delivered ones stay, the lost one is requeued.
        let turns = w.store.read(|s| s.turns.clone());
        let st = |id: &str| turns.iter().find(|t| t.id == id).unwrap().status;
        assert_eq!(st("aaaa0001"), TurnStatus::Delivered);
        assert_eq!(st("bbbb0001"), TurnStatus::Delivered);
        assert_eq!(st("bbbb0002"), TurnStatus::Queued);
        assert!(
            turns.iter().find(|t| t.id == "bbbb0002").unwrap().admission == Admission::Pending,
            "requeued turns go through the Governor again"
        );
        let resumes: Vec<_> = turns.iter().filter(|t| t.body == RESUME_MESSAGE).map(|t| t.agent_id.as_str()).collect();
        assert_eq!(resumes, ["bg", "tool"]);
        // 7. Gate back in the Inbox, hold interrupted; merge flagged.
        let s = w.store.read(|s| s.clone());
        assert!(s.inbox.iter().any(|i| i.kind == "gate" && i.body.contains("git status")));
        assert!(s.inbox.iter().any(|i| i.kind == "alert" && i.body.contains("MERGE_HEAD")));
        assert_eq!(s.holds[0].status, HoldStatus::Interrupted);
        assert_eq!(s.agents["merge"].tracker.state, AgentState::Error);
        assert!(s.agents.values().all(|a| a.pid.is_none()));
        assert_eq!(s.agents["tool"].tracker.state, AgentState::Suspended);
        // Worktrees all ok.
        assert!(r.worktrees.iter().all(|w| w.verdict == "ok"), "{:?}", r.worktrees);

        // Idempotent: a second pass finds nothing new to do.
        let r2 = reconcile(&ReconcileEnv {
            store: &w.store,
            paths: &w.paths,
            projects_root: &w.projects,
            procs: &FakeProcs(vec![]),
            kill: &kill,
            exclude_pids: &[],
            apply: true,
        })
        .unwrap();
        assert!(r2.spool.is_empty() && r2.turns.is_empty() && r2.holds.is_empty());
        assert!(r2.agents.iter().all(|a| !matches!(a.decision, Decision::Resume(_))));
    }

    #[test]
    fn finishes_creating_rows_and_reports_unknown_and_missing_worktrees() {
        let (_d, repo) = scratch_repo();
        let app = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(app.path());
        let store = Store::open(orch_store::Store::open(&paths.db_file()).unwrap(), None).unwrap();
        // Crash between `git worktree add` and the DB write: row `creating`, no lock.
        let (p, b) = worktree::layout(&repo, &WorktreeKindSpec::Solo { slug: "half".into() }).unwrap();
        worktree::git(&repo, &["worktree", "add", "-q", "-b", &b, &p.display().to_string(), "main"]).unwrap();
        let row = crate::store::WorktreeRow {
            path: p.clone(),
            repo: repo.clone(),
            branch: b,
            base: "main".into(),
            agent_id: None,
            state: WorktreeRowState::Creating,
            created_at_ms: 0,
        };
        store.write(|s| s.worktrees.insert(p.clone(), row)).unwrap();
        // Unknown: created by hand under the sibling folder.
        let (u, ub) = worktree::layout(&repo, &WorktreeKindSpec::Solo { slug: "stray".into() }).unwrap();
        worktree::git(&repo, &["worktree", "add", "-q", "-b", &ub, &u.display().to_string(), "main"]).unwrap();
        // Missing: a ready row whose folder is gone.
        let gone = worktree::worktree_root(&repo).join("solo/gone");
        let grow = crate::store::WorktreeRow {
            path: gone.clone(),
            repo: repo.clone(),
            branch: "orch/solo/gone".into(),
            base: "main".into(),
            agent_id: None,
            state: WorktreeRowState::Ready,
            created_at_ms: 0,
        };
        store.write(|s| s.worktrees.insert(gone.clone(), grow)).unwrap();

        let kill = |_: &[i32], _: &[i32]| vec![];
        let projects = app.path().join("none");
        let r = reconcile(&ReconcileEnv {
            store: &store,
            paths: &paths,
            projects_root: &projects,
            procs: &FakeProcs(vec![]),
            kill: &kill,
            exclude_pids: &[],
            apply: true,
        })
        .unwrap();
        let v =
            |path: &Path| r.worktrees.iter().find(|w| real(&w.path) == real(path)).map(|w| w.verdict.clone()).unwrap();
        assert!(v(&p).starts_with("creation finished"), "{}", v(&p));
        assert_eq!(v(&u), "unknown to the app");
        assert_eq!(v(&gone), "missing");
        assert_eq!(store.read(|s| s.worktrees[&p].state), WorktreeRowState::Ready);
        assert!(worktree::list(&repo).unwrap().iter().any(|w| real(&w.path) == real(&p) && w.locked));
        assert!(store.read(|s| s.inbox.iter().any(|i| i.body.contains("missing"))));
    }

    fn run(store: &Store, paths: &AppPaths, projects: &Path) -> Report {
        let kill = |_: &[i32], _: &[i32]| vec![];
        reconcile(&ReconcileEnv {
            store,
            paths,
            projects_root: projects,
            procs: &FakeProcs(vec![]),
            kill: &kill,
            exclude_pids: &[],
            apply: true,
        })
        .unwrap()
    }

    #[test]
    fn repairs_broken_links_and_alerts_on_a_missing_project() {
        let (d, repo) = scratch_repo();
        let app = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(app.path());
        let store = Store::open(orch_store::Store::open(&paths.db_file()).unwrap(), None).unwrap();
        let row =
            worktree::ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: "a".into() }, None, Some("a")).unwrap();
        store.write(|s| s.agents.insert("a".into(), agent("a", &repo, &row.path, AgentState::Idle))).unwrap();
        // The repo moved without its sibling folder; the rows were pointed at the
        // new place (as a relocate does) but the core died before the repair.
        let new_repo = d.path().join("moved/demo");
        std::fs::create_dir_all(new_repo.parent().unwrap()).unwrap();
        std::fs::rename(&repo, &new_repo).unwrap();
        let new_repo = real(&new_repo);
        store
            .write(|s| {
                for w in s.worktrees.values_mut() {
                    w.repo = new_repo.clone();
                }
                s.agents.get_mut("a").unwrap().repo = new_repo.clone();
            })
            .unwrap();
        assert!(!orch_git::repair::check_links(&row.path).ok);
        let r = run(&store, &paths, &app.path().join("none"));
        let v = &r.worktrees.iter().find(|w| real(&w.path) == real(&row.path)).unwrap().verdict;
        assert!(v.starts_with("repaired"), "{v}");
        assert!(orch_git::repair::check_links(&row.path).ok);
        assert!(worktree::list(&new_repo).unwrap().iter().any(|w| real(&w.path) == real(&row.path) && w.locked));

        // A project whose checkout is gone: one alert, the agent is flagged.
        let gone = d.path().join("gone/demo");
        store
            .write(|s| {
                let mut w = s.worktrees.values().next().unwrap().clone();
                w.repo = gone.clone();
                w.path = gone.with_extension("worktrees").join("solo/b");
                s.worktrees.insert(w.path.clone(), w.clone());
                s.agents.insert("b".into(), agent("b", &gone, &w.path, AgentState::Working));
            })
            .unwrap();
        let r = run(&store, &paths, &app.path().join("none"));
        assert!(r.inbox.iter().any(|m| m.contains("is missing (moved or deleted?)")), "{:?}", r.inbox);
        let b = r.agents.iter().find(|a| a.id == "b").unwrap();
        assert!(matches!(&b.decision, Decision::Flag(w) if w.contains("missing")), "{:?}", b.decision);
        // The alert is not repeated by the next start.
        let again = run(&store, &paths, &app.path().join("none"));
        assert!(!again.inbox.iter().any(|m| m.contains("moved or deleted")));
    }

    #[test]
    fn waiting_turns_keep_their_entry_and_permission_items_close() {
        let (_d, repo) = scratch_repo();
        let app = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(app.path());
        let store = Store::open(orch_store::Store::open(&paths.db_file()).unwrap(), None).unwrap();
        let row =
            worktree::ensure(&store, &repo, &WorktreeKindSpec::Solo { slug: "a".into() }, None, Some("a")).unwrap();
        store
            .write(|s| {
                s.agents.insert("a".into(), agent("a", &repo, &row.path, AgentState::WaitingForYou));
                s.turns.push(TurnRow::queued("a", "later", governor::TurnKind::Message, Admission::Waiting(7)));
                let item = s.add_inbox_item(Some("a"), "permission", "a: Bash: ls".into(), serde_json::Value::Null, 30);
                s.holds.push(HoldRow {
                    id: "h".into(),
                    agent_id: "a".into(),
                    tool_use_id: None,
                    kind: HoldKind::Gate,
                    status: HoldStatus::Held,
                    summary: "Bash: ls".into(),
                    created_at_ms: 0,
                    inbox_id: Some(item),
                });
            })
            .unwrap();
        let r = run(&store, &paths, &app.path().join("none"));
        let st = store.read(|s| s.clone());
        assert_eq!(st.turns[0].admission, Admission::Waiting(7), "the core restores the entry or asks again");
        assert_eq!(st.inbox[0].status, orch_store::InboxStatus::Interrupted);
        assert_eq!(st.inbox.iter().filter(|i| i.pending()).count(), 0, "no duplicate gate item");
        assert!(r.holds[0].1.contains("closed"));
    }
}
