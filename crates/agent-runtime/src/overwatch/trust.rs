//! The Overwatch answers Claude Code's workspace trust dialog itself, in its
//! own day folder only (fix-overwatch-trust-folder spec §3): every other
//! agent keeps J0, the app never answers theirs. The parser and the answer's
//! steps are pure; [`Runtime::overwatch_trust`] runs them from the pump.

use std::path::Path;

use serde_json::json;

use super::ID;
use crate::pty::{TRUST_DIALOG_TEXT, squash};
use crate::runtime::{Live, Runtime};

/// The auto-answer window, from the dialog's detection (spec §3.3).
pub const WINDOW_MS: u64 = 10_000;
/// How long an arrow waits for the dialog's re-render.
pub const RERENDER_MS: u64 = 1_000;
/// Arrows per spawn before giving up.
pub const MAX_ARROWS: u32 = 3;
/// The option the answer picks.
pub const YES: &str = "Yes, I trust this folder";
/// The selection mark of the dialog's options.
const SELECTED: char = '❯';

/// `overwatch.trust_failed` `why`s.
pub mod failed {
    pub const TIMEOUT: &str = "no answer within 10 s";
    pub const ARROWS: &str = "not on \"Yes, I trust this folder\" after 3 arrows";
    pub const NOT_TAKEN: &str = "the answer did not take";
}

/// A key the answer sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Up,
}

impl Key {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Key::Enter => b"\r",
            Key::Up => b"\x1b[A",
        }
    }
}

/// The key that moves the latest render of the dialog in `tail` (the PTY's
/// plain-text tail) towards trusting: Enter when "Yes, I trust this folder"
/// is selected, Up when an option below it is. `None` for anything else (no
/// dialog, a partial render, no or several selections, an unknown layout):
/// no key is sent then.
pub fn next_key(tail: &str) -> Option<Key> {
    // Squashed per line, like the detection (pty.rs): a stripped escape
    // leaves a run of spaces, even inside the header.
    let tail = tail.lines().map(squash).collect::<Vec<_>>().join("\n");
    let render = &tail[tail.rfind(TRUST_DIALOG_TEXT)?..];
    let mut yes = None;
    let mut selected = None;
    for line in render.lines() {
        let (marked, rest) = match line.strip_prefix(SELECTED) {
            Some(r) => (true, r.trim_start()),
            None => (false, line),
        };
        // A mark anywhere but in front of an option: not a layout we know.
        if rest.contains(SELECTED) {
            return None;
        }
        let option = rest.split_once(". ").and_then(|(n, text)| Some((n.parse::<u32>().ok()?, text)));
        let Some((n, text)) = option else {
            if marked {
                return None;
            }
            continue;
        };
        let is_yes = text.starts_with(YES);
        if (is_yes && yes.replace(n).is_some()) || (marked && selected.replace((n, is_yes)).is_some()) {
            return None;
        }
    }
    match (selected?, yes?) {
        ((_, true), _) => Some(Key::Enter),
        ((n, false), yes) if n > yes => Some(Key::Up),
        _ => None,
    }
}

/// §3.1: the Overwatch's own day folder, `<root>/overwatch/<YYYY-MM-DD>`,
/// compared after canonicalising both. Anything else is never auto-trusted.
pub fn auto_trusted(root: &Path, worktree: &Path) -> bool {
    let (Ok(base), Ok(dir)) = (root.join("overwatch").canonicalize(), worktree.canonicalize()) else {
        return false;
    };
    dir.parent() == Some(base.as_path())
        && dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| crate::chat::as_day(n).is_some())
}

/// What the pump does on this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing yet: the record flag stays down.
    Wait,
    Send(Key),
    /// The answer gave up (or the window ended): raise the flag, once.
    Fail(&'static str),
    /// Not auto-answered, or already failed: the flag is up, as J0.
    Raise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Answering,
    /// Enter was sent: waiting for SessionStart.
    Answered,
    /// Not auto-answered, or given up: the flag is up.
    Raised,
}

/// One spawn's answer (one per [`Live`]: a new spawn tries again).
#[derive(Debug)]
pub struct Answer {
    detected_ms: u64,
    phase: Phase,
    arrows: u32,
    /// The last arrow: when, and the PTY's byte count then.
    sent: Option<(u64, u64)>,
    /// The Overwatch's folder, for the events.
    pub(crate) dir: String,
}

impl Answer {
    pub fn new(detected_ms: u64, eligible: bool) -> Answer {
        let phase = if eligible { Phase::Answering } else { Phase::Raised };
        Answer { detected_ms, phase, arrows: 0, sent: None, dir: String::new() }
    }

    /// Settled: every later step is [`Step::Raise`], whatever the screen.
    fn raised(&self) -> bool {
        self.phase == Phase::Raised
    }

    /// `tail` and `total` are the PTY's plain-text tail and byte count now.
    pub fn step(&mut self, now: u64, tail: &str, total: u64) -> Step {
        if self.raised() {
            return Step::Raise;
        }
        if now.saturating_sub(self.detected_ms) >= WINDOW_MS {
            return self.fail(if self.phase == Phase::Answered { failed::NOT_TAKEN } else { failed::TIMEOUT });
        }
        if self.phase == Phase::Answered {
            return Step::Wait;
        }
        if let Some((at, seen)) = self.sent
            && total == seen
            && now.saturating_sub(at) < RERENDER_MS
        {
            return Step::Wait;
        }
        self.sent = None;
        match next_key(tail) {
            Some(Key::Enter) => {
                self.phase = Phase::Answered;
                Step::Send(Key::Enter)
            }
            Some(Key::Up) if self.arrows < MAX_ARROWS => {
                self.arrows += 1;
                self.sent = Some((now, total));
                Step::Send(Key::Up)
            }
            Some(Key::Up) => self.fail(failed::ARROWS),
            None => Step::Wait,
        }
    }

    fn fail(&mut self, why: &'static str) -> Step {
        self.phase = Phase::Raised;
        Step::Fail(why)
    }
}

impl Runtime {
    /// Its trust dialog outlived the auto-answer: the record flag is up (§3.4).
    pub(crate) fn overwatch_trust_raised(&self) -> bool {
        self.store.read(|s| s.agents.get(ID).is_some_and(|a| a.is_overwatch() && a.trust_dialog))
    }

    /// The Overwatch's trust dialog is on its PTY: answers it when §3.1
    /// holds. Returns whether its record flag goes up now (the Inbox card,
    /// the decision, `paused` / `why::TRUST`): only once the answer failed,
    /// or at once when it is not auto-answered. Delivery stays blocked on
    /// the PTY flag meanwhile.
    pub(crate) fn overwatch_trust(&self, l: &Live, now: u64) -> bool {
        let mut answer = l.trust.lock().unwrap_or_else(|e| e.into_inner());
        // Settled (often for long, in the fallback): no store read, no tail copy.
        if answer.as_ref().is_some_and(Answer::raised) {
            return true;
        }
        let answer = answer.get_or_insert_with(|| {
            let dir = self.store.read(|s| s.agents.get(ID).filter(|a| a.is_overwatch()).map(|a| a.worktree.clone()));
            let mut a = Answer::new(now, dir.as_deref().is_some_and(|d| auto_trusted(&self.cfg.paths.root, d)));
            a.dir = dir.map(|d| d.display().to_string()).unwrap_or_default();
            a
        });
        let step = answer.step(now, &l.pty.output.tail_text(), l.pty.output.total());
        let dir = &answer.dir;
        match step {
            Step::Wait => false,
            Step::Send(key) => {
                // Straight to the PTY: not a turn (no admission, marker or paste).
                let sent = l.pty.write(key.bytes()).is_ok();
                if sent && key == Key::Enter {
                    self.event(ID, "overwatch.trust_answered", json!({"dir": dir}));
                }
                false
            }
            Step::Fail(why) => {
                self.event(ID, "overwatch.trust_failed", json!({"dir": dir, "why": why}));
                true
            }
            Step::Raise => true,
        }
    }
}

#[cfg(test)]
mod tests;
