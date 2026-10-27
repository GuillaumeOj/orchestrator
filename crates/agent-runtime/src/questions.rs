//! Questions for the user always reach the Inbox. A team agent that asks in
//! its pane during an autonomous turn talks to nobody: Claude Code's own
//! `AskUserQuestion` becomes an Inbox question, and a turn that ends on a
//! plain-text question is sent back once to use `ask_user`, then forwarded.

use orch_protocol::{HookOutput, HookPayload, ToolDecision};
use serde_json::{Value, json};

use crate::inbox::PRIORITY_GATE;
use crate::runtime::Runtime;
use crate::state::TurnOrigin;
use crate::store::AgentRecord;
use crate::team::cut;

/// Phrases that end a turn on a question for the user, in its last paragraph.
const ASKING: &[&str] =
    &["decision for you", "do you want", "would you like", "should i ", "shall i ", "let me know", "your call"];

/// The last paragraph of an assistant message.
fn last_paragraph(text: &str) -> &str {
    let t = text.trim();
    t.rsplit("\n\n").find(|p| !p.trim().is_empty()).unwrap_or(t).trim()
}

/// Does this message end on a question for the user?
pub fn looks_like_question(text: &str) -> bool {
    let last = last_paragraph(text);
    if last.ends_with('?') {
        return true;
    }
    let lower = last.to_lowercase();
    ASKING.iter().any(|p| lower.contains(p))
}

/// The questions of an `AskUserQuestion` call as one text, and their options.
fn ask_user_question_text(input: &Value) -> (String, Vec<String>) {
    let qs = input.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut text = vec![];
    let mut options = vec![];
    for q in &qs {
        let s = |k: &str| q.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let opts: Vec<(String, Option<&str>)> = q
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|o| {
                let label = o.get("label").and_then(Value::as_str).unwrap_or("").to_string();
                (label, o.get("description").and_then(Value::as_str).filter(|d| !d.is_empty()))
            })
            .collect();
        let mut line = s("question");
        for (label, d) in &opts {
            match d {
                Some(d) => line.push_str(&format!("\n  - {label}: {d}")),
                None => line.push_str(&format!("\n  - {label}")),
            }
        }
        text.push(line);
        if qs.len() == 1 {
            // The labels as they are (a label may contain ": " itself).
            options = opts.into_iter().map(|(label, _)| label).collect();
        }
    }
    (text.join("\n\n"), options)
}

impl Runtime {
    fn question_item(&self, rec: &AgentRecord, question: &str, options: Vec<String>, via: &str) -> Option<u64> {
        let run = rec.team.as_ref().map(|s| s.run_id.clone());
        let id = self
            .add_item(
                Some(&rec.id),
                "question",
                format!("{}: {}", rec.id, cut(last_paragraph(question), 300)),
                json!({"action": "ask_user", "question": cut(question, 4_000), "options": options, "run": run, "via": via}),
                PRIORITY_GATE,
            )
            .ok()?;
        self.mark_asked(&rec.id);
        self.event(&rec.id, "asked_user", json!({"item": id, "via": via}));
        Some(id)
    }

    /// PreToolUse of `AskUserQuestion` in a team agent's autonomous turn: an
    /// Inbox question instead of a dialog nobody watches.
    pub(crate) fn forward_ask_user_question(&self, rec: &AgentRecord, p: &HookPayload) -> Option<HookOutput> {
        if rec.team.is_none()
            || p.tool_name.as_deref() != Some("AskUserQuestion")
            || rec.tracker.turn_origin == Some(TurnOrigin::User)
        {
            return None;
        }
        let (text, options) = ask_user_question_text(p.tool_input.as_ref().unwrap_or(&Value::Null));
        if text.trim().is_empty() {
            return None;
        }
        self.question_item(rec, &text, options, "AskUserQuestion")?;
        Some(HookOutput::pre_tool_use(
            ToolDecision::Deny,
            "ORCH: forwarded to the user through the Inbox; the answer arrives as a new turn. Continue with other work or end your turn.",
        ))
    }

    /// A team agent's Stop on a plain-text question it did not send with
    /// `ask_user`. In an autonomous turn it is sent back once to use the
    /// tool; after that, or in a turn the user started in the pane, the
    /// question goes to the Inbox as it is (the pane answer closes it).
    pub(crate) fn question_at_stop(&self, rec: &AgentRecord, p: &HookPayload) -> Option<HookOutput> {
        let text = p.last_assistant_message.as_deref().filter(|t| looks_like_question(t))?;
        if rec.team.is_none() || self.asked(&rec.id) {
            return None;
        }
        let users_turn = rec.tracker.turn_origin == Some(TurnOrigin::User);
        if !users_turn && p.stop_hook_active != Some(true) {
            return Some(HookOutput::stop_block(
                "ORCH: your turn ends on a question for the user, but nobody reads this pane. If it is for the user, call mcp__orch__ask_user with the question (and its options), then end your turn. If it is for your team, use mcp__orch__send_message. If it was not a question, just end your turn.",
            ));
        }
        self.question_item(rec, text, vec![], "pane");
        None
    }

    /// The user typed in the pane: questions the agent asked there are answered.
    pub(crate) fn close_pane_questions(&self, agent: &str) {
        let pane_question = |i: &crate::store::InboxItem| {
            i.agent_id.as_deref() == Some(agent)
                && i.kind == "question"
                && i.pending()
                && i.payload.get("via").and_then(Value::as_str) == Some("pane")
        };
        // Every user prompt passes here: the store is written only when one is open.
        if !self.store.read(|s| s.inbox.iter().any(pane_question)) {
            return;
        }
        let _ = self.store.write(|s| {
            let open: Vec<u64> = s.inbox.iter().filter(|i| pane_question(i)).map(|i| i.id).collect();
            for id in open {
                let _ = s.close_inbox(id, orch_store::InboxStatus::Decided, Some("answered in pane".into()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_at_the_end_of_a_turn() {
        assert!(looks_like_question("I merged T-1.\n\nDo you want that column, or no dollars at all?"));
        assert!(looks_like_question("Done.\n\n**Decision for you:** should the report show money."));
        assert!(looks_like_question("Shall I go on with T-3"));
        assert!(!looks_like_question("Why? Because the rules allow it.\n\nT-2 is merged and T-3 can start."));
        assert!(!looks_like_question("I'll review and merge each one as it reports done."));
        assert!(!looks_like_question(""));
    }

    #[test]
    fn ask_user_question_input_becomes_text_and_options() {
        let input = json!({"questions": [{"question": "Which db?", "header": "DB", "multiSelect": false,
            "options": [{"label": "SQLite", "description": "local"}, {"label": "Postgres"}]}]});
        let (text, options) = ask_user_question_text(&input);
        assert_eq!(text, "Which db?\n  - SQLite: local\n  - Postgres");
        assert_eq!(options, ["SQLite", "Postgres"]);
    }
}
