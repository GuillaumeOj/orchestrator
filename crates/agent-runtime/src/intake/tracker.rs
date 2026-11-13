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

/// The contract's `upstream` of a node (§2) from its cached meta.
pub(crate) fn upstream_json(meta: &Value, updated_at_ms: u64) -> Value {
    let k = |key: &str| meta.get(key).cloned().unwrap_or(Value::Null);
    let list = |key: &str| meta.get(key).filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([]));
    json!({
        "state": meta["state"].as_str().unwrap_or("open"), "state_type": k("state_type"),
        "closed_reason": k("closed_reason"), "issue_type": k("issue_type"),
        "labels": list("labels"), "assignees": list("assignees"), "milestone": k("milestone"), "cycle": k("cycle"),
        "priority": k("priority"), "estimate": k("estimate"), "target_date": k("target_date"), "due_on": k("due_on"),
        "board_status": k("board_status"), "blocked_by": list("blocked_by"), "blocking": list("blocking"),
        "linked_prs": list("linked_prs"), "deleted": false, "updated_at_ms": updated_at_ms,
    })
}

/// The first rules of the row actions (B3 replaces them with notes §5.1):
/// the team needs a team, a program needs children.
pub(crate) fn actions_json(can_team: bool, has_children: bool) -> Value {
    json!({
        "team": if can_team { json!("ok") } else { json!({"disabled": "This project has no team yet"}) },
        "solo": "ok",
        "program": if has_children { json!("ok") } else { json!({"disabled": "No sub-issues to run as a program"}) },
        "hide": "ok",
    })
}

/// What a preview of a tree read shows.
struct Preview<'a> {
    tree: &'a src::Tree,
    ready: Vec<usize>,
    /// Node id → index, and each node's cached facts, read once.
    by_id: std::collections::HashMap<&'a str, usize>,
    metas: Vec<Value>,
}

impl<'a> Preview<'a> {
    fn new(t: &TrackerSettings, tree: &'a src::Tree) -> Preview<'a> {
        let metas: Vec<Value> = tree.nodes.iter().map(|n| n.upsert().meta).collect();
        let ready = (0..tree.nodes.len()).filter(|i| t.is_ready(tree.nodes[*i].kind, &metas[*i])).collect();
        let by_id = tree.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
        Preview { tree, ready, by_id, metas }
    }

    fn roots(&self) -> usize {
        self.tree.nodes.iter().filter(|n| n.parent.as_deref().is_none_or(|p| !self.by_id.contains_key(p))).count()
    }

    fn items(&self) -> usize {
        self.tree.nodes.iter().filter(|n| matches!(n.kind, "issue" | "local")).count()
    }

    /// The ancestors of node `i`, root first.
    fn breadcrumb(&self, i: usize) -> Vec<Value> {
        let (mut chain, mut at) = (vec![], i);
        while let Some(p) = self.tree.nodes[at].parent.as_deref().and_then(|p| self.by_id.get(p)).copied() {
            if chain.len() > 50 {
                break;
            }
            chain.push(json!({"id": self.tree.nodes[p].id, "ref": self.tree.nodes[p].f.reference,
                              "title": self.tree.nodes[p].f.title}));
            at = p;
        }
        chain.reverse();
        chain
    }

    /// A preview row: the WorkNode shape of the tree (contract §2) as far as a read knows it.
    fn sample(&self, p: &Project, i: usize, can_team: bool) -> Value {
        let n = &self.tree.nodes[i];
        let meta = &self.metas[i];
        let kids: Vec<&src::Node> = self.tree.nodes.iter().filter(|c| c.parent.as_deref() == Some(&n.id)).collect();
        let open = kids
            .iter()
            .filter(|c| self.metas[self.by_id[c.id.as_str()]]["state"].as_str().unwrap_or("open") == "open")
            .count();
        let breadcrumb = self.breadcrumb(i);
        json!({
            "id": n.id, "project": p.id, "kind": n.kind, "ref": n.f.reference, "title": n.f.title, "url": n.f.url,
            "parent": n.parent, "depth": breadcrumb.len() + 1,
            "children": {"total": kids.len(), "open": open, "done": kids.len() - open},
            "team_tasks": meta["team_tasks"].as_u64().unwrap_or(0),
            "upstream": upstream_json(meta, n.f.updated_at_ms),
            "app": {"state": "none"}, "rollup": null, "ready": true, "hidden": false, "writeback": null,
            "actions": actions_json(can_team, !kids.is_empty()),
            "breadcrumb": breadcrumb,
        })
    }
}

impl Runtime {
    /// `tracker_settings`: the stored settings and the migration notice (no
    /// network: the migrated GitHub `ready` default is settled by the
    /// refresher, [`Runtime::resolve_pending_ready`]).
    pub fn tracker_settings(&self, project: &str) -> Result<Value> {
        self.intake_project(project)?;
        Ok(json!({
            "settings": self.db.tracker(project)?,
            "migration_notice": self.db.tracker_notice(project)?,
        }))
    }

    /// The migrated GitHub default (label `ready`) is asked of the repository
    /// once: an answer settles it (the label kept or the filter emptied), a
    /// failed check leaves it pending for the next refresh.
    pub(super) fn resolve_pending_ready(&self, p: &Project) {
        let Ok(t) = self.db.tracker(&p.id) else { return };
        let Tracker::Github { repo, .. } = &t.tracker else {
            // Not a GitHub tracker any more: nothing to ask.
            let _ = self.db.resolve_tracker_ready(&p.id, true);
            return;
        };
        // Asked by name (GitHub matches it without regard to case), not by reading the first 100 labels.
        let asked = self
            .intake_repo(p, repo.as_deref())
            .and_then(|r| src::github_has_label(self.gh_in(p).as_ref(), &r, "ready"));
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
        // The tree is cached whatever the Ready filter names.
        if super::readable(s) {
            self.intake_refresh_now(&p.id);
        }
        self.tracker_settings(project)
    }

    /// `tracker_preview`: a dry run of `settings` (nothing stored) through
    /// the tree readers: how many roots, items and Ready items the tracker
    /// has, a sample of the Ready ones with their breadcrumb, the `gh_sub_issues`
    /// `warning` and the source `error` (the backlog file names for Local).
    pub fn tracker_preview(&self, project: &str, settings: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let s = parse_settings(settings)?;
        let files = match &s.tracker {
            Tracker::Local { backlog_glob: Some(g) } => {
                src::repo_docs_paths(&p.path, g, &super::base_branch(&p)).unwrap_or_default()
            }
            _ => vec![],
        };
        Ok(match self.tree_fetch(&p, &s, None) {
            Ok((tree, _)) => {
                let v = Preview::new(&s, &tree);
                let can_team = self.project_facts(&p).can_team;
                let sample: Vec<Value> = v.ready.iter().take(PREVIEW).map(|i| v.sample(&p, *i, can_team)).collect();
                json!({"roots": v.roots(), "items": v.items(), "ready": v.ready.len(), "sample": sample,
                       "partial": tree.partial, "warning": tree.warning, "error": null, "files": files})
            }
            Err(e) => json!({"roots": 0, "items": 0, "ready": 0, "sample": [], "partial": false,
                             "warning": null, "error": e.json(), "files": files}),
        })
    }

    /// `intake_preview` (the old command): the Ready items `source` would list.
    pub(super) fn preview_source(&self, p: &Project, s: &TrackerSettings) -> Value {
        let facts = self.project_facts(p);
        let est = self.intake_est();
        match self.tree_fetch(p, s, None) {
            Ok((tree, _)) => {
                let v = Preview::new(s, &tree);
                let kind = super::tracker_source(s);
                let items: Vec<Value> = v
                    .ready
                    .iter()
                    .take(PREVIEW)
                    .map(|i| {
                        let u = tree.nodes[*i].upsert();
                        super::item_json(&super::ItemFields::of_upsert(&u, &p.id, kind), &facts, &est)
                    })
                    .collect();
                let n = v.ready.len();
                let shown = n.min(src::CAP);
                json!({"count": shown, "more": n - shown, "partial": tree.partial, "items": items, "error": null})
            }
            Err(e) => json!({"count": 0, "more": 0, "partial": false, "items": [], "error": e.json()}),
        }
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
                let Some(glob) = glob else { return Ok(json!({"matches": 0, "files": []})) };
                parse_settings(
                    &json!({"tracker": {"kind": "local", "backlog_glob": glob}, "ready": {"kind": "local"}}),
                )?;
                match src::repo_docs_paths(&p.path, glob, &super::base_branch(&p)) {
                    Ok(files) => json!({"matches": files.len(), "files": files}),
                    Err(e) => json!({"matches": 0, "files": [], "error": e.json()}),
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
