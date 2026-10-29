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

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::core::Core;
use agent_runtime::decision::Decision;

pub const SETTINGS_KEY: &str = "notifications";

/// What to show for the new items of one poll.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

fn notifies(d: &Decision) -> bool {
    d.level <= 2 && d.payload.get("notify").and_then(Value::as_bool) != Some(false)
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
        match fresh.as_slice() {
            [] => None,
            [one] => Some(Note { title: one.title.clone(), body: one.summary.chars().take(240).collect() }),
            many => {
                let agents: BTreeSet<&str> = many.iter().filter_map(|i| i.agent.as_deref()).collect();
                let title = match agents.len() {
                    0 | 1 => format!("{} items need you", many.len()),
                    n => format!("{n} agents need you"),
                };
                let body = many.iter().map(|i| i.title.as_str()).take(4).collect::<Vec<_>>().join("\n");
                Some(Note { title, body })
            }
        }
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
            if let Err(e) = n.show() {
                eprintln!("notification failed: {e}");
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
            created_ms: 0,
            agent: Some(agent.into()),
            payload: json!({}),
        }
    }

    #[test]
    fn new_items_notify_once_and_batch() {
        let mut w = Watch::default();
        assert_eq!(w.poll(&[item("q-1", 1, "a")]), None, "what is there at start is known");
        assert_eq!(w.poll(&[item("q-1", 1, "a")]), None);
        let n = w.poll(&[item("q-1", 1, "a"), item("q-2", 1, "b")]).unwrap();
        assert_eq!(n.title, "b asks");
        let n = w.poll(&[item("q-3", 2, "c"), item("perm-4", 1, "d"), item("info-5", 4, "e")]).unwrap();
        assert_eq!(n.title, "2 agents need you");
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
}
