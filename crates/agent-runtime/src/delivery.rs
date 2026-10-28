//! Programmatic input to an idle agent (FR-206, R4). Pure: what to type and
//! whether it may be typed now; the PTY write lives in the runtime.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Bracketed pastes of about 1000 chars or more reach the model as untrusted
/// `<pasted_content>` (P0); keep a margin.
pub const MAX_PASTE_CHARS: usize = 900;
/// The TUI drops keys sent too soon after it first renders (P0).
pub const AFTER_SESSION_START: Duration = Duration::from_millis(1_500);
/// Small settle time after a Stop before the next delivery.
pub const AFTER_STOP: Duration = Duration::from_millis(500);
/// The user's own typing wins: no delivery within this time of a keystroke (R4).
pub const USER_QUIET: Duration = Duration::from_secs(2);
/// Between the paste and the Enter that submits it (recovery harness: 0.3 s).
pub const PASTE_TO_ENTER: Duration = Duration::from_millis(300);

pub const PASTE_START: &[u8] = b"\x1b[200~";
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// The turn-id marker the reconciler looks for in the transcript (J10 step 2.6).
pub fn marker(turn_id: &str) -> String {
    format!("[t:{turn_id}]")
}

/// Finds every turn marker in a text (8 lowercase hex chars, as generated).
pub fn find_markers(text: &str) -> Vec<String> {
    let mut out = vec![];
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(p) = text[i..].find("[t:") {
        let s = i + p + 3;
        let e = s + 8;
        if e < b.len() && b[e] == b']' && b[s..e].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)) {
            out.push(text[s..e].to_string());
        }
        i = s;
    }
    out
}

pub fn new_turn_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// A message that starts with `/`, `!` or `#` would run as a slash command,
/// a shell command or a memory note, and no turn would start: start it with
/// a space. Tabs become 4 spaces anyway, so do it here.
pub fn escape_leading(text: &str) -> String {
    let text = text.replace('\t', "    ");
    match text.trim_start_matches([' ', '\n', '\r']).chars().next() {
        Some('/' | '!' | '#') if !text.starts_with(' ') => format!(" {text}"),
        _ => text,
    }
}

/// What to paste for one app-delivered turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub turn_id: String,
    /// The text inside the bracketed paste.
    pub text: String,
    /// The long body written to a file, when it did not fit.
    pub file: Option<(PathBuf, String)>,
}

impl Prepared {
    /// Bracketed paste bytes; Enter (`\r`) is sent after [`PASTE_TO_ENTER`].
    pub fn paste_bytes(&self) -> Vec<u8> {
        [PASTE_START, self.text.as_bytes(), PASTE_END].concat()
    }
}

/// Builds the turn: `[t:<id>] <body>`. A body that would make the paste longer
/// than [`MAX_PASTE_CHARS`] goes to `<agent dir>/messages/<id>.md`, and the
/// paste points to it.
pub fn prepare(body: &str, turn_id: &str, agent_dir: &Path) -> Prepared {
    prepare_many(&[(turn_id, body)], agent_dir)
}

/// Several queued turns of one agent in one paste: every marker leads, so
/// each turn is marked delivered when the prompt starts, even when the
/// bodies go to a file. The first id names the paste and its file.
pub fn prepare_many(turns: &[(&str, &str)], agent_dir: &Path) -> Prepared {
    let turn_id = turns.first().map(|t| t.0).unwrap_or_default();
    let head = turns.iter().map(|t| marker(t.0)).collect::<Vec<_>>().join(" ");
    let body = turns.iter().map(|t| escape_leading(t.1.trim_end())).collect::<Vec<_>>().join("\n\n---\n\n");
    let text = format!("{head} {body}");
    if text.chars().count() <= MAX_PASTE_CHARS {
        return Prepared { turn_id: turn_id.into(), text, file: None };
    }
    let path = agent_dir.join("messages").join(format!("{turn_id}.md"));
    let text =
        format!("{head} ORCH: the orchestrator's message is too long to paste. Read {} and follow it.", path.display());
    Prepared { turn_id: turn_id.into(), text, file: Some((path, body)) }
}

/// Why a delivery has to wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    NotLive,
    NotIdle,
    /// Background shells or subagents are still running (§6.2).
    BackgroundTasks(usize),
    StartingUp,
    UserTyping,
    /// The trust dialog is up: only the user may answer it (J0).
    TrustDialog,
}

/// The facts the delivery rule looks at (times are milliseconds on any one clock).
#[derive(Clone, Debug, Default)]
pub struct GateInputs {
    pub live: bool,
    pub idle: bool,
    pub trust_dialog: bool,
    pub background_tasks: usize,
    /// Earliest time input may be sent (SessionStart + 1.5 s, Stop + 0.5 s).
    pub ready_at_ms: Option<u64>,
    pub last_user_input_ms: Option<u64>,
    pub now_ms: u64,
}

pub fn can_deliver(g: &GateInputs) -> Result<(), Blocker> {
    if !g.live {
        return Err(Blocker::NotLive);
    }
    if g.trust_dialog {
        return Err(Blocker::TrustDialog);
    }
    if !g.idle {
        return Err(Blocker::NotIdle);
    }
    if g.background_tasks > 0 {
        return Err(Blocker::BackgroundTasks(g.background_tasks));
    }
    match g.ready_at_ms {
        Some(t) if g.now_ms >= t => {}
        _ => return Err(Blocker::StartingUp),
    }
    if let Some(t) = g.last_user_input_ms {
        if g.now_ms < t + USER_QUIET.as_millis() as u64 {
            return Err(Blocker::UserTyping);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_command_chars_get_a_space() {
        assert_eq!(escape_leading("/compact now"), " /compact now");
        assert_eq!(escape_leading("!rm -rf x"), " !rm -rf x");
        assert_eq!(escape_leading("# note"), " # note");
        assert_eq!(escape_leading("\n/x"), " \n/x");
        assert_eq!(escape_leading(" /already"), " /already");
        assert_eq!(escape_leading("plain / text"), "plain / text");
        assert_eq!(escape_leading("a\tb"), "a    b");
        assert_eq!(escape_leading(""), "");
    }

    #[test]
    fn short_message_is_pasted_with_its_marker() {
        let p = prepare("/review the diff", "0123abcd", Path::new("/ag"));
        assert_eq!(p.text, "[t:0123abcd]  /review the diff");
        assert!(p.file.is_none());
        let bytes = p.paste_bytes();
        assert!(bytes.starts_with(b"\x1b[200~[t:0123abcd]") && bytes.ends_with(b"\x1b[201~"));
        assert!(!bytes.contains(&b'\r'), "Enter is sent separately");
    }

    #[test]
    fn long_message_goes_to_a_file() {
        let body = "x".repeat(MAX_PASTE_CHARS);
        let p = prepare(&body, "0123abcd", Path::new("/ag"));
        let (path, content) = p.file.clone().unwrap();
        assert_eq!(path, PathBuf::from("/ag/messages/0123abcd.md"));
        assert_eq!(content, body);
        assert!(p.text.starts_with("[t:0123abcd] ORCH:"));
        assert!(p.text.contains("/ag/messages/0123abcd.md"));
        assert!(p.text.chars().count() < MAX_PASTE_CHARS);
        // Exactly at the limit still pastes.
        let fits = "y".repeat(MAX_PASTE_CHARS - "[t:0123abcd] ".len());
        assert!(prepare(&fits, "0123abcd", Path::new("/ag")).file.is_none());
    }

    #[test]
    fn several_turns_share_one_paste() {
        let p = prepare_many(&[("0000000a", "first"), ("0000000b", "second")], Path::new("/ag"));
        assert_eq!(p.text, "[t:0000000a] [t:0000000b] first\n\n---\n\nsecond");
        assert_eq!(find_markers(&p.text), ["0000000a", "0000000b"]);
        let long = "x".repeat(MAX_PASTE_CHARS);
        let p = prepare_many(&[("0000000a", "a"), ("0000000b", &long)], Path::new("/ag"));
        assert_eq!(find_markers(&p.text), ["0000000a", "0000000b"], "markers stay in the paste");
        assert!(p.file.unwrap().1.ends_with(&long));
    }

    #[test]
    fn markers() {
        let id = new_turn_id();
        assert_eq!(id.len(), 8);
        assert_eq!(find_markers(&format!("{} hello [t:zzzzzzzz] [t:0000000f]", marker(&id))), [id, "0000000f".into()]);
        assert!(find_markers("[t:short]").is_empty());
    }

    #[test]
    fn gate_rules() {
        let ok = GateInputs { live: true, idle: true, ready_at_ms: Some(1_000), now_ms: 5_000, ..Default::default() };
        assert_eq!(can_deliver(&ok), Ok(()));
        assert_eq!(can_deliver(&GateInputs { live: false, ..ok.clone() }), Err(Blocker::NotLive));
        assert_eq!(can_deliver(&GateInputs { idle: false, ..ok.clone() }), Err(Blocker::NotIdle));
        assert_eq!(can_deliver(&GateInputs { background_tasks: 2, ..ok.clone() }), Err(Blocker::BackgroundTasks(2)));
        assert_eq!(can_deliver(&GateInputs { ready_at_ms: None, ..ok.clone() }), Err(Blocker::StartingUp));
        assert_eq!(can_deliver(&GateInputs { ready_at_ms: Some(6_000), ..ok.clone() }), Err(Blocker::StartingUp));
        assert_eq!(
            can_deliver(&GateInputs { last_user_input_ms: Some(4_000), ..ok.clone() }),
            Err(Blocker::UserTyping)
        );
        assert_eq!(can_deliver(&GateInputs { last_user_input_ms: Some(2_900), ..ok.clone() }), Ok(()));
        assert_eq!(can_deliver(&GateInputs { trust_dialog: true, ..ok }), Err(Blocker::TrustDialog));
    }
}
