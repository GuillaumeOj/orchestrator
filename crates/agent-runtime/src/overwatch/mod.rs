//! The Overwatch (P4.4, spec §3.3, FR-1306): one global, event-driven
//! agent with a fixed role. The core keeps a queue of [`Event`]s, wakes it
//! at once for a blocking one and batches the rest, runs one turn at a time,
//! suspends it after an idle grace and starts a fresh session each working
//! day. Nothing waits on it: every card is posted by the core first, a
//! permission waits at most `permission_secs` on it, and `off` / `down` /
//! `paused` send everything straight to the user.
//!
//! This module holds the pure parts (settings, status, events, the
//! scheduler's state and the turn text); [`session`] drives the session
//! from the runtime, [`briefing`] writes the day's briefing.

pub mod briefing;
pub mod chatw;
pub mod deciding;
pub mod ground;
pub mod rules;
pub mod session;
pub mod stuck;
pub mod tools;
pub mod undo;

#[cfg(test)]
mod learn_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tools_tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The Overwatch's fixed agent id.
pub const ID: &str = "overwatch";
/// Refuses the Overwatch's reserved id on every start path but its own
/// (a Start, a solo session, a team member, an import).
pub fn refuse_reserved(id: &str) -> anyhow::Result<()> {
    if id == ID {
        anyhow::bail!("{ID:?} is reserved for the Overwatch: pick another name");
    }
    Ok(())
}
/// Its usage allocation and report row.
pub const ALLOCATION: &str = "overwatch";
/// Its system prompt (compiled in).
pub const ROLE_PROMPT: &str = include_str!("role.md");
/// FR-407 turn ceiling; two turns in a row past it put it `down`.
pub const TURN_CEILING_MIN: u32 = 10;
/// The briefing's cap (≈ 8 k tokens).
pub const BRIEFING_MAX: usize = 32_000;
/// A resume within this of the last turn finds the prompt cache warm.
pub const CACHE_TTL_MS: u64 = 5 * 60 * 1000;
/// The settings key.
pub const SETTINGS_KEY: &str = "overwatch";
/// The topic `now` carries when the status changes.
pub const TOPIC: &str = "overwatch";

/// `overwatch_settings` (api-contract §3.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub model: String,
    pub batch_secs: u64,
    pub permission_secs: u64,
    pub idle_secs: u64,
    pub rule_threshold: u32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            enabled: true,
            model: "sonnet".into(),
            batch_secs: 30,
            permission_secs: 60,
            idle_secs: 120,
            rule_threshold: 3,
        }
    }
}

impl Settings {
    /// Refuses values that can't work: `permission_secs` must be shorter
    /// than the permission hold (api-contract §5, F1).
    pub fn check(&self, hold_secs: u64) -> Result<(), String> {
        if self.model.trim().is_empty() || self.model.chars().any(char::is_whitespace) || self.model.starts_with('-') {
            return Err("model: an alias (sonnet, opus, haiku) or a model id".into());
        }
        // No hold (0): permissions go to the pane, nothing waits on it.
        if self.permission_secs == 0 || (hold_secs > 0 && self.permission_secs >= hold_secs) {
            return Err(format!("permission_secs must be between 1 and the hold ({hold_secs} s), exclusive"));
        }
        if self.batch_secs > 3_600 || self.idle_secs > 3_600 {
            return Err("batch_secs and idle_secs: at most 3600".into());
        }
        if self.rule_threshold < 2 {
            return Err("rule_threshold: at least 2".into());
        }
        Ok(())
    }
}

/// `now.overwatch.status` (api-contract §3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Off,
    Asleep,
    Waking,
    Working,
    Paused,
    Down,
}

impl Status {
    /// It may still settle a permission (deciding, F1).
    pub fn may_decide(self) -> bool {
        matches!(self, Status::Asleep | Status::Waking | Status::Working)
    }
}

/// The fixed `why` set (api-contract §5, F2).
pub mod why {
    pub const PAUSE_ALL: &str = "Pause all";
    pub const LIMIT: &str = "limit reached";
    pub const COULD_NOT_START: &str = "could not start";
    pub const STOPPED: &str = "stopped unexpectedly";
    pub const LOGIN: &str = "login expired";
    pub const TOO_SLOW: &str = "too slow, 2 turns over 10 min";
}

/// `now.overwatch`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusView {
    pub status: Status,
    /// Queued plus those in the running turn.
    pub events: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub since_ms: u64,
    pub agent: &'static str,
    pub has_session: bool,
}

/// What a turn carries (api-contract §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Permission,
    Question,
    Stuck,
    UserMessage,
    RuleCandidate,
    DecisionOpen,
    DecisionClosed,
    Outcome,
}

impl Kind {
    /// Wakes the Overwatch at once.
    pub fn blocking(self) -> bool {
        matches!(self, Kind::Permission | Kind::Question | Kind::Stuck | Kind::UserMessage | Kind::RuleCandidate)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Permission => "permission",
            Kind::Question => "question",
            Kind::Stuck => "stuck",
            Kind::UserMessage => "user_message",
            Kind::RuleCandidate => "rule_candidate",
            Kind::DecisionOpen => "decision_open",
            Kind::DecisionClosed => "decision_closed",
            Kind::Outcome => "outcome",
        }
    }
}

/// `stuck` (B3 fills it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stuck {
    pub why: String,
    pub since_ms: u64,
}

/// One event (api-contract §2.1). `id` is set by [`Sched::push`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub at_ms: u64,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Stuck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<Value>,
}

impl Event {
    pub fn new(kind: Kind, summary: impl Into<String>) -> Event {
        Event {
            id: String::new(),
            at_ms: 0,
            kind,
            decision: None,
            agent: None,
            run: None,
            summary: summary.into(),
            deadline_ms: None,
            stuck: None,
            text: None,
            chat: None,
            candidate: None,
        }
    }

    /// Its line in a turn (stable: B3's tests read it):
    /// `- [ev-12] permission inbox-42 back-1 · <summary> · until 14:03:05`.
    pub fn line(&self, tz: &jiff::tz::TimeZone) -> String {
        let mut l = format!("- [{}] {}", self.id, self.kind.as_str());
        for r in [&self.decision, &self.agent].into_iter().flatten() {
            l.push(' ');
            l.push_str(r);
        }
        let summary = self.summary.replace('\n', " ");
        if !summary.is_empty() {
            l.push_str(" · ");
            l.push_str(&summary);
        }
        if let Some(t) = self.deadline_ms {
            l.push_str(" · until ");
            l.push_str(&hms(t, tz));
        }
        l
    }
}

/// `HH:MM:SS` local.
pub fn hms(ms: u64, tz: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(ms as i64)
        .map(|t| t.to_zoned(tz.clone()).strftime("%H:%M:%S").to_string())
        .unwrap_or_default()
}

/// How a turn reached the session (`overwatch.turn.how`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// The day's first session (lazy start, or a new day).
    Spawned,
    /// A fresh session after `overwatch_restart` (or turning it on again).
    Fresh,
    /// The suspended session resumed (`--resume`).
    Resumed,
    /// The session was running.
    Live,
}

/// The turn in flight.
#[derive(Clone, Debug)]
pub struct Turn {
    pub events: Vec<Event>,
    /// The oldest event's.
    pub queued_at_ms: u64,
    /// Delivered at first; its TurnStarted hook once seen (`started`).
    pub started_at_ms: u64,
    pub started: bool,
    pub first_tool_ms: Option<u64>,
    pub how: How,
    pub cache_warm: bool,
}

/// The scheduler's state (one lock on the runtime).
#[derive(Debug, Default)]
pub struct Sched {
    pub queue: Vec<Event>,
    /// User messages kept while `off` / `down`, delivered on restart.
    pub pending_user: Vec<Event>,
    pub in_flight: Option<Turn>,
    /// The last id given (`ev-<n>`), seeded from the day folder.
    pub seq: u64,
    /// The day `seq` and the session belong to.
    pub day: Option<String>,
    /// `down` and why (its `since_ms` is `shown`'s).
    pub down: Option<String>,
    /// Turns in a row past the ceiling.
    pub long_turns: u32,
    pub last_turn_end_ms: Option<u64>,
    /// The next turn starts a new session (restart, turned on again).
    pub fresh: bool,
    /// What `now.overwatch` last said (its `since_ms` lasts the status).
    pub shown: Option<(Status, Option<String>, u64)>,
    /// Today's turns (`orchd overwatch`), from the events at first read.
    pub turns: Vec<Value>,
}

impl Sched {
    /// Queues `ev` with the next id; returns the id.
    pub fn push(&mut self, mut ev: Event, now: u64) -> String {
        self.seq += 1;
        ev.id = format!("ev-{}", self.seq);
        if ev.at_ms == 0 {
            ev.at_ms = now;
        }
        let id = ev.id.clone();
        self.queue.push(ev);
        id
    }

    /// A turn is due at `now`: one blocking event, or the oldest batched
    /// one has waited `batch_ms`.
    pub fn due(&self, now: u64, batch_ms: u64) -> bool {
        self.queue.iter().any(|e| e.kind.blocking())
            || self.queue.iter().map(|e| e.at_ms).min().is_some_and(|t| now >= t.saturating_add(batch_ms))
    }

    /// The count `now.overwatch.events` shows.
    pub fn events(&self) -> u32 {
        (self.queue.len() + self.in_flight.as_ref().map_or(0, |t| t.events.len())) as u32
    }
}

/// The turn text (api-contract §2.1): a header line, then one line per
/// event. A fresh session first reads its briefing.
pub fn turn_text(events: &[Event], fresh: bool, now: u64, tz: &jiff::tz::TimeZone) -> String {
    let n = events.len();
    let mut out = String::new();
    if fresh {
        out.push_str("ORCH: Read briefing.md, then handle these events.\n");
    }
    out.push_str(&format!("ORCH: Overwatch · {n} event{} · {}\n", if n == 1 { "" } else { "s" }, hms(now, tz)));
    for e in events {
        out.push_str(&e.line(tz));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// p50 and p90 of `v` (nearest rank), `None` when empty.
pub fn percentiles(v: &[u64]) -> Option<(u64, u64)> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    let at = |p: f64| s[((p * s.len() as f64).ceil() as usize).clamp(1, s.len()) - 1];
    Some((at(0.5), at(0.9)))
}

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// `orchd overwatch` (api-contract §4): the status, today's session, one
/// line per turn (`09:12:04 resumed warm events=3 wake=6.2s first_tool=8.1s 41.0s`)
/// and the p50 / p90 wake (oldest event queued → first tool call).
pub fn render_report(r: &Value, tz: &jiff::tz::TimeZone) -> String {
    let st = &r["status"];
    let mut out = format!("Overwatch: {}", st["status"].as_str().unwrap_or("?"));
    if let Some(w) = st["why"].as_str() {
        out.push_str(&format!(" · {w}"));
    }
    out.push_str(&format!(" · {} event(s)\n", st["events"].as_u64().unwrap_or(0)));
    out.push_str(&format!(
        "day {} · session {}\n",
        r["day"].as_str().unwrap_or("?"),
        r["session"].as_str().unwrap_or("none")
    ));
    let turns = r["turns"].as_array().cloned().unwrap_or_default();
    for t in &turns {
        let n = |k: &str| t[k].as_u64();
        let started = n("started_at_ms").unwrap_or(0);
        let mut l = format!(
            "{} {} {} events={}",
            hms(started, tz),
            t["how"].as_str().unwrap_or("?"),
            if t["cache_warm"].as_bool() == Some(true) { "warm" } else { "cold" },
            n("events").unwrap_or(0)
        );
        match (n("queued_at_ms"), n("first_tool_ms")) {
            (Some(q), Some(f)) => l.push_str(&format!(
                " wake={} first_tool={}",
                secs(f.saturating_sub(q)),
                secs(f.saturating_sub(started))
            )),
            _ => l.push_str(" wake=- first_tool=-"),
        }
        if let Some(e) = n("ended_at_ms") {
            l.push_str(&format!(" {}", secs(e.saturating_sub(started))));
        }
        out.push_str(&l);
        out.push('\n');
    }
    if turns.is_empty() {
        out.push_str("no turn today\n");
    }
    match (r["wake_p50_ms"].as_u64(), r["wake_p90_ms"].as_u64()) {
        (Some(a), Some(b)) => out.push_str(&format!("wake p50 {} · p90 {}\n", secs(a), secs(b))),
        _ => out.push_str("wake p50 - · p90 -\n"),
    }
    out
}
