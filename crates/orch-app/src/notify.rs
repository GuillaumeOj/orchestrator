//! Desktop notifications (SPEC §9.6): a new Inbox item that needs the user
//! (levels 1 and 2) raises a macOS notification with a sound, several at
//! once one batched notification ("3 agents need you"). Watched from the
//! core every few seconds, so it works while the window is hidden. A team
//! question asked as `fyi` never notifies. Settings key `notifications`:
//! `{desktop, sound}` (both on by default).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::core::Core;
use crate::inbox::InboxItem;

const EVERY: Duration = Duration::from_secs(3);
pub const SETTINGS_KEY: &str = "notifications";

/// What to show for the new items of one poll.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

fn notifies(i: &InboxItem) -> bool {
    i.level <= 2 && i.payload.get("notify").and_then(Value::as_bool) != Some(false)
}

/// Remembers the items already notified; the first poll only learns them
/// (no burst for what was there at start).
#[derive(Default)]
pub struct Watch {
    seen: BTreeSet<String>,
    started: bool,
}

impl Watch {
    pub fn poll(&mut self, items: &[InboxItem]) -> Option<Note> {
        let now: BTreeSet<String> = items.iter().filter(|i| notifies(i)).map(|i| i.id.clone()).collect();
        let fresh: Vec<&InboxItem> =
            items.iter().filter(|i| now.contains(&i.id) && !self.seen.contains(&i.id)).collect();
        self.seen = now;
        if !std::mem::replace(&mut self.started, true) {
            return None;
        }
        match fresh.as_slice() {
            [] => None,
            [one] => Some(Note { title: one.title.clone(), body: one.detail.chars().take(240).collect() }),
            many => {
                let agents: BTreeSet<&str> = many.iter().filter_map(|i| i.agent_id.as_deref()).collect();
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

/// Starts the watcher thread for the app's lifetime.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut watch = Watch::default();
        loop {
            std::thread::sleep(EVERY);
            let Some(core) = app.try_state::<Arc<Core>>() else { continue };
            let Some(note) = watch.poll(&core.attention()) else { continue };
            let (desktop, sound) = prefs(&core);
            if !desktop {
                continue;
            }
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

    fn item(id: &str, level: u8, agent: &str) -> InboxItem {
        InboxItem {
            id: id.into(),
            level,
            kind: "question".into(),
            agent_id: Some(agent.into()),
            title: format!("{agent} asks"),
            detail: "which db?".into(),
            preview: None,
            created_ms: 0,
            actions: vec![],
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
}
