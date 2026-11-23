//! allow-launch-team-on-update-pr §5.5: `request_pr` on a feature linked to
//! someone else's PR, before that PR is `pr_open` for the run. The user
//! approves a push of the team's commits to the PR's head branch (never a
//! `gh pr create`); the push is refused when the PR moved or closed since.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use orch_git::gh;
use serde_json::{Value, json};

use crate::inbox::{PRIORITY_GATE, Stage, diff_summary, short};
use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::RunCtx;

/// Why an approved linked push did not happen (F6): `pr_moved`, `pr_closed`, `push_failed`.
struct Refusal {
    code: &'static str,
    message: String,
}

impl Refusal {
    fn new(code: &'static str, message: String) -> Refusal {
        Refusal { code, message }
    }
}

impl Runtime {
    /// The `push_pr` gate of a linked feature: the team's commits
    /// (`origin/<head>..HEAD`) under a lease on the PR head the app last saw (G3).
    pub(crate) fn request_pr_linked(&self, rec: &AgentRecord, cx: &RunCtx) -> Result<String> {
        let l = cx.feature.pr_link.as_ref().ok_or_else(|| anyhow!("{} is not on a PR", cx.feature.slug))?;
        let (n, wt) = (l.number, &rec.worktree);
        if orch_git::commit::rebase_in_progress(wt) {
            bail!(
                "a rebase is in progress: finish it with mcp__orch__commit (or abort it with mcp__orch__rebase_base) first"
            );
        }
        match cx.run.pr.as_ref().map(|p| p.state.as_str()) {
            Some("closed") => bail!("PR #{n} is closed: ask the user"),
            Some("merged") => bail!("PR #{n} is merged: the feature is done"),
            _ => {}
        }
        // D1 guard: the docs live in the app folder; one committed by hand is refused.
        let docs = format!(".orchestrator/features/{}/", cx.feature.slug);
        let range = format!("origin/{}...HEAD", cx.feature.base);
        let touched = orch_git::git(wt, &["diff", "--name-only", &range, "--", &docs])?;
        if !touched.trim().is_empty() {
            bail!("{docs} is in the diff: PR #{n} is someone else's, remove it before pushing");
        }
        // G3: the head the watcher last saw, else the head at launch.
        let lease = cx
            .run
            .pr
            .as_ref()
            .map(|p| p.head_sha.clone())
            .filter(|s| orch_git::git::valid_sha(s))
            .unwrap_or_else(|| l.head_sha_at_link.clone());
        let remote = format!("origin/{}", l.head_ref);
        if gh::rev_sha(wt, &lease).is_none()
            && let Ok(_watch) = self.pr_watch.try_lock()
        {
            let spec = format!("+refs/heads/{0}:refs/remotes/{remote}", l.head_ref);
            let _ = gh::fetch_branches(wt, &[&spec], crate::pr::FETCH_DEADLINE);
        }
        let ff = orch_git::cleanup::is_merged(wt, &lease, "HEAD");
        // A force push never drops a foreign commit: only the team's own
        // rewrite (a rebase onto the base from a head holding the lease).
        if !ff && !self.rebased_from(&cx.run.id, wt, &lease, &cx.feature.base) {
            bail!(
                "PR #{n} has commits the branch does not have ({}): bring them in with mcp__orch__rebase_base \
                 {{\"onto\": \"pr_head\"}}, then ask again",
                short(&lease)
            );
        }
        // The team's commits only: past the lease (or the remote head, rewritten).
        let from = if ff { lease.clone() } else { remote.clone() };
        let commits = gh::commits_since(wt, &from)?;
        if commits.is_empty() {
            bail!("No commit to push to PR #{n}");
        }
        let branch = orch_git::worktree::current_branch(wt)?;
        let head = gh::rev_sha(wt, "HEAD").ok_or_else(|| anyhow!("{branch} has no commit"))?;
        let id = self.add_item(
            Some(&rec.id),
            "gate",
            format!("{}: push {} commit(s) to PR #{n} ({branch})", rec.id, commits.len()),
            json!({"action": "push_pr", "run": cx.run.id, "issue": null, "branch": branch, "base": cx.feature.base,
                   "head": head, "linked": {"number": n, "url": l.url}, "pr_number": n, "url": l.url,
                   "lease_sha": lease, "force": !ff,
                   "commits": commits.iter().map(|(sha, subject)| json!({"sha": sha, "subject": subject})).collect::<Vec<_>>(),
                   "worktree": wt, "diff_base": from, "diff": diff_summary(wt, &from, &branch)}),
            PRIORITY_GATE,
        )?;
        self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "pr_requested",
            Some(&branch),
            "pending",
            json!({"item": id, "linked": n, "commits": commits.len(), "force": !ff}),
        )?;
        Ok(format!(
            "Requested: decision {id} asks the user to push {} commit(s) of {branch} to PR #{n}. You will be told the \
             result; there is nothing else to do for it now.",
            commits.len()
        ))
    }

    /// The run's last `rebase_base` onto the base started from a head that
    /// held `lease`: the team rewrote the PR's commits itself.
    fn rebased_from(&self, run_id: &str, wt: &Path, lease: &str, base: &str) -> bool {
        let _ = self.db.flush();
        let last = self.db.run_events(run_id, 500).unwrap_or_default().into_iter().find(|e| e.kind == "git.rebase");
        last.is_some_and(|e| {
            e.detail["onto"] == json!(format!("origin/{base}"))
                && e.detail["from"].as_str().is_some_and(|f| orch_git::cleanup::is_merged(wt, lease, f))
        })
    }

    /// A linked `push_pr` gate approved: the PR re-read (still open, still
    /// at the lease), then the push (a fast-forward, or under the lease),
    /// then the run `pr_open` on that PR. A refusal fails the gate with its
    /// code (F6) and sends the run back to integrating.
    pub(crate) fn push_pr_linked(&self, item: u64, p: &Value) {
        let agent = self.store.read(|s| s.inbox.iter().find(|i| i.id == item).and_then(|i| i.agent_id.clone()));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (wt, branch, head, lease) = (PathBuf::from(s("worktree")), s("branch"), s("head"), s("lease_sha"));
        let n = p["linked"]["number"].as_u64().unwrap_or(0);
        let url = p["linked"]["url"].as_str().unwrap_or("").to_string();
        let commits = p["commits"].as_array().map_or(0, Vec::len);
        let force = !orch_git::cleanup::is_merged(&wt, &lease, &head);
        let r = self.push_linked(&wt, n, &branch, &head, &lease, force);
        let _ = self.db.audit(
            "core",
            agent.as_deref(),
            "pr_push_linked",
            Some(&branch),
            if r.is_ok() { "ok" } else { "failed" },
            json!({"item": item, "pr_number": n, "commits": commits, "force": force,
                   "result": r.as_ref().map_err(|e| json!({"code": e.code, "message": e.message}))}),
        );
        self.event(agent.as_deref().unwrap_or("-"), "pr", json!({"item": item, "ok": r.is_ok(), "linked": n}));
        match &r {
            Ok(()) => self.advance(item, Stage::Effect("pr_pushed", Some(url.clone()))),
            Err(e) => {
                let outcome = json!({"code": e.code, "message": e.message});
                let _ = self.store.write(|st| {
                    if let Some(i) = st.inbox.iter_mut().find(|i| i.id == item) {
                        i.payload["outcome"] = outcome;
                    }
                });
                self.advance(item, Stage::Failed(e.message.clone()));
            }
        }
        let Some(a) = agent else { return };
        let (check, msg) = match &r {
            Ok(()) => {
                let check = self.pr_opened(&a, Some(&url));
                if let Some(run) = &check {
                    self.run_event(run, None, "pr.pushed", json!({"number": n, "commits": commits, "force": force}));
                }
                let msg = format!(
                    "ORCH: the user approved; pushed {commits} commit(s) to PR #{n} ({url}). CI restarts; you will be \
                     told the result."
                );
                (check, msg)
            }
            Err(e) => {
                let _ = self.pr_opened(&a, None);
                let next = match e.code {
                    "pr_moved" => {
                        "Wait for the next check, bring the new head in with mcp__orch__rebase_base \
                         {\"onto\": \"pr_head\"}, then call mcp__orch__request_pr again."
                    }
                    "pr_closed" => "Ask the user what they want with mcp__orch__ask_user.",
                    _ => "Nothing was pushed; call mcp__orch__request_pr again once it is fixed.",
                };
                (None, format!("ORCH: the user approved, but the push to PR #{n} was refused: {} {next}", e.message))
            }
        };
        let _ = self.deliver_user(&a, &msg);
        if let Some(run) = check {
            self.pr_check_now(&run, None, Some(head));
        }
    }

    /// `gh pr view` then the push of `head` to the PR's branch.
    fn push_linked(
        &self,
        wt: &Path,
        n: u64,
        branch: &str,
        head: &str,
        lease: &str,
        force: bool,
    ) -> Result<(), Refusal> {
        let view = self.gh_pr_view(wt, n).map_err(|e| {
            Refusal::new("push_failed", format!("PR #{n} could not be read ({e:#}): nothing was pushed."))
        })?;
        if view.state != "open" {
            return Err(Refusal::new("pr_closed", format!("PR #{n} is {}: nothing was pushed.", view.state)));
        }
        let moved = |sha: &str| {
            Refusal::new(
                "pr_moved",
                format!(
                    "PR #{n} moved to {} since the push was asked (pushed outside the app): nothing was pushed.",
                    short(sha)
                ),
            )
        };
        if view.head_sha != lease {
            return Err(moved(&view.head_sha));
        }
        let pushed = if force {
            gh::push_with_lease(wt, "origin", head, branch, lease)
        } else {
            gh::push_rev(wt, "origin", head, branch)
        };
        pushed.map_err(|e| {
            let text = format!("{e:#}");
            // The lease refused, or a fast-forward rejected: the PR moved under the push.
            if text.contains("stale info") || text.contains("non-fast-forward") || text.contains("fetch first") {
                moved(lease)
            } else {
                Refusal::new("push_failed", format!("pushing to PR #{n} failed: {text}"))
            }
        })
    }
}

#[cfg(test)]
#[path = "pr_push_tests.rs"]
mod tests;
