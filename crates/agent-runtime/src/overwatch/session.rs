//! The Overwatch's session from the runtime (spec §3.3): events in, turns
//! out, the day's session started lazily and replaced at the day's
//! rollover, suspended when idle, its status, restart and settings.

use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::{Event, How, ID, Kind, Sched, Settings, Status, StatusView, Turn, why};
use crate::role::Role;
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::AgentRecord;

/// The Overwatch's runtime state.
#[derive(Default)]
pub struct Overwatch {
    pub(crate) sched: std::sync::Mutex<Sched>,
    /// Tests drive the scheduler by hand ([`Runtime::overwatch_pump_at`]):
    /// no background wake on a blocking event.
    pub(crate) manual: std::sync::atomic::AtomicBool,
    /// One pump at a time (a wake races the tick).
    pumping: std::sync::Mutex<()>,
    /// The status `now` last sent.
    published: std::sync::Mutex<Option<StatusView>>,
    /// The settings, read once and written through (hot paths read them).
    settings: std::sync::Mutex<Option<Settings>>,
}

impl Overwatch {
    /// A status was sent already.
    pub(crate) fn published(&self) -> bool {
        self.published.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Records `v` as sent; true when it differs from the last one.
    pub(crate) fn publish(&self, v: &StatusView) -> bool {
        let mut p = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let changed = p.as_ref() != Some(v);
        if changed {
            *p = Some(v.clone());
        }
        changed
    }
}

/// The day folder's file that keeps the last event id (ids never repeat
/// within a day's session, across app restarts).
const SEQ_FILE: &str = "events.seq";

impl Runtime {
    fn ow(&self) -> MutexGuard<'_, Sched> {
        self.overwatch.sched.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------ settings

    pub fn overwatch_settings(&self) -> Settings {
        let mut cache = self.overwatch.settings.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .get_or_insert_with(|| {
                self.db
                    .setting(super::SETTINGS_KEY)
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default()
            })
            .clone()
    }

    /// The settings with `patch`'s fields over the current ones.
    pub fn merge_overwatch_settings(&self, patch: &Value) -> Result<Settings> {
        let mut v = serde_json::to_value(self.overwatch_settings())?;
        if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, x) in p {
                o.insert(k.clone(), x.clone());
            }
        }
        serde_json::from_value(v).map_err(|e| anyhow!("overwatch settings: {e}"))
    }

    /// `set_overwatch_settings`: checked, stored, applied. Off kills the
    /// session (user messages are kept for when it is back); on again starts
    /// a fresh one at the next event.
    pub fn set_overwatch_settings(&self, s: Settings) -> Result<Settings> {
        let hold = self.hold_budget(u64::MAX).hold.as_secs();
        s.check(hold).map_err(|e| anyhow!(e))?;
        let was = self.overwatch_settings();
        self.db.set_setting(super::SETTINGS_KEY, &serde_json::to_value(&s)?)?;
        *self.overwatch.settings.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
        if was.enabled && !s.enabled {
            self.overwatch_stop_session();
            // A turn cut short by the kill never ends: it must not hold the
            // scheduler once it is on again.
            self.overwatch_abandon_turn();
            self.overwatch_drop_events();
            self.overwatch_hand_over_all();
        } else if !was.enabled && s.enabled {
            self.ow().fresh = true;
            self.overwatch_requeue_pending();
        } else if was.model != s.model && self.store.read(|st| st.agents.contains_key(ID)) && self.live(ID).is_none() {
            // The next session takes it; a live one keeps its model for the day.
            self.ow().fresh = true;
        }
        self.overwatch_touch();
        self.overwatch_wake();
        Ok(s)
    }

    // ------------------------------------------------------------ paths

    /// `<app data>/overwatch/<YYYY-MM-DD>/`.
    pub fn overwatch_day_dir(&self, day: &str) -> PathBuf {
        self.cfg.paths.root.join("overwatch").join(day)
    }

    /// The working day now (the chat's, `digest::today`).
    pub(crate) fn overwatch_today(&self) -> (u64, String) {
        let (now, tz) = self.now.clock();
        (now, crate::digest::today(now, &tz).1)
    }

    /// The day the stored session belongs to (its folder's name).
    fn session_day(rec: &AgentRecord) -> Option<String> {
        rec.worktree.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// What the Overwatch may read (spec §3.3, lead note 2): every
    /// registered project's checkout and its worktrees, its day folder and
    /// its `messages/`.
    pub(crate) fn overwatch_reads(&self, day_dir: &std::path::Path) -> Vec<PathBuf> {
        let mut reads: Vec<PathBuf> = self.db.projects(false).unwrap_or_default().into_iter().map(|p| p.path).collect();
        let worktrees: Vec<PathBuf> = self.store.read(|s| s.worktrees.keys().cloned().collect());
        reads.extend(worktrees);
        reads.push(day_dir.to_path_buf());
        reads.push(self.cfg.paths.agent_dir(ID).join("messages"));
        reads.sort();
        reads.dedup();
        reads
    }

    // ------------------------------------------------------------ status

    /// `now.overwatch` (api-contract §3.1, §5).
    pub fn overwatch_status(&self) -> StatusView {
        let (status, why, events) = self.overwatch_state();
        let has_session =
            self.store.read(|s| s.agents.get(ID).is_some_and(|a| a.spawns > 0)) && self.overwatch_session_is_today();
        let now = self.now.clock().0;
        let mut st = self.ow();
        let since = match &st.shown {
            Some((s, w, at)) if *s == status && *w == why => *at,
            _ => {
                st.shown = Some((status, why.clone(), now));
                now
            }
        };
        StatusView { status, events, why, since_ms: since, agent: ID, has_session }
    }

    fn overwatch_session_is_today(&self) -> bool {
        let day = self.overwatch_today().1;
        self.store.read(|s| s.agents.get(ID).and_then(Self::session_day)).as_deref() == Some(day.as_str())
    }

    /// The status, its `why` and the event count, from the state.
    fn overwatch_state(&self) -> (Status, Option<String>, u32) {
        if !self.overwatch_settings().enabled {
            return (Status::Off, None, 0);
        }
        let (down, in_flight, queued, events) = {
            let st = self.ow();
            (st.down.clone(), st.in_flight.is_some(), !st.queue.is_empty(), st.events())
        };
        if let Some(why) = down {
            return (Status::Down, Some(why), events);
        }
        if let Some(w) = self.overwatch_paused() {
            return (Status::Paused, Some(w.into()), events);
        }
        let status = if in_flight {
            Status::Working
        } else if queued {
            Status::Waking
        } else {
            Status::Asleep
        };
        (status, None, events)
    }

    /// Pause all or a reached limit holds it like any agent.
    fn overwatch_paused(&self) -> Option<&'static str> {
        if self.with_governor(|g| g.is_paused_all()) {
            return Some(why::PAUSE_ALL);
        }
        let view = self.usage_view();
        let full = |r: &Option<crate::usage::Reading>| r.as_ref().is_some_and(|r| r.used_pct >= 100.0);
        (full(&view.five_hour) || full(&view.weekly)).then_some(why::LIMIT)
    }

    /// May a permission wait on it now (F1: asleep, waking or working)?
    pub fn overwatch_may_decide(&self) -> bool {
        self.overwatch_state().0.may_decide()
    }

    pub(crate) fn overwatch_touch(&self) {
        self.touch(super::TOPIC);
    }

    // ------------------------------------------------------------ events

    /// Queues an event (api-contract §2.1); its id, or `None` when dropped
    /// (`off` / `down`: only user messages are kept, for the restart).
    /// The entry point of every producer, B3's `stuck` and
    /// `rule_candidate` included.
    pub fn overwatch_enqueue(&self, ev: Event) -> Option<String> {
        let (now, day) = self.overwatch_today();
        let enabled = self.overwatch_settings().enabled;
        let blocking = ev.kind.blocking();
        let id = {
            let mut st = self.ow();
            if !enabled || st.down.is_some() {
                if ev.kind == Kind::UserMessage {
                    let mut ev = ev;
                    ev.at_ms = if ev.at_ms == 0 { now } else { ev.at_ms };
                    st.pending_user.push(ev);
                }
                return None;
            }
            self.overwatch_seed(&mut st, &day);
            let id = st.push(ev, now);
            let seq = st.seq;
            drop(st);
            self.overwatch_save_seq(&day, seq);
            id
        };
        self.overwatch_touch();
        // Batched events wait for the tick; only a blocking one wakes it now.
        if blocking {
            self.overwatch_wake();
        }
        Some(id)
    }

    /// A day's first event: its ids follow the day folder's last one, and
    /// its turns so far (an app restart mid-day) are read back.
    fn overwatch_seed(&self, st: &mut Sched, day: &str) {
        if st.day.as_deref() == Some(day) {
            return;
        }
        let read = std::fs::read_to_string(self.overwatch_day_dir(day).join(SEQ_FILE)).ok();
        st.seq = read.and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        st.day = Some(day.to_string());
        st.turns = self.overwatch_turns_stored();
    }

    fn overwatch_save_seq(&self, day: &str, seq: u64) {
        let dir = self.overwatch_day_dir(day);
        let _ = std::fs::create_dir_all(&dir)
            .and_then(|()| crate::compile::write_atomic(&dir.join(SEQ_FILE), seq.to_string().as_bytes()));
    }

    /// The user messages kept while it was `off` / `down` go back in the
    /// queue, each with a new id of the day (a kept message may never have
    /// had one).
    fn overwatch_requeue_pending(&self) {
        let (now, day) = self.overwatch_today();
        let mut st = self.ow();
        if st.pending_user.is_empty() {
            return;
        }
        self.overwatch_seed(&mut st, &day);
        for ev in std::mem::take(&mut st.pending_user) {
            st.push(ev, now);
        }
        let seq = st.seq;
        drop(st);
        self.overwatch_save_seq(&day, seq);
    }

    /// The user's message (`chat_send`, P4.3's entry `chat`): a blocking
    /// event, kept while the Overwatch is `off` / `down`.
    pub fn overwatch_user_message(&self, chat: &str, text: &str) -> Option<String> {
        let mut ev = Event::new(Kind::UserMessage, crate::team::cut(text, 200));
        ev.text = Some(text.to_string());
        ev.chat = Some(chat.to_string());
        self.overwatch_enqueue(ev)
    }

    /// User messages kept while it was `off` / `down` (`delivery: pending`).
    pub fn overwatch_pending(&self) -> Vec<Event> {
        self.ow().pending_user.clone()
    }

    /// Wakes the scheduler off the caller's thread (a hook, a decision).
    pub(crate) fn overwatch_wake(&self) {
        if self.overwatch.manual.load(Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        std::thread::spawn(move || me.overwatch_pump());
    }

    // ------------------------------------------------------------ the pump

    pub fn overwatch_pump(&self) {
        let now = self.now.clock().0;
        self.overwatch_pump_at(now);
    }

    /// One scheduler step at `now`: suspends an idle session, or delivers
    /// the due events as one turn (starting, resuming or replacing the
    /// session first). Run by the tick, on a blocking event and at a turn's end.
    pub fn overwatch_pump_at(&self, now: u64) {
        let _one = self.overwatch.pumping.lock().unwrap_or_else(|e| e.into_inner());
        let settings = self.overwatch_settings();
        if !settings.enabled {
            return;
        }
        let (due, idle) = {
            let st = self.ow();
            if st.down.is_some() || st.in_flight.is_some() {
                return;
            }
            let idle = st.queue.is_empty()
                && st.last_turn_end_ms.is_some_and(|t| now >= t.saturating_add(settings.idle_secs * 1000));
            (st.due(now, settings.batch_secs * 1000), idle)
        };
        if !due {
            if idle {
                self.overwatch_suspend_idle();
            }
            return;
        }
        if self.overwatch_paused().is_some() {
            // Paused (Pause all, or a limit reached) decides nothing (F1).
            self.overwatch_hand_over_all();
            self.overwatch_touch();
            return;
        }
        let tz = self.now.clock().1;
        let day = crate::digest::today(now, &tz).1;
        let rec = self.store.read(|s| s.agents.get(ID).cloned());
        let fresh = self.ow().fresh;
        let stale = rec
            .as_ref()
            .is_none_or(|r| r.tracker.state == AgentState::Archived || Self::session_day(r).as_deref() != Some(&day));
        let new = stale || fresh;
        if new && self.live(ID).is_some() {
            // The day's rollover (or a restart): the old session goes first
            // (never mid-turn: no turn is in flight here).
            self.overwatch_stop_session();
        }
        let how = if new {
            if fresh { How::Fresh } else { How::Spawned }
        } else if self.live(ID).is_some() {
            How::Live
        } else {
            How::Resumed
        };
        if new && let Err(e) = self.overwatch_new_session(&day, now, &settings) {
            self.event(ID, "overwatch.start_failed", json!({"error": format!("{e:#}")}));
            self.overwatch_down(why::COULD_NOT_START);
            return;
        }
        let (events, cache_warm) = {
            let mut st = self.ow();
            st.fresh = false;
            let events = std::mem::take(&mut st.queue);
            let warm = !new && st.last_turn_end_ms.is_some_and(|t| now.saturating_sub(t) < super::CACHE_TTL_MS);
            (events, warm)
        };
        if events.is_empty() {
            return;
        }
        let text = super::turn_text(&events, new, now, &tz);
        let queued_at_ms = events.iter().map(|e| e.at_ms).min().unwrap_or(now);
        self.ow().in_flight = Some(Turn {
            events,
            queued_at_ms,
            started_at_ms: now,
            started: false,
            first_tool_ms: None,
            how,
            cache_warm,
        });
        self.overwatch_touch();
        if let Err(e) = self.deliver(ID, &text) {
            self.event(ID, "overwatch.deliver_failed", json!({"error": format!("{e:#}")}));
            self.overwatch_down(why::COULD_NOT_START);
        }
    }

    /// A new session for `day`: its folder, a new record (new session id,
    /// never started: the pump spawns it with `--session-id`) and the
    /// briefing. Past days' usage stays under the same id and allocation.
    fn overwatch_new_session(&self, day: &str, now: u64, settings: &Settings) -> Result<()> {
        let dir = self.overwatch_day_dir(day);
        std::fs::create_dir_all(&dir)?;
        let rec = AgentRecord {
            id: ID.into(),
            repo: dir.clone(),
            worktree: dir.clone(),
            role: Role::overwatch(&settings.model),
            session_id: uuid::Uuid::new_v4().to_string(),
            tracker: Default::default(),
            pid: None,
            pgid: None,
            allocation: super::ALLOCATION.into(),
            tier: governor::Tier::Critical,
            created_at_ms: now,
            last_event: None,
            last_event_at_ms: None,
            spawns: 0,
            flag: None,
            trust_dialog: false,
            team: None,
            resume_failed: false,
            start_fresh: false,
            spawned_at_ms: None,
            crash_restarts: vec![],
        };
        self.store.write(|s| {
            // The old session's undelivered turns are not the new one's.
            s.turns.retain(|t| t.agent_id != ID);
            s.agents.insert(ID.into(), rec);
        })?;
        let pending: Vec<Event> = {
            let mut st = self.ow();
            st.long_turns = 0;
            st.last_turn_end_ms = None;
            st.pending_user.clone()
        };
        let text = self.overwatch_briefing(day, now, &pending);
        crate::compile::write_atomic(&dir.join("briefing.md"), text.as_bytes())?;
        self.event(ID, "overwatch.session", json!({"day": day, "dir": dir.display().to_string()}));
        // Messages kept while it was down go in this first turn.
        self.overwatch_requeue_pending();
        Ok(())
    }

    /// Suspends the idle session (resumed within the day, cache warm).
    fn overwatch_suspend_idle(&self) {
        let idle = self.live(ID).is_some()
            && self.store.read(|s| {
                s.agents.get(ID).is_some_and(|a| a.tracker.state == AgentState::Idle && a.tracker.background_tasks == 0)
                    && !s.turns.iter().any(|t| {
                        t.agent_id == ID
                            && matches!(
                                t.status,
                                crate::store::TurnStatus::Queued | crate::store::TurnStatus::Delivering
                            )
                    })
            });
        if idle {
            let r = self.suspend(ID);
            self.event(ID, "overwatch.suspended", json!({"error": r.err().map(|e| format!("{e:#}"))}));
            self.overwatch_touch();
        }
    }

    /// Kills the session (restart, rollover, turned off): an exit we asked for.
    fn overwatch_stop_session(&self) {
        if self.live(ID).is_some()
            && let Err(e) = self.kill(ID)
        {
            self.event(ID, "overwatch.kill_failed", json!({"error": format!("{e:#}")}));
        }
    }

    // ------------------------------------------------------------ the session's hooks

    /// TurnStarted of the Overwatch: the turn's start.
    pub(crate) fn overwatch_turn_started(&self, now: u64) {
        if let Some(t) = self.ow().in_flight.as_mut().filter(|t| !t.started) {
            (t.started_at_ms, t.started) = (now, true);
        }
    }

    /// PreToolUse of the Overwatch: the first tool call of the turn (the wake time's end).
    pub(crate) fn overwatch_tool_call(&self, now: u64) {
        if let Some(t) = self.ow().in_flight.as_mut() {
            t.first_tool_ms.get_or_insert(now);
        }
    }

    /// The Overwatch's turn ended: `overwatch.turn` recorded, the permissions
    /// it carried and did not settle handed over to the user (F1), two
    /// turns in a row past the ceiling put it `down`, and what came meanwhile
    /// goes as the next turn.
    pub(crate) fn overwatch_turn_ended(&self, now: u64) {
        let Some(t) = self.ow().in_flight.take() else { return };
        let ceiling = u64::from(super::TURN_CEILING_MIN) * 60_000;
        let slow = {
            let mut st = self.ow();
            st.last_turn_end_ms = Some(now);
            st.long_turns = if now.saturating_sub(t.started_at_ms) >= ceiling { st.long_turns + 1 } else { 0 };
            st.long_turns >= 2
        };
        self.overwatch_record_turn(&t, now);
        for d in carried_permissions(&t) {
            self.overwatch_hand_over(d);
        }
        if slow {
            self.overwatch_down(why::TOO_SLOW);
            return;
        }
        self.overwatch_touch();
        self.overwatch_pump_at(now);
    }

    fn overwatch_record_turn(&self, t: &Turn, ended: u64) {
        let v = json!({
            "events": t.events.len(),
            "queued_at_ms": t.queued_at_ms,
            "started_at_ms": t.started_at_ms,
            "first_tool_ms": t.first_tool_ms,
            "ended_at_ms": ended,
            "how": t.how,
            "cache_warm": t.cache_warm,
            "ids": t.events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        });
        self.event(ID, "overwatch.turn", v.clone());
        self.ow().turns.push(v);
    }

    /// Its process exited. Asked for (suspend, kill): nothing. Before its
    /// session started: `could not start`; otherwise `stopped unexpectedly`.
    pub(crate) fn overwatch_exited(&self, asked: bool, failed_start: bool) {
        if asked {
            return;
        }
        self.overwatch_down(if failed_start { why::COULD_NOT_START } else { why::STOPPED });
    }

    /// An error on its turn: a login error puts it `down`; another API error
    /// only ends the turn (the session waits at its prompt: the next turn
    /// goes to it). A reached limit puts it `down` too.
    pub(crate) fn overwatch_failed(&self, error: &str, now: u64) {
        if crate::registry::looks_like_auth(error) || error.to_ascii_lowercase().contains("login") {
            self.overwatch_down(why::LOGIN);
            return;
        }
        let _ = self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(ID).filter(|a| a.tracker.state == AgentState::Error) {
                a.tracker.state = AgentState::Idle;
            }
        });
        self.overwatch_turn_ended(now);
    }

    pub(crate) fn overwatch_rate_limited(&self) {
        self.overwatch_down(why::LIMIT);
    }

    /// `down` with `why` (spec §3.3): no notification, the queue dropped
    /// (user messages kept), the turn in flight abandoned, every deciding
    /// permission handed over to the user at once.
    pub(crate) fn overwatch_down(&self, why: &str) {
        {
            let mut st = self.ow();
            if st.down.is_some() {
                return;
            }
            st.down = Some(why.to_string());
        }
        self.overwatch_abandon_turn();
        self.overwatch_drop_events();
        self.overwatch_hand_over_all();
        self.event(ID, "overwatch.down", json!({"why": why}));
        self.overwatch_touch();
    }

    /// The turn in flight will never end (killed, down): dropped, its
    /// permissions handed over, a user message in it kept for the restart.
    fn overwatch_abandon_turn(&self) {
        let Some(t) = self.ow().in_flight.take() else { return };
        for d in carried_permissions(&t) {
            self.overwatch_hand_over(d);
        }
        self.ow().pending_user.extend(t.events.into_iter().filter(|e| e.kind == Kind::UserMessage));
    }

    fn overwatch_drop_events(&self) {
        let mut st = self.ow();
        let queue = std::mem::take(&mut st.queue);
        st.pending_user.extend(queue.into_iter().filter(|e| e.kind == Kind::UserMessage));
    }

    /// `overwatch_restart`: a fresh session (new briefing) at once, with the
    /// user messages kept while it was down.
    pub fn overwatch_restart(&self) -> Result<()> {
        if !self.overwatch_settings().enabled {
            anyhow::bail!("the Overwatch is off: turn it on in Settings");
        }
        self.overwatch_stop_session();
        {
            let mut st = self.ow();
            st.down = None;
            st.long_turns = 0;
            st.fresh = true;
        }
        // A user message in the turn cut short goes to the fresh session.
        self.overwatch_abandon_turn();
        self.event(ID, "overwatch.restart", json!({}));
        // Its own turn reads the briefing even with nothing queued.
        let mut ev = Event::new(Kind::UserMessage, "restarted by the user: read briefing.md and catch up with world()");
        ev.text = Some("Restarted.".into());
        self.overwatch_enqueue(ev);
        self.overwatch_pump();
        Ok(())
    }

    /// `orchd overwatch`: the status, today's session and turns, and the
    /// p50 / p90 wake time (oldest event queued → first tool call).
    pub fn overwatch_report(&self) -> Value {
        let status = self.overwatch_status();
        let (_, day) = self.overwatch_today();
        let session = self.store.read(|s| {
            s.agents
                .get(ID)
                .filter(|a| Self::session_day(a).as_deref() == Some(day.as_str()))
                .map(|a| a.session_id.clone())
        });
        let turns = self.overwatch_turns_today();
        let wakes: Vec<u64> = turns
            .iter()
            .filter_map(|t| Some(t["first_tool_ms"].as_u64()?.saturating_sub(t["queued_at_ms"].as_u64()?)))
            .collect();
        let pct = super::percentiles(&wakes);
        json!({
            "status": status,
            "day": day,
            "session": session,
            "turns": turns,
            "wake_p50_ms": pct.map(|p| p.0),
            "wake_p90_ms": pct.map(|p| p.1),
        })
    }

    /// Today's `overwatch.turn` events (kept since the day's first event,
    /// else the store's).
    fn overwatch_turns_today(&self) -> Vec<Value> {
        let day = self.overwatch_today().1;
        {
            let st = self.ow();
            if st.day.as_deref() == Some(day.as_str()) {
                return st.turns.clone();
            }
        }
        self.overwatch_turns_stored()
    }

    /// Today's `overwatch.turn` events in the store, oldest first.
    fn overwatch_turns_stored(&self) -> Vec<Value> {
        let (now, tz) = self.now.clock();
        let midnight = crate::digest::today(now, &tz).0;
        self.db
            .events_of_kinds_in(midnight, u64::MAX, &["overwatch.turn"])
            .unwrap_or_default()
            .into_iter()
            .filter(|(e, _)| e.agent_id.as_deref() == Some(ID))
            .map(|(e, _)| e.detail)
            .collect()
    }
}

/// The permission decisions a turn carried (`inbox-<n>` → n).
fn carried_permissions(t: &Turn) -> Vec<u64> {
    t.events
        .iter()
        .filter(|e| e.kind == Kind::Permission)
        .filter_map(|e| e.decision.as_deref()?.rsplit('-').next()?.parse().ok())
        .collect()
}
