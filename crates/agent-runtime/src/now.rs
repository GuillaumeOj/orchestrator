//! The `now` push (FR-1301, P4.1 spec §3.2): the attention world is
//! recomputed when a write changed something (checked on the 200 ms pump)
//! and on the 30 s tick, diffed against the last one sent, and only the
//! changed records go to the sinks, with `rev` +1 per event. Panes are told
//! what to refetch through `topics`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use governor::{QueueEntry, Window};
use orch_protocol::now_ms;
use serde_json::{Value, json};

use crate::attention::{self, AgentAttention, AskFacts, Episode, GovFacts, RunFacts, World};
use crate::decision::{self, Decision};
use crate::digest::Outcome;
use crate::runtime::Runtime;
use crate::state::AgentState;

/// Called with every `now` event (api-contract §3.1).
pub type NowSink = Arc<dyn Fn(&Value) + Send + Sync>;

/// How many messages of a run are read for the agent-on-agent waits.
const MESSAGES_PER_RUN: usize = 200;

/// What tells the Governor moved: its queue, paused agents and pending ask.
type GovSeen = (Vec<QueueEntry>, BTreeSet<String>, Option<governor::ActionId>);

/// The team data `compute` reads, re-read only when the store's team data
/// changed (or on the tick).
#[derive(Default)]
struct TeamCache {
    changes: Option<u64>,
    runs: Vec<RunFacts>,
    tasks: Vec<orch_store::Task>,
    messages: Vec<orch_store::Message>,
    projects: BTreeMap<PathBuf, String>,
}

#[derive(Default)]
struct State {
    rev: u64,
    /// The last world sent (`None` before the first computation).
    last: Option<World>,
    /// When each agent's current episode was first seen (D7).
    firsts: BTreeMap<String, (Episode, u64)>,
    team: TeamCache,
    /// The Governor's queue, paused set and pending ask last seen.
    queue: Option<GovSeen>,
    /// The decisions last sent (open ones and those closed recently), by id.
    decisions: BTreeMap<String, Decision>,
    outcomes: OutcomeCache,
}

/// Today's outcomes (the local date), and the last event id read.
#[derive(Default)]
struct OutcomeCache {
    day: String,
    after: u64,
    list: Vec<Outcome>,
}

/// The publisher's state, on the runtime.
#[derive(Default)]
pub struct Now {
    state: Mutex<State>,
    sinks: Mutex<Vec<NowSink>>,
    /// Topics noted by the runtime ([`Runtime::touch`]) since the last event.
    topics: Mutex<BTreeSet<String>>,
    /// One computation at a time, so `rev` follows the order events are sent.
    turn: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The ids in `new` whose record differs from `old`'s, and those gone.
fn changed<'a, T: PartialEq>(old: &BTreeMap<String, T>, new: &'a BTreeMap<String, T>) -> (Vec<&'a T>, Vec<String>) {
    let upsert = new.iter().filter(|(k, v)| old.get(*k) != Some(*v)).map(|(_, v)| v).collect();
    let removed = old.keys().filter(|k| !new.contains_key(*k)).cloned().collect();
    (upsert, removed)
}

/// The `now` event from `old` to `new`, with the decisions that changed
/// or closed, new outcomes and `topics`; `None` when nothing changed.
fn delta(
    rev: u64,
    old: &World,
    new: &World,
    (upsert, closed): (&[Decision], &[String]),
    outcomes: &[Outcome],
    topics: &BTreeSet<String>,
) -> Option<Value> {
    let (agents, agents_gone) = changed(&old.agents, &new.agents);
    let (runs, runs_gone) = changed(&old.runs, &new.runs);
    let counts = (old.counts != new.counts).then_some(&new.counts);
    let nothing = agents.is_empty()
        && agents_gone.is_empty()
        && runs.is_empty()
        && runs_gone.is_empty()
        && counts.is_none()
        && upsert.is_empty()
        && closed.is_empty()
        && outcomes.is_empty()
        && topics.is_empty();
    if nothing {
        return None;
    }
    let mut v = json!({
        "rev": rev,
        "attention": {"agents": agents, "runs": runs, "removed": {"agents": agents_gone, "runs": runs_gone}},
        "decisions": {"upsert": upsert, "closed": closed},
        "outcomes": outcomes,
        "topics": topics,
    });
    if let Some(c) = counts {
        v["counts"] = json!(c);
    }
    Some(v)
}

/// Decisions new or changed since `old`, and the ids that left the open
/// list (their last upsert carries the final lifecycle).
fn decision_delta(old: &BTreeMap<String, Decision>, new: &BTreeMap<String, Decision>) -> (Vec<Decision>, Vec<String>) {
    let upsert = new.iter().filter(|(k, v)| old.get(*k) != Some(*v)).map(|(_, v)| v.clone()).collect();
    let closed = old
        .iter()
        .filter(|(_, d)| d.is_open())
        .filter(|(k, _)| new.get(*k).is_none_or(|d| !d.is_open()))
        .map(|(k, _)| k.clone())
        .collect();
    (upsert, closed)
}

/// Fills in `since` where no stored time exists: when this episode was
/// first seen (D7), now for a new one.
fn remember_since(world: &mut World, firsts: &mut BTreeMap<String, (Episode, u64)>, now: u64) {
    firsts.retain(|id, _| world.agents.contains_key(id));
    for (id, a) in world.agents.iter_mut() {
        let ep = a.episode();
        if !a.since_stored {
            let since = match firsts.get(id) {
                Some((e, t)) if *e == ep => *t,
                _ => now,
            };
            a.set_since(since);
        }
        firsts.insert(id.clone(), (ep, a.since_ms));
    }
    // The blocker is picked again now every member's `since` is known: the
    // oldest wait wins, and it carries its member's wait.
    for r in world.runs.values_mut() {
        let members: Vec<&AgentAttention> = r.members.iter().filter_map(|m| world.agents.get(m)).collect();
        r.blocker = attention::blocker(&members);
    }
}

/// A snapshot narrowed to one run's agents and record (`orchd now --run`).
pub fn only_run(mut snap: Value, run: Option<&str>) -> Value {
    if let Some(run) = run {
        for k in ["agents", "runs"] {
            if let Some(v) = snap[k].as_array_mut() {
                v.retain(|r| r["run"].as_str() == Some(run));
            }
        }
    }
    snap
}

/// "just now", "3 min", "1 h 12 min", "2 d".
pub fn duration(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m} min"),
        60..=1439 => format!("{} h {:02} min", m / 60, m % 60),
        _ => format!("{} d", m / 1440),
    }
}

/// `orchd now`: one line per agent (`agent  level/status  reason  since`),
/// then one per run (`run  phase  level  blocker`).
pub fn render(snap: &Value, now_ms: u64) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut rows: Vec<[String; 4]> = vec![];
    for a in snap["agents"].as_array().into_iter().flatten() {
        let label = crate::attention::Status::label_of(a["status"].as_str().unwrap_or(""));
        let what = match (a["reason"].as_str().filter(|r| !r.is_empty()), a["activity"].as_str()) {
            (Some(r), _) => format!("{label} · {r}"),
            (None, Some(act)) => format!("{label} · {act}"),
            (None, None) => label.to_string(),
        };
        let since = a["waiting_on"]["since_ms"].as_u64().or(a["since_ms"].as_u64()).unwrap_or(now_ms);
        rows.push([
            s(&a["agent"]),
            format!("{}/{}", s(&a["level"]), s(&a["status"])),
            what,
            duration(now_ms.saturating_sub(since)),
        ]);
    }
    let mut out = table(&rows);
    let mut runs: Vec<[String; 4]> = vec![];
    for r in snap["runs"].as_array().into_iter().flatten() {
        let blocker = match r.get("blocker") {
            Some(b) => format!("{}: {}", s(&b["agent"]), s(&b["reason"])),
            None => "-".into(),
        };
        runs.push([format!("{} ({})", s(&r["run"]), s(&r["feature"])), s(&r["phase"]), s(&r["level"]), blocker]);
    }
    if !runs.is_empty() {
        out.push('\n');
        out.push_str(&table(&runs));
    }
    out
}

fn table(rows: &[[String; 4]]) -> String {
    let w = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let (w0, w1, w2) = (w(0), w(1), w(2));
    rows.iter()
        .map(|r| {
            let pad = |t: &str, n: usize| format!("{t}{}", " ".repeat(n - t.chars().count()));
            format!("{}  {}  {}  {}", pad(&r[0], w0), pad(&r[1], w1), pad(&r[2], w2), r[3]).trim_end().to_string()
                + "\n"
        })
        .collect()
}

impl Runtime {
    /// Calls `f` with every `now` event from now on (the app's Tauri event,
    /// the desktop notifier).
    pub fn subscribe_now(&self, f: NowSink) {
        lock(&self.now.sinks).push(f);
    }

    /// Notes what a pane should refetch (`worktree:<path>`, `power`…); the
    /// next pump sends it.
    pub fn touch(&self, topic: impl Into<String>) {
        lock(&self.now.topics).insert(topic.into());
    }

    /// Files in `worktree` changed (an edit, a commit, a merge): its Diff pane refetches.
    pub fn touch_worktree(&self, worktree: &std::path::Path) {
        self.touch(orch_protocol::Topic::Worktree(worktree.display().to_string()));
    }

    /// The open decisions last published, oldest first (the notifier's).
    pub fn open_decisions(&self) -> Vec<Decision> {
        // Before the first computation there is nothing cached: compute, so
        // the notifier learns what is there at start (no burst later).
        if lock(&self.now.state).last.is_none() {
            self.now_pump(true);
        }
        let st = lock(&self.now.state);
        let mut v: Vec<Decision> = st.decisions.values().filter(|d| d.is_open()).cloned().collect();
        v.sort_by_key(|d| (d.level, d.created_ms));
        v
    }

    /// `digest_seen` moved today's watermark: the cached outcomes follow.
    pub(crate) fn mark_seen(&self, up_to: u64) {
        for o in lock(&self.now.state).outcomes.list.iter_mut() {
            let id: u64 = o.id.trim_start_matches("ev-").parse().unwrap_or(u64::MAX);
            o.seen |= id <= up_to;
        }
    }

    /// The agent's last published record.
    pub fn attention_of(&self, id: &str) -> Option<AgentAttention> {
        lock(&self.now.state).last.as_ref()?.agents.get(id).cloned()
    }

    /// The whole world (`now_snapshot`, `CtlRequest::Now`): `{rev, agents,
    /// runs, counts, decisions, outcomes}`.
    pub fn now_snapshot(&self) -> Value {
        if lock(&self.now.state).last.is_none() {
            self.now_pump(true);
        }
        let st = lock(&self.now.state);
        let empty = World::default();
        let w = st.last.as_ref().unwrap_or(&empty);
        json!({
            "rev": st.rev,
            "agents": w.agents.values().collect::<Vec<_>>(),
            "runs": w.runs.values().collect::<Vec<_>>(),
            "counts": w.counts,
            "decisions": st.decisions.values().collect::<Vec<_>>(),
            "outcomes": st.outcomes.list,
        })
    }

    /// Recomputes and sends what changed: when a write changed the state,
    /// the Governor's queue moved or a topic is pending, or always with
    /// `force` (the 30 s tick: time-based reasons and waits).
    pub fn now_pump(&self, force: bool) {
        let _turn = lock(&self.now.turn);
        let changed = self.store.take_changed();
        let mut topics = std::mem::take(&mut *lock(&self.now.topics));
        let team_changes = self.db.changes();
        let db_topics = self.db.take_topics();
        // The queue as it is (cheap): what tells it moved. An answered ask
        // moves nothing else.
        let queue_now: GovSeen = self.with_governor(|g| {
            (g.queue().to_vec(), g.paused_agents().iter().map(|a| a.0.clone()).collect(), g.pending_ask())
        });
        let now = now_ms();
        let mut st = lock(&self.now.state);
        let queue_moved = st.queue.as_ref() != Some(&queue_now);
        let team_moved = st.team.changes != Some(team_changes);
        let first = st.last.is_none();
        if !(force
            || first
            || changed.any()
            || queue_moved
            || team_moved
            || !topics.is_empty()
            || !db_topics.is_empty())
        {
            return;
        }
        if team_moved {
            st.team = self.team_cache(team_changes);
        }
        topics.extend(db_topics);
        if changed.inbox {
            topics.insert(orch_protocol::Topic::Inbox.into());
        }
        if queue_moved && !first {
            topics.insert(orch_protocol::Topic::Queue.into());
            topics.insert(orch_protocol::Topic::Planner.into());
        }
        let live: BTreeSet<String> = self.live_ids().into_iter().collect();
        // Open items, and those closed recently or still being delivered to
        // an agent that is still there (`now_snapshot` keeps them 10 min);
        // older ones (P4.0's decided rows, up to KEEP_FINISHED) never show.
        let recent = now.saturating_sub(decision::RECENT_MS);
        let (agents, items): (_, Vec<_>) = self.store.read(|s| {
            let delivering = |i: &orch_store::InboxItem| {
                i.status == orch_store::InboxStatus::Sent
                    && i.agent_id
                        .as_deref()
                        .and_then(|a| s.agents.get(a))
                        .is_some_and(|a| a.tracker.state != AgentState::Archived)
            };
            let keep = |i: &&orch_store::InboxItem| {
                i.open()
                    || delivering(i)
                    || [i.decided_at_ms, i.sent_at_ms, i.effect_at_ms].into_iter().flatten().max().unwrap_or(0)
                        >= recent
            };
            (s.agents.clone(), s.inbox.iter().filter(keep).cloned().collect())
        });
        let (queue, ask, gov) = self.governor_facts();
        // The PTY tail of a synthesized card (trust dialog, a prompt in the pane, an error).
        let tails: BTreeMap<String, String> = agents
            .values()
            .filter(|a| a.trust_dialog || matches!(a.tracker.state, AgentState::WaitingForYou | AgentState::Error))
            .filter_map(|a| Some((a.id.clone(), self.live(&a.id)?.pty.output.tail_text())))
            .collect();
        let decisions = decision::build(&decision::Ctx {
            items: &items,
            agents: &agents,
            tails: &tails,
            ask: ask.as_ref(),
            runs: &st.team.runs,
            projects: &st.team.projects,
            startup_timeout_ms: self.cfg.startup_timeout.as_millis() as u64,
        });
        // Today's outcomes: the new ones since the last read (all on a new
        // day), read for the same day the cache is keyed by.
        let tz = jiff::tz::TimeZone::system();
        let (midnight, day) = crate::digest::today(now, &tz);
        if st.outcomes.day != day {
            st.outcomes = OutcomeCache { day, ..Default::default() };
        }
        let runs: BTreeMap<String, (String, String)> =
            st.team.runs.iter().map(|r| (r.run.id.clone(), (r.feature.clone(), r.project.clone()))).collect();
        let (new_outcomes, last_read) =
            self.outcomes_of_day(midnight, &st.outcomes.day, st.outcomes.after, &tz, &runs, &st.team.projects);
        st.outcomes.after = last_read;
        st.outcomes.list.extend(new_outcomes.iter().cloned());
        let done_today = st.outcomes.list.len() as u32;
        let paused = &queue_now.1;
        let mut world = attention::compute(
            &attention::Inputs {
                agents: &agents,
                inbox: &items,
                live: &live,
                queue: &queue,
                paused,
                ask: ask.as_ref(),
                gov: &gov,
                runs: &st.team.runs,
                tasks: &st.team.tasks,
                messages: &st.team.messages,
                projects: &st.team.projects,
                startup_timeout_ms: self.cfg.startup_timeout.as_millis() as u64,
                live_cap: self.live_cap(),
                done_today,
                tz: &tz,
                decisions: &decisions,
            },
            now,
        );
        let state = &mut *st;
        remember_since(&mut world, &mut state.firsts, now);
        state.queue = Some(queue_now);
        let old = state.last.take().unwrap_or_default();
        // An agent's raw row changes on every hook (last_event_at_ms): the
        // shell refetches only when what it shows (the record) changed.
        if changed.agents && old.agents != world.agents {
            topics.insert(orch_protocol::Topic::Agents.into());
        }
        let decisions: BTreeMap<String, Decision> = decisions.into_iter().map(|d| (d.id.clone(), d)).collect();
        let (upsert, closed) = decision_delta(&state.decisions, &decisions);
        let event = delta(state.rev + 1, &old, &world, (&upsert, &closed), &new_outcomes, &topics);
        state.decisions = decisions;
        state.last = Some(world);
        let Some(event) = event else { return };
        state.rev += 1;
        drop(st);
        let sinks = lock(&self.now.sinks).clone();
        for f in sinks {
            f(&event);
        }
    }

    /// The Governor's queue (release order), pending ask and the facts
    /// reasons need; one look under its lock.
    fn governor_facts(&self) -> (Vec<QueueEntry>, Option<AskFacts>, GovFacts) {
        let view = self.usage_view();
        self.with_governor(|g| {
            let by_id: BTreeMap<_, _> = g.queue().iter().map(|e| (e.id, e)).collect();
            let queue: Vec<QueueEntry> =
                g.release_order().iter().filter_map(|id| by_id.get(id).map(|e| (*e).clone())).collect();
            let ask = g.pending_ask().and_then(|id| {
                let a = g.action(id)?;
                let agent = a.action.agent().map(|a| a.0.clone());
                let cost_pp = agent
                    .as_deref()
                    .and_then(|ag| g.queue().iter().find(|e| e.agent.0 == ag))
                    .map(|e| e.estimated_cost_pct);
                let ask = a.ask.as_ref()?;
                Some(AskFacts {
                    id: id.0,
                    agent,
                    created_ms: a.ts.as_millisecond().max(0) as u64,
                    cost_pp,
                    reason: a.reason.clone(),
                    effect: a.expected_effect.clone(),
                    deadline_ms: ask.deadline.as_millisecond().max(0) as u64,
                    default_apply: matches!(ask.default, governor::AskDefault::Apply),
                })
            });
            let plan = g.plan();
            let hard_caps = plan
                .allocations
                .iter()
                .filter_map(|a| a.hard_cap_pct.map(|cap| (a.id.0.clone(), (g.allocation_usage_pct(&a.id), cap))))
                .collect();
            let full = |r: &Option<crate::usage::Reading>| r.as_ref().is_some_and(|r| r.used_pct >= 100.0);
            let gov = GovFacts {
                live_autonomous: g.live_autonomous(),
                hard_caps,
                exhausted: if full(&view.five_hour) {
                    Some(Window::FiveHour)
                } else if full(&view.weekly) {
                    Some(Window::Weekly)
                } else {
                    None
                },
                five_hour_resets_ms: view.five_hour.as_ref().map(|r| (r.resets_at.max(0) as u64) * 1000),
                week_resets_ms: Some(plan.reset_at.as_millisecond().max(0) as u64),
                five_hour_pct: view.five_hour.as_ref().map(|r| r.used_pct),
            };
            (queue, ask, gov)
        })
    }

    /// Runs (active ones and those agents still sit in), their tasks and
    /// recent messages and projects, from the store.
    fn team_cache(&self, changes: u64) -> TeamCache {
        let mut runs = self.db.runs(None, true).unwrap_or_default();
        let seated: BTreeSet<String> =
            self.store.read(|s| s.agents.values().filter_map(|a| a.team.as_ref().map(|t| t.run_id.clone())).collect());
        for id in seated {
            if !runs.iter().any(|r| r.id == id)
                && let Ok(Some(r)) = self.db.run(&id)
            {
                runs.push(r);
            }
        }
        let mut c = TeamCache { changes: Some(changes), ..Default::default() };
        for run in runs {
            let feature = self.db.feature(&run.feature_id).ok().flatten();
            let phase_since_ms = self
                .db
                .last_run_event(&run.id, "run.state")
                .ok()
                .flatten()
                .map(|e| e.ts_ms)
                .unwrap_or(run.started_at_ms);
            c.tasks.extend(self.db.tasks(None, None, Some(&run.id)).unwrap_or_default());
            c.messages.extend(self.db.recent_messages(&run.id, MESSAGES_PER_RUN).unwrap_or_default());
            c.runs.push(RunFacts {
                project: feature.as_ref().map(|f| f.project_id.clone()).unwrap_or_default(),
                feature: feature.as_ref().map(|f| f.slug.clone()).unwrap_or_else(|| run.feature_id.clone()),
                phase_since_ms,
                run,
            });
        }
        c.projects = self.db.projects(false).unwrap_or_default().into_iter().map(|p| (p.path, p.id)).collect();
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use crate::state::AgentState;

    fn collect(rt: &Runtime) -> Arc<Mutex<Vec<Value>>> {
        let got = Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        rt.subscribe_now(Arc::new(move |v| g.lock().unwrap().push(v.clone())));
        got
    }

    #[test]
    fn only_changed_records_with_a_gapless_rev() {
        let (_d, rt) = test_runtime(true);
        let got = collect(&rt);
        rt.now_pump(false);
        assert_eq!(got.lock().unwrap().len(), 1, "the first computation sends everything");
        let first = got.lock().unwrap()[0].clone();
        assert_eq!(first["rev"], 1);
        assert_eq!(first["attention"]["agents"][0]["agent"], "a1");
        assert!(first["counts"]["global"].is_object());
        // Nothing changed: no event, and no recomputation either.
        rt.now_pump(false);
        rt.now_pump(false);
        assert_eq!(got.lock().unwrap().len(), 1);
        // A read is not a change.
        rt.store.read(|s| s.agents.len());
        rt.now_pump(false);
        assert_eq!(got.lock().unwrap().len(), 1);
        // A write that changes nothing is not one either.
        rt.store.write(|_| ()).unwrap();
        rt.now_pump(false);
        assert_eq!(got.lock().unwrap().len(), 1);
        // A second agent: only its record, and the counts.
        let mut b = crate::runtime::tests::test_agent(_d.path());
        b.id = "b1".into();
        b.tracker.state = AgentState::Created;
        rt.store.write(|s| s.agents.insert("b1".into(), b)).unwrap();
        rt.now_pump(false);
        let ev = got.lock().unwrap()[1].clone();
        assert_eq!(ev["rev"], 2);
        let agents = ev["attention"]["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent"], "b1");
        assert_eq!(agents[0]["status"], "not_started");
        assert_eq!(ev["counts"]["global"]["queued"], 2, "a1 and b1 have not started");
        assert_eq!(ev["topics"], json!(["agents"]));
        assert_eq!(ev["decisions"], json!({"upsert": [], "closed": []}));
        // The tick recomputes; with nothing new it sends nothing.
        rt.now_pump(true);
        assert_eq!(got.lock().unwrap().len(), 2);
        // A removal.
        rt.store.write(|s| s.agents.remove("b1")).unwrap();
        rt.now_pump(false);
        let ev = got.lock().unwrap()[2].clone();
        assert_eq!(ev["rev"], 3);
        assert_eq!(ev["attention"]["removed"]["agents"], json!(["b1"]));
        assert_eq!(ev["attention"]["agents"], json!([]));
        // A topic alone is an event.
        rt.touch("worktree:/x");
        rt.now_pump(false);
        let ev = got.lock().unwrap()[3].clone();
        assert_eq!((ev["rev"].as_u64(), ev["topics"].clone()), (Some(4), json!(["worktree:/x"])));
        assert!(ev.get("counts").is_none(), "counts only when they changed");
        // The snapshot is at the last rev.
        let snap = rt.now_snapshot();
        assert_eq!(snap["rev"], 4);
        assert_eq!(snap["agents"].as_array().unwrap().len(), 1);
        assert_eq!(snap["decisions"], json!([]));
    }

    #[test]
    fn a_remembered_since_lasts_the_episode() {
        let (_d, rt) = test_runtime(true);
        rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Idle).unwrap();
        rt.pause_agent("a1").unwrap();
        rt.now_pump(false);
        let since = rt.attention_of("a1").unwrap().since_ms;
        assert_eq!(rt.attention_of("a1").unwrap().status, attention::Status::Paused);
        std::thread::sleep(std::time::Duration::from_millis(5));
        rt.now_pump(true);
        assert_eq!(rt.attention_of("a1").unwrap().since_ms, since, "same episode, same since");
    }

    #[test]
    fn orchd_now_lines() {
        let snap = json!({
            "agents": [
                {"agent": "front-1", "run": "run-1", "level": "needs_you", "status": "waiting_for_you",
                 "waiting_on": {"kind": "user", "ref": "inbox-4", "since_ms": 1_000}, "reason": "permission: Bash ls", "since_ms": 1_000},
                {"agent": "solo", "level": "running", "status": "working", "reason": "", "activity": "Edit a.rs", "since_ms": 0},
            ],
            "runs": [{"run": "run-1", "feature": "checkout", "phase": "executing", "level": "needs_you",
                      "blocker": {"agent": "front-1", "reason": "permission: Bash ls"}}],
        });
        let now = 1_000 + 12 * 60_000;
        assert_eq!(
            render(&snap, now),
            "front-1  needs_you/waiting_for_you  Waiting for you · permission: Bash ls  12 min\n\
             solo     running/working            Working · Edit a.rs                    12 min\n\
             \n\
             run-1 (checkout)  executing  needs_you  front-1: permission: Bash ls\n"
        );
        let one = only_run(snap, Some("run-1"));
        assert_eq!(one["agents"].as_array().unwrap().len(), 1);
        assert_eq!(
            [duration(0), duration(72 * 60_000), duration(3 * 1440 * 60_000)],
            ["just now", "1 h 12 min", "3 d"]
        );
    }

    #[test]
    fn team_writes_are_topics() {
        let (_d, rt) = test_runtime(true);
        let got = collect(&rt);
        rt.now_pump(false);
        let mut t = orch_store::Task::new("p1", "local", "x", 1);
        t.feature_id = Some("f9".into());
        rt.db.upsert_task(&t).unwrap();
        rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().unwrap();
        assert_eq!(ev["rev"], 2);
        assert_eq!(ev["topics"], json!(["board:p1", "feature:f9"]));
    }

    #[test]
    fn decisions_and_outcomes_ride_the_now_event() {
        let (_d, rt) = test_runtime(true);
        let got = collect(&rt);
        rt.now_pump(false);
        let q =
            rt.add_item(Some("a1"), "question", "a1: which db?".into(), json!({"question": "which db?"}), 20).unwrap();
        rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().unwrap();
        let up = &ev["decisions"]["upsert"];
        assert_eq!(
            (up[0]["id"].as_str(), up[0]["lifecycle"]["state"].as_str()),
            (Some(format!("q-{q}").as_str()), Some("open"))
        );
        assert_eq!(ev["counts"]["global"]["decisions"], 1);
        assert_eq!(ev["decisions"]["closed"], json!([]));
        // Answered: it leaves the open list, its last upsert says sent.
        rt.decide(q, orch_protocol::InboxDecision::Reply, None, Some("sqlite")).unwrap();
        rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().unwrap();
        assert_eq!(ev["decisions"]["closed"], json!([format!("q-{q}")]));
        assert_eq!(ev["decisions"]["upsert"][0]["lifecycle"]["state"], "sent");
        assert_eq!(ev["counts"]["global"]["decisions"], 0);
        // Its effect comes in the next push; the snapshot keeps it (closed < 10 min).
        rt.answers_resumed("a1");
        rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().unwrap();
        assert_eq!(ev["decisions"]["upsert"][0]["lifecycle"]["effect"], "resumed");
        assert_eq!(ev["decisions"]["closed"], json!([]));
        let snap = rt.now_snapshot();
        assert_eq!(snap["decisions"][0]["lifecycle"]["effect"], "resumed");
        // A new outcome is pushed once and counts in done_today.
        rt.event("a1", "turn.done", json!({"summary": "Fixed the feed"}));
        rt.db.flush().unwrap();
        rt.touch("x");
        rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().unwrap();
        assert_eq!(ev["outcomes"][0]["kind"], "turn_done");
        assert_eq!(ev["counts"]["global"]["done_today"], 1);
        let id = ev["outcomes"][0]["id"].as_str().unwrap().to_string();
        rt.touch("y");
        rt.now_pump(false);
        assert_eq!(got.lock().unwrap().last().unwrap()["outcomes"], json!([]), "pushed once");
        // Clear: seen in the snapshot, still counted.
        rt.digest_seen(&id).unwrap();
        let snap = rt.now_snapshot();
        assert_eq!(
            (snap["outcomes"][0]["seen"].as_bool(), snap["counts"]["global"]["done_today"].as_u64()),
            (Some(true), Some(1))
        );
    }
}
