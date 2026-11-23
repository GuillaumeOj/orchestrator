//! allow-launch-team-on-update-pr §5.3: a feature launched on someone else's
//! pull request. Its integration branch is the PR's head branch, checked out
//! from `origin/<head>`; the launch re-reads the PR (G2) and refuses, before
//! anything is stored, what would make the team work on the wrong commits.

use std::path::Path;

use anyhow::{Result, anyhow};
use orch_git::gh::{self, PrHead};
use orch_git::{cleanup, worktree};
use orch_store::{Feature, PrLink, Project};
use orch_team::docs_location::{DesignsChoice, DocsChoice};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// A command error whose text is the JSON `{code, message, details?, feature?}`
/// (api-contract §3, F10–F11).
pub(crate) fn refused(code: &str, message: impl Into<String>, extra: Value) -> anyhow::Error {
    let mut v = json!({"code": code, "message": message.into()});
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    anyhow!(v.to_string())
}

/// The code of a failed `gh` call (api-contract §2–3): `gh_missing` (not
/// installed), `gh_auth` (not logged in), else `fallback`.
pub fn gh_error_code(e: &anyhow::Error, fallback: &'static str) -> &'static str {
    if e.chain().any(|c| c.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound))
    {
        return "gh_missing";
    }
    let m = format!("{e:#}").to_ascii_lowercase();
    if ["gh auth login", "not logged in", "authentication", "http 401", "bad credentials"].iter().any(|k| m.contains(k))
    {
        "gh_auth"
    } else {
        fallback
    }
}

/// `{id, slug, run_status}` of a feature (F8, F10); `run_status` null without a run.
pub fn feature_ref(f: &Feature, run: Option<&orch_store::TeamRun>) -> Value {
    json!({"id": f.id, "slug": f.slug, "run_status": run.map(crate::team::effective)})
}

/// The feature in JSON with `branch` always the real integration branch
/// (api-contract §4: present for unlinked features too).
pub fn feature_json(f: &Feature) -> Value {
    let mut v = json!(f);
    v["branch"] = json!(f.integration_branch());
    v
}

/// What a launch on a PR checked: the PR as GitHub has it now.
pub(crate) struct PrLaunch {
    pub head: PrHead,
}

impl PrLaunch {
    /// The feature fields a PR launch sets: the head as the integration
    /// branch, the PR base, the link, and docs and designs in the app folder
    /// whatever the project setting (D1/G1), so none enters the PR.
    pub fn apply(&self, f: Feature, sent: &PrLink, app_base: &str) -> Feature {
        let h = &self.head;
        let docs = orch_team::FeatureDocsSetting { docs: DocsChoice::App, designs: DesignsChoice::App };
        Feature {
            branch: Some(h.head_ref.clone()),
            base: h.base_ref.clone(),
            docs_location: Some(docs.resolve(&f.slug, app_base)),
            pr_link: Some(PrLink {
                number: h.number,
                url: if h.url.is_empty() { sent.url.clone() } else { h.url.clone() },
                head_ref: h.head_ref.clone(),
                base_ref: h.base_ref.clone(),
                head_sha_at_link: h.head_sha.clone(),
                author: h.author.clone().or_else(|| sent.author.clone()),
            }),
            ..f
        }
    }
}

impl Runtime {
    /// A linked feature whose PR the watcher saw merged (its latest run).
    pub(crate) fn linked_pr_merged(&self, f: &Feature) -> bool {
        f.is_linked()
            && self
                .db
                .runs(Some(&f.id), false)
                .ok()
                .and_then(|r| r.into_iter().next())
                .is_some_and(|r| r.pr.as_ref().is_some_and(|p| p.state == "merged"))
    }

    /// D2: a PR head branch kept for commits `origin/<head>` lacks goes
    /// anyway when its linked PR is merged (a merge usually deletes the head
    /// on GitHub). Never on the remote.
    pub(crate) fn drop_merged_pr_branch(&self, project: &Project, out: &mut cleanup::RemoveOutcome) {
        let Some(b) = out.kept_branch.clone().filter(|_| out.kept_unpushed.is_some()) else { return };
        let features = self.db.features(Some(&project.id)).unwrap_or_default();
        if !features.iter().any(|f| f.integration_branch() == b && self.linked_pr_merged(f)) {
            return;
        }
        match orch_git::git(&project.path, &["branch", "-D", &b]) {
            Ok(_) => {
                out.log.push(format!("deleted branch {b}: its PR is merged"));
                (out.branch_deleted, out.kept_branch, out.kept_unpushed) = (true, None, None);
            }
            Err(e) => out.log.push(format!("kept branch {b}: {e}")),
        }
    }

    /// `gh pr view <n>` for a launch; tests replace `gh` through `cfg.gh`.
    pub(crate) fn gh_pr_head(&self, dir: &Path, number: u64) -> Result<PrHead> {
        match self.gh_override() {
            Some(g) => gh::parse_pr_head(&g.run(&gh::pr_head_args(number))?),
            None => gh::pr_head(dir, number),
        }
    }

    /// The feature of the project linked to PR `number` with an active run,
    /// else the newest one linked to it, with its latest run (archived
    /// features left out). Several may be linked (finished ones are kept).
    pub(crate) fn feature_on_pr(
        &self,
        project_id: &str,
        number: u64,
    ) -> Result<Option<(Feature, Option<orch_store::TeamRun>)>> {
        let mut first = None;
        for f in self.db.features(Some(project_id))? {
            if !f.pr_link.as_ref().is_some_and(|l| l.number == number) {
                continue;
            }
            let run = self.db.runs(Some(&f.id), false)?.into_iter().next();
            if run.as_ref().is_some_and(|r| !crate::team::effective(r).finished()) {
                return Ok(Some((f, run)));
            }
            first.get_or_insert((f, run));
        }
        Ok(first)
    }

    /// Step 1 of §5.3: the refusals that need no fetch, then the PR re-read
    /// (G2). Nothing is stored or created.
    pub(crate) fn check_pr_launch(&self, project: &Project, link: &PrLink, per_issue: bool) -> Result<PrLaunch> {
        let n = link.number;
        if n == 0 {
            return Err(refused("bad_request", "no PR number", json!({})));
        }
        if per_issue {
            return Err(refused(
                "per_issue",
                format!("a feature on PR #{n} pushes to that PR only: one PR per issue is not available"),
                json!({}),
            ));
        }
        // D3: one active feature per PR (no run at all counts as not active).
        if let Some((f, run)) = self.feature_on_pr(&project.id, n)?
            && run.as_ref().is_some_and(|r| !crate::team::effective(r).finished())
        {
            return Err(refused(
                "active_feature",
                format!("PR #{n} already has a team: feature {}", f.slug),
                json!({"feature": feature_ref(&f, run.as_ref())}),
            ));
        }
        if !self.cfg.conditions.online() {
            return Err(refused("offline", format!("offline: PR #{n} can't be read"), json!({})));
        }
        let head = self.gh_pr_head(&project.path, n).map_err(|e| {
            refused(gh_error_code(&e, "fetch_failed"), format!("could not read PR #{n}: {e:#}"), json!({}))
        })?;
        if head.cross_repo {
            return Err(refused("fork", "Fork PR: the app pushes to origin only", json!({})));
        }
        match head.state.as_str() {
            "open" => {}
            "merged" => return Err(refused("pr_merged", format!("PR #{n} is merged"), json!({}))),
            _ => return Err(refused("pr_closed", format!("PR #{n} is closed"), json!({}))),
        }
        let b = &head.head_ref;
        if !orch_git::valid_branch(b) || !orch_git::valid_branch(&head.base_ref) {
            return Err(refused("bad_request", format!("PR #{n}: invalid branch {b:?}"), json!({})));
        }
        // The app's own branches have their own feature (and the worktree
        // layout cuts a missing `orch/*` branch from the base, G5).
        if b.starts_with("orch/") {
            return Err(refused(
                "bad_request",
                format!("PR #{n} is on the app's branch {b}: open its feature"),
                json!({}),
            ));
        }
        if let Some(e) = self.branch_checked_out(project, b)? {
            return Err(e);
        }
        Ok(PrLaunch { head })
    }

    /// `branch_checked_out` when `branch` is checked out in a worktree of the
    /// project, the main checkout included (F1: naming the feature holding it).
    pub(crate) fn branch_checked_out(&self, project: &Project, branch: &str) -> Result<Option<anyhow::Error>> {
        let Some(w) = worktree::list(&project.path)?.into_iter().find(|w| w.branch.as_deref() == Some(branch)) else {
            return Ok(None);
        };
        let path = crate::paths::real(&w.path);
        let holder = self
            .db
            .features(Some(&project.id))?
            .into_iter()
            .find(|f| f.integration_worktree.as_deref().map(crate::paths::real).as_deref() == Some(path.as_path()));
        let shown = crate::paths::tilde(&path);
        Ok(Some(match holder {
            Some(f) => {
                let run = self.db.runs(Some(&f.id), false)?.into_iter().next();
                refused(
                    "branch_checked_out",
                    format!("{branch} is checked out in {shown} (feature {}): delete that feature first", f.slug),
                    json!({"details": {"path": path, "feature": feature_ref(&f, run.as_ref())}}),
                )
            }
            None => refused(
                "branch_checked_out",
                format!("{branch} is checked out in {shown}; switch that checkout to another branch first"),
                json!({"details": {"path": path}}),
            ),
        }))
    }

    /// Step 2 and 4 of §5.3: fetches the head and base, refuses a local head
    /// with commits the PR lacks, fast-forwards one that is behind. The
    /// worktree is then cut from `origin/<head>` when there is no local
    /// branch (G5). Returns the head commit the feature is linked at.
    pub(crate) fn prepare_pr_branch(&self, repo: &Path, launch: &mut PrLaunch) -> Result<()> {
        let h = &mut launch.head;
        let n = h.number;
        let fetch = |e: anyhow::Error| refused("fetch_failed", format!("could not fetch PR #{n}: {e:#}"), json!({}));
        // Explicit refspecs: `origin/<head>` is updated even when the remote's
        // fetch refspec leaves that branch out (a single-branch clone).
        let specs: Vec<String> =
            [&h.head_ref, &h.base_ref].iter().map(|b| format!("+refs/heads/{b}:refs/remotes/origin/{b}")).collect();
        let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
        gh::fetch_branches(repo, &specs, crate::pr::FETCH_DEADLINE).map_err(fetch)?;
        let remote = format!("origin/{}", h.head_ref);
        let tip = gh::rev_sha(repo, &format!("refs/remotes/{remote}"))
            .ok_or_else(|| fetch(anyhow!("{remote} not found after the fetch")))?;
        if worktree::branch_exists(repo, &h.head_ref) {
            let local = format!("refs/heads/{}", h.head_ref);
            // Unknown is not "0 ahead": the local commits would be kept unseen.
            let (ahead, behind) = cleanup::try_ahead_behind(repo, &local, &remote)
                .ok_or_else(|| fetch(anyhow!("can't compare {local} with {remote}")))?;
            if ahead > 0 {
                return Err(refused(
                    "local_ahead",
                    format!(
                        "local {} has {ahead} commit(s) that PR #{n} does not have; push or delete them first",
                        h.head_ref
                    ),
                    json!({"details": {"commits": ahead}}),
                ));
            }
            if behind > 0 {
                // Not checked out anywhere (checked before): moved by ref, with
                // the old value as a guard against a concurrent change.
                let old = gh::rev_sha(repo, &local).unwrap_or_default();
                orch_git::git(repo, &["update-ref", "-m", "orch: fast-forward to the PR head", &local, &tip, &old])?;
            }
        }
        // The fetch is fresher than `gh pr view`: the team starts from it.
        h.head_sha = tip;
        Ok(())
    }
}

#[cfg(test)]
#[path = "pr_link_tests.rs"]
mod tests;
