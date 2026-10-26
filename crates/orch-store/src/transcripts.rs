//! FTS5 index over Claude Code transcripts (FR-209, ADR-005). The JSONL files
//! stay the source of truth: the index holds the searchable text of user and
//! assistant messages plus a pointer back (path, entry uuid). Files are read
//! incrementally from the last indexed byte; only complete lines are indexed.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Store;

/// A transcript to index: its file, its session and the project it belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub session_id: String,
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexStats {
    pub files: usize,
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub session_id: String,
    pub project_id: Option<String>,
    pub role: String,
    pub ts: Option<String>,
    pub uuid: Option<String>,
    pub path: PathBuf,
    /// The match in context, terms wrapped in `[` `]`.
    pub snippet: String,
}

/// Never index more than this per file per pass (memory bound on 8 GB).
const MAX_CHUNK: u64 = 16 * 1024 * 1024;
/// Text kept per entry.
const MAX_ENTRY_TEXT: usize = 8 * 1024;

/// Claude Code's folder name for a project path under `~/.claude/projects`
/// (`/` and `.` become `-`).
pub fn encode_project_dir(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c == '/' || c == '.' { '-' } else { c }).collect()
}

/// The searchable text of one transcript entry: user text, assistant text,
/// and tool calls as `tool: <name> <command|file_path|…>`. Tool results and
/// meta entries are skipped.
pub fn extract_text(entry: &Value) -> Option<(String, String)> {
    let t = entry.get("type").and_then(Value::as_str)?;
    if t != "user" && t != "assistant" {
        return None;
    }
    if entry.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
    let mut out = String::new();
    match content {
        Value::String(s) => out.push_str(s),
        Value::Array(blocks) => {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(s) = b.get("text").and_then(Value::as_str) {
                            out.push_str(s);
                            out.push('\n');
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                        let input = b.get("input");
                        let arg = ["command", "file_path", "pattern", "url", "description"]
                            .iter()
                            .find_map(|k| input.and_then(|i| i.get(*k)).and_then(Value::as_str))
                            .unwrap_or("");
                        out.push_str(&format!("tool: {name} {arg}\n"));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let out = out.trim();
    if out.is_empty() {
        return None;
    }
    let mut text: String = out.chars().take(MAX_ENTRY_TEXT).collect();
    if text.len() < out.len() {
        text.push('…');
    }
    Some((t.to_string(), text))
}

/// Turns user input into an FTS5 query: every whitespace-separated term is
/// quoted (all terms must match), so `-`, `:` or `*` are never operators.
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> =
        q.split_whitespace().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect();
    if terms.is_empty() { None } else { Some(terms.join(" ")) }
}

impl Store {
    /// Indexes what was appended to each file since the last pass. A file
    /// that shrank (rewritten) is re-indexed from the start.
    pub fn index_transcripts(&self, files: &[TranscriptFile]) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        for f in files {
            let Ok(meta) = std::fs::metadata(&f.path) else { continue };
            let size = meta.len();
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let key = f.path.to_string_lossy().into_owned();
            let k2 = key.clone();
            let prev: Option<(i64, i64)> = self.read(move |c| {
                Ok(c.query_row("SELECT offset, size FROM transcript_files WHERE path=?1", [k2], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?)
            })?;
            let mut offset = prev.map(|p| p.0 as u64).unwrap_or(0);
            let restart = offset > size;
            if restart {
                offset = 0;
            }
            if prev.is_some() && offset == size && !restart {
                continue;
            }
            let mut file = std::fs::File::open(&f.path)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut buf = Vec::new();
            file.take(MAX_CHUNK).read_to_end(&mut buf)?;
            // Only complete lines; a partial last line is read next time.
            let end = buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
            let mut rows: Vec<(String, String, Option<String>, Option<String>)> = vec![];
            for line in buf[..end].split(|b| *b == b'\n') {
                let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
                if let Some((role, text)) = extract_text(&v) {
                    let ts = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
                    let uuid = v.get("uuid").and_then(Value::as_str).map(str::to_string);
                    rows.push((role, text, ts, uuid));
                }
            }
            let new_offset = offset + end as u64;
            stats.files += 1;
            stats.entries += rows.len();
            stats.bytes += end as u64;
            let f = f.clone();
            self.write(move |tx| {
                if restart {
                    tx.execute("DELETE FROM transcript_fts WHERE path=?1", [&key])?;
                }
                {
                    let mut ins = tx.prepare_cached(
                        "INSERT INTO transcript_fts(body, session_id, project_id, role, ts, uuid, path) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    )?;
                    for (role, text, ts, uuid) in &rows {
                        ins.execute(params![text, f.session_id, f.project_id, role, ts, uuid, key])?;
                    }
                }
                tx.execute(
                    "INSERT INTO transcript_files(path, session_id, project_id, offset, size, mtime_ms, indexed_at_ms)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)
                     ON CONFLICT(path) DO UPDATE SET session_id=excluded.session_id, project_id=excluded.project_id,
                       offset=excluded.offset, size=excluded.size, mtime_ms=excluded.mtime_ms, indexed_at_ms=excluded.indexed_at_ms",
                    params![
                        key,
                        f.session_id,
                        f.project_id,
                        new_offset as i64,
                        size as i64,
                        mtime_ms,
                        crate::now_ms() as i64
                    ],
                )?;
                Ok(())
            })?;
        }
        Ok(stats)
    }

    /// Full-text search, best matches first; `project_id` limits it to one project.
    pub fn search_transcripts(&self, query: &str, project_id: Option<&str>, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let project_id = project_id.map(str::to_string);
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT session_id, project_id, role, ts, uuid, path,
                        snippet(transcript_fts, 0, '[', ']', '…', 16)
                 FROM transcript_fts WHERE transcript_fts MATCH ?1 AND (?2 IS NULL OR project_id = ?2)
                 ORDER BY bm25(transcript_fts) LIMIT ?3",
            )?;
            Ok(st
                .query_map(params![q, project_id, limit as i64], |r| {
                    Ok(SearchHit {
                        session_id: r.get(0)?,
                        project_id: r.get(1)?,
                        role: r.get(2)?,
                        ts: r.get(3)?,
                        uuid: r.get(4)?,
                        path: PathBuf::from(r.get::<_, String>(5)?),
                        snippet: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn indexes_incrementally_and_searches() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let path = d.path().join("-x-demo/sid.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lines = [
            r#"{"type":"user","uuid":"u1","timestamp":"2026-10-26T10:00:00Z","message":{"role":"user","content":"Fix the flaky checkout-service test"}}"#,
            r#"{"type":"assistant","uuid":"u2","message":{"content":[{"type":"text","text":"Looking at the retry logic."},{"type":"tool_use","name":"Bash","input":{"command":"cargo test -p checkout"}}]}}"#,
            r#"{"type":"user","uuid":"u3","message":{"content":[{"type":"tool_result","tool_use_id":"x","content":"secret tool output"}]}}"#,
        ];
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        let tf = TranscriptFile { path: path.clone(), session_id: "sid".into(), project_id: Some("demo".into()) };
        let st = s.index_transcripts(std::slice::from_ref(&tf)).unwrap();
        assert_eq!(st.entries, 2, "tool results are not indexed");
        let hits = s.search_transcripts("flaky checkout-service", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].uuid.as_deref(), Some("u1"));
        assert!(hits[0].snippet.contains("[flaky]"), "{}", hits[0].snippet);
        assert_eq!(s.search_transcripts("cargo", Some("demo"), 10).unwrap()[0].role, "assistant");
        assert!(s.search_transcripts("cargo", Some("other"), 10).unwrap().is_empty());
        assert!(s.search_transcripts("secret", None, 10).unwrap().is_empty());
        assert!(s.search_transcripts("  ", None, 10).unwrap().is_empty());
        // Nothing new: nothing read. A partial line waits for its newline.
        assert_eq!(s.index_transcripts(std::slice::from_ref(&tf)).unwrap().files, 0);
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, r#"{{"type":"user","uuid":"u4","message":{{"content":"zebra crossing"}}}}"#).unwrap();
        s.index_transcripts(std::slice::from_ref(&tf)).unwrap();
        assert!(s.search_transcripts("zebra", None, 10).unwrap().is_empty());
        writeln!(f).unwrap();
        s.index_transcripts(std::slice::from_ref(&tf)).unwrap();
        assert_eq!(s.search_transcripts("zebra", None, 10).unwrap().len(), 1);
        // Rewritten shorter: re-indexed from scratch, no duplicates.
        std::fs::write(&path, format!("{}\n", lines[0])).unwrap();
        s.index_transcripts(std::slice::from_ref(&tf)).unwrap();
        assert_eq!(s.search_transcripts("flaky", None, 10).unwrap().len(), 1);
        assert!(s.search_transcripts("zebra", None, 10).unwrap().is_empty());
    }

    #[test]
    fn project_dir_encoding() {
        assert_eq!(
            encode_project_dir(Path::new("/Users/g/dev/orchestrator/.claude/worktrees/epic")),
            "-Users-g-dev-orchestrator--claude-worktrees-epic"
        );
    }
}
