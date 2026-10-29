//! A team run's pull request once it is open (feature improve-end-of-feature
//! §3.2–3.5): the watcher that follows its state, CI and position against
//! its base, tells the lead what calls for work, and moves the run when the
//! PR is merged (→ done) or closed (→ integrating); and the gated update push
//! of new commits to the open PR (ADR-009: only the app pushes).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use governor::TurnKind;
use orch_git::gh::{self, PrView};
use orch_protocol::now_ms;
use orch_store::{CiJob, Feature, PrStatus, TeamRun};
use orch_team::RunState;
use serde_json::{Value, json};

use crate::inbox::{PRIORITY_ALERT, PRIORITY_GATE};
use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::effective;

/// How often the watcher checks each open PR.
pub const PR_EVERY: Duration = Duration::from_secs(60);
/// The base and PR head are fetched at most this often per run.
const FETCH_BASE_EVERY_MS: u64 = 5 * 60 * 1000;
const FETCH_DEADLINE: Duration = Duration::from_secs(120);

/// The watcher's own clock: when each run last fetched. Held for a whole
/// check, so two checks of the PRs never interleave (tick, refresh, push).
pub(crate) type PrWatch = BTreeMap<String, u64>;

/// What changed between two checks of a PR (see [`pr_changes`]).
#[derive(Clone, Debug, PartialEq)]
pub enum PrChange {
    Merged,
    Closed,
    /// `ci.overall` changed to or from `error` / `ok` (or failed again on a new head).
    Ci {
        from: String,
        to: String,
        failed: Vec<CiJob>,
    },
    /// `position.status` changed; `tell` when the lead has work (behind on a new base head, conflicts).
    Position {
        from: String,
        to: String,
        behind: u32,
        tell: bool,
    },
}

/// The changes from `old` (the stored status) to `new`. Comparing with what
/// is stored makes a restart repeat nothing.
pub fn pr_changes(old: Option<&PrStatus>, new: &PrStatus) -> Vec<PrChange> {
    // A status stored only from a failed first check was never seen.
    let old = old.filter(|o| !o.state.is_empty());
    let old_state = old.map_or("open", |o| o.state.as_str());
    if new.state != "open" {
        return match (new.state.as_str(), new.state == old_state) {
            (_, true) => vec![],
            ("merged", _) => vec![PrChange::Merged],
            _ => vec![PrChange::Closed],
        };
    }
    let mut out = vec![];
    let from = old.map_or("none", |o| o.ci.overall.as_str());
    let to = new.ci.overall.as_str();
    let decisive = |s: &str| s == "error" || s == "ok";
    let failed_again = to == "error" && from == "error" && old.is_some_and(|o| o.head_sha != new.head_sha);
    if (from != to && (decisive(from) || decisive(to))) || failed_again {
        let failed = if to == "error" {
            new.ci.jobs.iter().filter(|j| gh::job_failed(&j.state)).cloned().collect()
        } else {
            vec![]
        };
        out.push(PrChange::Ci { from: from.into(), to: to.into(), failed });
    }
    let from = old.map_or("unknown", |o| o.position.status.as_str());
    let to = new.position.status.as_str();
    let new_base = old.is_none_or(|o| o.position.base_sha != new.position.base_sha);
    // `unknown` (GitHub recomputing mergeability) between two `behind` on the
    // same base head is not news: its counts still say behind.
    let was_behind = from == "behind" || (from == "unknown" && old.is_some_and(|o| o.position.behind > 0));
    let tell = match to {
        "conflicts" => from != "conflicts",
        "behind" => !was_behind || new_base,
        _ => false,
    };
    if tell || (from != to && from != "unknown" && to != "unknown") {
        out.push(PrChange::Position { from: from.into(), to: to.into(), behind: new.position.behind, tell });
    }
    out
}

fn job_line(j: &CiJob) -> String {
    let wf = j.workflow.as_deref().map(|w| format!("{w} / ")).unwrap_or_default();
    let url = j.url.as_deref().map(|u| format!(", {u}")).unwrap_or_default();
    format!("{wf}{} ({}{url})", j.name, j.state)
}

impl Runtime {
    // ------------------------------------------------------------ gh seam

    /// `gh pr view` in `dir`; tests replace `gh` through `cfg.gh`.
    fn gh_pr_view(&self, dir: &Path, number: u64) -> Result<PrView> {
        match self.gh_override() {
            Some(g) => gh::parse_pr_view(&g.run(&gh::pr_view_args(number))?),
            None => gh::pr_view(dir, number),
        }
    }

    /// `gh pr view <branch>`: the number and URL of its PR.
    fn gh_pr_lookup(&self, dir: &Path, branch: &str) -> Result<(u64, String)> {
        match self.gh_override() {
            Some(g) => gh::parse_pr_lookup(&g.run(&gh::pr_lookup_args(branch))?),
            None => gh::pr_lookup(dir, branch),
        }
    }

    /// `gh pr create`; its URL. Tests replace `gh` through `cfg.gh`.
    pub(crate) fn gh_create_pr(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String> {
        match self.gh_override() {
            Some(g) => Ok(gh::created_pr_url(&g.run(&gh::create_pr_args(base, head, title, body, draft))?)),
            None => gh::create_pr(dir, base, head, title, body, draft),
        }
    }

    // ------------------------------------------------------------ watcher

    /// The background pass (every [`PR_EVERY`], on the fetch thread): checks
    /// each run whose PR is open, paused and deferred runs included. Nothing
    /// while offline or on low battery (FR-103).
    pub fn pr_tick(&self, now: u64) {
        if orch_git::fetch::blocked(&self.cfg.fetch, self.cfg.conditions.as_ref()).is_some() {
            return;
        }
        let runs = self.db.runs(None, true).unwrap_or_default();
        for r in runs.iter().filter(|r| effective(r) == RunState::PrOpen && r.pr_number.is_some()) {
            if let Err(e) = self.pr_check(&r.id, false, now, None) {
                self.run_event(&r.id, None, "pr.error", json!(format!("{e:#}")));
            }
        }
    }

    /// `pr_refresh`: checks the PR of the feature's latest run now (the fetch
    /// throttle skipped, not the offline check). `feature` is an id or a slug.
    pub fn pr_refresh(&self, feature: &str) -> Result<TeamRun> {
        let f = match self.db.feature(feature)? {
            Some(f) => f,
            None => self
                .db
                .projects(false)?
                .iter()
                .find_map(|p| self.db.feature_by_slug(&p.id, feature).ok().flatten())
                .ok_or_else(|| anyhow!("no feature {feature}"))?,
        };
        let run =
            self.db.runs(Some(&f.id), false)?.into_iter().next().ok_or_else(|| anyhow!("{} has no run", f.slug))?;
        if run.pr_number.is_none() {
            bail!("{} has no pull request", f.slug);
        }
        if !self.cfg.conditions.online() {
            bail!("offline: the pull request can't be checked");
        }
        self.pr_check(&run.id, true, now_ms(), None)
    }

    /// Checks a run's PR once: `gh pr view`, the base and PR head fetched
    /// when due (or `force_fetch`), the counts, then the changes against the
    /// stored status applied. `pushed` is the head the app just pushed.
    pub(crate) fn pr_check(
        &self,
        run_id: &str,
        force_fetch: bool,
        now: u64,
        pushed: Option<String>,
    ) -> Result<TeamRun> {
        let mut watch = self.pr_watch.lock().unwrap_or_else(|e| e.into_inner());
        let mut run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let n = run.pr_number.ok_or_else(|| anyhow!("run {run_id} has no pull request"))?;
        let feature = self.db.feature(&run.feature_id)?.ok_or_else(|| anyhow!("no feature {}", run.feature_id))?;
        let wt = feature
            .integration_worktree
            .clone()
            .ok_or_else(|| anyhow!("{} has no integration worktree", feature.slug))?;
        let old = run.pr.clone();
        let view = match self.gh_pr_view(&wt, n) {
            Ok(v) => v,
            Err(e) => {
                // The last good status stays, marked stale; the run does not move.
                let mut st = old.unwrap_or_default();
                st.error = Some(format!("{e:#}"));
                st.checked_at_ms = now;
                st.pushed_sha = pushed.or(st.pushed_sha);
                self.db.set_run_pr(run_id, &st, None)?;
                run.pr = Some(st);
                return Ok(run);
            }
        };
        let branch = feature.integration_branch();
        if force_fetch || watch.get(run_id).is_none_or(|t| now.saturating_sub(*t) >= FETCH_BASE_EVERY_MS) {
            // A failed fetch leaves the last counts (or `unknown`); tried again when due.
            let _ = gh::fetch_branches(&wt, &[&view.base, &branch], FETCH_DEADLINE);
            watch.insert(run_id.to_string(), now);
        }
        let base_ref = format!("origin/{}", view.base);
        let base_sha = gh::rev_sha(&wt, &base_ref);
        let counts = base_sha
            .as_ref()
            .and_then(|_| orch_git::cleanup::try_ahead_behind(&wt, &view.head_sha, &base_ref))
            .map(|(ahead, behind)| (behind, ahead));
        let pushed_sha = pushed.or_else(|| old.as_ref().and_then(|o| o.pushed_sha.clone()));
        let new = PrStatus {
            state: view.state.clone(),
            head_sha: view.head_sha.clone(),
            base: view.base.clone(),
            ci: view.ci.clone(),
            position: gh::position(&view.mergeable, &view.merge_state, counts, base_sha),
            unpushed: orch_git::cleanup::try_ahead_behind(&wt, "HEAD", &view.head_sha).map_or(0, |(a, _)| a),
            rewritten: pushed_sha.as_deref().is_some_and(|p| !orch_git::cleanup::is_merged(&wt, p, "HEAD")),
            pushed_sha,
            checked_at_ms: now,
            error: None,
        };
        if new.state != "open" {
            watch.remove(run_id);
        }
        // Only the PR columns: a pause or stop made while `gh` and the fetch ran stays.
        self.db.set_run_pr(run_id, &new, Some(&view.url).filter(|u| !u.is_empty()).map(String::as_str))?;
        drop(watch);
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        // A refresh of a run no longer `pr_open` (done, failed, back to
        // integrating) stores the status but moves and tells nothing.
        if effective(&run) == RunState::PrOpen {
            self.pr_apply(&run, &feature, n, &pr_changes(old.as_ref(), &new));
        }
        Ok(self.db.run(run_id)?.unwrap_or(run))
    }

    /// The timeline events, lead turns, run moves and Inbox items of `changes`.
    fn pr_apply(&self, run: &TeamRun, feature: &Feature, n: u64, changes: &[PrChange]) {
        let base = run.pr.as_ref().map(|p| p.base.clone()).unwrap_or_else(|| feature.base.clone());
        let url = run.pr_url.clone().unwrap_or_default();
        let mut status = json!({"number": n});
        let mut tell: Vec<String> = vec![];
        for c in changes {
            match c {
                PrChange::Merged | PrChange::Closed => {
                    let (to, why, what, lead) = if *c == PrChange::Merged {
                        (RunState::Done, "pull request merged", "merged", "The feature is done; end your turn.")
                    } else {
                        (
                            RunState::Integrating,
                            "pull request closed",
                            "closed",
                            "The run is back to integrating. Ask the user what they want with mcp__orch__ask_user; \
                             mcp__orch__request_pr opens a new pull request.",
                        )
                    };
                    // A merge is not held back by a pause: the run is done (spec §3.3).
                    let moved = if to == RunState::Done {
                        self.set_run_state(&run.id, to, why).map(|_| ())
                    } else {
                        self.advance_run(&run.id, to, why).map(|_| ())
                    };
                    if let Err(e) = moved {
                        self.run_event(&run.id, None, "team_error", json!(format!("{e:#}")));
                    }
                    self.run_event(&run.id, None, &format!("pr.{what}"), json!({"number": n}));
                    self.withdraw_pr_updates(&run.id, why);
                    let title = if what == "merged" {
                        format!("Feature {} merged", feature.slug)
                    } else {
                        format!("Feature {} PR closed", feature.slug)
                    };
                    let _ = self.add_item(
                        None,
                        "info",
                        title,
                        json!({"action": format!("pr_{what}"), "run": run.id, "feature": feature.id, "pr_number": n, "url": url}),
                        PRIORITY_ALERT,
                    );
                    let how = if what == "merged" { "merged" } else { "closed without being merged" };
                    tell.push(format!("ORCH: PR #{n} was {how}. {lead}"));
                }
                PrChange::Ci { from, to, failed } => {
                    let mut ci = json!({"from": from, "to": to});
                    if to == "error" {
                        ci["failed_jobs"] = json!(
                            failed.iter().map(|j| json!({"name": j.name, "workflow": j.workflow})).collect::<Vec<_>>()
                        );
                        tell.push(format!(
                            "ORCH: CI fails on PR #{n}: {}. Find whether the feature caused it; if so, fix it with a \
                             task (or yourself), merge, and call mcp__orch__request_pr to push the fix. If the failure \
                             is not the feature's (flaky, infra), say so with mcp__orch__ask_user.",
                            failed.iter().map(job_line).collect::<Vec<_>>().join("; ")
                        ));
                    }
                    status["ci"] = ci;
                }
                PrChange::Position { from, to, behind, tell: work } => {
                    status["position"] = json!({"from": from, "to": to, "behind": behind});
                    if *work {
                        let what = if to == "conflicts" {
                            format!("in conflict with {base}")
                        } else {
                            format!("behind {base} by {behind} commit(s)")
                        };
                        tell.push(format!(
                            "ORCH: PR #{n} is {what}. Call mcp__orch__rebase_base, resolve any conflicts, run the \
                             tests, then mcp__orch__request_pr to push."
                        ));
                    }
                }
            }
        }
        if status.get("ci").is_some() || status.get("position").is_some() {
            self.run_event(&run.id, None, "pr.status", status);
        }
        if tell.is_empty() {
            return;
        }
        let lead = self.run_ctx(&run.id).ok().and_then(|cx| self.lead_of(&cx));
        if let Some(lead) = lead {
            for t in tell {
                let _ = self.deliver_kind(&lead.id, &t, TurnKind::Continue);
            }
        }
    }

    /// Closes the pending update gates of a run (its PR merged or closed).
    fn withdraw_pr_updates(&self, run_id: &str, why: &str) {
        self.close_items_where(
            |i| {
                i.payload.get("action") == Some(&json!("push_pr_update"))
                    && i.payload.get("run").and_then(Value::as_str) == Some(run_id)
            },
            why,
        );
    }

    /// The PR gate was approved and the PR opened (`url`), or it failed. The
    /// run is `pr_open` with the PR's number; the run to check at once
    /// ([`Runtime::pr_check_now`], once the lead is told).
    pub(crate) fn pr_opened(&self, agent: &str, url: Option<&str>) -> Option<String> {
        let seat = self.store.read(|s| s.agents.get(agent).and_then(|a| a.team.clone()))?;
        let mut run = self.db.run(&seat.run_id).ok()??;
        let Some(u) = url else {
            let _ = self.advance_run(&run.id, RunState::Integrating, "pull request failed");
            return None;
        };
        // `gh pr create` printed no `…/pull/N`: ask GitHub for the branch's PR.
        let (number, url) = match gh::pr_number_from_url(u) {
            Some(n) => (n, u.to_string()),
            None => {
                let feature = self.db.feature(&run.feature_id).ok().flatten();
                let found = feature.as_ref().and_then(|f| {
                    let wt = f.integration_worktree.clone()?;
                    self.gh_pr_lookup(&wt, &f.integration_branch()).ok()
                });
                match found {
                    Some((n, url)) => (n, if url.is_empty() { u.to_string() } else { url }),
                    None => {
                        let _ = self.advance_run(&run.id, RunState::Integrating, "pull request number unknown");
                        let _ = self.deliver_user(
                            agent,
                            &format!(
                                "ORCH: the pull request was opened ({u}) but its number could not be read, so it \
                                 can't be followed; the run is back to integrating. Check it on GitHub and ask the \
                                 user what to do with mcp__orch__ask_user."
                            ),
                        );
                        return None;
                    }
                }
            }
        };
        let u = url.as_str();
        run.pr_url = Some(url.clone());
        run.pr_number = Some(number);
        // A new PR (after one was closed): the old one's status goes.
        run.pr = None;
        let _ = self.db.upsert_run(&run);
        let _ = self.advance_run(&run.id, RunState::PrOpen, "pull request opened");
        self.run_event(&run.id, None, "pr.opened", json!({"number": number, "url": u}));
        Some(run.id)
    }

    /// Checks a run's PR now (just opened or pushed to: `pushed` is the head the app pushed).
    pub(crate) fn pr_check_now(&self, run_id: &str, pushed: Option<String>) {
        if let Err(e) = self.pr_check(run_id, true, now_ms(), pushed) {
            self.run_event(run_id, None, "pr.error", json!(format!("{e:#}")));
        }
    }

    // ------------------------------------------------------------ update push

    /// `request_pr` on a run whose PR is open: a gate to push the new
    /// commits to that PR (no new PR), a force push with lease when the
    /// branch was rebased. `title` / `body` are ignored.
    pub(crate) fn request_pr_update(&self, rec: &AgentRecord, run: &TeamRun) -> Result<String> {
        let n = run.pr_number.ok_or_else(|| anyhow!("this run has no pull request"))?;
        let wt = &rec.worktree;
        if orch_git::commit::rebasing(&orch_git::worktree::inspect(wt)) {
            bail!(
                "a rebase is in progress: finish it with mcp__orch__commit (or abort it with mcp__orch__rebase_base) first"
            );
        }
        let pr = run
            .pr
            .as_ref()
            .filter(|p| !p.head_sha.is_empty())
            .ok_or_else(|| anyhow!("PR #{n} was not checked yet: try again in a minute"))?;
        let base_ref = format!("origin/{}", pr.base);
        let branch = orch_git::worktree::current_branch(wt)?;
        // The head the user approves is the one pushed (not what the branch holds by then).
        let head = gh::rev_sha(wt, "HEAD").ok_or_else(|| anyhow!("{branch} has no commit"))?;
        // Someone else pushed to the PR: its commits must already be here or
        // in the base (GitHub's "Update branch"), or the user decides.
        let pushed = pr.pushed_sha.clone().unwrap_or_else(|| pr.head_sha.clone());
        let mut foreign_replaced = 0;
        if pr.head_sha != pushed {
            if gh::rev_sha(wt, &pr.head_sha).is_none() {
                bail!("PR #{n} has commits pushed outside the app that are not fetched yet: try again in a minute");
            }
            let foreign = orch_git::git(
                wt,
                &["log", "--max-count=50", "--format=%h %s", &pr.head_sha, "^HEAD", &format!("^{base_ref}"), "--"],
            )?;
            if !foreign.trim().is_empty() {
                bail!(
                    "PR #{n} has commits pushed outside the app that {branch} lacks:\n{}\nAsk the user what to do with \
                     mcp__orch__ask_user (list these commits): pushing would drop them.",
                    foreign.trim()
                );
            }
            foreign_replaced = orch_git::cleanup::try_ahead_behind(wt, &pr.head_sha, "HEAD").map_or(0, |(a, _)| a);
        }
        // The lease is the PR head as last seen: what the app pushed, or
        // foreign commits already in the base (checked above).
        let lease = pr.head_sha.clone();
        let force = !orch_git::cleanup::is_merged(wt, &lease, "HEAD");
        let commits = gh::commits_since(wt, if force { &base_ref } else { &lease })?;
        if commits.is_empty() || head == lease {
            bail!("nothing to push: {branch} has no commit that PR #{n} lacks");
        }
        let title = if force {
            format!("{}: force-push PR #{n} rebased onto {}", rec.id, pr.base)
        } else {
            format!("{}: push {} commit(s) to PR #{n} ({branch})", rec.id, commits.len())
        };
        let url = run.pr_url.clone().unwrap_or_default();
        let id = self.add_item(
            Some(&rec.id),
            "gate",
            title,
            json!({"action": "push_pr_update", "run": run.id, "branch": branch, "pr_number": n, "url": url,
                   "commits": commits.iter().map(|(sha, subject)| json!({"sha": sha, "subject": subject})).collect::<Vec<_>>(),
                   "force": force, "lease_sha": lease, "foreign_replaced": foreign_replaced, "head": head,
                   "base": pr.base, "worktree": wt}),
            PRIORITY_GATE,
        )?;
        self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "pr_update_requested",
            Some(&branch),
            "pending",
            json!({"item": id, "pr_number": n, "commits": commits.len(), "force": force,
                   "foreign_replaced": foreign_replaced}),
        )?;
        let what = if force {
            format!("force-push {branch} (rebased onto {}) to PR #{n}", pr.base)
        } else {
            format!("push {} commit(s) of {branch} to PR #{n}", commits.len())
        };
        Ok(format!(
            "Requested: Inbox item {id} asks the user to {what}. You will be told the result; there is nothing else to do for it now."
        ))
    }

    /// After an update gate is approved: push with the user's credentials (a
    /// fast-forward), check the PR at once, tell the lead. The run stays `pr_open`.
    pub(crate) fn push_pr_update(&self, item: u64, p: &Value) {
        let agent = self.store.read(|s| s.inbox.iter().find(|i| i.id == item).and_then(|i| i.agent_id.clone()));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (wt, branch, run_id) = (PathBuf::from(s("worktree")), s("branch"), s("run"));
        let n = p["pr_number"].as_u64().unwrap_or(0);
        let commits = p["commits"].as_array().map_or(0, Vec::len);
        let (head, force) = (s("head"), p["force"].as_bool().unwrap_or(false));
        // A rebased branch replaces the PR's history only while the PR is
        // still at the head the user saw (never a bare --force).
        let r = if force {
            gh::push_with_lease(&wt, "origin", &head, &branch, &s("lease_sha"))
        } else {
            gh::push_rev(&wt, "origin", &head, &branch)
        };
        let _ = self.db.audit(
            "core",
            agent.as_deref(),
            "pr_update",
            Some(&branch),
            if r.is_ok() { "ok" } else { "failed" },
            json!({"item": item, "pr_number": n, "commits": commits, "force": force,
                   "result": r.as_ref().map_err(|e| format!("{e:#}"))}),
        );
        let msg = match &r {
            Ok(()) => {
                self.run_event(&run_id, None, "pr.pushed", json!({"number": n, "commits": commits, "force": force}));
                format!(
                    "ORCH: the user approved; pushed {commits} commit(s) to PR #{n}. CI restarts; you will be told the result."
                )
            }
            Err(e) if force => format!(
                "ORCH: the user approved, but the force push to PR #{n} was refused: the PR moved since it was \
                 asked (someone pushed to it). Nothing was pushed. Run mcp__orch__request_pr again once the PR is \
                 checked ({e:#})."
            ),
            Err(e) => format!("ORCH: the user approved, but pushing to PR #{n} failed: {e:#}"),
        };
        if let Some(a) = agent {
            let _ = self.deliver_user(&a, &msg);
        }
        if r.is_ok() {
            self.pr_check_now(&run_id, Some(head));
        }
    }

    /// The run of a team agent when its PR is open (paused or deferred included).
    pub(crate) fn open_pr_run(&self, rec: &AgentRecord) -> Option<TeamRun> {
        let seat = rec.team.as_ref()?;
        self.db.run(&seat.run_id).ok().flatten().filter(|r| effective(r) == RunState::PrOpen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_cfg;
    use crate::store::TurnStatus;
    use crate::team::{GhFactory, LaunchSpec};
    use orch_git::fixtures::{commit_file, git as g, scratch_repo};
    use orch_protocol::{InboxDecision, McpMsg};
    use orch_store::{CiSummary, PrPosition};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    const LEAD: &str = "fx-lead-1";
    const MIN: u64 = 60_000;

    /// GitHub as the fake `gh` sees it: the PR's state, mergeability and
    /// checks; its head is the branch in the bare `origin`.
    struct Hub {
        origin: PathBuf,
        state: String,
        mergeable: String,
        rollup: Value,
        fail: bool,
        created: u64,
        views: u32,
        /// `gh pr create` prints no `…/pull/N`.
        no_url: bool,
        /// `gh pr view <branch>` finds no PR.
        no_lookup: bool,
    }

    struct FakeGh(Arc<Mutex<Hub>>);

    impl orch_tasks::github::Gh for FakeGh {
        fn run(&self, args: &[String]) -> Result<String> {
            let mut h = self.0.lock().unwrap();
            match (args[0].as_str(), args[1].as_str()) {
                ("pr", "create") => {
                    h.created += 1;
                    if h.no_url {
                        return Ok("Creating pull request…\n".into());
                    }
                    Ok(format!("Creating…\nhttps://github.com/o/r/pull/{}\n", 11 + h.created))
                }
                ("pr", "view") if args[2].parse::<u64>().is_err() => {
                    if h.no_lookup {
                        bail!("no pull requests found for branch {:?}", args[2]);
                    }
                    let n = 11 + h.created;
                    Ok(json!({"number": n, "url": format!("https://github.com/o/r/pull/{n}")}).to_string())
                }
                ("pr", "view") => {
                    h.views += 1;
                    if h.fail {
                        bail!("gh: HTTP 502");
                    }
                    let head = g(&h.origin, &["rev-parse", "orch/fx/integration"]).trim().to_string();
                    Ok(json!({"number": args[2].parse::<u64>()?, "url": format!("https://github.com/o/r/pull/{}", args[2]),
                              "state": h.state, "headRefOid": head, "baseRefName": "main", "mergeable": h.mergeable,
                              "mergeStateStatus": "CLEAN", "statusCheckRollup": h.rollup})
                    .to_string())
                }
                _ => bail!("unexpected gh {args:?}"),
            }
        }
    }

    #[derive(Debug, Default)]
    struct Net(AtomicBool);

    impl orch_git::fetch::Conditions for Net {
        fn battery(&self) -> Option<u8> {
            None
        }
        fn online(&self) -> bool {
            !self.0.load(Ordering::SeqCst)
        }
    }

    struct Fx {
        _d: tempfile::TempDir,
        _rd: tempfile::TempDir,
        rt: Arc<Runtime>,
        hub: Arc<Mutex<Hub>>,
        offline: Arc<Net>,
        run: String,
        /// A clone of `origin` to move `main` from.
        other: PathBuf,
    }

    fn check(rollup: &[(&str, &str, &str)]) -> Value {
        json!(
            rollup
                .iter()
                .map(|(n, st, c)| json!({"__typename": "CheckRun", "name": n, "workflowName": "ci", "status": st,
                                     "conclusion": c, "detailsUrl": format!("https://ci/{n}")}))
                .collect::<Vec<_>>()
        )
    }

    fn call(rt: &Runtime, agent: &str, tool: &str, args: Value) -> (bool, String) {
        let (err, text, _) = rt
            .on_mcp(&McpMsg { agent_id: agent.into(), tool: tool.into(), arguments: args, deadline_ms: 10_000 }, None);
        (err, text)
    }

    fn ok(rt: &Runtime, agent: &str, tool: &str, args: Value) -> String {
        let (err, text) = call(rt, agent, tool, args);
        assert!(!err, "{agent} {tool}: {text}");
        text
    }

    fn commit(rt: &Runtime, file: &str) {
        let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
        std::fs::write(wt.join(file), file).unwrap();
        ok(rt, LEAD, "commit", json!({"message": format!("add {file}")}));
    }

    /// The lead's queued turns that contain `needle`.
    fn told(rt: &Runtime, needle: &str) -> usize {
        rt.store.read(|s| {
            s.turns
                .iter()
                .filter(|t| t.agent_id == LEAD && t.status == TurnStatus::Queued && t.body.contains(needle))
                .count()
        })
    }

    fn run(fx: &Fx) -> TeamRun {
        fx.rt.db.run(&fx.run).unwrap().unwrap()
    }

    fn pending_gate(rt: &Runtime) -> Option<orch_store::InboxItem> {
        rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned())
    }

    fn wait(what: &str, f: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The run's events of `kind`, oldest first.
    fn events(fx: &Fx, kind: &str) -> Vec<Value> {
        fx.rt.db.flush().unwrap(); // events are written in the background
        let mut v: Vec<Value> = fx
            .rt
            .db
            .run_events(&fx.run, 500)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.detail)
            .collect();
        v.reverse();
        v
    }

    /// A feature run at its integration step, with a bare `origin`.
    fn integrating() -> Fx {
        let d = tempfile::tempdir().unwrap();
        let (rd, repo) = scratch_repo();
        let origin = rd.path().join("origin.git");
        g(rd.path(), &["clone", "-q", "--bare", repo.to_str().unwrap(), origin.to_str().unwrap()]);
        g(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        g(&repo, &["fetch", "-q", "origin"]);
        let other = rd.path().join("other");
        g(rd.path(), &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()]);
        let hub = Arc::new(Mutex::new(Hub {
            origin,
            state: "OPEN".into(),
            mergeable: "MERGEABLE".into(),
            rollup: check(&[("test", "QUEUED", "")]),
            fail: false,
            created: 0,
            views: 0,
            no_url: false,
            no_lookup: false,
        }));
        let mut cfg = test_cfg(d.path(), false);
        let h = hub.clone();
        cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(h.clone())))));
        let offline = Arc::new(Net::default());
        cfg.conditions = offline.clone();
        let rt = Runtime::new(cfg).unwrap();
        let p = orch_git::project::register(&rt.db, &repo).unwrap();
        let (roles, team) = orch_team::presets::bugfix_squad();
        for r in &roles {
            rt.db.upsert_role(r, 1).unwrap();
        }
        rt.db.save_team(&team, &roles, 1).unwrap();
        let run = rt
            .launch(&LaunchSpec {
                project_id: p.id,
                team_id: "bugfix-squad".into(),
                slug: "fx".into(),
                title: "Fix X".into(),
                ..Default::default()
            })
            .unwrap();
        let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
        std::fs::create_dir_all(wt.join(".orchestrator/features/fx")).unwrap();
        std::fs::write(wt.join(".orchestrator/features/fx/spec.md"), "# Fix X\n").unwrap();
        ok(&rt, LEAD, "commit", json!({"message": "spec"}));
        ok(&rt, LEAD, "request_spec_approval", json!({"summary": "s"}));
        rt.decide(pending_gate(&rt).unwrap().id, InboxDecision::Approve, None, None).unwrap();
        rt.set_run_state(&run.id, RunState::Integrating, "test").unwrap();
        Fx { _d: d, _rd: rd, rt, hub, offline, run: run.id, other }
    }

    /// `request_pr` approved: the PR opened by the fake `gh`.
    fn open(fx: &Fx) {
        ok(&fx.rt, LEAD, "request_pr", json!({"title": "Fix X"}));
        assert_eq!(run(fx).status, RunState::PrGate);
        fx.rt.decide(pending_gate(&fx.rt).unwrap().id, InboxDecision::Approve, None, None).unwrap();
        wait("the PR opened", || {
            let r = run(fx);
            r.status == RunState::PrOpen && r.pr.is_some()
        });
    }

    fn main_moves(fx: &Fx, file: &str) {
        commit_file(&fx.other, file, file, &format!("main: {file}"));
        g(&fx.other, &["push", "-q", "origin", "main"]);
    }

    #[test]
    fn an_opened_pr_is_watched_and_each_change_told_once() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        let r = run(&fx);
        assert_eq!((r.status, r.pr_number), (RunState::PrOpen, Some(12)));
        let pr = r.pr.clone().unwrap();
        let head = g(&fx.hub.lock().unwrap().origin, &["rev-parse", "orch/fx/integration"]).trim().to_string();
        assert_eq!(
            (pr.state.as_str(), pr.head_sha.as_str(), pr.pushed_sha.as_deref()),
            ("open", head.as_str(), Some(head.as_str()))
        );
        assert_eq!((pr.ci.overall.as_str(), pr.ci.done, pr.ci.total), ("waiting", 0, 1));
        assert_eq!(
            (pr.position.status.as_str(), pr.position.behind, pr.unpushed, pr.rewritten),
            ("up_to_date", 0, 0, false)
        );
        assert!(pr.position.base_sha.is_some());
        assert_eq!(events(&fx, "pr.opened")[0]["number"], 12);
        assert!(events(&fx, "run.state").iter().any(|e| e["why"] == "pull request opened" && e["to"] == "pr_open"));
        // The lead may create tasks to fix the PR.
        ok(rt, LEAD, "create_task", json!({"title": "Fix CI", "node": "back"}));

        // CI fails: told once, not again on the next ticks nor with a fresh watcher clock (a restart).
        let t0 = now_ms();
        fx.hub.lock().unwrap().rollup = check(&[("test", "COMPLETED", "FAILURE"), ("lint", "COMPLETED", "SUCCESS")]);
        rt.pr_tick(t0 + MIN);
        assert_eq!(told(rt, "ORCH: CI fails on PR #12: ci / test (failure, https://ci/test)"), 1);
        rt.pr_tick(t0 + 2 * MIN);
        rt.pr_watch.lock().unwrap().clear();
        rt.pr_tick(t0 + 3 * MIN);
        assert_eq!(told(rt, "CI fails"), 1, "the stored status is compared: nothing repeated");
        let st = events(&fx, "pr.status");
        assert_eq!(
            st.last().unwrap()["ci"],
            json!({"from": "waiting", "to": "error", "failed_jobs": [{"name": "test", "workflow": "ci"}]})
        );
        // Green again: on the timeline only.
        fx.hub.lock().unwrap().rollup = check(&[("test", "COMPLETED", "SUCCESS")]);
        rt.pr_tick(t0 + 4 * MIN);
        assert_eq!(events(&fx, "pr.status").last().unwrap()["ci"], json!({"from": "error", "to": "ok"}));
        assert_eq!(run(&fx).pr.unwrap().ci.overall, "ok");

        // main moves: seen once the base is fetched (at most every 5 min), told once per base head.
        main_moves(&fx, "m1.txt");
        rt.pr_tick(t0 + 4 * MIN + 1);
        assert_eq!(run(&fx).pr.unwrap().position.status, "up_to_date", "the base is not fetched yet");
        rt.pr_tick(t0 + 10 * MIN);
        let pos = run(&fx).pr.unwrap().position;
        assert_eq!((pos.status.as_str(), pos.behind), ("behind", 1));
        assert_eq!(told(rt, "ORCH: PR #12 is behind main by 1 commit(s). Call mcp__orch__rebase_base"), 1);
        rt.pr_tick(t0 + 16 * MIN);
        assert_eq!(told(rt, "is behind main"), 1, "same base head: not told again");
        main_moves(&fx, "m2.txt");
        rt.pr_tick(t0 + 22 * MIN);
        assert_eq!(told(rt, "is behind main by 2"), 1, "a new base head: told again");

        // Conflicts: told once.
        fx.hub.lock().unwrap().mergeable = "CONFLICTING".into();
        rt.pr_tick(t0 + 23 * MIN);
        rt.pr_tick(t0 + 24 * MIN);
        assert_eq!(told(rt, "is in conflict with main"), 1);

        // gh fails: the status stays (stale, with the error); the run does not move.
        fx.hub.lock().unwrap().fail = true;
        rt.pr_tick(t0 + 25 * MIN);
        let r = run(&fx);
        assert_eq!(r.status, RunState::PrOpen);
        let pr = r.pr.unwrap();
        assert!(
            pr.error.unwrap().contains("502") && pr.position.status == "conflicts" && pr.checked_at_ms == t0 + 25 * MIN
        );
        fx.hub.lock().unwrap().fail = false;

        // Offline: no gh call; a refresh is refused.
        fx.offline.0.store(true, Ordering::SeqCst);
        let views = fx.hub.lock().unwrap().views;
        rt.pr_tick(t0 + 26 * MIN);
        assert_eq!(fx.hub.lock().unwrap().views, views);
        assert!(rt.pr_refresh("fx").unwrap_err().to_string().contains("offline"));
        fx.offline.0.store(false, Ordering::SeqCst);
        let r = rt.pr_refresh("fx").unwrap();
        assert_eq!((fx.hub.lock().unwrap().views, r.pr.unwrap().error), (views + 1, None));
        let json = serde_json::to_value(run(&fx)).unwrap();
        assert_eq!((json["pr_number"].clone(), json["pr"]["ci"]["overall"].clone()), (json!(12), json!("ok")));
    }

    #[test]
    fn new_commits_go_to_the_open_pr_through_an_update_gate() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        assert!(call(rt, LEAD, "request_pr", json!({})).1.contains("nothing to push"));
        commit(rt, "fix.rs");
        let text = ok(rt, LEAD, "request_pr", json!({}));
        assert!(text.contains("push 1 commit(s)") && text.contains("PR #12"), "{text}");
        assert_eq!(run(&fx).status, RunState::PrOpen, "an update leaves the run pr_open");
        assert!(call(rt, LEAD, "request_pr", json!({})).1.contains("already waiting"));
        let gate = pending_gate(rt).unwrap();
        let pushed = run(&fx).pr.unwrap().pushed_sha.unwrap();
        assert_eq!(gate.payload["action"], "push_pr_update");
        assert_eq!(gate.body, format!("{LEAD}: push 1 commit(s) to PR #12 (orch/fx/integration)"));
        assert_eq!((gate.payload["force"].clone(), gate.payload["lease_sha"].clone()), (json!(false), json!(pushed)));
        assert_eq!(gate.payload["commits"][0]["subject"], "add fix.rs");
        assert_eq!((gate.payload["pr_number"].clone(), gate.payload["base"].clone()), (json!(12), json!("main")));
        assert_eq!(run(&fx).pr.unwrap().unpushed, 0, "not checked since the commit");
        rt.pr_refresh("fx").unwrap();
        assert_eq!(run(&fx).pr.unwrap().unpushed, 1);

        // Rejected: the run stays; then asked again and approved.
        rt.decide(gate.id, InboxDecision::Reject, None, Some("later")).unwrap();
        assert_eq!(run(&fx).status, RunState::PrOpen);
        assert_eq!(told(rt, "ORCH: the user declined pushing to PR #12. They said: later"), 1);
        ok(rt, LEAD, "request_pr", json!({}));
        rt.decide(pending_gate(rt).unwrap().id, InboxDecision::Approve, None, None).unwrap();
        let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
        let local = g(&wt, &["rev-parse", "HEAD"]).trim().to_string();
        // The lead is told, then the PR is checked.
        wait("the update pushed", || {
            !events(&fx, "pr.pushed").is_empty()
                && told(rt, "pushed 1 commit(s)") == 1
                && run(&fx).pr.unwrap().pushed_sha.as_deref() == Some(local.as_str())
        });
        assert_eq!(g(&fx.hub.lock().unwrap().origin, &["rev-parse", "orch/fx/integration"]).trim(), local);
        let r = run(&fx);
        let pr = r.pr.unwrap();
        assert_eq!(
            (r.status, pr.pushed_sha.as_deref(), pr.head_sha.as_str(), pr.unpushed),
            (RunState::PrOpen, Some(local.as_str()), local.as_str(), 0)
        );
        assert_eq!(events(&fx, "pr.pushed")[0], json!({"number": 12, "commits": 1, "force": false}));
        assert_eq!(told(rt, "ORCH: the user approved; pushed 1 commit(s) to PR #12. CI restarts"), 1);
        let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
        assert!(audit.iter().any(|a| a.action == "pr_update" && a.decision == "ok"));
        assert!(call(rt, LEAD, "request_pr", json!({})).1.contains("nothing to push"));
    }

    /// A paused run whose PR merges ends done once resumed; a pending update gate is withdrawn.
    #[test]
    fn a_merged_pr_ends_the_run_even_while_paused() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        commit(rt, "late.rs");
        ok(rt, LEAD, "request_pr", json!({}));
        let gate = pending_gate(rt).unwrap();
        rt.run_pause(&fx.run).unwrap();
        fx.hub.lock().unwrap().state = "MERGED".into();
        let t0 = now_ms();
        rt.pr_tick(t0 + MIN);
        rt.pr_tick(t0 + 2 * MIN);
        let r = run(&fx);
        assert_eq!((r.status, r.resume_status), (RunState::Done, None), "a pause does not hold the merge back");
        assert!(r.ended_at_ms.is_some());
        assert_eq!(told(rt, "ORCH: PR #12 was merged. The feature is done; end your turn."), 1);
        assert!(
            rt.store.read(|s| !s.inbox.iter().any(|i| i.id == gate.id && i.pending())),
            "the update gate is withdrawn"
        );
        let info: Vec<_> = rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "info").cloned().collect());
        assert_eq!(info.len(), 1);
        assert_eq!(
            (info[0].body.as_str(), info[0].payload["action"].clone(), info[0].payload["pr_number"].clone()),
            ("Feature fx merged", json!("pr_merged"), json!(12))
        );
        assert_eq!(events(&fx, "pr.merged"), [json!({"number": 12})]);
        assert!(rt.run_resume(&fx.run).is_err(), "nothing to resume");
        assert!(call(rt, LEAD, "create_task", json!({"title": "x", "node": "back"})).0, "the team tools refuse work");
        // A refresh of the done run (its status forgotten, as a run from before) stores, tells nothing.
        let mut r = run(&fx);
        r.pr = None;
        rt.db.upsert_run(&r).unwrap();
        rt.pr_refresh("fx").unwrap();
        assert_eq!(told(rt, "was merged"), 1);
        assert_eq!(rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "info").count()), 1);
        assert_eq!(run(&fx).pr.unwrap().state, "merged");
    }

    #[test]
    fn a_closed_pr_goes_back_to_integrating_and_a_new_one_can_open() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        fx.hub.lock().unwrap().state = "CLOSED".into();
        rt.pr_tick(now_ms() + MIN);
        let r = run(&fx);
        assert_eq!(r.status, RunState::Integrating);
        assert!(events(&fx, "run.state").iter().any(|e| e["why"] == "pull request closed"));
        assert_eq!(told(rt, "ORCH: PR #12 was closed without being merged"), 1);
        assert!(rt.store.read(|s| s.inbox.iter().any(|i| i.kind == "info" && i.payload["action"] == "pr_closed")));
        rt.pr_tick(now_ms() + 2 * MIN);
        assert_eq!(fx.hub.lock().unwrap().views, 2, "an integrating run is not watched");
        fx.hub.lock().unwrap().state = "OPEN".into();
        open(&fx);
        let r = run(&fx);
        assert_eq!(
            (r.status, r.pr_number, r.pr_url.as_deref()),
            (RunState::PrOpen, Some(13), Some("https://github.com/o/r/pull/13"))
        );
        assert_eq!(r.pr.unwrap().state, "open");
    }

    fn st(ci: &str, pos: &str, base_sha: &str, head: &str) -> PrStatus {
        PrStatus {
            state: "open".into(),
            head_sha: head.into(),
            base: "main".into(),
            ci: CiSummary {
                overall: ci.into(),
                jobs: vec![CiJob {
                    name: "t".into(),
                    workflow: None,
                    state: if ci == "error" { "failure" } else { "success" }.into(),
                    url: None,
                }],
                ..Default::default()
            },
            position: PrPosition { status: pos.into(), behind: 1, ahead: 1, base_sha: Some(base_sha.into()) },
            ..Default::default()
        }
    }

    #[test]
    fn changes_against_the_stored_status() {
        let a = st("waiting", "up_to_date", "b1", "h1");
        assert_eq!(pr_changes(None, &a), vec![], "first check: nothing decisive");
        assert!(pr_changes(Some(&a), &a).is_empty());
        let ci = |c: &[PrChange]| c.iter().filter(|c| matches!(c, PrChange::Ci { .. })).count();
        let err = st("error", "up_to_date", "b1", "h1");
        assert!(
            matches!(&pr_changes(Some(&a), &err)[0], PrChange::Ci { to, failed, .. } if to == "error" && failed.len() == 1)
        );
        assert_eq!(ci(&pr_changes(Some(&err), &err)), 0);
        assert_eq!(ci(&pr_changes(Some(&err), &st("error", "up_to_date", "b1", "h2"))), 1, "fails again on a new head");
        assert_eq!(ci(&pr_changes(Some(&a), &st("running", "up_to_date", "b1", "h1"))), 0, "running: no event");
        assert_eq!(
            ci(&pr_changes(Some(&st("running", "up_to_date", "b1", "h1")), &st("ok", "up_to_date", "b1", "h1"))),
            1,
            "any change to ok"
        );
        let behind = st("waiting", "behind", "b1", "h1");
        let told = |c: Vec<PrChange>| c.iter().any(|c| matches!(c, PrChange::Position { tell: true, .. }));
        assert!(told(pr_changes(Some(&a), &behind)));
        assert!(!told(pr_changes(Some(&behind), &behind)));
        assert!(told(pr_changes(Some(&behind), &st("waiting", "behind", "b2", "h1"))));
        let computing = st("waiting", "unknown", "b1", "h1");
        assert!(!told(pr_changes(Some(&computing), &behind)), "behind, unknown, behind on one base head: told once");
        let never_seen = PrStatus { error: Some("502".into()), ..Default::default() };
        assert_eq!(pr_changes(Some(&never_seen), &a), pr_changes(None, &a), "a failed first check was never seen");
        assert!(told(pr_changes(Some(&behind), &st("waiting", "conflicts", "b1", "h1"))));
        assert!(
            !told(pr_changes(Some(&behind), &a)) && !pr_changes(Some(&behind), &a).is_empty(),
            "back up to date: an event only"
        );
        let merged = PrStatus { state: "merged".into(), ..a.clone() };
        assert_eq!(pr_changes(Some(&a), &merged), vec![PrChange::Merged]);
        assert!(pr_changes(Some(&merged), &merged).is_empty());
        assert_eq!(pr_changes(Some(&a), &PrStatus { state: "closed".into(), ..a.clone() }), vec![PrChange::Closed]);
    }
}
