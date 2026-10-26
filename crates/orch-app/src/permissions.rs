//! Permission prompts answered from the Inbox (FR-705, J5).
//!
//! A `PermissionRequest` hook fires when Claude Code would show a permission
//! dialog. The core holds it (orch-hook waits up to its deadline) while the
//! request sits in the Inbox and in the agent pane's strip. The answer is one
//! of: allow, deny (with a message for Claude), or "answer in the pane", which
//! returns no decision so the CLI shows its own dialog (needed for "don't ask
//! again"). Esc in the pane kills orch-hook: the connection's EOF closes the
//! request as interrupted. With no answer before the deadline the request
//! also falls back to the pane's dialog.

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use orch_protocol::{HookOutput, HookPayload, PermissionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Allow,
    Deny { message: Option<String> },
    /// No decision: the CLI shows its own dialog in the pane.
    InPane,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Answered(Answer),
    Interrupted,
    TimedOut,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingPermission {
    pub id: String,
    pub agent_id: String,
    pub tool: String,
    /// The command, file or URL the tool wants, as Claude Code shows it.
    pub summary: String,
    pub description: Option<String>,
    pub created_ms: u64,
    #[serde(skip)]
    pub tool_use_id: Option<String>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    pending: BTreeMap<String, (PendingPermission, Option<Answer>)>,
}

#[derive(Default)]
pub struct PermissionBroker {
    inner: Mutex<Inner>,
    cv: Condvar,
}

/// What the dialog would ask about: the Bash command, the file path, or the input.
pub fn summarize(tool: &str, input: Option<&Value>) -> (String, Option<String>) {
    let Some(v) = input else { return (tool.to_string(), None) };
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let desc = s("description");
    let main = match tool {
        "Bash" => s("command"),
        "Edit" | "Write" | "Read" | "MultiEdit" | "NotebookEdit" => s("file_path").or_else(|| s("notebook_path")),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        _ => None,
    };
    let main = main.unwrap_or_else(|| {
        let t = v.to_string();
        if t.chars().count() > 200 { t.chars().take(200).collect::<String>() + "…" } else { t }
    });
    (main, desc)
}

impl PermissionBroker {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn open(&self, agent_id: &str, payload: &HookPayload, now_ms: u64) -> String {
        let tool = payload.tool_name.clone().unwrap_or_else(|| "tool".into());
        let (summary, description) = summarize(&tool, payload.tool_input.as_ref());
        let mut g = self.lock();
        g.next += 1;
        let id = format!("perm-{}", g.next);
        let p = PendingPermission {
            id: id.clone(),
            agent_id: agent_id.into(),
            tool,
            summary,
            description,
            created_ms: now_ms,
            tool_use_id: payload.tool_use_id.clone(),
        };
        g.pending.insert(id.clone(), (p, None));
        self.cv.notify_all();
        id
    }

    pub fn list(&self) -> Vec<PendingPermission> {
        self.lock().pending.values().filter(|(_, a)| a.is_none()).map(|(p, _)| p.clone()).collect()
    }

    /// Records the user's answer. False when the request is gone (answered,
    /// interrupted or timed out).
    pub fn answer(&self, id: &str, answer: Answer) -> bool {
        let mut g = self.lock();
        match g.pending.get_mut(id) {
            Some((_, slot @ None)) => {
                *slot = Some(answer);
                self.cv.notify_all();
                true
            }
            _ => false,
        }
    }

    /// The user answered in the pane (the CLI shows its dialog while the hook
    /// is held): the agent's next hook for that call (`tool_use_id` given and
    /// equal to the request's), or a turn boundary (`None`), closes the request
    /// with no decision. A call hook can't be matched to a request without an
    /// id (parallel calls), so it closes nothing. Returns how many were closed.
    pub fn answered_in_pane(&self, agent_id: &str, tool_use_id: Option<&str>) -> usize {
        let mut g = self.lock();
        let mut n = 0;
        for (p, slot) in g.pending.values_mut() {
            let same_call = match (tool_use_id, p.tool_use_id.as_deref()) {
                (None, _) => true,
                (Some(a), Some(b)) => a == b,
                (Some(_), None) => false,
            };
            if p.agent_id == agent_id && slot.is_none() && same_call {
                *slot = Some(Answer::InPane);
                n += 1;
            }
        }
        if n > 0 {
            self.cv.notify_all();
        }
        n
    }

    /// Holds until an answer, `deadline`, or `closed()` (polled every `poll`).
    pub fn wait(&self, id: &str, deadline: Instant, poll: Duration, closed: impl Fn() -> bool) -> Outcome {
        let outcome = loop {
            {
                let g = self.lock();
                if let Some((_, Some(a))) = g.pending.get(id) {
                    break Outcome::Answered(a.clone());
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break Outcome::TimedOut;
                }
                let _ = self.cv.wait_timeout(g, left.min(poll)).unwrap_or_else(|e| e.into_inner());
            }
            if closed() {
                break Outcome::Interrupted;
            }
        };
        self.lock().pending.remove(id);
        self.cv.notify_all();
        outcome
    }

    /// The hook's reply for an outcome.
    pub fn hook_output(outcome: &Outcome) -> Option<HookOutput> {
        match outcome {
            Outcome::Answered(Answer::Allow) => Some(HookOutput::permission_request(PermissionBehavior::Allow, None)),
            Outcome::Answered(Answer::Deny { message }) => Some(HookOutput::permission_request(
                PermissionBehavior::Deny,
                Some(message.as_deref().unwrap_or("ORCH: the user denied this from the Inbox.")),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn payload(cmd: &str) -> HookPayload {
        HookPayload::parse(&json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
            "tool_input": {"command": cmd, "description": "Install deps"}}))
    }

    #[test]
    fn summarizes_like_the_dialog() {
        assert_eq!(summarize("Bash", Some(&json!({"command": "pnpm add x", "description": "d"}))), ("pnpm add x".into(), Some("d".into())));
        assert_eq!(summarize("Edit", Some(&json!({"file_path": "/a/b.rs"}))).0, "/a/b.rs");
        assert_eq!(summarize("mcp__x", Some(&json!({"q": 1}))).0, r#"{"q":1}"#);
        assert_eq!(summarize("Bash", None).0, "Bash");
    }

    #[test]
    fn answer_from_another_thread_releases_the_hold() {
        let b = Arc::new(PermissionBroker::default());
        let id = b.open("a1", &payload("pnpm add x"), 1);
        assert_eq!(b.list()[0].summary, "pnpm add x");
        let (b2, id2) = (b.clone(), id.clone());
        let t = std::thread::spawn(move || {
            b2.wait(&id2, Instant::now() + Duration::from_secs(5), Duration::from_millis(20), || false)
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(b.answer(&id, Answer::Allow));
        assert_eq!(t.join().unwrap(), Outcome::Answered(Answer::Allow));
        assert!(b.list().is_empty());
        assert!(!b.answer(&id, Answer::Allow), "already closed");
        let out = PermissionBroker::hook_output(&Outcome::Answered(Answer::Allow)).unwrap();
        assert_eq!(out.0["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn answering_in_the_pane_closes_the_request() {
        let b = PermissionBroker::default();
        let mut p = payload("ls");
        p.tool_use_id = Some("tu1".into());
        let id = b.open("a1", &p, 1);
        assert_eq!(b.answered_in_pane("a1", Some("other")), 0, "another call");
        let no_id = b.open("a1", &payload("pwd"), 1);
        assert_eq!(b.answered_in_pane("a1", Some("tu9")), 0, "a call hook can't close a request without an id");
        assert_eq!(b.answered_in_pane("a2", None), 0, "another agent");
        assert_eq!(b.answered_in_pane("a1", Some("tu1")), 1);
        let o = b.wait(&id, Instant::now() + Duration::from_secs(1), Duration::from_millis(10), || false);
        assert_eq!(o, Outcome::Answered(Answer::InPane));
        assert!(PermissionBroker::hook_output(&o).is_none());
        assert_eq!(b.answered_in_pane("a1", None), 1, "the turn's end closes it");
        let o = b.wait(&no_id, Instant::now() + Duration::from_secs(1), Duration::from_millis(10), || false);
        assert_eq!(o, Outcome::Answered(Answer::InPane));
    }

    #[test]
    fn eof_interrupts_and_deadline_falls_back_to_the_pane() {
        let b = PermissionBroker::default();
        let id = b.open("a1", &payload("rm -rf build"), 1);
        let o = b.wait(&id, Instant::now() + Duration::from_secs(5), Duration::from_millis(10), || true);
        assert_eq!(o, Outcome::Interrupted);
        let id = b.open("a1", &payload("ls"), 1);
        let t0 = Instant::now();
        let o = b.wait(&id, Instant::now() + Duration::from_millis(60), Duration::from_millis(10), || false);
        assert_eq!(o, Outcome::TimedOut);
        assert!(t0.elapsed() >= Duration::from_millis(60));
        assert!(PermissionBroker::hook_output(&o).is_none(), "no decision: the CLI shows its dialog");
        let deny = PermissionBroker::hook_output(&Outcome::Answered(Answer::Deny { message: None })).unwrap();
        assert_eq!(deny.0["hookSpecificOutput"]["decision"]["behavior"], "deny");
    }
}
