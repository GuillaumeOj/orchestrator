//! P4.3: the chat log (FR-1305, api-contract §1): one entry per decision
//! the core produced (its line and card snapshot once it closed), the
//! morning line, and the Overwatch's entries from P4.4. One thread per local
//! working day; nothing is purged.

use anyhow::Result;
use rusqlite::{Row, params};
use serde::Serialize;
use serde::ser::SerializeMap;
use serde_json::Value;

use crate::db::Store;
use crate::util::{from_js, opt_u64};

/// One chat entry (api-contract §1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatEntry {
    /// The row id: `c-<n>` on the wire.
    pub n: u64,
    /// `YYYY-MM-DD`, the local working day.
    pub day: String,
    pub at_ms: u64,
    /// core | overwatch | user
    pub author: String,
    /// decision | morning | text | annotation | rule_proposal
    pub kind: String,
    pub thread: Option<String>,
    pub decision: Option<String>,
    pub text: Option<String>,
    /// Once the decision closed.
    pub line: Option<Value>,
    /// The decision as it was: at close (sent with `line`), or as first
    /// listed while open (kept for a close seen only after a restart, not sent).
    pub card: Option<Value>,
    /// Open across midnight: the previous day's entry of the same decision.
    pub continued_from: Option<u64>,
    /// On that previous day's entry: the one that carries on.
    pub continued_to: Option<u64>,
    pub rev: u64,
}

impl ChatEntry {
    pub fn id(&self) -> String {
        format!("c-{}", self.n)
    }

    /// What `chat_search` matches, lower-cased.
    fn haystack(&self) -> String {
        let s = |v: Option<&Value>, k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
        let (line, card) = (self.line.as_ref(), self.card.as_ref());
        [
            self.text.clone().unwrap_or_default(),
            s(line, "what"),
            s(line, "agent"),
            s(line, "verb"),
            s(line, "by_detail"),
            s(card, "title"),
            s(card, "summary"),
        ]
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n")
    }
}

impl Serialize for ChatEntry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("id", &self.id())?;
        m.serialize_entry("day", &self.day)?;
        m.serialize_entry("at_ms", &self.at_ms)?;
        m.serialize_entry("author", &self.author)?;
        m.serialize_entry("kind", &self.kind)?;
        for (k, v) in [("thread", &self.thread), ("decision", &self.decision), ("text", &self.text)] {
            if let Some(v) = v {
                m.serialize_entry(k, v)?;
            }
        }
        // The card goes with the line: an open entry renders from `now.decisions`.
        // A "left open" entry (carried on to a later day) keeps its snapshot:
        // its decision may have closed and left `now` since.
        if let Some(line) = &self.line {
            m.serialize_entry("line", line)?;
        }
        if (self.line.is_some() || self.continued_to.is_some())
            && let Some(card) = &self.card
        {
            m.serialize_entry("card", card)?;
        }
        for (k, v) in [("continued_from", self.continued_from), ("continued_to", self.continued_to)] {
            if let Some(n) = v {
                m.serialize_entry(k, &format!("c-{n}"))?;
            }
        }
        m.serialize_entry("rev", &self.rev)?;
        m.end()
    }
}

/// One day of the log in `chat_days`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChatDay {
    pub day: String,
    pub entries: u64,
    pub decisions: u64,
}

const COLS: &str =
    "id, day, at_ms, author, kind, thread, decision, text, line, card, rev, continued_from, continued_to";

fn row(r: &Row) -> rusqlite::Result<ChatEntry> {
    let json = |i: usize| -> rusqlite::Result<Option<Value>> {
        r.get::<_, Option<String>>(i)?.map(|s| from_js(&s)).transpose()
    };
    Ok(ChatEntry {
        n: r.get::<_, i64>(0)? as u64,
        day: r.get(1)?,
        at_ms: r.get::<_, i64>(2)? as u64,
        author: r.get(3)?,
        kind: r.get(4)?,
        thread: r.get(5)?,
        decision: r.get(6)?,
        text: r.get(7)?,
        line: json(8)?,
        card: json(9)?,
        rev: r.get::<_, i64>(10)? as u64,
        continued_from: opt_u64(r, 11)?,
        continued_to: opt_u64(r, 12)?,
    })
}

fn js_opt(v: &Option<Value>) -> Option<String> {
    v.as_ref().map(crate::util::js)
}

impl Store {
    /// Posts a new entry: its id and `rev` 1 are set on the returned copy.
    pub fn chat_insert(&self, e: &ChatEntry) -> Result<ChatEntry> {
        let mut e = e.clone();
        e.rev = 1;
        let row = e.clone();
        e.n = self.write(move |tx| {
            tx.execute(
                "INSERT INTO chat_entries(day, at_ms, author, kind, thread, decision, text, line, card, rev, haystack,
                   continued_from, continued_to)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?12)",
                params![
                    row.day,
                    row.at_ms as i64,
                    row.author,
                    row.kind,
                    row.thread,
                    row.decision,
                    row.text,
                    js_opt(&row.line),
                    js_opt(&row.card),
                    row.haystack(),
                    row.continued_from.map(|n| n as i64),
                    row.continued_to.map(|n| n as i64)
                ],
            )?;
            Ok(tx.last_insert_rowid() as u64)
        })?;
        Ok(e)
    }

    /// Writes an entry back with `rev` + 1 (set on the returned copy).
    pub fn chat_update(&self, e: &ChatEntry) -> Result<ChatEntry> {
        let mut e = e.clone();
        e.rev += 1;
        let row = e.clone();
        self.write(move |tx| {
            tx.execute(
                "UPDATE chat_entries SET day=?2, at_ms=?3, thread=?4, decision=?5, text=?6, line=?7, card=?8,
                   rev=?9, haystack=?10, continued_from=?11, continued_to=?12 WHERE id=?1",
                params![
                    row.n as i64,
                    row.day,
                    row.at_ms as i64,
                    row.thread,
                    row.decision,
                    row.text,
                    js_opt(&row.line),
                    js_opt(&row.card),
                    row.rev as i64,
                    row.haystack(),
                    row.continued_from.map(|n| n as i64),
                    row.continued_to.map(|n| n as i64)
                ],
            )?;
            Ok(())
        })?;
        Ok(e)
    }

    /// A day's entries, oldest first.
    pub fn chat_day(&self, day: &str) -> Result<Vec<ChatEntry>> {
        let day = day.to_string();
        self.read(move |c| {
            let mut st =
                c.prepare_cached(&format!("SELECT {COLS} FROM chat_entries WHERE day=?1 ORDER BY at_ms, id"))?;
            Ok(st.query_map([day], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Days with at least one entry before `before` (exclusive), newest first.
    pub fn chat_days(&self, before: &str, limit: usize) -> Result<Vec<ChatDay>> {
        let before = before.to_string();
        self.read(move |c| {
            let mut st = c.prepare_cached(
                "SELECT day, count(*), sum(kind = 'decision') FROM chat_entries WHERE day < ?1
                 GROUP BY day ORDER BY day DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![before, limit as i64], |r| {
                Ok(ChatDay {
                    day: r.get(0)?,
                    entries: r.get::<_, i64>(1)? as u64,
                    decisions: r.get::<_, i64>(2)? as u64,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Entries whose text, line (what, agent) or card (title, summary)
    /// holds `query`, case-insensitive; newest first.
    pub fn chat_search(&self, query: &str, limit: usize) -> Result<Vec<ChatEntry>> {
        let q = query.to_lowercase();
        self.read(move |c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {COLS} FROM chat_entries WHERE instr(haystack, ?1) > 0 ORDER BY at_ms DESC, id DESC LIMIT ?2"
            ))?;
            Ok(st.query_map(params![q, limit as i64], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Decision entries still waiting for their line (and not carried on),
    /// any day, oldest first.
    pub fn chat_open(&self) -> Result<Vec<ChatEntry>> {
        self.read(|c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {COLS} FROM chat_entries WHERE kind = 'decision' AND line IS NULL AND continued_to IS NULL
                 ORDER BY at_ms, id"
            ))?;
            Ok(st.query_map([], row)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The decision ids among `ids` that have an entry already.
    pub fn chat_has_decisions(&self, ids: &[String]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = ids.to_vec();
        self.read(move |c| {
            let marks = vec!["?"; ids.len()].join(",");
            let mut st =
                c.prepare(&format!("SELECT DISTINCT decision FROM chat_entries WHERE decision IN ({marks})"))?;
            Ok(st.query_map(rusqlite::params_from_iter(ids), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Is there an entry of `kind` on `day`?
    pub fn chat_has_kind(&self, day: &str, kind: &str) -> Result<bool> {
        let (day, kind) = (day.to_string(), kind.to_string());
        self.read(move |c| {
            Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM chat_entries WHERE day=?1 AND kind=?2)", [day, kind], |r| {
                r.get(0)
            })?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        (d, s)
    }

    fn entry(day: &str, at: u64, kind: &str) -> ChatEntry {
        ChatEntry { day: day.into(), at_ms: at, author: "core".into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn entries_days_and_search() {
        let (_d, s) = store();
        let mut m = entry("2026-11-01", 5, "morning");
        m.text = Some("Good morning.".into());
        let m = s.chat_insert(&m).unwrap();
        assert_eq!((m.id(), m.rev), ("c-1".to_string(), 1));
        let mut d = entry("2026-11-01", 6, "decision");
        d.decision = Some("inbox-3".into());
        d.thread = Some("run-1".into());
        d.card = Some(json!({"title": "Bash · cargo add Serde_json", "summary": "open"}));
        let mut d = s.chat_insert(&d).unwrap();
        // Open: the snapshot is kept and searched (its title), not sent.
        let v = json!(d);
        assert!(v.get("card").is_none() && v.get("line").is_none() && v.get("text").is_none(), "{v}");
        assert_eq!(s.chat_search("serde", 10).unwrap().len(), 1);
        assert_eq!(s.chat_open().unwrap().len(), 1);
        assert_eq!(s.chat_has_decisions(&["inbox-3".into(), "inbox-9".into()]).unwrap(), ["inbox-3"]);
        d.line = Some(json!({"verb": "allowed", "agent": "back-1", "what": "cargo add serde_json", "by": "rule",
                             "by_detail": "Bash(cargo add:*)"}));
        let d = s.chat_update(&d).unwrap();
        assert_eq!(d.rev, 2);
        assert!(json!(d)["card"].is_object());
        assert!(s.chat_open().unwrap().is_empty());
        let mut today = entry("2026-11-02", 100, "morning");
        today.text = Some("Good morning. 1 run in progress.".into());
        s.chat_insert(&today).unwrap();
        // Case-insensitive, over the line and card; newest first, every day.
        let hits = s.chat_search("SERDE", 10).unwrap();
        assert_eq!(hits.iter().map(ChatEntry::id).collect::<Vec<_>>(), ["c-2"]);
        let hits = s.chat_search("good MORNING", 10).unwrap();
        assert_eq!(hits.iter().map(|e| e.day.as_str()).collect::<Vec<_>>(), ["2026-11-02", "2026-11-01"]);
        assert_eq!(s.chat_search("back-1", 1).unwrap().len(), 1);
        assert_eq!(s.chat_search("ALLOWED", 9).unwrap().len(), 1, "the verb");
        assert_eq!(s.chat_search("cargo add:*", 9).unwrap().len(), 1, "the rule");
        assert_eq!(s.chat_day("2026-11-01").unwrap(), [m, d]);
        assert_eq!(
            s.chat_days("2026-11-02", 30).unwrap(),
            [ChatDay { day: "2026-11-01".into(), entries: 2, decisions: 1 }]
        );
        // Carried on to the next day: the old entry leaves the open list, both say so.
        let mut old = entry("2026-11-02", 101, "decision");
        old.decision = Some("q-1".into());
        let mut old = s.chat_insert(&old).unwrap();
        let mut new = entry("2026-11-03", 200, "decision");
        (new.decision, new.continued_from) = (Some("q-1".into()), Some(old.n));
        let new = s.chat_insert(&new).unwrap();
        old.continued_to = Some(new.n);
        let old = s.chat_update(&old).unwrap();
        assert_eq!(s.chat_open().unwrap(), std::slice::from_ref(&new));
        assert_eq!(
            (json!(old)["continued_to"].clone(), json!(new)["continued_from"].clone()),
            (json!(new.id()), json!(old.id()))
        );
        assert_eq!(s.chat_day("2026-11-02").unwrap().last(), Some(&old), "returned with no line");
        assert!(s.chat_has_kind("2026-11-02", "morning").unwrap());
        assert!(!s.chat_has_kind("2026-11-03", "morning").unwrap());
    }

    /// 90 days of 300 entries: a day and a search load in < 200 ms (spec §5).
    #[test]
    fn a_day_and_a_search_are_fast_over_90_days() {
        let (_d, s) = store();
        let rows: Vec<ChatEntry> = (0..90u64)
            .flat_map(|d| {
                let day = format!("2031-{:02}-{:02}", 1 + d / 28, 1 + d % 28);
                (0..300u64).map(move |i| {
                    let mut e = entry(&day, d * 86_400_000 + i * 1000, "decision");
                    e.decision = Some(format!("inbox-{}", d * 300 + i));
                    e.thread = Some(format!("run-{}", i % 7));
                    e.line = Some(json!({"verb": "allowed", "agent": format!("back-{}", i % 5),
                                         "what": format!("cargo test -p crate{i}"), "by": "you", "state": "effect"}));
                    e.card = Some(json!({"title": format!("Bash · cargo test -p crate{i}"), "summary": "Run the tests",
                                         "context": [{"type": "tool_call", "tool": "Bash", "input": {"command": "x".repeat(200)}}]}));
                    e
                })
            })
            .collect();
        s.write(move |tx| {
            for e in rows {
                tx.execute(
                    "INSERT INTO chat_entries(day, at_ms, author, kind, thread, decision, line, card, haystack)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        e.day,
                        e.at_ms as i64,
                        e.author,
                        e.kind,
                        e.thread,
                        e.decision,
                        js_opt(&e.line),
                        js_opt(&e.card),
                        e.haystack()
                    ],
                )?;
            }
            Ok(())
        })
        .unwrap();
        let t = std::time::Instant::now();
        assert_eq!(s.chat_day("2031-02-10").unwrap().len(), 300);
        let day = t.elapsed();
        let t = std::time::Instant::now();
        assert_eq!(s.chat_search("crate299", 50).unwrap().len(), 50);
        assert!(s.chat_search("nothing like it", 50).unwrap().is_empty());
        let search = t.elapsed() / 2;
        let t = std::time::Instant::now();
        assert_eq!(s.chat_days("2031-12-31", 30).unwrap().len(), 30);
        let days = t.elapsed();
        for (what, took) in [("chat_day", day), ("chat_search", search), ("chat_days", days)] {
            assert!(took < std::time::Duration::from_millis(200), "{what} took {took:?}");
        }
    }
}
