//! The trust dialog's parser, the §3.1 guard, the answer's steps, and the
//! pump on a fake terminal that plays the dialog (spec §6).

use std::sync::{Arc, Mutex};

use orch_protocol::now_ms;
use serde_json::Value;

use super::*;
use crate::overwatch::Status;
use crate::overwatch::tests::{at, manual, t0};
use crate::overwatch::why;
use crate::pty::{Output, PtyProcess};
use crate::runtime::tests::{insert_test_live_with, test_agent};

/// A render like Claude Code's (plain text once the ANSI is stripped).
fn render(selected: u32) -> String {
    let mark = |n| if n == selected { "❯" } else { " " };
    format!(
        "\n ╭──────────╮\n │ Quick safety check: Is this a project you created or one you trust?\n │\n \
         │ {} 1. Yes, I trust this folder\n │ {} 2. No, exit\n │\n │ Enter to confirm · Esc to cancel\n ╰──────────╯\n",
        mark(1),
        mark(2)
    )
    .replace('│', "")
    .replace(['╭', '╮', '╰', '╯', '─'], "")
}

#[test]
fn the_parser_reads_the_latest_render() {
    assert_eq!(next_key(&render(1)), Some(Key::Enter), "on Yes");
    assert_eq!(next_key(&render(2)), Some(Key::Up), "on No: towards Yes");
    // After an arrow, the re-render is the one read.
    assert_eq!(next_key(&format!("{}{}", render(2), render(1))), Some(Key::Enter));
    assert_eq!(next_key(&format!("{}{}", render(1), render(2))), Some(Key::Up));
    // Ink's padding and the words squashed together by stripped escapes.
    assert_eq!(
        next_key("Quick safety check\n   ❯   1.  Yes, I trust this folder  \n    2. No, exit\n"),
        Some(Key::Enter)
    );
    // An escape stripped inside the header (detected squashed, pty.rs): still read.
    let split =
        crate::pty::strip_ansi("Quick \x1b[1msafety\x1b[0m check\n  1. Yes, I trust this folder\n❯ 2. No, exit\n");
    assert_eq!(next_key(&split), Some(Key::Up), "{split:?}");
}

#[test]
fn an_unknown_layout_sends_no_key() {
    let none = |tail: &str| assert_eq!(next_key(tail), None, "{tail:?}");
    none("");
    none("1. Yes, I trust this folder\n❯ 2. No, exit"); // no dialog header
    none("Quick safety check\n"); // a partial render
    none("Quick safety check\n  1. Yes, I trust this folder\n  2. No, exit"); // nothing selected
    none("Quick safety check\n❯ 1. Yes, I trust this folder\n❯ 2. No, exit"); // two selections
    none("Quick safety check\n❯ 1. Trust and continue\n  2. No, exit"); // no "Yes" option
    none("Quick safety check\n  1. No, exit\n❯ 2. Something else\n  3. Yes, I trust this folder"); // Yes below: no Down
    none("Quick safety check\n❯ 1. No, exit\n  2. Yes, I trust this folder");
    none("Quick safety check\n  1. Yes, I trust this folder\n  ❯ Continue\n  2. No, exit"); // a mark off an option
    none("Quick safety check\n  1. Yes, I trust this folder\n  2. No ❯ exit");
    none("Quick safety check\n❯ 1. Yes, I trust this folder\n  2. Yes, I trust this folder");
}

#[test]
fn only_the_overwatch_day_folder_is_auto_trusted() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("app");
    let day = root.join("overwatch/2026-11-05");
    std::fs::create_dir_all(&day).unwrap();
    assert!(auto_trusted(&root, &day));
    // Compared canonicalised: the same folder by another path.
    assert!(auto_trusted(&root, &root.join("overwatch/../overwatch/2026-11-05")));
    let other = |p: &Path| {
        std::fs::create_dir_all(p).unwrap();
        assert!(!auto_trusted(&root, p), "{p:?}");
    };
    other(&root.join("overwatch"));
    other(&root.join("overwatch/notes"));
    other(&root.join("overwatch/2026-11-05/sub"));
    other(&root.join("worktrees/2026-11-05"));
    other(&d.path().join("elsewhere/overwatch/2026-11-05"));
    assert!(!auto_trusted(&root, &root.join("overwatch/2026-11-06")), "missing");
    #[cfg(unix)]
    {
        // A link from the day folder's place to a repo is the repo.
        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&repo, root.join("overwatch/2026-11-07")).unwrap();
        assert!(!auto_trusted(&root, &root.join("overwatch/2026-11-07")));
    }
}

#[test]
fn the_answer_moves_up_then_enters_within_its_budget() {
    let mut a = Answer::new(0, true);
    assert_eq!(a.step(0, &render(2), 100), Step::Send(Key::Up));
    assert_eq!(a.step(200, &render(2), 100), Step::Wait, "waits for the re-render");
    let tail = format!("{}{}", render(2), render(1));
    assert_eq!(a.step(400, &tail, 180), Step::Send(Key::Enter), "re-rendered on Yes");
    assert_eq!(a.step(600, &tail, 180), Step::Wait, "one Enter");
    assert_eq!(a.step(WINDOW_MS, &tail, 180), Step::Fail(failed::NOT_TAKEN));
    assert_eq!(a.step(WINDOW_MS + 200, &tail, 180), Step::Raise);

    // No re-render: an arrow a second, three at most.
    let mut a = Answer::new(0, true);
    assert_eq!(a.step(0, &render(2), 100), Step::Send(Key::Up));
    assert_eq!(a.step(999, &render(2), 100), Step::Wait);
    assert_eq!(a.step(1_000, &render(2), 100), Step::Send(Key::Up));
    assert_eq!(a.step(2_000, &render(2), 100), Step::Send(Key::Up));
    assert_eq!(a.step(3_000, &render(2), 100), Step::Fail(failed::ARROWS));
    assert_eq!(a.step(3_200, &render(1), 100), Step::Raise, "gave up: never Enter after");

    // An unknown layout: no key, then the fallback when the window ends.
    let mut a = Answer::new(0, true);
    assert_eq!(a.step(0, "Quick safety check\n", 10), Step::Wait);
    assert_eq!(a.step(WINDOW_MS - 1, "Quick safety check\n", 10), Step::Wait);
    assert_eq!(a.step(WINDOW_MS, "Quick safety check\n", 10), Step::Fail(failed::TIMEOUT));

    // Not the Overwatch's day folder: J0, the flag at once.
    assert_eq!(Answer::new(0, false).step(0, &render(1), 10), Step::Raise);
}

// ------------------------------------------------------------ on the pump

/// Claude Code's dialog on a fake terminal: "No, exit" selected; Up selects
/// "Yes" (and re-renders) unless `stuck`; Enter on "Yes" starts it.
struct Dialog {
    out: Arc<Mutex<Option<Arc<Output>>>>,
    keys: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Dialog {
    fn spawn(rt: &Runtime, id: &str, stuck: bool) -> Dialog {
        let out: Arc<Mutex<Option<Arc<Output>>>> = Arc::default();
        let keys: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let (o2, k2) = (out.clone(), keys.clone());
        let pty = PtyProcess::fake(
            Box::new(move |bytes| {
                k2.lock().unwrap().push(bytes.to_vec());
                if bytes == Key::Up.bytes()
                    && !stuck
                    && let Some(o) = o2.lock().unwrap().as_ref()
                {
                    o.push(render(1).as_bytes());
                }
            }),
            Box::new(|_| {}),
        );
        pty.output.push(render(2).as_bytes());
        *out.lock().unwrap() = Some(pty.output.clone());
        insert_test_live_with(rt, id, pty);
        Dialog { out, keys }
    }

    fn keys(&self) -> Vec<Vec<u8>> {
        self.keys.lock().unwrap().clone()
    }

    /// SessionStart: the dialog is gone.
    fn started(&self) {
        self.out.lock().unwrap().as_ref().unwrap().clear_trust_dialog();
    }
}

/// The Overwatch's record, spawned today in `dir`.
fn overwatch(rt: &Runtime, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let mut a = test_agent(dir);
    (a.id, a.overwatch, a.spawns) = (ID.into(), true, 1);
    rt.store.write(|s| s.agents.insert(ID.into(), a)).unwrap();
}

fn flag(rt: &Runtime, id: &str) -> bool {
    rt.store.read(|s| s.agents[id].trust_dialog)
}

fn events(rt: &Runtime, kind: &str) -> Vec<Value> {
    rt.db.flush().unwrap();
    rt.db.events(0, None, Some(kind), 50).unwrap().into_iter().map(|e| e.detail).collect()
}

fn trust_card(rt: &Runtime, id: &str) -> Option<Value> {
    let snap = rt.now_snapshot();
    snap["decisions"].as_array().unwrap().iter().find(|d| d["id"] == format!("trust-{id}")).cloned()
}

#[test]
fn the_overwatch_answers_its_own_dialog_and_nobody_sees_it() {
    let (_d, rt) = manual();
    let day = rt.overwatch_day_dir("2026-11-05");
    overwatch(&rt, &day);
    let dialog = Dialog::spawn(&rt, ID, false);
    rt.deliver(ID, "ORCH: Read briefing.md").unwrap();
    let t = now_ms();
    rt.pump_trust(t);
    assert_eq!(dialog.keys(), vec![Key::Up.bytes().to_vec()], "No was selected: one arrow");
    // The pump's delivery: blocked on the PTY flag, nothing pasted into it.
    rt.pump();
    rt.pump_trust(t + 200);
    assert_eq!(dialog.keys(), vec![Key::Up.bytes().to_vec(), Key::Enter.bytes().to_vec()], "Enter on Yes");
    assert!(!flag(&rt, ID), "no record flag");
    assert!(trust_card(&rt, ID).is_none(), "no Inbox card");
    assert_ne!(rt.overwatch_status().status, Status::Paused);
    let answered = events(&rt, "overwatch.trust_answered");
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0]["dir"], day.display().to_string());
    // Started: the window ending afterwards changes nothing.
    dialog.started();
    rt.pump_trust(t + WINDOW_MS + 1);
    assert!(!flag(&rt, ID));
    assert!(events(&rt, "overwatch.trust_failed").is_empty());
    assert_eq!(dialog.keys().len(), 2, "no key after the answer");
    assert!(events(&rt, "trust_dialog").is_empty(), "the flag never moved");
}

#[test]
fn a_dialog_that_never_takes_the_answer_falls_back_to_the_user() {
    let (_d, rt) = manual();
    at(&rt, t0());
    overwatch(&rt, &rt.overwatch_day_dir("2026-11-05"));
    let dialog = Dialog::spawn(&rt, ID, true);
    let t = now_ms();
    for dt in [0, 200, 1_000, 2_000] {
        rt.pump_trust(t + dt);
        assert!(!flag(&rt, ID), "+{dt}: still answering");
    }
    assert_eq!(dialog.keys(), vec![Key::Up.bytes().to_vec(); 3], "three arrows, never Enter on No, never Esc");
    rt.pump_trust(t + 3_000);
    assert!(flag(&rt, ID), "gave up: the record flag");
    let failed = events(&rt, "overwatch.trust_failed");
    assert_eq!((failed.len(), failed[0]["why"].as_str()), (1, Some(failed::ARROWS)));
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref(), s.has_session), (Status::Paused, Some(why::TRUST), true));
    let card = trust_card(&rt, ID).expect("the Inbox card");
    assert_eq!(card["title"], "Claude Code asks to trust today's Overwatch folder", "{card}");
    assert_eq!(card["summary"], "The app could not answer it: answer it in its session", "{card}");
    rt.pump_trust(t + WINDOW_MS);
    assert_eq!(events(&rt, "overwatch.trust_failed").len(), 1, "once");
    assert_eq!(dialog.keys().len(), 3);
    // The user answered in its session: SessionStart clears it all.
    dialog.started();
    rt.pump_trust(t + WINDOW_MS + 200);
    assert!(!flag(&rt, ID));
    assert!(trust_card(&rt, ID).is_none());
    assert_ne!(rt.overwatch_status().status, Status::Paused);
}

#[test]
fn the_window_ends_in_the_fallback_when_the_layout_is_unknown() {
    let (_d, rt) = manual();
    overwatch(&rt, &rt.overwatch_day_dir("2026-11-05"));
    let pty = crate::pty::echoing_fake(Box::new(|_| {}));
    pty.output.push(b"Quick safety check\n  Trust this folder?  [y/N]\n");
    let out = pty.output.clone();
    insert_test_live_with(&rt, ID, pty);
    let t = now_ms();
    rt.pump_trust(t);
    rt.pump_trust(t + WINDOW_MS - 1);
    assert!(!flag(&rt, ID));
    assert!(out.tail_text().matches("Quick safety check").count() == 1, "no key echoed");
    rt.pump_trust(t + WINDOW_MS);
    assert!(flag(&rt, ID));
    assert_eq!(events(&rt, "overwatch.trust_failed")[0]["why"], failed::TIMEOUT);
}

#[test]
fn any_other_session_keeps_j0() {
    let (d, rt) = manual();
    // The Overwatch's record in a folder that is not its day folder.
    overwatch(&rt, &d.path().join("repo"));
    let ow = Dialog::spawn(&rt, ID, false);
    // A regular agent.
    let a1 = Dialog::spawn(&rt, "a1", false);
    rt.pump_trust(now_ms());
    assert!(flag(&rt, ID) && flag(&rt, "a1"), "the flag at once");
    assert!(ow.keys().is_empty() && a1.keys().is_empty(), "no key");
    assert!(events(&rt, "overwatch.trust_answered").is_empty() && events(&rt, "overwatch.trust_failed").is_empty());
    let card = trust_card(&rt, "a1").expect("a1's card");
    assert_eq!(card["title"], "Claude Code asks to trust this folder");
    assert_eq!(card["summary"], "Answer it in a1's pane: the app never does");
    // A record named like the Overwatch but not made by its start.
    rt.store.write(|s| s.agents.get_mut(ID).unwrap().overwatch = false).unwrap();
    assert_ne!(rt.overwatch_status().why.as_deref(), Some(why::TRUST));
}
