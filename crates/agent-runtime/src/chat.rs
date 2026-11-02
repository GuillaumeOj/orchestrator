//! The chat log (FR-1305, P4.3 spec §3.2, api-contract §1–3): one entry
//! per decision the core produced, posted when it is first listed open and
//! given its line and card snapshot when it closes; permissions a store rule
//! answered, posted settled from the broker's audit write; the morning line
//! once per working day. Kept in the store (`chat_entries`), synced on every
//! `now` computation and pushed with it.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use jiff::tz::TimeZone;
use orch_store::{ChatDay, ChatEntry, PermissionRule};
use serde_json::{Value, json};

use crate::attention::copy::cut;
use crate::decision::{self, Decision};
use crate::runtime::Runtime;

/// A line's `what`: the short input or title, cut by the core.
pub const WHAT_MAX: usize = 48;
/// `chat_days` and `chat_search` defaults (api-contract §3).
pub const DAYS_LIMIT: usize = 30;
pub const SEARCH_LIMIT: usize = 50;

/// What the `now` publisher keeps between computations.
#[derive(Default)]
pub(crate) struct ChatState {
    /// The working day of the last computation (`""` before the first).
    pub(crate) day: String,
    /// The open entries were read from the store (once per process).
    loaded: bool,
    /// Decision id → its entry still waiting for a line.
    open: BTreeMap<String, ChatEntry>,
    /// Decision id → its entry with a line, while the decision is still
    /// listed (its lifecycle goes on: sent, then effect), and its last
    /// snapshot while open.
    closed: BTreeMap<String, (ChatEntry, Option<Value>)>,
    /// Stored decisions known to have an entry (asked of the store once).
    known: BTreeSet<String>,
}

/// The card snapshot: the Decision as it is, payload omitted.
pub fn card_of(d: &Decision) -> Value {
    let mut v = json!(d);
    if let Some(m) = v.as_object_mut() {
        m.remove("payload");
    }
    v
}

/// `by` from `lifecycle.decided_by` (api-contract §1), and the rule's id.
fn by_of(decided_by: Option<&str>) -> (&'static str, Option<u64>) {
    match decided_by.unwrap_or("core") {
        "user" => ("you", None),
        "pane" => ("pane", None),
        "expired" => ("expired", None),
        "overwatch" => ("overwatch", None),
        r if r.starts_with("rule:") => ("rule", r["rule:".len()..].parse().ok()),
        _ => ("core", None),
    }
}

/// The line's verb: what was decided, by kind (api-contract §1).
pub fn verb(d: &Decision) -> &'static str {
    let lc = &d.lifecycle;
    let by = by_of(lc.decided_by.as_deref()).0;
    match lc.state {
        "failed" => return "failed",
        "stale" => return "dismissed",
        "interrupted" => return "interrupted",
        _ => {}
    }
    if lc.effect.as_deref() == Some("restarted") {
        return "restarted";
    }
    // Synthesized cards carry their answer as the effect.
    match (d.kind, lc.effect.as_deref()) {
        ("ask", Some("applied")) => return "run now",
        ("ask", Some("skipped")) => return "kept queued",
        ("wait", Some("delivered" | "resumed" | "queued")) => return "answered",
        ("error", Some("resumed" | "delivered" | "queued")) if by == "you" => return "restarted",
        _ => {}
    }
    let word = d.decision.as_deref().unwrap_or("");
    let gate = d.gate.unwrap_or("");
    match (d.kind, word) {
        ("permission", "approve" | "always_allow") => "allowed",
        ("permission", "reject" | "reply") => "denied",
        ("gate", "approve") => "approved",
        ("gate", "reply") => "sent back",
        ("gate", "reject") if gate == "spec" => "sent back",
        ("gate", "reject") => "rejected",
        ("budget", "approve") => "applied",
        ("budget", "reject") => "skipped",
        ("question", "reply") => "answered",
        (_, _) if by == "expired" => "expired",
        // Answered in Claude Code's own dialog (a prompt, a held permission).
        (_, _) if by == "pane" => "answered",
        _ => "dismissed",
    }
}

/// The closed decision's line (api-contract §1). `rule`: the rule that
/// decided, when it is still in the store (its pattern names it); `prior`:
/// the entry's snapshot while it was open.
///
/// A synthesized decision that cleared by itself is `dismissed · core ·
/// stale` (ux F8); `expired` is for a default taken at its deadline (the
/// Governor's ask) and a parked permission the core closed (ux F10).
pub fn line_of(d: &Decision, rule: Option<&PermissionRule>, prior: Option<&Value>, now: u64) -> Value {
    let lc = &d.lifecycle;
    let (mut by, rule_id) = by_of(lc.decided_by.as_deref());
    let (mut verb, mut state) = (verb(d), lc.state);
    let was_parked = prior.and_then(|c| c["lifecycle"]["state"].as_str()) == Some("parked");
    if d.synthesized && !matches!(by, "you" | "overwatch") {
        let deadline = d.payload.get("deadline_ms").and_then(Value::as_u64);
        match deadline {
            Some(t) if d.kind == "ask" && lc.decided_at_ms.unwrap_or(now) >= t => {
                let apply = d.payload.get("default").and_then(Value::as_str) == Some("approve");
                (verb, by, state) = (if apply { "run now" } else { "kept queued" }, "expired", "effect");
            }
            _ => (verb, by, state) = ("dismissed", "core", "stale"),
        }
    } else if d.kind == "permission" && by == "core" && lc.state == "dismissed" && was_parked {
        (verb, by) = ("expired", "expired");
    }
    let what = match d.kind {
        "permission" => {
            let input = d.payload.get("tool_input").or_else(|| d.payload.get("input")).cloned().unwrap_or_default();
            let tool = d.payload.get("tool").and_then(Value::as_str).unwrap_or("");
            crate::inbox::main_arg(tool, &input).unwrap_or_else(|| d.title.clone())
        }
        _ => d.title.clone(),
    };
    let mut line = json!({
        "verb": verb,
        "what": cut(&what, WHAT_MAX),
        "by": by,
        "state": state,
        "at_ms": lc.decided_at_ms.or(lc.effect_at_ms).or(lc.sent_at_ms).unwrap_or(now),
    });
    if let Some(a) = d.agent.as_deref().or_else(|| d.subject.get("agent").and_then(Value::as_str)) {
        line["agent"] = json!(a);
    }
    if by == "rule" {
        let detail = rule.map(|r| r.pattern.clone()).or(rule_id.map(|n| n.to_string()));
        if let Some(t) = detail {
            line["by_detail"] = json!(t);
        }
    }
    line
}

/// "Good morning. 1 run in progress, 1 decision waiting, week 38 % against
/// 35 % planned." Parts at zero are left out (P4.2 notes §8.6), the week
/// with no plan too; all zero says so (ux F5). `week`: (used, planned) %.
pub fn morning_text(runs: u32, decisions: u32, week: Option<(f64, f64)>) -> String {
    let n = |k: u32, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
    let mut parts = vec![];
    if runs > 0 {
        parts.push(n(runs, "run in progress", "runs in progress"));
    }
    if decisions > 0 {
        parts.push(n(decisions, "decision waiting", "decisions waiting"));
    }
    if let Some((used, plan)) = week.filter(|(u, p)| u.round() > 0.0 && p.round() > 0.0) {
        parts.push(format!("week {used:.0} % against {plan:.0} % planned"));
    }
    if parts.is_empty() {
        "Good morning. Nothing is running and nothing is waiting for you.".into()
    } else {
        format!("Good morning. {}.", parts.join(", "))
    }
}

/// `allowed back-1: cargo add serde_json · rule: installs` (the UI's line
/// copy is its own; this is `orchd chat`'s).
pub fn line_text(line: &Value) -> String {
    let s = |k: &str| line.get(k).and_then(Value::as_str).unwrap_or("");
    let who = if s("agent").is_empty() { String::new() } else { format!(" {}", s("agent")) };
    let by = if s("by_detail").is_empty() { s("by").to_string() } else { format!("{}: {}", s("by"), s("by_detail")) };
    format!("{}{who}: {} · {by}", s("verb"), s("what"))
}

/// `orchd chat`: one line per entry, `14:02 core decision inbox-42 allowed …`.
pub fn render(entries: &[Value], tz: &TimeZone) -> String {
    let mut out = String::new();
    for e in entries {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
        let at = e["at_ms"].as_i64().and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()).unwrap_or_default();
        let hm = at.to_zoned(tz.clone()).strftime("%H:%M").to_string();
        let what = match (e.get("line"), s("kind")) {
            (Some(l), _) => format!("{} {}", s("decision"), line_text(l)),
            (None, "decision") => format!("{} open", s("decision")),
            (None, _) => s("text").to_string(),
        };
        out.push_str(&format!("{hm} {} {} {what}\n", s("author"), s("kind")));
    }
    out
}

/// A `YYYY-MM-DD` day, else `bad_request`.
fn parse_day(day: &str) -> Result<jiff::civil::Date> {
    match day.parse::<jiff::civil::Date>() {
        Ok(d) if d.to_string() == day => Ok(d),
        _ => bail!("bad_request: day must be YYYY-MM-DD (got {day:?})"),
    }
}

/// Local midnight of `date` and of the day after.
fn day_bounds(date: jiff::civil::Date, tz: &TimeZone) -> Result<(u64, u64)> {
    let ms = |d: jiff::civil::Date| -> Result<u64> {
        Ok(d.to_zoned(tz.clone())?.start_of_day()?.timestamp().as_millisecond().max(0) as u64)
    };
    Ok((ms(date)?, ms(date.tomorrow()?)?))
}

/// A core decision entry; `line` set when it is posted settled.
fn decision_entry(day: &str, at_ms: u64, d: &Decision, line: Option<Value>) -> ChatEntry {
    ChatEntry {
        day: day.into(),
        at_ms,
        author: "core".into(),
        kind: "decision".into(),
        thread: d.thread.clone(),
        decision: Some(d.id.clone()),
        line,
        card: Some(card_of(d)),
        ..Default::default()
    }
}

/// An open entry whose decision is gone with no row (a synthesized card that
/// cleared while the app was down): its snapshot, closed by the core (ux F8).
fn close_from_snapshot(e: &mut ChatEntry, now: u64) {
    let card = e.card.as_ref();
    let what = card.and_then(|c| c["title"].as_str()).or(e.decision.as_deref()).unwrap_or("");
    let mut line =
        json!({"verb": "dismissed", "what": cut(what, WHAT_MAX), "by": "core", "state": "stale", "at_ms": now});
    if let Some(a) = card.and_then(|c| c["subject"]["agent"].as_str()) {
        line["agent"] = json!(a);
    }
    e.line = Some(line);
    if let Some(c) = e.card.as_mut() {
        let version = c["lifecycle"]["version"].as_u64().unwrap_or(0) + 1;
        c["lifecycle"] = json!({"state": "dismissed", "decided_by": "core", "decided_at_ms": now,
                                "effect": "dismissed", "effect_at_ms": now, "version": version});
        c["waits_on"] = json!("core");
        c["actions"] = json!([]);
    }
}

fn upsert(out: &mut Vec<ChatEntry>, e: ChatEntry) {
    match out.iter_mut().find(|o| o.n == e.n) {
        Some(o) => *o = e,
        None => out.push(e),
    }
}

/// What a sync reads besides the decisions.
pub(crate) struct SyncIn<'a> {
    pub now: u64,
    pub day: &'a str,
    /// Decisions as now listed (open and closed recently), by id.
    pub decisions: &'a BTreeMap<String, Decision>,
    /// Last upserts of decisions that vanished (synthesized ones): their final lifecycle.
    pub gone: &'a [Decision],
    /// The morning line's text, asked only when it is posted.
    pub morning: &'a dyn Fn() -> String,
}

impl Runtime {
    /// Brings the log in step with the decisions just computed; the entries
    /// written (new or changed), for the `now` event.
    pub(crate) fn chat_sync(&self, cs: &mut ChatState, x: SyncIn) -> Vec<ChatEntry> {
        let mut out = vec![];
        let write = |e: &ChatEntry, out: &mut Vec<ChatEntry>| -> Option<ChatEntry> {
            let e = if e.n == 0 { self.db.chat_insert(e) } else { self.db.chat_update(e) };
            let e = e.ok()?;
            upsert(out, e.clone());
            Some(e)
        };
        if !cs.loaded {
            for e in self.db.chat_open().unwrap_or_default() {
                if let Some(d) = e.decision.clone() {
                    cs.open.insert(d, e);
                }
            }
            cs.loaded = true;
        }
        let is_open = |id: &str| x.decisions.get(id).is_some_and(Decision::is_open);
        // The rule that decided (its pattern names it), read once and only when asked.
        let rules = std::cell::OnceCell::new();
        let rule_of = |d: &Decision| -> Option<PermissionRule> {
            let id = by_of(d.lifecycle.decided_by.as_deref()).1?;
            rules.get_or_init(|| self.db.rules().unwrap_or_default()).iter().find(|r| r.id == id).cloned()
        };
        // A new working day (start, or local midnight while running).
        if cs.day != x.day {
            cs.day = x.day.to_string();
            cs.known.clear();
            if !self.db.chat_has_kind(x.day, "morning").unwrap_or(true) {
                let e = ChatEntry {
                    day: x.day.into(),
                    at_ms: x.now,
                    author: "core".into(),
                    kind: "morning".into(),
                    text: Some((x.morning)()),
                    ..Default::default()
                };
                write(&e, &mut out);
            }
            // A card still open from an earlier day carries on in a new entry
            // of today's thread, after the morning line (ux F1); the old one
            // keeps no line ("left open"). Same `at_ms` as the morning line:
            // the id orders them after it, and before the cards first listed
            // in this computation.
            for (id, e) in cs.open.iter_mut() {
                if e.day == x.day || !is_open(id) {
                    continue;
                }
                let next = ChatEntry {
                    day: x.day.into(),
                    at_ms: x.now,
                    author: e.author.clone(),
                    kind: e.kind.clone(),
                    thread: e.thread.clone(),
                    decision: e.decision.clone(),
                    card: Some(card_of(&x.decisions[id])),
                    continued_from: Some(e.n),
                    ..Default::default()
                };
                let Some(next) = write(&next, &mut out) else { continue };
                let mut old = e.clone();
                old.continued_to = Some(next.n);
                write(&old, &mut out);
                *e = next;
            }
        }
        // Open ones: the snapshot follows what the card shows (parked, its
        // title), not pushed (the card renders from `now.decisions`).
        for (id, e) in cs.open.iter_mut() {
            let Some(d) = x.decisions.get(id).filter(|d| d.is_open()) else { continue };
            // Compared on the decision itself: the card is built only when it moved.
            let same = e.card.as_ref().is_some_and(|c| {
                c["lifecycle"]["state"].as_str() == Some(d.lifecycle.state)
                    && c["title"].as_str() == Some(d.title.as_str())
                    && c["summary"].as_str() == Some(d.summary.as_str())
            });
            if !same {
                let mut next = e.clone();
                next.card = Some(card_of(d));
                if let Ok(w) = self.db.chat_update(&next) {
                    *e = w;
                }
            }
        }
        // Closed (or gone): the line and card, one write.
        let closing: Vec<String> = cs.open.keys().filter(|id| !is_open(id)).cloned().collect();
        let mut rebuilt = self.closed_elsewhere(&closing, x.decisions, x.gone);
        for id in closing {
            let Some(mut e) = cs.open.remove(&id) else { continue };
            let (before, prior) = (e.clone(), e.card.clone());
            let last = x
                .decisions
                .get(&id)
                .or_else(|| x.gone.iter().find(|d| d.id == id))
                .cloned()
                .or_else(|| rebuilt.remove(&id));
            match last {
                Some(d) => {
                    e.line = Some(line_of(&d, rule_of(&d).as_ref(), prior.as_ref(), x.now));
                    e.card = Some(card_of(&d));
                }
                None => close_from_snapshot(&mut e, x.now),
            }
            match write(&e, &mut out) {
                Some(w) if x.decisions.contains_key(&id) => {
                    cs.closed.insert(id, (w, prior));
                }
                Some(_) => {}
                // The store refused: still waiting for its line, tried again next time.
                None => {
                    cs.open.insert(id, before);
                }
            }
        }
        // Closed ones still listed: their lifecycle goes on (or an undo opened them again).
        let ids: Vec<String> = cs.closed.keys().cloned().collect();
        for id in ids {
            let Some(d) = x.decisions.get(&id) else {
                cs.closed.remove(&id);
                continue;
            };
            if d.is_open() {
                let Some((mut e, _)) = cs.closed.remove(&id) else { continue };
                (e.line, e.card) = (None, Some(card_of(d)));
                if let Some(w) = write(&e, &mut out) {
                    cs.open.insert(id, w);
                }
                continue;
            }
            // Compared on what moves after a close (lifecycle, actions, title):
            // the card is built only when one did.
            let (e, prior) = &cs.closed[&id];
            let same = e.card.as_ref().is_some_and(|c| {
                c["lifecycle"] == json!(d.lifecycle)
                    && c["actions"] == json!(d.actions)
                    && c["title"] == d.title.as_str()
            });
            if !same {
                let mut e = e.clone();
                (e.line, e.card) = (Some(line_of(d, rule_of(d).as_ref(), prior.as_ref(), x.now)), Some(card_of(d)));
                if let Some(w) = write(&e, &mut out) {
                    cs.closed.get_mut(&id).expect("listed").0 = w;
                }
            }
        }
        // First listed open: a new entry (a synthesized id back later is a new one).
        let mut fresh: Vec<&Decision> =
            x.decisions.values().filter(|d| d.is_open() && !cs.open.contains_key(&d.id)).collect();
        fresh.sort_by_key(|d| (d.created_ms, d.id.clone()));
        for d in fresh {
            if let Some(w) = write(&decision_entry(x.day, x.now, d, None), &mut out) {
                cs.known.insert(d.id.clone());
                cs.open.insert(d.id.clone(), w);
            }
        }
        // Stored decisions closed before they were ever listed (answered in
        // the pane at once, or while the app was down): posted settled.
        let unseen: Vec<String> = x
            .decisions
            .values()
            .filter(|d| !d.synthesized && !d.is_open())
            .filter(|d| !cs.open.contains_key(&d.id) && !cs.closed.contains_key(&d.id) && !cs.known.contains(&d.id))
            .map(|d| d.id.clone())
            .collect();
        if !unseen.is_empty() {
            let have: BTreeSet<String> = self.db.chat_has_decisions(&unseen).unwrap_or_default().into_iter().collect();
            for id in unseen {
                cs.known.insert(id.clone());
                if have.contains(&id) {
                    continue;
                }
                let d = &x.decisions[&id];
                let line = line_of(d, rule_of(d).as_ref(), None, x.now);
                if let Some(w) = write(&decision_entry(x.day, x.now, d, Some(line)), &mut out) {
                    cs.closed.insert(id, (w, None));
                }
            }
        }
        // Posted elsewhere since (the broker's rule answers).
        for e in std::mem::take(&mut *self.now.chat_posted()) {
            upsert(&mut out, e);
        }
        out
    }

    /// Stored decisions among `ids` that left the list (closed while the
    /// app was down, or longer ago than `RECENT_MS`), rebuilt from their rows.
    fn closed_elsewhere(
        &self,
        ids: &[String],
        listed: &BTreeMap<String, Decision>,
        gone: &[Decision],
    ) -> BTreeMap<String, Decision> {
        let want: BTreeSet<&String> =
            ids.iter().filter(|id| !listed.contains_key(*id) && !gone.iter().any(|d| &d.id == *id)).collect();
        if want.is_empty() {
            return BTreeMap::new();
        }
        let items: Vec<orch_store::InboxItem> = self.store.read(|s| {
            s.inbox.iter().filter(|i| want.contains(&decision::decision_id(&i.kind, i.id))).cloned().collect()
        });
        if items.is_empty() {
            return BTreeMap::new();
        }
        self.build_bare(&items).into_iter().map(|d| (d.id.clone(), d)).collect()
    }

    /// Stored items as Decisions, without the live state (agents, runs,
    /// tails): what their rows say.
    fn build_bare(&self, items: &[orch_store::InboxItem]) -> Vec<Decision> {
        let (agents, tails, projects) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        decision::build(&decision::Ctx {
            items,
            agents: &agents,
            tails: &tails,
            ask: None,
            runs: &[],
            projects: &projects,
            startup_timeout_ms: self.cfg.startup_timeout.as_millis() as u64,
        })
    }

    /// A permission a store rule answered (FR-709's audit write, `audit` its
    /// id): posted settled, as a line, never as an open card (J5 · 2).
    pub(crate) fn chat_rule_answered(
        &self,
        rec: &crate::store::AgentRecord,
        call: &crate::inbox::Call,
        rule: &PermissionRule,
        allow: bool,
        audit: u64,
    ) {
        let (now, tz) = self.now.clock();
        let (_, day) = crate::digest::today(now, &tz);
        let subject = self.subject_now(Some(&rec.id), None);
        // The permission card as the Inbox builds it, closed by the rule.
        let item = serde_json::from_value::<orch_store::InboxItem>(json!({
            "id": audit, "kind": "permission", "agent_id": rec.id, "body": "", "created_at_ms": now,
            "payload": {"tool": call.tool, "tool_input": call.input, "cwd": call.cwd,
                        "description": call.input.get("description")},
            "status": "effect", "decided_at_ms": now, "decided_by": format!("rule:{}", rule.id),
            "decision": if allow { "approve" } else { "reject" }, "effect": if allow { "allowed" } else { "denied" },
            "effect_at_ms": now, "subject": subject, "thread": decision::thread_of(&subject), "version": 1,
        }));
        let Some(mut d) = item.ok().and_then(|i| self.build_bare(&[i]).pop()) else { return };
        (d.id, d.actions, d.waits_on) = (format!("audit-{audit}"), vec![], "core");
        let e = decision_entry(&day, now, &d, Some(line_of(&d, Some(rule), None, now)));
        if let Ok(e) = self.db.chat_insert(&e) {
            self.now.chat_posted().push(e);
        }
    }

    /// `chat_days {limit?, before?}`: days with entries before `before`
    /// (today at most, which is excluded), newest first.
    pub fn chat_days(&self, limit: Option<usize>, before: Option<&str>) -> Result<Vec<ChatDay>> {
        let (now, tz) = self.now.clock();
        let (_, today) = crate::digest::today(now, &tz);
        let before = match before {
            Some(b) => {
                parse_day(b)?;
                b.min(today.as_str()).to_string()
            }
            None => today,
        };
        self.db.chat_days(&before, limit.unwrap_or(DAYS_LIMIT))
    }

    /// `chat_day {day}`: its entries, oldest first, and that day's outcomes
    /// (rebuilt from events, `seen` omitted).
    pub fn chat_day(&self, day: &str) -> Result<Value> {
        let date = parse_day(day)?;
        let (_, tz) = self.now.clock();
        let (from, until) = day_bounds(date, &tz)?;
        let entries = self.db.chat_day(day)?;
        let outcomes: Vec<Value> = self
            .outcomes_between(from, until, &tz)
            .into_iter()
            .map(|o| {
                let mut v = json!(o);
                if let Some(m) = v.as_object_mut() {
                    m.remove("seen");
                }
                v
            })
            .collect();
        Ok(json!({"day": day, "entries": entries, "outcomes": outcomes}))
    }

    /// `chat_search {query, limit?}`: `[{day, entry}]`, newest first.
    pub fn chat_search(&self, query: &str, limit: Option<usize>) -> Result<Value> {
        if query.trim().is_empty() {
            bail!("bad_request: an empty query");
        }
        let hits = self.db.chat_search(query.trim(), limit.unwrap_or(SEARCH_LIMIT))?;
        Ok(Value::Array(hits.into_iter().map(|e| json!({"day": e.day, "entry": e})).collect()))
    }
}

#[cfg(test)]
mod tests;
