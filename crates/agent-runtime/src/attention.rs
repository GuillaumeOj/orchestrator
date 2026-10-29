//! Attention records (FR-1301, P4.1 spec §3.1): what each agent and each team
//! run is doing or waiting on, and the counts every surface shows. One pure
//! pass over the runtime's state: no I/O, no locks, no clock but `now_ms`.
//!
//! The rules are the single definition of "needs you", "queued" and the
//! rest: `needs_you` ⇔ `waiting_on.kind = user`, first matching rule wins.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use governor::{QueueEntry, QueueReason, Window};
use jiff::tz::TimeZone;
use orch_store::{InboxItem, InboxStatus, Message, Task, TeamRun};
use orch_team::{RunState, TaskStatus};
use serde::Serialize;
use serde_json::Value;

use crate::state::AgentState;
use crate::store::AgentRecord;

/// An MCP tool call running this long is a wait on something outside (D4).
pub const EXTERNAL_AFTER_MS: u64 = 5 * 60 * 1000;

/// The lane of an agent or a run. Declared in rank order: a run's level is
/// its highest member's ([`Level::rank`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Done,
    Idle,
    Queued,
    Waiting,
    Running,
    NeedsYou,
    Error,
}

/// The §9.5 badge (replaces the UI's `displayState()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    WaitingForYou,
    WaitingGate,
    Queued,
    NotStarted,
    Paused,
    BlockedByLimit,
    Idle,
    Suspended,
    Error,
    Done,
}

impl Status {
    /// The §9.5 badge label (text with no badge prints `<label> · <reason>`).
    pub fn label(self) -> &'static str {
        match self {
            Status::Working => "Working",
            Status::WaitingForYou => "Waiting for you",
            Status::WaitingGate => "At gate",
            Status::Queued => "Queued",
            Status::NotStarted => "Not started yet",
            Status::Paused => "Paused",
            Status::BlockedByLimit => "Blocked by limit",
            Status::Idle => "Idle",
            Status::Suspended => "Suspended",
            Status::Error => "Error",
            Status::Done => "Done",
        }
    }

    /// [`Status::label`] of a serialized key ("Idle" for an unknown one).
    pub fn label_of(key: &str) -> &'static str {
        serde_json::from_value(Value::String(key.to_string())).map_or("Idle", Status::label)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    User,
    Agent,
    Task,
    Governor,
    Limit,
    Startup,
    External,
}

impl WaitKind {
    /// A wait on something outside the run (a run's `blocker`).
    pub fn external_to_run(self) -> bool {
        matches!(self, WaitKind::User | WaitKind::Governor | WaitKind::Limit | WaitKind::Startup | WaitKind::External)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WaitKind::User => "user",
            WaitKind::Agent => "agent",
            WaitKind::Task => "task",
            WaitKind::Governor => "governor",
            WaitKind::Limit => "limit",
            WaitKind::Startup => "startup",
            WaitKind::External => "external",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaitingOn {
    pub kind: WaitKind,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    pub since_ms: u64,
}

/// One agent's record (api-contract §1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentAttention {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<u64>,
    pub level: Level,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<WaitingOn>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub live: bool,
    pub since_ms: u64,
    /// `since_ms` came from a stored timestamp; else the publisher fills in
    /// when it first saw this `(level, kind, ref)` (D7).
    #[serde(skip)]
    pub since_stored: bool,
}

impl AgentAttention {
    /// What identifies a wait episode: a new one restarts `since`.
    pub fn episode(&self) -> (Level, Option<WaitKind>, Option<String>) {
        let w = self.waiting_on.as_ref();
        (self.level, w.map(|w| w.kind), w.and_then(|w| w.r#ref.clone()))
    }

    /// Sets `since_ms` (and the wait's) once known.
    pub fn set_since(&mut self, ms: u64) {
        self.since_ms = ms;
        if let Some(w) = &mut self.waiting_on {
            w.since_ms = ms;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Blocker {
    pub agent: String,
    pub waiting_on: WaitingOn,
    pub reason: String,
}

/// One team run's record (api-contract §1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunAttention {
    pub run: String,
    pub feature: String,
    pub project: String,
    pub phase: RunState,
    pub phase_since_ms: u64,
    pub level: Level,
    pub members: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<Blocker>,
    pub counts: Counts,
}

/// The numbers every surface shows and none computes (spec §3.1 Counts).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub needs_you: u32,
    /// Open decisions of levels 1–3 waiting on the user (the title badge).
    pub decisions: u32,
    pub running: u32,
    pub waiting: u32,
    /// Agents at level `queued`: queue entries, not started and paused.
    pub queued: u32,
    pub idle: u32,
    pub error: u32,
    /// Live processes, archived agents excluded.
    pub live: u32,
    /// Global only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_cap: Option<u32>,
    pub done_today: u32,
}

impl Counts {
    fn add(&mut self, a: &AgentAttention) {
        match a.level {
            Level::NeedsYou => self.needs_you += 1,
            Level::Running => self.running += 1,
            Level::Waiting => self.waiting += 1,
            Level::Queued => self.queued += 1,
            Level::Idle => self.idle += 1,
            Level::Error => self.error += 1,
            Level::Done => {}
        }
        if a.live {
            self.live += 1;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AllCounts {
    pub global: Counts,
    pub projects: BTreeMap<String, Counts>,
}

/// Everything `compute` returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    pub agents: BTreeMap<String, AgentAttention>,
    pub runs: BTreeMap<String, RunAttention>,
    pub counts: AllCounts,
}

/// A team run with what `compute` needs besides its row.
#[derive(Clone, Debug)]
pub struct RunFacts {
    pub run: TeamRun,
    pub project: String,
    /// The feature's slug (reasons name it: "spec gate · checkout").
    pub feature: String,
    /// When the run entered its phase (its last `run.state` event, else its start).
    pub phase_since_ms: u64,
}

/// What the Governor says beyond its queue: usage limits ("5h window full ·
/// resets 16:00"), the live count its cap counts, allocation hard caps.
#[derive(Clone, Debug, Default)]
pub struct GovFacts {
    /// Live autonomous agents (what "live cap 3/3" counts).
    pub live_autonomous: u32,
    /// Allocations with a hard cap: `(used pp, cap pp)`.
    pub hard_caps: BTreeMap<String, (f64, f64)>,
    /// The window an agent blocked by a limit waits on (the exhausted one).
    pub exhausted: Option<Window>,
    pub five_hour_resets_ms: Option<u64>,
    pub week_resets_ms: Option<u64>,
    /// The 5h reading (%), for the 5h guard.
    pub five_hour_pct: Option<f64>,
}

/// The Governor's pending ask.
#[derive(Clone, Debug)]
pub struct AskFacts {
    pub id: u64,
    /// The agent it is about (a queued turn it would run), if any.
    pub agent: Option<String>,
    pub created_ms: u64,
    /// Running it now costs this, pp of the week.
    pub cost_pp: Option<f64>,
    /// The action's reason, when it names no cost.
    pub reason: String,
}

/// What `compute` reads. The runtime gathers it under its locks, then lets go.
pub struct Inputs<'a> {
    pub agents: &'a BTreeMap<String, AgentRecord>,
    pub inbox: &'a [InboxItem],
    /// Agents with a `claude` process.
    pub live: &'a BTreeSet<String>,
    /// The Governor's queue, in release order.
    pub queue: &'a [QueueEntry],
    /// Agents paused one by one (FR-805).
    pub paused: &'a BTreeSet<String>,
    /// The Governor's pending ask (a level-3 decision), if any.
    pub ask: Option<&'a AskFacts>,
    pub gov: &'a GovFacts,
    pub runs: &'a [RunFacts],
    /// The tasks of `runs`.
    pub tasks: &'a [Task],
    /// The messages of `runs`, oldest first.
    pub messages: &'a [Message],
    /// Registered projects by main checkout.
    pub projects: &'a BTreeMap<PathBuf, String>,
    pub startup_timeout_ms: u64,
    pub live_cap: u32,
    pub done_today: u32,
    /// Local time for clock times ("≈14:20", "resets Mon 09:00").
    pub tz: &'a TimeZone,
}

/// The listed id of a store item (the Inbox's, P4.0): `inbox-3`, `gate-5`…
pub fn decision_id(kind: &str, id: u64) -> String {
    match kind {
        "permission" => format!("inbox-{id}"),
        "startup" => format!("startup-{id}"),
        "gate" => format!("gate-{id}"),
        "question" => format!("q-{id}"),
        _ => format!("alert-{id}"),
    }
}

/// An open store item's decision level (1 blocking, 2 errors, 3 budget, 4 info).
// TODO(T-22): B2 moves this into the core's Decision builder; mirrors orch-app inbox.rs `build` until then.
fn item_level(i: &InboxItem) -> u8 {
    match i.kind.as_str() {
        "permission" | "gate" | "question" => 1,
        "notion" | "budget" => 3,
        "info" => 4,
        _ => 2,
    }
}

/// An open item that puts its agent in error (spec rule 1, D3).
fn error_item(i: &InboxItem) -> bool {
    match i.kind.as_str() {
        "startup" => true,
        "alert" => {
            matches!(i.payload.get("alert").and_then(Value::as_str), Some("api_error" | "spawn_failed"))
                || i.payload.get("action").and_then(Value::as_str) == Some("watchdog")
        }
        _ => false,
    }
}

/// The reason strings (ux-brief §2, p4-design-notes §2, D11): what the
/// agent waits on, never the status label, never a duration (the UI
/// appends `since`). UX copy changes are one-line edits here.
pub mod copy {
    use super::*;

    pub fn permission(tool: &str, input: &str, parked: bool) -> String {
        let what = if parked { "permission (parked)" } else { "permission" };
        if input.is_empty() { format!("{what}: {tool}") } else { format!("{what}: {tool} {input}") }
    }
    pub fn question(first_line: &str) -> String {
        format!("question: {}", cut(first_line, 60))
    }
    pub fn questions(n: usize, header: &str) -> String {
        format!("{n} questions: {header}")
    }
    pub fn spec_gate(feature: &str) -> String {
        format!("spec gate · {feature}")
    }
    pub fn pr_gate(feature: &str) -> String {
        format!("PR gate · {feature}")
    }
    pub fn restart_gate(detail: &str) -> String {
        format!("restart gate · {}", cut(detail, 40))
    }
    pub fn ask(cost_pp: Option<f64>, reason: &str) -> String {
        match cost_pp {
            Some(c) => format!("run now? ≈ +{c:.1} pp of the week"),
            None => cut(reason, 60),
        }
    }
    pub const TRUST: &str = "trust dialog in its pane";
    pub const PANE_PROMPT: &str = "a prompt in its pane";
    pub fn review(to: &str, task: &str) -> String {
        format!("→ {to} · review of {task}")
    }
    pub fn plan(to: &str, task: &str) -> String {
        format!("→ {to} · plan for {task}")
    }
    /// `what`: message, handoff, review.
    pub fn message(to: &str, what: &str) -> String {
        format!("→ {to} · {what}")
    }
    pub fn lead_waits(agents: &[String], tasks: &[String]) -> String {
        let list = |v: &[String]| match v.len() {
            0..=2 => v.join(", "),
            n => format!("{} +{} more", v[..2].join(", "), n - 2),
        };
        format!("→ {} · {}", list(agents), list(tasks))
    }
    pub fn blocked_by(task: &str, owner: &str, status: &str) -> String {
        format!("blocked by {task} ({owner}, {status})")
    }
    pub fn limit(window: Option<Window>, resets: Option<&str>) -> String {
        let w = match window {
            Some(Window::Weekly) => "week full",
            Some(Window::FiveHour) => "5h window full",
            None => "limit reached",
        };
        match resets {
            Some(t) => format!("{w} · resets {t}"),
            None => w.to_string(),
        }
    }
    pub const STARTING: &str = "starting";
    pub fn starting(timeout_s: u64) -> String {
        format!("starting · times out after {timeout_s} s")
    }
    pub fn external(server: &str, tool: &str) -> String {
        format!("waiting on {server} {tool}")
    }
    pub const PAUSED_BY_YOU: &str = "by you";
    pub const PAUSE_ALL: &str = "Pause all";
    pub const WAITING_FOR_SLOT: &str = "waiting for a slot";
    pub const STOPPED_WITH_ERROR: &str = "stopped with an error";
    pub fn no_session(timeout_s: u64) -> String {
        format!("no session after {timeout_s} s")
    }
    pub fn start_exited(mode: &str, status: Option<u64>) -> String {
        match status {
            Some(n) => format!("{mode} start exited (status {n})"),
            None => format!("{mode} start exited"),
        }
    }
    pub const WRITING: &str = "writing";

    /// p4-design-notes §2.2: `<reason>[ · <position>][ · ≈<eta>]`. The
    /// position is left out for a not-started agent; reasons with their own
    /// time carry no ETA and no position.
    pub fn queued(e: &QueueEntry, g: &GovFacts, clock: &dyn Fn(u64) -> String) -> (String, bool) {
        let at = |t: &jiff::Timestamp| clock(t.as_millisecond().max(0) as u64);
        let opt = |ms: Option<u64>| ms.map(clock);
        let live = g.live_autonomous;
        let s = match &e.reason {
            QueueReason::LiveCap { state, cap } => match state {
                governor::PressureState::Green => format!("live cap {live}/{cap}"),
                s => format!("live cap {live}/{cap} ({})", pressure(*s)),
            },
            QueueReason::RedTier => format!("Red · {} priority waits", tier_word(e.effective_tier)),
            QueueReason::OffHours => "outside working hours".into(),
            QueueReason::FiveHourGuard => match g.five_hour_pct {
                Some(f) => format!("5h guard · 5h window at {f:.0} %"),
                None => "5h guard".into(),
            },
            QueueReason::LimitReached { window } => {
                let resets = match window {
                    Window::FiveHour => g.five_hour_resets_ms,
                    Window::Weekly => g.week_resets_ms,
                };
                return (limit(Some(*window), opt(resets).as_deref()), true);
            }
            QueueReason::DeferredUntilReset => {
                let t = opt(g.week_resets_ms);
                return (
                    match t {
                        Some(t) => format!("deferred to the weekly reset · {t}"),
                        None => "deferred to the weekly reset".into(),
                    },
                    true,
                );
            }
            QueueReason::HardCap => match g.hard_caps.get(&e.allocation.0) {
                Some((used, cap)) => format!("{} cap reached · {used:.0} of {cap:.0} pp", e.allocation.0),
                None => format!("{} cap reached", e.allocation.0),
            },
            QueueReason::PausedAll => PAUSE_ALL.into(),
            QueueReason::PausedAgent => PAUSED_BY_YOU.into(),
            QueueReason::MemoryPressure { cap } => format!("memory pressure · live cap {live}/{cap}"),
            QueueReason::BehindQueue => "behind higher-priority work".into(),
            QueueReason::ScheduledOffHours { starts_at } => {
                return (format!("off-hours · starts {}", at(starts_at)), true);
            }
            QueueReason::ModelWindow { model, resets_at, exhausted: true } => {
                return (format!("{model} week full · starts {}", at(resets_at)), true);
            }
            QueueReason::ModelWindow { model, resets_at, .. } => {
                return (format!("{model} week in Red · starts after reset {}", at(resets_at)), true);
            }
        };
        (s, false)
    }

    pub fn position(n: usize) -> String {
        match n {
            1 => "next".into(),
            n => format!("{n}{} in line", ordinal_suffix(n)),
        }
    }

    pub fn eta(clock: &str) -> String {
        format!("≈{clock}")
    }

    fn ordinal_suffix(n: usize) -> &'static str {
        match (n % 10, n % 100) {
            (_, 11..=13) => "th",
            (1, _) => "st",
            (2, _) => "nd",
            (3, _) => "rd",
            _ => "th",
        }
    }

    fn pressure(s: governor::PressureState) -> &'static str {
        match s {
            governor::PressureState::Green => "Green",
            governor::PressureState::Amber => "Amber",
            governor::PressureState::Red => "Red",
            governor::PressureState::CatchUp => "Catch-up",
        }
    }

    fn tier_word(t: governor::Tier) -> &'static str {
        match t {
            governor::Tier::Low => "low",
            governor::Tier::Normal => "normal",
            governor::Tier::High => "high",
            governor::Tier::Critical => "critical",
        }
    }

    pub fn task_status(s: TaskStatus) -> &'static str {
        match s {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Ready => "ready",
            TaskStatus::InProgress => "in progress",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
        }
    }

    /// A tool and its short input: "Bash cargo test", "Edit src/feed.ts",
    /// "penpot execute_code" (p4-design-notes §2.1 notes).
    pub fn tool(tool: &str, brief: Option<&str>) -> String {
        if let Some((server, t)) = crate::registry::mcp_tool(tool) {
            return format!("{server} {t}");
        }
        match brief.filter(|b| !b.is_empty()) {
            Some(b) => format!("{tool} {}", cut(b, 60)),
            None => tool.to_string(),
        }
    }

    /// The first line, at most `n` characters ("…" when cut).
    pub fn cut(s: &str, n: usize) -> String {
        let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        if line.chars().count() <= n {
            line.to_string()
        } else {
            format!("{}…", line.chars().take(n.saturating_sub(1)).collect::<String>())
        }
    }
}

/// Local clock time of `ms` seen from `now_ms`: "16:00" today, "Mon 09:00"
/// within 6 days, else "2 Nov 09:00".
pub fn clock(ms: u64, now_ms: u64, tz: &TimeZone) -> String {
    let z = |m: u64| jiff::Timestamp::from_millisecond(m as i64).unwrap_or_default().to_zoned(tz.clone());
    let (t, now) = (z(ms), z(now_ms));
    if t.date() == now.date() {
        t.strftime("%H:%M").to_string()
    } else if (t.date() - now.date()).get_days().abs() <= 6 && t.timestamp() > now.timestamp() {
        t.strftime("%a %H:%M").to_string()
    } else {
        t.strftime("%-d %b %H:%M").to_string()
    }
}

/// What one rule found.
struct Found {
    level: Level,
    status: Status,
    wait: Option<(WaitKind, Option<String>)>,
    reason: String,
    since: Option<u64>,
}

impl Found {
    fn new(level: Level, status: Status, reason: impl Into<String>, since: Option<u64>) -> Found {
        Found { level, status, wait: None, reason: reason.into(), since }
    }
    fn wait(mut self, kind: WaitKind, r: Option<String>) -> Found {
        self.wait = Some((kind, r));
        self
    }
}

/// Indexes built once per `compute`.
struct Ctx<'a> {
    i: &'a Inputs<'a>,
    now: u64,
    open: BTreeMap<&'a str, Vec<&'a InboxItem>>,
    entries: BTreeMap<&'a str, (usize, &'a QueueEntry)>,
    runs: BTreeMap<&'a str, &'a RunFacts>,
    tasks: BTreeMap<u64, &'a Task>,
    /// Agents by (run, node).
    seats: BTreeMap<(&'a str, &'a str), &'a str>,
    live_count: u32,
}

fn is_lead(a: &AgentRecord) -> bool {
    crate::runtime::reserved_slot(a)
}

/// Between turns: idle, or suspended/killed with its session kept.
fn at_rest(s: AgentState) -> bool {
    matches!(s, AgentState::Idle | AgentState::Suspended | AgentState::Killed)
}

impl<'a> Ctx<'a> {
    fn new(i: &'a Inputs<'a>, now: u64) -> Ctx<'a> {
        let mut open: BTreeMap<&str, Vec<&InboxItem>> = BTreeMap::new();
        for it in i.inbox.iter().filter(|it| it.open()) {
            if let Some(a) = it.agent_id.as_deref() {
                open.entry(a).or_default().push(it);
            }
        }
        for v in open.values_mut() {
            v.sort_by_key(|it| (it.created_at_ms, it.id));
        }
        let mut entries = BTreeMap::new();
        for (n, e) in i.queue.iter().enumerate() {
            entries.entry(e.agent.0.as_str()).or_insert((n + 1, e));
        }
        let seats = i
            .agents
            .values()
            .filter_map(|a| a.team.as_ref().map(|t| ((t.run_id.as_str(), t.node.as_str()), a.id.as_str())))
            .collect();
        let live_count =
            i.agents.values().filter(|a| a.tracker.state != AgentState::Archived && i.live.contains(&a.id)).count()
                as u32;
        Ctx {
            i,
            now,
            open,
            entries,
            runs: i.runs.iter().map(|r| (r.run.id.as_str(), r)).collect(),
            tasks: i.tasks.iter().map(|t| (t.id, t)).collect(),
            seats,
            live_count,
        }
    }

    fn clock(&self, ms: u64) -> String {
        clock(ms, self.now, self.i.tz)
    }

    fn items(&self, id: &str) -> &[&'a InboxItem] {
        self.open.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn agent_of(&self, run: &str, node: &str) -> String {
        self.seats.get(&(run, node)).map(|s| s.to_string()).unwrap_or_else(|| node.to_string())
    }

    fn feature_of<'s>(&'s self, a: &'s AgentRecord) -> Option<&'s str> {
        let seat = a.team.as_ref()?;
        Some(self.runs.get(seat.run_id.as_str()).map(|r| r.feature.as_str()).unwrap_or(seat.feature_id.as_str()))
    }

    /// Rule 1: error.
    fn error(&self, a: &AgentRecord) -> Option<Found> {
        let started = a.tracker.turn_started_ms.unwrap_or(0);
        let running = matches!(a.tracker.state, AgentState::Working | AgentState::Starting);
        // An alert from before the turn now running is history, not the agent's state.
        let item = self.items(&a.id).iter().find(|it| error_item(it) && (!running || it.created_at_ms >= started));
        if let Some(it) = item {
            let s = |k: &str| it.payload.get(k).and_then(Value::as_str);
            return Some(if it.kind == "startup" {
                let reason = match s("failed") {
                    Some(mode) => copy::start_exited(mode, it.payload.get("exit_status").and_then(Value::as_u64)),
                    None => copy::no_session(self.i.startup_timeout_ms / 1000),
                };
                Found::new(Level::Error, Status::Error, reason, Some(it.created_at_ms))
                    .wait(WaitKind::Startup, Some(decision_id(&it.kind, it.id)))
            } else {
                let line = it.body.strip_prefix(&format!("{}: ", a.id)).unwrap_or(&it.body);
                Found::new(Level::Error, Status::Error, copy::cut(line, 120), Some(it.created_at_ms))
            });
        }
        (a.tracker.state == AgentState::Error).then(|| {
            let why = a.tracker.last_error.as_deref().filter(|e| !e.is_empty()).unwrap_or(copy::STOPPED_WITH_ERROR);
            Found::new(Level::Error, Status::Error, copy::cut(why, 120), a.last_event_at_ms)
        })
    }

    /// Rule 2: needs you. Ranked permission > question > gate > ask > trust > pane prompt, oldest first.
    fn needs_you(&self, a: &AgentRecord) -> Option<Found> {
        let rank = |it: &InboxItem| match it.kind.as_str() {
            "permission" => Some(0),
            "question" => Some(1),
            "gate" => Some(2),
            _ => None,
        };
        let items = self.items(&a.id);
        if let Some(it) = items
            .iter()
            .filter_map(|it| rank(it).map(|r| (r, *it)))
            .min_by_key(|(r, it)| (*r, it.created_at_ms))
            .map(|(_, it)| it)
        {
            let s = |k: &str| it.payload.get(k).and_then(Value::as_str).unwrap_or("");
            let (status, reason) = match it.kind.as_str() {
                "permission" => {
                    let input =
                        it.payload.get("tool_input").or_else(|| it.payload.get("input")).unwrap_or(&Value::Null);
                    let short = crate::permissions::primary_input(s("tool"), input).unwrap_or_default();
                    let tool = copy::tool(s("tool"), Some(&short));
                    let (tool, input) = tool.split_once(' ').unwrap_or((&tool, ""));
                    (Status::WaitingForYou, copy::permission(tool, input, it.status == InboxStatus::Parked))
                }
                "question" => {
                    let qs = it.payload.get("questions").and_then(Value::as_array);
                    let reason = match qs {
                        Some(q) if q.len() > 1 => {
                            copy::questions(q.len(), q[0].get("header").and_then(Value::as_str).unwrap_or(""))
                        }
                        _ => copy::question(if s("question").is_empty() { &it.body } else { s("question") }),
                    };
                    (Status::WaitingForYou, reason)
                }
                _ => {
                    let feature = self.feature_of(a).map(str::to_string).unwrap_or_else(|| s("feature").to_string());
                    let reason = match s("action") {
                        "push_pr" => copy::pr_gate(&feature),
                        "restart_gate" => copy::restart_gate(s("detail")),
                        _ => copy::spec_gate(&feature),
                    };
                    (Status::WaitingGate, reason)
                }
            };
            return Some(
                Found::new(Level::NeedsYou, status, reason, Some(it.created_at_ms))
                    .wait(WaitKind::User, Some(decision_id(&it.kind, it.id))),
            );
        }
        if let Some(ask) = self.i.ask.filter(|k| k.agent.as_deref() == Some(a.id.as_str())) {
            return Some(
                Found::new(
                    Level::NeedsYou,
                    Status::WaitingForYou,
                    copy::ask(ask.cost_pp, &ask.reason),
                    Some(ask.created_ms),
                )
                .wait(WaitKind::User, Some(format!("ask-{}", ask.id))),
            );
        }
        let last = a.last_event_at_ms.or(Some(a.created_at_ms));
        if a.trust_dialog {
            return Some(
                Found::new(Level::NeedsYou, Status::WaitingForYou, copy::TRUST, last)
                    .wait(WaitKind::User, Some(format!("trust-{}", a.id))),
            );
        }
        (a.tracker.state == AgentState::WaitingForYou).then(|| {
            Found::new(Level::NeedsYou, Status::WaitingForYou, copy::PANE_PROMPT, last)
                .wait(WaitKind::User, Some(format!("wait-{}", a.id)))
        })
    }

    /// Rule 3: blocked by a usage limit.
    fn limit(&self, a: &AgentRecord) -> Option<Found> {
        (a.tracker.state == AgentState::BlockedByLimit).then(|| {
            let l = self.i.gov;
            let resets = match l.exhausted {
                Some(Window::Weekly) => l.week_resets_ms,
                Some(Window::FiveHour) => l.five_hour_resets_ms,
                None => None,
            };
            let window = l.exhausted.map(|w| match w {
                Window::FiveHour => "five_hour",
                Window::Weekly => "weekly",
            });
            Found::new(
                Level::Waiting,
                Status::BlockedByLimit,
                copy::limit(l.exhausted, resets.map(|t| self.clock(t)).as_deref()),
                a.last_event_at_ms,
            )
            .wait(WaitKind::Limit, window.map(str::to_string))
        })
    }

    /// Rule 4: queued (a queue entry), not started, or paused.
    fn queued(&self, a: &AgentRecord, live: bool) -> Option<Found> {
        let s = a.tracker.state;
        // A running turn whose next one is queued is running; a held tool call is queued.
        let in_turn = matches!(s, AgentState::Working | AgentState::Starting);
        let governor = |status, reason: String, since| {
            Found::new(Level::Queued, status, reason, since).wait(WaitKind::Governor, Some(a.id.clone()))
        };
        let not_started = s == AgentState::Created || !live;
        if let Some((pos, e)) = self.entries.get(a.id.as_str()).filter(|_| !in_turn) {
            let since = Some(e.enqueued_at.as_millisecond().max(0) as u64);
            match e.reason {
                QueueReason::PausedAll => return Some(governor(Status::Paused, copy::PAUSE_ALL.into(), since)),
                QueueReason::PausedAgent => return Some(governor(Status::Paused, copy::PAUSED_BY_YOU.into(), since)),
                _ => {}
            }
            let clock = |ms| self.clock(ms);
            let (mut reason, own_time) = copy::queued(e, self.i.gov, &clock);
            if !own_time {
                if !not_started {
                    reason = format!("{reason} · {}", copy::position(*pos));
                }
                if let Some(t) = e.estimated_start {
                    reason = format!("{reason} · {}", copy::eta(&self.clock(t.as_millisecond().max(0) as u64)));
                }
            }
            let status = if not_started { Status::NotStarted } else { Status::Queued };
            return Some(governor(status, reason, since));
        }
        let busy = matches!(
            s,
            AgentState::Working
                | AgentState::Starting
                | AgentState::WaitingForYou
                | AgentState::WaitingGate
                | AgentState::Error
        );
        if self.i.paused.contains(&a.id) && !busy {
            return Some(governor(Status::Paused, copy::PAUSED_BY_YOU.into(), None));
        }
        if s == AgentState::Created {
            return Some(governor(Status::NotStarted, copy::WAITING_FOR_SLOT.into(), Some(a.created_at_ms)));
        }
        (s == AgentState::Queued)
            .then(|| governor(Status::Queued, "behind higher-priority work".into(), a.last_event_at_ms))
    }

    /// Rules 5 and 6: starting, running, or waiting on an external tool.
    fn running(&self, a: &AgentRecord, live: bool) -> Option<Found> {
        let t = &a.tracker;
        if t.state == AgentState::Starting && live {
            let spawned = a.spawned_at_ms.unwrap_or(a.created_at_ms);
            if self.now.saturating_sub(spawned) < self.i.startup_timeout_ms {
                return Some(
                    Found::new(
                        Level::Waiting,
                        Status::Working,
                        copy::starting(self.i.startup_timeout_ms / 1000),
                        Some(spawned),
                    )
                    .wait(WaitKind::Startup, None),
                );
            }
        }
        // A `Queued` agent never gets here: rule 4 always takes it.
        if !matches!(t.state, AgentState::Working | AgentState::Starting) {
            return None;
        }
        if let (Some(tool), Some(at)) = (&t.current_tool, t.tool_started_ms)
            && let Some((server, name)) = crate::registry::mcp_tool(tool)
            && self.now.saturating_sub(at) >= EXTERNAL_AFTER_MS
        {
            return Some(
                Found::new(Level::Waiting, Status::Working, copy::external(server, name), Some(at))
                    .wait(WaitKind::External, Some(tool.clone())),
            );
        }
        Some(Found::new(Level::Running, Status::Working, "", t.turn_started_ms.or(a.spawned_at_ms)))
    }

    fn activity(&self, a: &AgentRecord) -> String {
        match &a.tracker.current_tool {
            Some(tool) => copy::tool(tool, a.tracker.tool_brief.as_deref()),
            None if a.tracker.state == AgentState::Starting => copy::STARTING.into(),
            None => copy::WRITING.into(),
        }
    }

    /// The node `to` has answered `m` (sent it anything after it).
    fn answered(&self, m: &Message, my_node: &str) -> bool {
        self.i.messages.iter().any(|r| {
            r.team_run_id == m.team_run_id
                && r.from_node == m.to_node
                && r.to_node == my_node
                && r.created_at_ms >= m.created_at_ms
                && r.id != m.id
        })
    }

    /// Rule 7: waiting on another agent.
    fn on_agent(&self, a: &AgentRecord) -> Option<Found> {
        let seat = a.team.as_ref()?;
        let run = self.runs.get(seat.run_id.as_str())?;
        let task = seat.task.and_then(|t| self.tasks.get(&t)).filter(|t| t.status != TaskStatus::Done);
        let lead = is_lead(a);
        if task.is_none() && !(lead && !run.run.status.finished()) {
            return None;
        }
        let agent_wait = |reason: String, to: String, since: u64| {
            Some(Found::new(Level::Waiting, Status::Idle, reason, Some(since)).wait(WaitKind::Agent, Some(to)))
        };
        // The last message this agent sent since its last turn started, unanswered.
        let turn = a.tracker.turn_started_ms.unwrap_or(0);
        let sent = self.i.messages.iter().rev().find(|m| {
            m.team_run_id == run.run.id
                && m.from_agent.as_deref() == Some(a.id.as_str())
                && matches!(m.kind.as_str(), "message" | "handoff" | "review_request")
                && m.created_at_ms >= turn
        });
        let unanswered = sent.filter(|m| !self.answered(m, &seat.node));
        if let Some(t) = task {
            if let Some(m) = unanswered.filter(|m| m.kind == "review_request" && t.status == TaskStatus::Review) {
                let to = m.to_agent.clone().unwrap_or_else(|| self.agent_of(&run.run.id, &m.to_node));
                return agent_wait(copy::review(&to, &t.handle()), to, m.created_at_ms);
            }
            if t.flow.plan_first && t.flow.plan.is_some() && !t.flow.plan_approved {
                let to = self.lead_of(&run.run.id).unwrap_or_else(|| "lead".into());
                return agent_wait(copy::plan(&to, &t.handle()), to, t.updated_at_ms);
            }
        }
        if let Some(m) = unanswered {
            let to = m.to_agent.clone().unwrap_or_else(|| self.agent_of(&run.run.id, &m.to_node));
            let what = match m.kind.as_str() {
                "review_request" => "review",
                k => k,
            };
            return agent_wait(copy::message(&to, what), to, m.created_at_ms);
        }
        if lead && matches!(run.run.status, RunState::Executing | RunState::Integrating) {
            let mut working: Vec<&Task> = self
                .i
                .tasks
                .iter()
                .filter(|t| t.team_run_id.as_deref() == Some(run.run.id.as_str()) && t.status == TaskStatus::InProgress)
                .collect();
            working.sort_by_key(|t| t.id);
            if !working.is_empty() {
                let mut agents: Vec<String> = vec![];
                for t in &working {
                    let who = t
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| self.agent_of(&run.run.id, t.node.as_deref().unwrap_or("?")));
                    if !agents.contains(&who) {
                        agents.push(who);
                    }
                }
                let tasks: Vec<String> = working.iter().map(|t| t.handle()).collect();
                let since = working.iter().map(|t| t.updated_at_ms).min().unwrap_or(self.now);
                let first = agents[0].clone();
                return agent_wait(copy::lead_waits(&agents, &tasks), first, since);
            }
        }
        None
    }

    fn lead_of(&self, run: &str) -> Option<String> {
        self.i
            .agents
            .values()
            .find(|a| a.team.as_ref().is_some_and(|t| t.run_id == run) && is_lead(a))
            .map(|a| a.id.clone())
    }

    /// The tasks of `a`'s node in its run, by id.
    fn node_tasks(&self, a: &AgentRecord) -> Vec<&'a Task> {
        let Some(seat) = &a.team else { return vec![] };
        let mut v: Vec<&Task> = self
            .i
            .tasks
            .iter()
            .filter(|t| {
                t.team_run_id.as_deref() == Some(seat.run_id.as_str()) && t.node.as_deref() == Some(seat.node.as_str())
            })
            .collect();
        v.sort_by_key(|t| t.id);
        v
    }

    /// Rule 8: a member whose next task waits on unfinished dependencies.
    fn on_task(&self, a: &AgentRecord) -> Option<Found> {
        let seat = a.team.as_ref()?;
        if is_lead(a) {
            return None;
        }
        let tasks = self.node_tasks(a);
        if tasks.iter().any(|t| t.status == TaskStatus::InProgress) {
            return None;
        }
        let next = tasks.iter().find(|t| matches!(t.status, TaskStatus::Ready | TaskStatus::Backlog))?;
        let deps: Vec<String> = next.depends_on.iter().map(u64::to_string).collect();
        let done = |d: &str| {
            d.parse::<u64>().ok().and_then(|id| self.tasks.get(&id)).is_some_and(|t| t.status == TaskStatus::Done)
        };
        let blocked = orch_team::run::blocked_by(&deps, done);
        let first: u64 = blocked.first()?.parse().ok()?;
        let (handle, owner, status) = match self.tasks.get(&first) {
            Some(t) => (
                t.handle(),
                t.agent_id.clone().unwrap_or_else(|| self.agent_of(&seat.run_id, t.node.as_deref().unwrap_or("?"))),
                copy::task_status(t.status),
            ),
            None => (format!("T-{first}"), "?".into(), "unknown"),
        };
        Some(
            Found::new(
                Level::Waiting,
                Status::Idle,
                copy::blocked_by(&handle, &owner, status),
                Some(next.updated_at_ms),
            )
            .wait(WaitKind::Task, Some(handle)),
        )
    }

    /// Rule 9: done.
    fn done(&self, a: &AgentRecord) -> Option<Found> {
        let seat = a.team.as_ref()?;
        if let Some(r) = self.runs.get(seat.run_id.as_str())
            && r.run.status == RunState::Done
        {
            return Some(Found::new(Level::Done, Status::Done, "", r.run.ended_at_ms.or(Some(r.phase_since_ms))));
        }
        if is_lead(a) {
            return None;
        }
        let tasks = self.node_tasks(a);
        (!tasks.is_empty() && tasks.iter().all(|t| t.status == TaskStatus::Done))
            .then(|| Found::new(Level::Done, Status::Done, "", tasks.iter().map(|t| t.updated_at_ms).max()))
    }

    fn record(&self, a: &AgentRecord) -> AgentAttention {
        let live = self.i.live.contains(&a.id);
        let s = a.tracker.state;
        let found = if s == AgentState::Archived {
            Found::new(Level::Done, Status::Done, "", a.last_event_at_ms)
        } else {
            self.error(a)
                .or_else(|| self.needs_you(a))
                .or_else(|| self.limit(a))
                .or_else(|| self.queued(a, live))
                .or_else(|| self.running(a, live))
                .or_else(|| {
                    at_rest(s).then(|| self.on_agent(a).or_else(|| self.on_task(a)).or_else(|| self.done(a))).flatten()
                })
                .unwrap_or_else(|| {
                    let status = if matches!(s, AgentState::Suspended | AgentState::Killed) {
                        Status::Suspended
                    } else {
                        Status::Idle
                    };
                    Found::new(Level::Idle, status, "", a.last_event_at_ms)
                })
        };
        let seat = a.team.as_ref();
        let since = found.since;
        let mut rec = AgentAttention {
            agent: a.id.clone(),
            project: seat.map(|t| t.project_id.clone()).or_else(|| self.i.projects.get(&a.repo).cloned()),
            run: seat.map(|t| t.run_id.clone()),
            node: seat.map(|t| t.node.clone()),
            task: seat.and_then(|t| t.task),
            activity: (found.level == Level::Running).then(|| self.activity(a)),
            level: found.level,
            status: found.status,
            waiting_on: found.wait.map(|(kind, r)| WaitingOn { kind, r#ref: r, since_ms: 0 }),
            reason: found.reason,
            live: live && s != AgentState::Archived,
            since_ms: 0,
            since_stored: since.is_some(),
        };
        rec.set_since(since.unwrap_or(0));
        rec
    }
}

/// A run's blocker: its highest, oldest member wait on something outside the
/// run. The publisher picks it again once every `since_ms` is known (D7).
pub fn blocker(members: &[&AgentAttention]) -> Option<Blocker> {
    let mut ranked: Vec<&&AgentAttention> = members.iter().collect();
    ranked.sort_by_key(|m| (std::cmp::Reverse(m.level), m.since_ms, m.agent.clone()));
    ranked.iter().find_map(|m| {
        let w = m.waiting_on.as_ref().filter(|w| w.kind.external_to_run())?;
        Some(Blocker { agent: m.agent.clone(), waiting_on: w.clone(), reason: m.reason.clone() })
    })
}

/// The run's record from its members' records.
fn run_record(r: &RunFacts, members: &[&AgentAttention]) -> RunAttention {
    let mut counts = Counts::default();
    for m in members {
        counts.add(m);
    }
    let mut level = members.iter().map(|m| m.level).max().unwrap_or(Level::Idle);
    if matches!(r.run.status, RunState::SpecGate | RunState::PrGate) {
        level = level.max(Level::NeedsYou);
    }
    let blocker = blocker(members);
    RunAttention {
        run: r.run.id.clone(),
        feature: r.feature.clone(),
        project: r.project.clone(),
        phase: r.run.status,
        phase_since_ms: r.phase_since_ms,
        level,
        members: members.iter().map(|m| m.agent.clone()).collect(),
        blocker,
        counts,
    }
}

/// The attention world now.
pub fn compute(i: &Inputs, now_ms: u64) -> World {
    let cx = Ctx::new(i, now_ms);
    let agents: BTreeMap<String, AgentAttention> = i.agents.values().map(|a| (a.id.clone(), cx.record(a))).collect();
    let mut counts = AllCounts::default();
    for a in agents.values() {
        counts.global.add(a);
        if let Some(p) = &a.project {
            counts.projects.entry(p.clone()).or_default().add(a);
        }
    }
    // Open decisions of levels 1–3: stored items, then the synthesized ones
    // (trust dialog, a prompt only in the pane, an agent in error, the ask).
    // TODO(T-22): counted by the Decision builder once B2 lands; this mirrors orch-app inbox.rs.
    let mut by_agent: BTreeMap<&str, u32> = BTreeMap::new();
    for it in i.inbox.iter().filter(|it| it.open() && item_level(it) <= 3) {
        counts.global.decisions += 1;
        match it.agent_id.as_deref() {
            Some(a) if agents.contains_key(a) => *by_agent.entry(a).or_default() += 1,
            _ => {
                if let Some(p) = &it.project_id {
                    counts.projects.entry(p.clone()).or_default().decisions += 1;
                }
            }
        }
    }
    for a in i.agents.values() {
        let held = cx.items(&a.id).iter().any(|it| it.kind == "permission");
        let synthesized = a.trust_dialog
            || (a.tracker.state == AgentState::WaitingForYou && !held)
            || a.tracker.state == AgentState::Error;
        if synthesized {
            counts.global.decisions += 1;
            *by_agent.entry(a.id.as_str()).or_default() += 1;
        }
    }
    if let Some(ask) = i.ask {
        counts.global.decisions += 1;
        if let Some(a) = ask.agent.as_deref().filter(|a| agents.contains_key(*a)) {
            *by_agent.entry(a).or_default() += 1;
        }
    }
    for (id, n) in &by_agent {
        if let Some(p) = agents.get(*id).and_then(|a| a.project.clone()) {
            counts.projects.entry(p).or_default().decisions += n;
        }
    }
    counts.global.live = cx.live_count;
    counts.global.live_cap = Some(i.live_cap);
    counts.global.done_today = i.done_today;
    let runs = i
        .runs
        .iter()
        .map(|r| {
            let members: Vec<&AgentAttention> =
                agents.values().filter(|a| a.run.as_deref() == Some(r.run.id.as_str())).collect();
            let mut rec = run_record(r, &members);
            rec.counts.decisions = members.iter().map(|m| by_agent.get(m.agent.as_str()).copied().unwrap_or(0)).sum();
            (r.run.id.clone(), rec)
        })
        .collect();
    World { agents, runs, counts }
}

#[cfg(test)]
mod tests;
