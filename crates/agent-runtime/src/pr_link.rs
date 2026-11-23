//! allow-launch-team-on-update-pr §5.3: a feature launched on someone else's
//! pull request. Its integration branch is the PR's head branch, checked out
//! from `origin/<head>`; the launch re-reads the PR (G2) and refuses, before
//! anything is stored, what would make the team work on the wrong commits.

use std::path::Path;

use anyhow::{Result, anyhow};
use orch_git::gh::{self, PrHead};
use orch_git::{cleanup, worktree};
use orch_store::{Feature, PrLink, Project};
use orch_tasks::intake::gh_error;
use orch_team::docs_location::{DesignsChoice, DocsChoice};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// A command error whose text is the JSON `{code, message, details?, feature?}`
/// (api-contract §3, F10–F11).
pub(crate) fn refused(code: &str, message: impl Into<String>, extra: Value) -> anyhow::Error {
    let e = crate::issues::coded(code, message, None);
    let Value::Object(extra) = extra else { return e };
    let mut v: Value = serde_json::from_str(&e.to_string()).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.extend(extra);
    }
    anyhow!(v.to_string())
}

/// The code of a failed `gh` call (api-contract §2–3), as intake reads it:
/// `offline`, `gh_missing`, `gh_auth` (a missing scope too), else `fallback`.
pub fn gh_error_code(e: &anyhow::Error, fallback: &'static str) -> &'static str {
    match gh_error(e).code {
        "offline" => "offline",
        "gh_missing" => "gh_missing",
        "gh_auth" | "gh_scope" => "gh_auth",
        _ => fallback,
    }
}

/// A PR launch in `per_issue` mode (asked, or kept by a feature coming back).
pub(crate) fn per_issue_refused(n: u64) -> anyhow::Error {
    let m = format!("a feature on PR #{n} pushes to that PR only: one PR per issue is not available");
    refused("per_issue", m, json!({}))
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

/// The feature fields a PR launch sets from the PR as GitHub has it now
/// (`h`): the head as the integration branch, the PR base, the link, and
/// docs and designs in the app folder whatever the project setting (D1/G1),
/// so none enters the PR.
pub(crate) fn linked_feature(f: Feature, h: &PrHead, sent: &PrLink, app_base: &str) -> Feature {
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

impl Runtime {
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
    pub(crate) fn check_pr_launch(&self, project: &Project, link: &PrLink, per_issue: bool) -> Result<PrHead> {
        let n = link.number;
        if n == 0 {
            return Err(refused("bad_request", "no PR number", json!({})));
        }
        if per_issue {
            return Err(per_issue_refused(n));
        }
        self.no_active_feature(project, n, None)?;
        let head = self.read_open_pr(project, n)?;
        let b = &head.head_ref;
        // The app's own branches have their own feature (and the worktree
        // layout cuts a missing `orch/*` branch from the base, G5).
        if let Some(reason) = crate::pr_prefill::unusable_branch(n, b, &head.base_ref) {
            return Err(refused("bad_request", reason, json!({})));
        }
        self.branch_checked_out(project, b)?;
        Ok(head)
    }

    /// A linked feature launched again without a `pr` (its link kept): its
    /// PR re-read with the same refusals (D3, fork, closed, merged). Its own
    /// worktree may still hold the branch: that is not checked.
    pub(crate) fn check_pr_relaunch(&self, project: &Project, f: &Feature) -> Result<()> {
        let Some(l) = &f.pr_link else { return Ok(()) };
        self.no_active_feature(project, l.number, Some(&f.id))?;
        self.read_open_pr(project, l.number).map(|_| ())
    }

    /// Relaunch of a linked feature (§5.3 relaunch): fetches `origin/<head>`,
    /// fast-forwards a local branch that is only behind, keeps one that is
    /// ahead (the commits a resumed run kept) or diverged (the watcher's
    /// `foreign_head` and `rebase_base` onto `pr_head` cover it), and returns
    /// the fetched origin head, the new `head_sha_at_link`. The feature's own
    /// kept worktree holding the branch is reused; another one refuses.
    pub(crate) fn prepare_pr_relaunch(&self, project: &Project, f: &Feature, l: &PrLink) -> Result<String> {
        let repo = &project.path;
        let branch = &l.head_ref;
        let own = f.integration_worktree.as_deref().map(crate::paths::real);
        let holder = worktree::list(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(branch.as_str()));
        let in_own = holder.as_ref().is_some_and(|w| own.as_deref() == Some(crate::paths::real(&w.path).as_path()));
        if holder.is_some() && !in_own {
            self.branch_checked_out(project, branch)?;
        }
        let tip = fetch_pr_head(repo, l.number, branch, &[])?;
        if worktree::branch_exists(repo, branch) {
            let local = format!("refs/heads/{branch}");
            let (ahead, behind) = local_position(repo, l.number, &local, branch)?;
            if ahead == 0 && behind > 0 {
                match holder.as_ref().filter(|_| in_own) {
                    // A refused merge (local edits in the way) leaves the branch as it was. Hooks and
                    // fsmonitor off: this runs in the core, outside the sandbox, in a worktree the team
                    // wrote to (a tracked hook script is the agent's to edit).
                    Some(w) => {
                        let hooks_off = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"];
                        drop(orch_git::git(&w.path, &[&hooks_off[..], &["merge", "--ff-only", "-q", &tip]].concat()))
                    }
                    None => {
                        let old = gh::rev_sha(repo, &local).unwrap_or_default();
                        orch_git::git(
                            repo,
                            &["update-ref", "-m", "orch: fast-forward to the PR head", &local, &tip, &old],
                        )?;
                    }
                }
            }
        }
        Ok(tip)
    }

    /// D3: one active feature per PR (no run at all counts as not active);
    /// `but` is the feature being launched.
    fn no_active_feature(&self, project: &Project, n: u64, but: Option<&str>) -> Result<()> {
        if let Some((f, run)) = self.feature_on_pr(&project.id, n)?
            && Some(f.id.as_str()) != but
            && run.as_ref().is_some_and(|r| !crate::team::effective(r).finished())
        {
            return Err(refused(
                "active_feature",
                format!("PR #{n} already has a team: feature {}", f.slug),
                json!({"feature": feature_ref(&f, run.as_ref())}),
            ));
        }
        Ok(())
    }

    /// The PR re-read (G2): refused offline, from a fork, closed or merged.
    fn read_open_pr(&self, project: &Project, n: u64) -> Result<PrHead> {
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
        Ok(head)
    }

    /// `branch_checked_out` when `branch` is checked out in a worktree of the
    /// project, the main checkout included (F1: naming the feature holding
    /// it, a deleted linked one whose worktree was kept too).
    pub(crate) fn branch_checked_out(&self, project: &Project, branch: &str) -> Result<()> {
        let Some(w) = worktree::list(&project.path)?.into_iter().find(|w| w.branch.as_deref() == Some(branch)) else {
            return Ok(());
        };
        let path = crate::paths::real(&w.path);
        let holds =
            |f: &Feature| f.integration_worktree.as_deref().map(crate::paths::real).as_deref() == Some(path.as_path());
        let live = self.db.features(Some(&project.id))?.into_iter().find(holds);
        let deleted = match &live {
            Some(_) => None,
            None => self.db.linked_features(&project.id)?.into_iter().find(|f| f.archived && holds(f)),
        };
        let shown = crate::paths::tilde(&path);
        Err(match (live, deleted) {
            (_, Some(f)) => refused(
                "branch_checked_out",
                format!(
                    "{branch} is checked out in {shown} (deleted feature {}, its worktree kept): remove that \
                     worktree first",
                    f.slug
                ),
                json!({"details": {"path": path, "feature": feature_ref(&f, None)}}),
            ),
            (Some(f), None) => {
                let run = self.db.runs(Some(&f.id), false)?.into_iter().next();
                refused(
                    "branch_checked_out",
                    format!("{branch} is checked out in {shown} (feature {}): delete that feature first", f.slug),
                    json!({"details": {"path": path, "feature": feature_ref(&f, run.as_ref())}}),
                )
            }
            (None, None) => refused(
                "branch_checked_out",
                format!("{branch} is checked out in {shown}; switch that checkout to another branch first"),
                json!({"details": {"path": path}}),
            ),
        })
    }

    /// Step 2 and 4 of §5.3: fetches the head and base, refuses a local head
    /// with commits the PR lacks, fast-forwards one that is behind. The
    /// worktree is then cut from `origin/<head>` when there is no local
    /// branch (G5). `h.head_sha` becomes the head the feature is linked at.
    pub(crate) fn prepare_pr_branch(&self, repo: &Path, h: &mut PrHead) -> Result<()> {
        let n = h.number;
        let tip = fetch_pr_head(repo, n, &h.head_ref, &[&h.base_ref])?;
        if worktree::branch_exists(repo, &h.head_ref) {
            let local = format!("refs/heads/{}", h.head_ref);
            // Unknown is not "0 ahead": the local commits would be kept unseen.
            let (ahead, behind) = local_position(repo, n, &local, &h.head_ref)?;
            if ahead > 0 {
                return Err(refused(
                    "local_ahead",
                    format!(
                        "Local {} has {ahead} commit(s) PR #{n} does not have. Rename or delete that local branch first.",
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

fn fetch_failed(n: u64, e: anyhow::Error) -> anyhow::Error {
    refused("fetch_failed", format!("could not fetch PR #{n}: {e:#}"), json!({}))
}

/// Fetches `head` (and the `also` branches) with explicit refspecs, so
/// `origin/<head>` is updated even when the remote's fetch refspec leaves
/// that branch out (a single-branch clone); returns the head's sha.
fn fetch_pr_head(repo: &Path, n: u64, head: &str, also: &[&str]) -> Result<String> {
    let specs: Vec<String> = std::iter::once(head).chain(also.iter().copied()).map(gh::branch_refspec).collect();
    let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
    gh::fetch_branches(repo, &specs, crate::pr::FETCH_DEADLINE).map_err(|e| fetch_failed(n, e))?;
    let remote = format!("origin/{head}");
    gh::rev_sha(repo, &format!("refs/remotes/{remote}"))
        .ok_or_else(|| fetch_failed(n, anyhow!("{remote} not found after the fetch")))
}

/// `(ahead, behind)` of the local `local` ref against `origin/<head>`. Unknown
/// is not "0 ahead": the local commits would be kept unseen.
fn local_position(repo: &Path, n: u64, local: &str, head: &str) -> Result<(u32, u32)> {
    let remote = format!("origin/{head}");
    cleanup::try_ahead_behind(repo, local, &remote)
        .ok_or_else(|| fetch_failed(n, anyhow!("can't compare {local} with {remote}")))
}

#[cfg(test)]
#[path = "pr_link_tests.rs"]
pub(crate) mod tests;
