//! allow-multiple-prs: the user sets a feature's issues and PR mode
//! (`feature_set_prs`, spec §3.1), finds issues to add (`issue_suggest`),
//! and every run JSON carries its PRs (`prs_view`, api-contract §2).

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use governor::TurnKind;
use orch_store::{Feature, PrMode, TeamRun};
use orch_team::{RunState, TaskStatus};
use serde_json::{Value, json};

use crate::issues::{self, GateRef, coded};
use crate::runtime::Runtime;
use crate::team::RunCtx;

/// A PR mode as sent by the UI or orchd (`None` = unchanged).
pub fn pr_mode_arg(m: Option<&str>) -> Result<Option<PrMode>> {
    m.map(|m| {
        PrMode::parse(m).ok_or_else(|| coded("bad_request", format!("pr_mode is single or per_issue, not {m:?}"), None))
    })
    .transpose()
}

/// A `push_pr` or `push_pr_update` gate's payload.
pub(crate) fn is_pr_gate(p: &Value) -> bool {
    matches!(p.get("action").and_then(Value::as_str), Some("push_pr" | "push_pr_update"))
}

impl Runtime {
    /// A feature by id, else by slug in any project.
    pub fn feature_by_id_or_slug(&self, feature: &str) -> Result<Feature> {
        if let Some(f) = self.db.feature(feature)? {
            return Ok(f);
        }
        self.db
            .projects(false)?
            .iter()
            .find_map(|p| self.db.feature_by_slug(&p.id, feature).ok().flatten())
            .ok_or_else(|| anyhow!("no feature {feature}"))
    }

    /// Every run's PR gates (`pending_only`: still waiting for the user), in
    /// one pass over the inbox. A gate names its run; an older one is the
    /// run's through its agent.
    pub fn pr_gates(&self, pending_only: bool) -> HashMap<String, Vec<GateRef>> {
        self.store.read(|s| {
            let mut out: HashMap<String, Vec<GateRef>> = HashMap::new();
            let gates = s.inbox.iter().filter(|i| i.kind == "gate" && (!pending_only || i.pending()));
            for i in gates.filter(|i| is_pr_gate(&i.payload)) {
                let run = match i.payload.get("run").and_then(Value::as_str) {
                    Some(r) => Some(r.to_string()),
                    None => i
                        .agent_id
                        .as_ref()
                        .and_then(|a| s.agents.get(a))
                        .and_then(|a| a.team.as_ref())
                        .map(|t| t.run_id.clone()),
                };
                if let Some(run) = run {
                    out.entry(run).or_default().push(GateRef {
                        id: i.id,
                        action: i.payload["action"].as_str().unwrap_or_default().to_string(),
                        issue: i.payload.get("issue").and_then(Value::as_str).map(str::to_string),
                    });
                }
            }
            out
        })
    }

    /// One run's pending PR gates, as `prs_view` reads them.
    pub(crate) fn run_pr_gates(&self, run_id: &str) -> Vec<GateRef> {
        self.pr_gates(true).remove(run_id).unwrap_or_default()
    }

    /// The run has had a PR or a PR gate, in any state: its mode is chosen.
    fn run_had_pr(&self, run: &TeamRun) -> bool {
        run.pr_number.is_some()
            || run.pr_url.is_some()
            || !run.prs.is_empty()
            || self.pr_gates(false).contains_key(&run.id)
    }

    /// A run as the UI reads it: the stored run plus `prs_view` (`gates`:
    /// the run's pending PR gates).
    pub fn run_value(run: &TeamRun, feature: &Feature, gates: &[GateRef]) -> Value {
        let mut v = json!(run);
        v["prs_view"] = json!(issues::prs_view(run, feature, gates));
        v
    }

    /// [`Runtime::run_value`] of one run, reading its feature and gates.
    pub fn run_value_of(&self, run: &TeamRun) -> Value {
        match self.db.feature(&run.feature_id).ok().flatten() {
            Some(f) => Self::run_value(run, &f, &self.run_pr_gates(&run.id)),
            None => json!(run),
        }
    }

    /// `issue` (as an agent typed it) as one of the feature's issue refs.
    pub(crate) fn feature_issue(&self, cx: &RunCtx, text: &str) -> Result<String> {
        let repo = crate::github_issue::known_repo(&cx.project);
        let r = issues::parse_ref(text, repo.as_deref()).map_err(|_| anyhow!("{text:?} is not an issue ref"))?;
        if !cx.feature.issues.iter().any(|i| i.reference == r) {
            let list = cx.feature.issue_refs();
            return Err(anyhow!(
                "{r} is not an issue of this feature ({})",
                if list.is_empty() { "it has none".to_string() } else { list.join(", ") }
            ));
        }
        Ok(r)
    }

    /// `feature_set_prs` (api-contract §3): the user sets the feature's
    /// issues (the full new list) and/or its PR mode. Returns the feature.
    pub fn feature_set_prs(
        &self,
        feature: &str,
        issues_in: Option<Vec<String>>,
        mode: Option<PrMode>,
    ) -> Result<Value> {
        let f = self.feature_by_id_or_slug(feature)?;
        let project = self.db.project(&f.project_id)?.ok_or_else(|| anyhow!("no project {}", f.project_id))?;
        let run = self.db.runs(Some(&f.id), false)?.into_iter().next();
        if run.as_ref().is_some_and(|r| r.status == RunState::Done) {
            return Err(coded("finished", "the feature is done: its issues and PRs can no longer change", None));
        }
        let repo = crate::github_issue::known_repo(&project);
        let refs = match &issues_in {
            Some(list) => issues::normalize(list, repo.as_deref())?,
            None => f.issue_refs(),
        };
        let new_mode = mode.unwrap_or(f.pr_mode);
        let had_pr = run.as_ref().is_some_and(|r| self.run_had_pr(r));
        if new_mode != f.pr_mode && had_pr {
            return Err(coded("pr_busy", "the PR mode is set before the first PR: this feature already has one", None));
        }
        if new_mode != f.pr_mode || !had_pr {
            issues::check_mode(new_mode, refs.len())?;
        }
        // Once a PR exists the list may drop to one issue, never to none: a
        // per-issue feature with no issue could ask for no PR at all.
        if new_mode == PrMode::PerIssue && refs.is_empty() {
            return Err(coded("per_issue_needs_two", "one PR per issue keeps at least 1 issue", None));
        }
        // An issue with a PR pending, open or merged stays.
        if let Some(r) = &run
            && f.pr_mode == PrMode::PerIssue
        {
            let view = issues::prs_view(r, &f, &self.run_pr_gates(&r.id));
            for gone in f.issue_refs().iter().filter(|i| !refs.contains(i)) {
                let held = view.iter().find(|v| v.issue.as_deref() == Some(gone.as_str()));
                if let Some(v) = held.filter(|v| matches!(v.state.as_str(), "pending" | "open" | "merged")) {
                    let what = v.number.map(|n| format!("PR #{n}")).unwrap_or_else(|| "a PR waiting".into());
                    return Err(coded("issue_has_pr", format!("{gone} has {what}: it cannot be removed"), Some(gone)));
                }
            }
        }
        let rows = self.db.intake_items(Some(&project.id)).unwrap_or_default();
        // Known issues keep what was stored about them; new ones read the cache.
        let links = issues::links(&refs, &rows, repo.as_deref())
            .into_iter()
            .map(|l| match f.issues.iter().find(|o| o.reference == l.reference) {
                Some(o) => orch_store::IssueLink {
                    title: l.title.or_else(|| o.title.clone()),
                    url: l.url.or_else(|| o.url.clone()),
                    ..l
                },
                None => l,
            })
            .collect::<Vec<_>>();
        let changed =
            links.iter().map(|l| &l.reference).ne(f.issues.iter().map(|l| &l.reference)) || new_mode != f.pr_mode;
        let nf = Feature { issues: links, pr_mode: new_mode, ..f.clone() };
        self.db.upsert_feature(&nf)?;
        if let Some(r) = run.filter(|r| changed && !r.status.finished()) {
            self.run_event(
                &r.id,
                None,
                "feature.prs",
                json!({"issues": nf.issue_refs(), "pr_mode": nf.pr_mode, "by": "user"}),
            );
            self.tell_lead_prs(&r, &nf, f.pr_mode);
            self.reevaluate_run_prs(&r.id, None)?;
        }
        Ok(json!(nf))
    }

    /// The lead's one turn about a change of the issues or the mode (spec
    /// §3.7); a switch to `per_issue` lists the merged tasks to tag.
    fn tell_lead_prs(&self, run: &TeamRun, f: &Feature, was: PrMode) {
        let Ok(cx) = self.run_ctx(&run.id) else { return };
        let Some(lead) = self.lead_of(&cx) else { return };
        let per_issue = f.pr_mode == PrMode::PerIssue;
        let mut text = format!(
            "ORCH: the user changed the pull requests of this feature. Pull requests: {}",
            orch_team::prs_line(&f.issue_refs(), per_issue)
        );
        if per_issue && was != PrMode::PerIssue {
            let untagged: Vec<String> = self
                .db
                .tasks(None, None, Some(&run.id))
                .unwrap_or_default()
                .iter()
                .filter(|t| t.status == TaskStatus::Done && t.issue.is_none())
                .map(|t| format!("{} (id {}, {})", t.handle(), t.id, t.title))
                .collect();
            if !untagged.is_empty() {
                text.push_str(&format!(
                    "\nMerged tasks with no issue yet: {}. Tag each with mcp__orch__update_task {{id, issue}}; \
                     untagged work goes into every PR.",
                    untagged.join("; ")
                ));
            }
        }
        let _ = self.deliver_kind(&lead.id, &text, TurnKind::Continue);
    }

    /// Re-evaluates the run's state after its issues or mode changed (spec
    /// §3.5). The per-issue rules (every issue PR merged → done, no PR open
    /// → integrating) come with the per-issue PR watcher; until then a
    /// change leaves the state as it is.
    /// `issue_suggest` (api-contract §3): the project's intake issues whose
    /// ref or title matches `text`, at most 8. No network.
    pub fn issue_suggest(&self, project: &str, text: &str) -> Result<Value> {
        let p = self.db.project(project)?.ok_or_else(|| anyhow!("no project {project}"))?;
        let rows = self.db.intake_items(Some(&p.id))?;
        let out: Vec<Value> = issues::suggest(text, &rows, crate::github_issue::known_repo(&p).as_deref())
            .into_iter()
            .map(|l| json!({"ref": l.reference, "key": l.key, "title": l.title, "url": l.url}))
            .collect();
        Ok(json!(out))
    }
}

#[cfg(test)]
#[path = "feature_prs_tests.rs"]
mod tests;
