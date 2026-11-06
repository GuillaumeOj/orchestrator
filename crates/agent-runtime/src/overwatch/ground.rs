//! Grounding for `answer_question` (spec §3.5): a question is answered only
//! from a source the core can check — a file under the run's feature folder
//! (any file there, `decisions.md` included) or the project's `docs/`, read
//! on the integration branch's worktree, or `chat:<id>` of an earlier
//! decision the user answered.

use std::path::{Path, PathBuf};

use crate::runtime::Runtime;
use crate::store::AgentRecord;

impl Runtime {
    /// Checks one source cited for an answer to a question of `agent`;
    /// the source as shown on the line (`api-contract.md#3`), or why not.
    pub(crate) fn ground_source(&self, agent: Option<&AgentRecord>, source: &str) -> Result<String, String> {
        let source = source.trim();
        if let Some(rest) = source.strip_prefix("chat:") {
            return self.ground_chat(rest.trim());
        }
        let (path, section) = match source.split_once('#') {
            Some((p, s)) => (p.trim(), Some(s.trim()).filter(|s| !s.is_empty())),
            None => (source, None),
        };
        if path.is_empty() {
            return Err("an empty source".into());
        }
        let (worktree, feature_dir) = self.source_roots(agent);
        let Some(worktree) = worktree else {
            return Err(format!("{source}: no feature or project folder to read it from"));
        };
        // Relative to the worktree (`docs/x.md`, `.orchestrator/features/f/x.md`)
        // or to the feature folder (`api-contract.md`).
        let mut candidates = vec![worktree.join(path)];
        if let Some(f) = &feature_dir {
            candidates.push(f.join(path));
        }
        let roots: Vec<PathBuf> = feature_dir
            .into_iter()
            .chain(std::iter::once(worktree.join("docs")))
            .filter_map(|r| r.canonicalize().ok())
            .collect();
        for c in candidates {
            let Ok(real) = c.canonicalize() else { continue };
            if real.is_file() && roots.iter().any(|r| real.starts_with(r)) {
                let Some(sec) = section else { return Ok(path.to_string()) };
                let text = std::fs::read_to_string(&real).unwrap_or_default();
                return if has_section(&text, sec) {
                    Ok(format!("{path}#{sec}"))
                } else {
                    Err(format!("{source}: no section {sec:?} in {path}"))
                };
            }
        }
        Err(format!("{source}: not a file under the feature folder or the project's docs/"))
    }

    /// `(the integration worktree, the run's feature folder)` of an agent
    /// that asked; a solo session reads its project's main checkout.
    fn source_roots(&self, agent: Option<&AgentRecord>) -> (Option<PathBuf>, Option<PathBuf>) {
        let Some(a) = agent else { return (None, None) };
        let Some(seat) = &a.team else { return (Some(crate::paths::real(&a.repo)), None) };
        let feature = self.db.feature(&seat.feature_id).ok().flatten();
        let wt = feature.as_ref().and_then(|f| f.integration_worktree.clone());
        let dir = feature.map(|f| Path::new(".orchestrator/features").join(f.slug));
        match (wt, dir) {
            (Some(w), Some(d)) => (Some(w.clone()), Some(w.join(d))),
            _ => (None, None),
        }
    }

    /// `chat:c-12` (or `chat:12`): a decision entry whose question or gate
    /// the user answered.
    fn ground_chat(&self, id: &str) -> Result<String, String> {
        let n: u64 = id.trim_start_matches("c-").parse().map_err(|_| format!("chat:{id}: not an entry id"))?;
        let e = self.db.chat_get(n).ok().flatten().ok_or_else(|| format!("chat:{id}: no such entry"))?;
        let answered_by_user = e.line.as_ref().and_then(|l| l["by"].as_str()) == Some("you");
        let kind_ok = e.decision.as_deref().is_some_and(|d| d.starts_with("q-") || d.starts_with("gate-"));
        if answered_by_user && kind_ok {
            Ok(format!("chat:{}", e.id()))
        } else {
            Err(format!("chat:{id}: not a question or gate the user answered"))
        }
    }
}

/// Does a heading of `text` name `section`: by its anchor (`#cursor-paging`)
/// or by its number or leading words (`3.2`, `§3`, `Cursor paging`)?
fn has_section(text: &str, section: &str) -> bool {
    let slug = |h: &str| -> String {
        h.to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect()
    };
    let want = section.trim().trim_start_matches('§').trim().to_lowercase();
    text.lines().filter(|l| l.starts_with('#')).any(|l| {
        let h = l.trim_start_matches('#').trim().trim_start_matches('§').to_lowercase();
        slug(&h) == slug(&want)
            || h.strip_prefix(&want).is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
    })
}
