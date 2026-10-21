//! Claude Code transcripts, found **by session id** across
//! `~/.claude/projects/*/` (a resumed session keeps writing in its original
//! project folder after a repo move). Read only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::delivery::find_markers;

/// `~/.claude/projects`.
pub fn projects_root(home: &Path) -> PathBuf {
    home.join(".claude/projects")
}

pub fn find(projects_root: &Path, session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    std::fs::read_dir(projects_root).ok()?.flatten().map(|e| e.path().join(&name)).find(|p| p.is_file())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub found: bool,
    pub path: Option<PathBuf>,
    pub entries: usize,
    /// `user` or `assistant`: who wrote last.
    pub last_type: Option<String>,
    pub last_stop_reason: Option<String>,
    /// tool_use ids (with tool name) that have no tool_result.
    pub pending_tool_uses: BTreeMap<String, String>,
    /// Turn markers seen in user messages, tool results and queue operations.
    pub markers: BTreeSet<String>,
}

pub fn summarize(path: &Path) -> Summary {
    let mut s = Summary { path: Some(path.to_path_buf()), ..Default::default() };
    let Ok(text) = std::fs::read_to_string(path) else { return s };
    s.found = true;
    let mut uses = BTreeMap::new();
    let mut results = BTreeSet::new();
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
        s.entries += 1;
        let t = e.get("type").and_then(Value::as_str).unwrap_or("");
        // Markers only from what was sent to the agent, never from its own words.
        if t == "user" || t == "queue-operation" {
            s.markers.extend(find_markers(line));
        }
        if t != "user" && t != "assistant" {
            continue;
        }
        s.last_type = Some(t.to_string());
        let msg = e.get("message");
        if t == "assistant" {
            s.last_stop_reason = msg.and_then(|m| m.get("stop_reason")).and_then(Value::as_str).map(str::to_string);
        }
        if let Some(Value::Array(blocks)) = msg.and_then(|m| m.get("content")) {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        if let Some(id) = b.get("id").and_then(Value::as_str) {
                            let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                            uses.insert(id.to_string(), name.to_string());
                        }
                    }
                    Some("tool_result") => {
                        if let Some(id) = b.get("tool_use_id").and_then(Value::as_str) {
                            results.insert(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    s.pending_tool_uses = uses.into_iter().filter(|(k, _)| !results.contains(k)).collect();
    s
}

pub fn summarize_session(projects_root: &Path, session_id: &str) -> Summary {
    match find(projects_root, session_id) {
        Some(p) => summarize(&p),
        None => Summary::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_session_id_in_any_project_folder_and_summarizes() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join("-old-slug");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(d.path().join("-other")).unwrap();
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"[t:0123abcd] run the tests"}}"#,
            r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"ok"}]}}"#,
            r#"{"type":"queue-operation","operation":"enqueue","content":"[t:0000beef] also"}"#,
            r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"text","text":"I saw [t:ffffffff]"},{"type":"tool_use","id":"tu2","name":"Bash","input":{}}]}}"#,
            "not json",
        ];
        std::fs::write(old.join("sid-1.jsonl"), lines.join("\n")).unwrap();
        let s = summarize_session(d.path(), "sid-1");
        assert!(s.found);
        assert_eq!(s.entries, 5);
        assert_eq!(s.last_type.as_deref(), Some("assistant"));
        assert_eq!(s.pending_tool_uses.keys().collect::<Vec<_>>(), ["tu2"]);
        assert_eq!(s.markers.iter().collect::<Vec<_>>(), ["0000beef", "0123abcd"], "assistant text is not a delivery");
        assert!(!summarize_session(d.path(), "nope").found);
    }
}
