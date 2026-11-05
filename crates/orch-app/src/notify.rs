//! Desktop notifications (SPEC §9.6): a new Inbox item that needs the user
//! (levels 1 and 2) raises a macOS notification with a sound, several at
//! once one batched notification ("3 agents need you"). Woken in-process by
//! the core's `now` push when needs-you or open decisions go up, or the
//! stored Inbox changed (P4.1), so it works while the window is hidden. A team question asked as `fyi` never
//! notifies. Settings key `notifications`: `{desktop, sound}` (both on by
//! default).

use std::collections::BTreeSet;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::core::Core;
use agent_runtime::decision::Decision;

pub const SETTINGS_KEY: &str = "notifications";

/// What to show for the new items of one poll, and the decision a click
/// (or the app being activated soon after) focuses: the oldest of the batch.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub decision_id: String,
}

/// How long after a notification the app being activated counts as the
/// click (the platform plugin reports none on desktop; spec §3.5, D7).
pub const CLICK_WINDOW: Duration = Duration::from_secs(60);

/// The last notification's decision, waiting for the app to be activated.
/// One-shot: `take` hands the id out once.
#[derive(Default)]
pub struct Pending {
    last: Mutex<Option<(String, Instant)>>,
}

impl Pending {
    pub fn arm(&self, id: String, now: Instant) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, now));
    }

    /// The id to focus if the activation at `now` is within the window of the
    /// last notification; clears it either way.
    pub fn take(&self, now: Instant) -> Option<String> {
        let (id, at) = self.last.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        (now.saturating_duration_since(at) <= CLICK_WINDOW).then_some(id)
    }
}

/// Emits `focus_card {id}` if the app's activation follows a notification.
pub fn on_activated(app: &AppHandle) {
    let Some(p) = app.try_state::<Arc<Pending>>() else { return };
    if let Some(id) = p.take(Instant::now()) {
        let _ = app.emit("focus_card", json!({ "id": id }));
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(120).collect()
}

/// Levels 1–2 that wait on the user: a permission the Overwatch is still
/// deciding notifies once, when it is handed over (P4.4 F1).
fn notifies(d: &Decision) -> bool {
    d.level <= 2 && d.waits_on == "user" && d.payload.get("notify").and_then(Value::as_bool) != Some(false)
}

/// Remembers the items already notified; the first poll only learns them
/// (no burst for what was there at start).
#[derive(Default)]
pub struct Watch {
    seen: BTreeSet<String>,
    started: bool,
}

impl Watch {
    pub fn poll(&mut self, items: &[Decision]) -> Option<Note> {
        let items: Vec<&Decision> = items.iter().filter(|i| notifies(i)).collect();
        let fresh: Vec<&Decision> = items.iter().copied().filter(|i| !self.seen.contains(&i.id)).collect();
        self.seen = items.iter().map(|i| i.id.clone()).collect();
        if !std::mem::replace(&mut self.started, true) {
            return None;
        }
        let oldest = fresh.iter().min_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)))?;
        let decision_id = oldest.id.clone();
        // Copy: design notes §8.3.
        let (title, body) = match fresh.as_slice() {
            [one] => (
                format!("{} needs you", one.agent.as_deref().unwrap_or("Orchestrator")),
                format!("{}\n{}", one.title, first_line(&one.summary)).trim_end().to_string(),
            ),
            many => {
                // A decision with no agent (budget, battery) counts as its own "Orchestrator".
                let agents: BTreeSet<&str> =
                    many.iter().map(|i| i.agent.as_deref().unwrap_or("Orchestrator")).collect();
                let title = match agents.iter().next() {
                    Some(a) if agents.len() == 1 => format!("{} decisions need you · {a}", many.len()),
                    _ => format!("{} decisions need you · {} agents", many.len(), agents.len()),
                };
                let mut lines: Vec<String> = many
                    .iter()
                    .take(3)
                    .map(|i| match (agents.len() > 1, i.agent.as_deref()) {
                        (true, Some(a)) => format!("{a}: {}", i.title),
                        _ => i.title.clone(),
                    })
                    .collect();
                if many.len() > 3 {
                    lines.push(format!("+{} more", many.len() - 3));
                }
                (title, lines.join("\n"))
            }
        };
        Some(Note { title, body, decision_id })
    }
}

/// `(desktop, sound)` from the settings.
fn prefs(core: &Core) -> (bool, bool) {
    let v = core.settings.get(SETTINGS_KEY).unwrap_or(Value::Null);
    let on = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(true);
    (on("desktop"), on("sound"))
}

/// Wakes the notifier when a `now` event's global needs-you or decisions
/// count goes up, or the stored Inbox changed (an item answered and another
/// raised in the same event leave the counts as they were); not on every
/// event.
#[derive(Default)]
pub struct Trigger {
    last: Mutex<(u64, u64)>,
}

impl Trigger {
    pub fn fires(&self, event: &Value) -> bool {
        let inbox = event["topics"].as_array().is_some_and(|t| t.iter().any(|t| t == "inbox"));
        let g = &event["counts"]["global"];
        let (Some(n), Some(d)) = (g["needs_you"].as_u64(), g["decisions"].as_u64()) else { return inbox };
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let up = n > last.0 || d > last.1;
        *last = (n, d);
        up || inbox
    }
}

/// Starts the notifier thread for the app's lifetime, woken by the core's
/// `now` push.
pub fn start(app: AppHandle, core: Arc<Core>) {
    let pending = Arc::new(Pending::default());
    app.manage(pending.clone());
    let Ok(rt) = core.runtime().cloned() else { return };
    let (tx, rx) = channel::<()>();
    let trigger = Trigger::default();
    rt.subscribe_now(Arc::new(move |v| {
        if trigger.fires(v) {
            let _ = tx.send(());
        }
    }));
    std::thread::spawn(move || {
        let mut watch = Watch::default();
        // What is there at start is known: no burst.
        watch.poll(&core.attention());
        while rx.recv().is_ok() {
            while rx.try_recv().is_ok() {} // coalesce a burst of wake-ups
            let (desktop, sound) = prefs(&core);
            if !desktop {
                // Off: what is there is learnt, not shown, so turning it back
                // on brings no burst of what came meanwhile.
                watch = Watch::default();
                watch.poll(&core.attention());
                continue;
            }
            let Some(note) = watch.poll(&core.attention()) else { continue };
            let mut n = app.notification().builder().title(&note.title).body(&note.body);
            if sound {
                n = n.sound("Glass");
            }
            match n.show() {
                // Already looking at the app: nothing to bring forward.
                Ok(()) if !app.webview_windows().values().any(|w| w.is_focused().unwrap_or(false)) => {
                    pending.arm(note.decision_id, Instant::now())
                }
                Ok(()) => {}
                Err(e) => eprintln!("notification failed: {e}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, level: u8, agent: &str) -> Decision {
        Decision {
            id: id.into(),
            kind: "question",
            gate: None,
            level,
            subject: json!({"agent": agent}),
            title: format!("{agent} asks"),
            summary: "which db?".into(),
            context: vec![],
            actions: vec![],
            lifecycle: Default::default(),
            thread: None,
            thread_prev: None,
            waits_on: "user",
            deciding_until_ms: None,
            created_ms: 0,
            agent: Some(agent.into()),
            synthesized: false,
            decision: None,
            payload: json!({}),
        }
    }

    #[test]
    fn new_items_notify_once_and_batch() {
        let mut w = Watch::default();
        assert_eq!(w.poll(&[item("q-1", 1, "a")]), None, "what is there at start is known");
        assert_eq!(w.poll(&[item("q-1", 1, "a")]), None);
        let n = w.poll(&[item("q-1", 1, "a"), item("q-2", 1, "b")]).unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str()), ("b needs you", "b asks\nwhich db?"));
        assert_eq!(n.decision_id, "q-2");
        let mut c = item("q-3", 2, "c");
        c.created_ms = 50;
        let mut d = item("perm-4", 1, "d");
        d.created_ms = 10;
        let n = w.poll(&[c, d, item("info-5", 4, "e")]).unwrap();
        assert_eq!(n.title, "2 decisions need you · 2 agents");
        assert_eq!(n.body, "c: c asks\nd: d asks");
        assert_eq!(n.decision_id, "perm-4", "the oldest of the batch");
        let mut fyi = item("q-6", 1, "f");
        fyi.payload = json!({"notify": false});
        assert_eq!(w.poll(&[fyi]), None);
    }

    #[test]
    fn wakes_only_when_needs_you_or_decisions_go_up() {
        let t = Trigger::default();
        let ev = |n: u64, d: u64| json!({"counts": {"global": {"needs_you": n, "decisions": d}}});
        assert!(t.fires(&ev(1, 1)));
        assert!(!t.fires(&ev(1, 1)));
        assert!(!t.fires(&json!({"topics": ["agents"]})), "no counts: nothing changed");
        assert!(!t.fires(&ev(0, 0)), "down");
        assert!(t.fires(&ev(0, 1)), "a decision with no agent (budget)");
        assert!(t.fires(&ev(1, 1)));
        // One item answered and another raised at once: same counts, the Inbox moved.
        assert!(t.fires(&json!({"topics": ["inbox"]})));
    }

    #[test]
    fn batch_copy_one_agent_and_overflow() {
        let mut w = Watch::default();
        w.poll(&[]);
        let items: Vec<Decision> = (0..5).map(|i| item(&format!("q-{i}"), 1, "back-1")).collect();
        let n = w.poll(&items).unwrap();
        assert_eq!(n.title, "5 decisions need you · back-1");
        assert_eq!(n.body, "back-1 asks\nback-1 asks\nback-1 asks\n+2 more");
        let mut b = item("budget-1", 1, "x");
        b.agent = None;
        b.summary = "line one\nline two".into();
        let n = w
            .poll(&[items[0].clone(), items[1].clone(), items[2].clone(), items[3].clone(), items[4].clone(), b])
            .unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str()), ("Orchestrator needs you", "x asks\nline one"));
    }

    #[test]
    fn activation_within_the_window_focuses_once() {
        let p = Pending::default();
        let t0 = Instant::now();
        assert_eq!(p.take(t0), None, "nothing notified");
        p.arm("q-1".into(), t0);
        assert_eq!(p.take(t0 + Duration::from_secs(59)), Some("q-1".into()));
        assert_eq!(p.take(t0 + Duration::from_secs(60)), None, "one-shot: later focuses do nothing");
        p.arm("q-2".into(), t0);
        assert_eq!(p.take(t0 + Duration::from_secs(61)), None, "too late");
        assert_eq!(p.take(t0 + Duration::from_secs(1)), None, "and cleared");
        p.arm("q-3".into(), t0);
        p.arm("q-4".into(), t0 + Duration::from_secs(5));
        assert_eq!(p.take(t0 + Duration::from_secs(6)), Some("q-4".into()), "the newer notification wins");
    }
}
