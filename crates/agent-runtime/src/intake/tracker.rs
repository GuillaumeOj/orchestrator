//! Improve intake B1: the tracker settings commands (`tracker_settings`,
//! `tracker_settings_set`, `tracker_preview`, `tracker_options`,
//! `tracker_notice_dismiss`; contract §1). The settings are the store's
//! [`TrackerSettings`]; the readers below them are still the flat intake
//! readers (B2 lands the tree readers), fed with the tracker's views.

use anyhow::Result;
use orch_store::{Project, Tracker, TrackerSettings};
use orch_tasks::intake::{self as src, SourceError};
use serde_json::{Value, json};

use super::{PREVIEW, bad_request};
use crate::runtime::Runtime;

/// Parses, normalizes and validates the settings the UI sends (field errors
/// as `{code: "bad_request", field, message}`, like the old intake source).
pub fn parse_settings(v: &Value) -> Result<TrackerSettings> {
    let s: TrackerSettings = serde_json::from_value(v.clone()).map_err(|e| bad_request("settings", e.to_string()))?;
    let s = s.normalized();
    s.validate().map_err(|e| bad_request(e.field, e.message))?;
    if let Tracker::Local { backlog_glob: Some(g) } = &s.tracker {
        src::compile_glob(g).map_err(|e| bad_request("backlog_glob", format!("the glob does not parse: {e}")))?;
    }
    Ok(s)
}

/// A preview row: the WorkNode shape of the tree (contract §2) as far as the
/// flat listing knows it, with an empty breadcrumb.
fn sample_node(p: &Project, kind: &str, repo: Option<&str>, f: &orch_tasks::intake::Fetched) -> Value {
    let (id, node_kind) = match kind {
        "github" => (format!("gh:{}{}", repo.unwrap_or(""), f.reference), "issue"),
        "linear" => (format!("lin:{}", f.reference), "issue"),
        _ => (format!("local:{}", f.reference), "local"),
    };
    json!({
        "id": id, "project": p.id, "kind": node_kind, "ref": f.reference, "title": f.title, "url": f.url,
        "parent": null, "depth": 1,
        "children": {"total": 0, "open": 0, "done": 0}, "team_tasks": 0,
        "upstream": {"state": "open", "state_type": null, "closed_reason": null, "issue_type": null,
                     "labels": f.labels, "assignees": [], "milestone": f.extra.get("milestone"),
                     "board_status": null, "blocked_by": [], "blocking": [], "linked_prs": [],
                     "deleted": false, "updated_at_ms": f.updated_at_ms},
        "app": {"state": "none"}, "rollup": null, "ready": true, "hidden": false, "writeback": null,
        "actions": {"team": "ok", "solo": "ok", "program": "ok", "hide": "ok"},
        "breadcrumb": [],
    })
}

impl Runtime {
    /// `tracker_settings`: the stored settings and the migration notice. The
    /// migrated GitHub `ready` default is checked against the repository
    /// once: an answer settles it (the label kept or the filter emptied), a
    /// failed check leaves it pending for the next read.
    pub fn tracker_settings(&self, project: &str) -> Result<Value> {
        let p = self.intake_project(project)?;
        if self.db.tracker_ready_pending(project)? {
            self.resolve_pending_ready(&p);
        }
        Ok(json!({
            "settings": self.db.tracker(project)?,
            "migration_notice": self.db.tracker_notice(project)?,
        }))
    }

    fn resolve_pending_ready(&self, p: &Project) {
        let Ok(t) = self.db.tracker(&p.id) else { return };
        let Tracker::Github { repo, .. } = &t.tracker else {
            // Not a GitHub tracker any more: nothing to ask.
            let _ = self.db.resolve_tracker_ready(&p.id, true);
            return;
        };
        let asked = self
            .intake_repo(p, repo.as_deref())
            .and_then(|r| src::github_options(self.gh_in(p).as_ref(), &r))
            .map(|o| {
                // GitHub matches label names without regard to case.
                o["labels"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|n| n.as_str().is_some_and(|n| n.eq_ignore_ascii_case("ready"))))
            });
        if let Ok(has) = asked
            && self.db.resolve_tracker_ready(&p.id, has).is_ok()
            && !has
        {
            self.intake_request(Some(&p.id));
        }
    }

    /// `tracker_settings_set`: validates, stores (resolving a `mismatch`
    /// notice), refreshes at once and returns what `tracker_settings` does.
    pub fn tracker_settings_set(&self, project: &str, settings: &Value) -> Result<Value> {
        let s = parse_settings(settings)?;
        self.tracker_commit(project, &s)
    }

    /// Stores validated settings (also the way the D8 flow of the project
    /// settings saves them).
    pub fn tracker_commit(&self, project: &str, s: &TrackerSettings) -> Result<Value> {
        let p = self.intake_project(project)?;
        self.db.set_tracker(&p.id, s)?;
        if !s.intake_source().is_none() {
            self.intake_refresh_now(&p.id);
        }
        self.tracker_settings(project)
    }

    /// `tracker_preview`: a dry run of `settings` (nothing stored) over the
    /// flat listing of today's readers: `roots`, `items` and `ready` count
    /// the listing until the tree readers land.
    pub fn tracker_preview(&self, project: &str, settings: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let s = parse_settings(settings)?;
        let source = s.intake_source();
        Ok(match self.intake_fetch(&p, &source) {
            Ok((l, _)) => {
                let repo = self.intake_repo_known(&p, &source);
                let sample: Vec<Value> = l
                    .items
                    .iter()
                    .take(PREVIEW)
                    .map(|f| sample_node(&p, s.tracker.kind(), repo.as_deref(), f))
                    .collect();
                let n = l.total.max(l.items.len());
                json!({"roots": n, "items": n, "ready": n, "sample": sample, "more": l.more(),
                       "partial": l.partial, "error": null})
            }
            Err(e) => json!({"roots": 0, "items": 0, "ready": 0, "sample": [], "more": 0, "partial": false,
                             "error": e.json()}),
        })
    }

    /// `tracker_options`: what the form offers for a (partial) tracker.
    pub fn tracker_options(&self, project: &str, tracker: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let t: Tracker = serde_json::from_value(tracker.clone()).map_err(|e| bad_request("kind", e.to_string()))?;
        let err = |e: SourceError| json!({"error": e.json()});
        Ok(match t {
            Tracker::Github { repo, .. } => {
                let repo = repo.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
                if let Some(r) = &repo
                    && !orch_store::valid_repo(r)
                {
                    return Err(bad_request("repo", "the repository must be owner/name"));
                }
                match self.intake_repo(&p, repo.as_deref()) {
                    Ok(r) => src::github_tracker_options(self.gh_in(&p).as_ref(), &r).unwrap_or_else(err),
                    Err(e) => err(e),
                }
            }
            Tracker::Linear { teams, .. } => {
                let gql = self.linear_transport();
                let keys: Vec<String> = teams.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                src::linear_tracker_options(gql.as_ref(), &self.intake.backoff(&p.id), orch_protocol::now_ms(), &keys)
                    .unwrap_or_else(err)
            }
            Tracker::Local { backlog_glob } => {
                let glob = backlog_glob.as_deref().map(str::trim).filter(|g| !g.is_empty());
                let Some(glob) = glob else { return Ok(json!({"matches": 0})) };
                parse_settings(
                    &json!({"tracker": {"kind": "local", "backlog_glob": glob}, "ready": {"kind": "local"}}),
                )?;
                match src::repo_docs_matches(&p.path, glob, &super::base_branch(&p)) {
                    Ok(n) => json!({"matches": n}),
                    Err(e) => json!({"matches": 0, "error": e.json()}),
                }
            }
        })
    }

    /// `tracker_notice_dismiss`.
    pub fn tracker_notice_dismiss(&self, project: &str) -> Result<Value> {
        self.intake_project(project)?;
        self.db.dismiss_tracker_notice(project)?;
        Ok(json!({}))
    }
}
