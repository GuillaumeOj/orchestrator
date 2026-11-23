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
use orch_store::{CiJob, Feature, PrMode, PrStatus, TeamRun};
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
pub(crate) const FETCH_DEADLINE: Duration = Duration::from_secs(120);

/// The watcher's own clock: when each run last fetched. Held for a whole
/// check, so two checks of the PRs never interleave (tick, refresh, push).
pub(crate) type PrWatch = BTreeMap<String, u64>;

/// What changed between two checks of a PR (see [`pr_changes`]).
#[derive(Clone, Debug, PartialEq)]
pub enum PrChange {
    Merged,
    Closed,
    /// Open again after it was seen closed (a linked PR, F5).
    Reopened,
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
    if old_state == "closed" {
        out.push(PrChange::Reopened);
    }
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

/// A PR as [`Runtime::read_pr`] saw it: its status and URL, or (gh failed)
/// the last good status marked stale.
enum PrRead {
    Seen(PrStatus, String),
    Failed(PrStatus),
}

/// `PR #34 (issue #12)`; the single PR keeps its plain `PR #12`.
pub(crate) fn pr_label(n: u64, issue: Option<&str>) -> String {
    match issue {
        Some(i) => format!("PR #{n} (issue {i})"),
        None => format!("PR #{n}"),
    }
}

/// The issues whose latest PR is open (or not checked yet), in `prs` order.
pub(crate) fn open_issues(run: &TeamRun) -> Vec<String> {
    run.latest_prs().into_iter().filter(|p| p.is_open()).map(|p| p.issue.clone()).collect()
}

fn job_line(j: &CiJob) -> String {
    let wf = j.workflow.as_deref().map(|w| format!("{w} / ")).unwrap_or_default();
    let url = j.url.as_deref().map(|u| format!(", {u}")).unwrap_or_default();
    format!("{wf}{} ({}{url})", j.name, j.state)
}

impl Runtime {
    // ------------------------------------------------------------ gh seam

    /// `gh pr view` in `dir`; tests replace `gh` through `cfg.gh`.
    pub(crate) fn gh_pr_view(&self, dir: &Path, number: u64) -> Result<PrView> {
        match self.gh_override() {
            Some(g) => gh::parse_pr_view(&g.run(&gh::pr_view_args(number))?),
            None => gh::pr_view(dir, number),
        }
    }

    /// `gh pr view <branch>`: the number and URL of its PR.
    pub(crate) fn gh_pr_lookup(&self, dir: &Path, branch: &str) -> Result<(u64, String)> {
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
    /// each open PR of each `pr_open` run (one per issue in `per_issue`
    /// mode), paused and deferred runs included, and the PR of each linked
    /// run from its launch (§5.6). Nothing while offline or on low battery (FR-103).
    pub fn pr_tick(&self, now: u64) {
        if orch_git::fetch::blocked(&self.cfg.fetch, self.cfg.conditions.as_ref()).is_some() {
            return;
        }
        let runs = self.db.runs(None, true).unwrap_or_default();
        for r in runs.iter().filter(|r| effective(r) == RunState::PrOpen || self.linked_watch(r)) {
            if r.pr_number.is_some() {
                if let Err(e) = self.pr_check(&r.id, false, now, None) {
                    self.run_event(&r.id, None, "pr.error", json!(format!("{e:#}")));
                }
                continue;
            }
            for issue in open_issues(r) {
                if let Err(e) = self.pr_check_issue(&r.id, &issue, false, now, None) {
                    self.run_event(&r.id, None, "pr.error", json!({"issue": issue, "error": format!("{e:#}")}));
                }
            }
        }
    }

    /// A linked run not yet (or no longer) `pr_open`: its PR is still watched.
    fn linked_watch(&self, r: &TeamRun) -> bool {
        r.pr_number.is_some()
            && !effective(r).finished()
            && self.db.feature(&r.feature_id).ok().flatten().is_some_and(|f| f.pr_link.is_some())
    }

    /// `pr_refresh`: checks the open PRs of the feature's latest run now
    /// (the fetch throttle skipped, not the offline check). `feature` is an id or a slug.
    pub fn pr_refresh(&self, feature: &str) -> Result<TeamRun> {
        let f = self.feature_by_id_or_slug(feature)?;
        let run =
            self.db.runs(Some(&f.id), false)?.into_iter().next().ok_or_else(|| anyhow!("{} has no run", f.slug))?;
        if run.pr_number.is_none() && (f.pr_mode != PrMode::PerIssue || run.prs.is_empty()) {
            bail!("{} has no pull request", f.slug);
        }
        if !self.cfg.conditions.online() {
            bail!("offline: the pull request can't be checked");
        }
        if run.pr_number.is_some() {
            return self.pr_check(&run.id, true, now_ms(), None);
        }
        // Every open PR; with none open, each issue's last one is checked once more (a run from before).
        let open = open_issues(&run);
        let issues =
            if open.is_empty() { run.latest_prs().into_iter().map(|p| p.issue.clone()).collect() } else { open };
        let mut last = run;
        // One fetch covers every issue branch: only the first check forces it.
        for (k, i) in issues.iter().enumerate() {
            last = self.pr_check_issue(&last.id, i, k == 0, now_ms(), None)?;
        }
        Ok(last)
    }

    /// `gh pr view` and the fetch of the base and `heads` (when due under
    /// `key`, or `force_fetch`),
    /// then the status as seen, or the last good one marked stale (gh
    /// failed). `unpushed` / `rewritten` are left for the caller.
    #[allow(clippy::too_many_arguments)]
    fn read_pr(
        &self,
        watch: &mut PrWatch,
        key: &str,
        wt: &Path,
        n: u64,
        heads: &[String],
        old: Option<&PrStatus>,
        force_fetch: bool,
        now: u64,
        pushed: Option<String>,
    ) -> PrRead {
        let view = match self.gh_pr_view(wt, n) {
            Ok(v) => v,
            Err(e) => {
                let mut st = old.cloned().unwrap_or_default();
                st.error = Some(format!("{e:#}"));
                st.checked_at_ms = now;
                st.pushed_sha = pushed.or(st.pushed_sha);
                return PrRead::Failed(st);
            }
        };
        if force_fetch || watch.get(key).is_none_or(|t| now.saturating_sub(*t) >= FETCH_BASE_EVERY_MS) {
            // A failed fetch leaves the last counts (or `unknown`); tried again when due.
            let mut branches = vec![view.base.as_str()];
            branches.extend(heads.iter().map(String::as_str));
            let _ = gh::fetch_branches(wt, &branches, FETCH_DEADLINE);
            watch.insert(key.to_string(), now);
        }
        let base_ref = format!("origin/{}", view.base);
        let base_sha = gh::rev_sha(wt, &base_ref);
        let counts = base_sha
            .as_ref()
            .and_then(|_| orch_git::cleanup::try_ahead_behind(wt, &view.head_sha, &base_ref))
            .map(|(ahead, behind)| (behind, ahead));
        let pushed_sha = pushed.or_else(|| old.and_then(|o| o.pushed_sha.clone()));
        let st = PrStatus {
            state: view.state.clone(),
            head_sha: view.head_sha.clone(),
            base: view.base.clone(),
            ci: view.ci.clone(),
            review_decision: view.review_decision.clone(),
            position: gh::position(&view.mergeable, &view.merge_state, counts, base_sha),
            unpushed: 0,
            rewritten: false,
            pushed_sha,
            checked_at_ms: now,
            error: None,
            // Set by the caller (a linked feature's PR).
            linked: false,
            foreign_head: false,
            foreign_head_sha: None,
        };
        if st.state != "open" {
            watch.remove(key);
        }
        PrRead::Seen(st, view.url)
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
        let heads = [feature.integration_branch()];
        let (mut new, url) =
            match self.read_pr(&mut watch, run_id, &wt, n, &heads, old.as_ref(), force_fetch, now, pushed) {
                PrRead::Seen(st, url) => (st, url),
                PrRead::Failed(st) => {
                    // The last good status stays, marked stale; the run does not move.
                    self.db.set_run_pr(run_id, &st, None)?;
                    run.pr = Some(st);
                    return Ok(run);
                }
            };
        new.unpushed = orch_git::cleanup::try_ahead_behind(&wt, "HEAD", &new.head_sha).map_or(0, |(a, _)| a);
        new.rewritten = new.pushed_sha.as_deref().is_some_and(|p| !orch_git::cleanup::is_merged(&wt, p, "HEAD"));
        if let Some(l) = &feature.pr_link {
            // G6: a head that is neither what the app pushed (or linked at)
            // nor in the branch was pushed by someone else.
            let known = new.pushed_sha.as_deref().unwrap_or(&l.head_sha_at_link);
            new.linked = true;
            new.foreign_head = new.state == "open"
                && new.head_sha != known
                && !orch_git::cleanup::is_merged(&wt, &new.head_sha, "HEAD");
            new.foreign_head_sha = new.foreign_head.then(|| new.head_sha.clone());
        }
        // Only the PR columns: a pause or stop made while `gh` and the fetch ran stays.
        self.db.set_run_pr(run_id, &new, Some(&url).filter(|u| !u.is_empty()).map(String::as_str))?;
        drop(watch);
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let state = effective(&run);
        let changes = pr_changes(old.as_ref(), &new);
        // A refresh of a run no longer `pr_open` (done, failed, back to
        // integrating) stores the status but moves and tells nothing; a
        // linked run's PR is followed from launch, but before `pr_open`
        // only its end, closing and reopening count (§5.6).
        if state == RunState::PrOpen {
            self.pr_apply(&run, &feature, None, n, &changes);
        } else if feature.pr_link.is_some() && !state.finished() {
            let ends: Vec<PrChange> = changes
                .into_iter()
                .filter(|c| matches!(c, PrChange::Merged | PrChange::Closed | PrChange::Reopened))
                .collect();
            self.pr_apply(&run, &feature, None, n, &ends);
        }
        if new.foreign_head && !state.finished() && old.and_then(|o| o.foreign_head_sha) != new.foreign_head_sha {
            self.foreign_head_told(&run, &feature, n, &new.head_sha);
        }
        Ok(self.db.run(run_id)?.unwrap_or(run))
    }

    /// One ORCH turn per new foreign head of a linked PR (§5.6, G6).
    fn foreign_head_told(&self, run: &TeamRun, feature: &Feature, n: u64, head: &str) {
        let by = feature.pr_link.as_ref().and_then(|l| l.author.as_deref()).map(|a| format!(", probably by {a}"));
        self.run_event(&run.id, None, "pr.foreign_head", json!({"number": n, "head_sha": head}));
        // G4: rebase_base onto pr_head is open from planning on.
        let when = if matches!(effective(run), RunState::Draft | RunState::Spec | RunState::SpecGate) {
            "once the spec is approved and no task is in progress"
        } else {
            "once no task is in progress"
        };
        let text = format!(
            "ORCH: PR #{n}'s head moved to {} (pushed outside the app{}). Bring it in with mcp__orch__rebase_base \
             {{\"onto\": \"pr_head\"}} {when}, before asking for the push.",
            crate::inbox::short(head),
            by.unwrap_or_default()
        );
        if let Some(lead) = self.run_ctx(&run.id).ok().and_then(|cx| self.lead_of(&cx)) {
            let _ = self.deliver_kind(&lead.id, &text, TurnKind::Continue);
        }
    }

    /// [`Runtime::pr_check`] for the PR of `issue` (`per_issue` mode):
    /// `unpushed` / `rewritten` from the build. The run's one fetch takes
    /// the base and every open issue branch.
    pub(crate) fn pr_check_issue(
        &self,
        run_id: &str,
        issue: &str,
        force_fetch: bool,
        now: u64,
        pushed: Option<String>,
    ) -> Result<TeamRun> {
        let mut watch = self.pr_watch.lock().unwrap_or_else(|e| e.into_inner());
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let feature = self.db.feature(&run.feature_id)?.ok_or_else(|| anyhow!("no feature {}", run.feature_id))?;
        let p = run.latest_pr(issue).cloned().ok_or_else(|| anyhow!("issue {issue} has no pull request"))?;
        let wt = feature
            .integration_worktree
            .clone()
            .ok_or_else(|| anyhow!("{} has no integration worktree", feature.slug))?;
        let heads: Vec<String> = run
            .latest_prs()
            .into_iter()
            .filter(|o| o.is_open() || o.issue == issue)
            .map(|o| o.branch.clone())
            .collect();
        let (n, old) = (p.number, p.pr.clone());
        let new = match self.read_pr(&mut watch, run_id, &wt, n, &heads, old.as_ref(), force_fetch, now, pushed) {
            PrRead::Seen(mut st, _) => {
                (st.unpushed, st.rewritten) = self.issue_pr_drift(&run, &feature, &wt, &p);
                st
            }
            PrRead::Failed(st) => st,
        };
        let ok = new.error.is_none();
        let stored = new.clone();
        self.db.update_run_prs(run_id, move |prs| {
            if let Some(e) = prs.iter_mut().rev().find(|e| e.number == n) {
                e.pr = Some(stored);
            }
        })?;
        drop(watch);
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        if ok && effective(&run) == RunState::PrOpen {
            self.pr_apply(&run, &feature, Some(issue), n, &pr_changes(old.as_ref(), &new));
        }
        Ok(self.db.run(run_id)?.unwrap_or(run))
    }

    /// The timeline events, lead turns, run moves and Inbox items of
    /// `changes` to PR `n` (of `issue` in `per_issue` mode).
    fn pr_apply(&self, run: &TeamRun, feature: &Feature, issue: Option<&str>, n: u64, changes: &[PrChange]) {
        let entry = issue.and_then(|i| run.latest_pr(i));
        let pr = entry.map_or(run.pr.as_ref(), |e| e.pr.as_ref());
        let base = pr.map(|p| p.base.clone()).unwrap_or_else(|| feature.base.clone());
        let url = entry.map(|e| e.url.clone()).or_else(|| run.pr_url.clone()).unwrap_or_default();
        let label = pr_label(n, issue);
        let with_issue = |mut v: Value| {
            if let Some(i) = issue {
                v["issue"] = json!(i);
            }
            v
        };
        let mut status = with_issue(json!({"number": n, "base": base}));
        let mut tell: Vec<String> = vec![];
        for c in changes {
            match c {
                PrChange::Merged | PrChange::Closed if issue.is_some() => {
                    let what = if *c == PrChange::Merged { "merged" } else { "closed" };
                    self.run_event(&run.id, None, &format!("pr.{what}"), with_issue(json!({"number": n})));
                    self.withdraw_pr_updates(&run.id, issue, &format!("pull request {what}"));
                    let _ = self.add_item(
                        None,
                        "info",
                        format!("Feature {}: {label} {what}", feature.slug),
                        json!({"action": format!("pr_{what}"), "run": run.id, "feature": feature.id, "pr_number": n,
                               "url": url, "issue": issue}),
                        PRIORITY_ALERT,
                    );
                    tell.push(self.issue_pr_ended(
                        run,
                        feature,
                        issue.unwrap_or_default(),
                        &label,
                        *c == PrChange::Merged,
                    ));
                }
                PrChange::Reopened if issue.is_none() && feature.pr_link.is_some() => {
                    self.run_event(&run.id, None, "pr.reopened", json!({"number": n}));
                    // Closed once `pr_open` (the app had pushed): back to it, followed again.
                    let pushed = pr.is_some_and(|p| p.pushed_sha.is_some());
                    if pushed
                        && run.status == RunState::Integrating
                        && self.move_run(&run.id, RunState::PrOpen, "pull request reopened", false).is_ok()
                    {
                        tell.push(format!(
                            "ORCH: PR #{n} is open again: the run is back to pr_open; mcp__orch__request_pr pushes \
                             new commits to it."
                        ));
                    } else {
                        tell.push(format!(
                            "ORCH: PR #{n} is open again: mcp__orch__request_pr pushes the team's commits to it."
                        ));
                    }
                }
                PrChange::Reopened => {}
                PrChange::Merged | PrChange::Closed if feature.pr_link.is_some() => {
                    tell.push(self.linked_pr_ended(run, feature, n, &url, *c == PrChange::Merged));
                }
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
                        self.set_run_state(&run.id, to, why)
                    } else {
                        self.advance_run(&run.id, to, why)
                    };
                    if let Err(e) = moved {
                        self.run_event(&run.id, None, "team_error", json!(format!("{e:#}")));
                    }
                    self.run_event(&run.id, None, &format!("pr.{what}"), json!({"number": n}));
                    self.withdraw_pr_updates(&run.id, None, why);
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
                        let push = match issue {
                            Some(i) => format!("mcp__orch__request_pr {{\"issue\": \"{i}\"}}"),
                            None => "mcp__orch__request_pr".into(),
                        };
                        tell.push(format!(
                            "ORCH: CI fails on {label}: {}. Find whether the feature caused it; if so, fix it with a \
                             task (or yourself), merge, and call {push} to push the fix. If the failure \
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
                        let push = match issue {
                            Some(_) => "mcp__orch__request_pr for each issue PR",
                            None => "mcp__orch__request_pr",
                        };
                        tell.push(format!(
                            "ORCH: {label} is {what}. Call mcp__orch__rebase_base, resolve any conflicts, run the \
                             tests, then {push} to push."
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

    /// A linked PR merged (→ done, from any state) or closed outside the app
    /// (§5.5: a `pr_open` run goes back to integrating, any other keeps its
    /// state; `request_pr` is refused until it is reopened). The lead's turn.
    fn linked_pr_ended(&self, run: &TeamRun, feature: &Feature, n: u64, url: &str, merged: bool) -> String {
        let what = if merged { "merged" } else { "closed" };
        let why = format!("pull request {what}");
        let moved = if merged {
            // Linked: done from whatever step the team is at, a pause included.
            self.move_run(&run.id, RunState::Done, &why, false).map(|_| ())
        } else if effective(run) == RunState::PrOpen {
            self.advance_run(&run.id, RunState::Integrating, &why).map(|_| ())
        } else {
            Ok(())
        };
        if let Err(e) = moved {
            self.run_event(&run.id, None, "team_error", json!(format!("{e:#}")));
        }
        self.run_event(&run.id, None, &format!("pr.{what}"), json!({"number": n, "linked": true}));
        self.withdraw_pr_updates(&run.id, None, &why);
        if merged {
            // The run is done: a push asked before the merge has nothing left to push to.
            let run_id = run.id.as_str();
            self.close_items_where(
                |i| {
                    i.payload.get("action") == Some(&json!("push_pr"))
                        && i.payload.get("linked").is_some_and(Value::is_object)
                        && i.payload.get("run").and_then(Value::as_str) == Some(run_id)
                },
                &why,
            );
        }
        let title = if merged {
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
        if merged {
            format!("ORCH: PR #{n} was merged. The feature is done; end your turn.")
        } else {
            format!(
                "ORCH: PR #{n} was closed outside the app. Ask the user what they want with mcp__orch__ask_user; \
                 mcp__orch__request_pr is refused while it is closed (never open another pull request)."
            )
        }
    }

    /// Closes the pending update gates of a run's PR (merged or closed):
    /// in `per_issue` mode only those of `issue`.
    fn withdraw_pr_updates(&self, run_id: &str, issue: Option<&str>, why: &str) {
        self.close_items_where(
            |i| {
                i.payload.get("action") == Some(&json!("push_pr_update"))
                    && i.payload.get("run").and_then(Value::as_str) == Some(run_id)
                    && issue.is_none_or(|r| i.payload.get("issue").and_then(Value::as_str) == Some(r))
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
        // A new PR (after one was closed): the old one's status goes. A
        // linked PR is the same one, watched since launch: it stays.
        if !run.pr.as_ref().is_some_and(|p| p.linked) {
            run.pr = None;
        }
        let _ = self.db.upsert_run(&run);
        let _ = self.advance_run(&run.id, RunState::PrOpen, "pull request opened");
        self.run_event(&run.id, None, "pr.opened", json!({"number": number, "url": u}));
        Some(run.id)
    }

    /// Checks a run's PR now (just opened or pushed to: `pushed` is the head the app pushed).
    /// `issue`: the PR of that issue (`per_issue` mode).
    pub(crate) fn pr_check_now(&self, run_id: &str, issue: Option<&str>, pushed: Option<String>) {
        let r = match issue {
            Some(i) => self.pr_check_issue(run_id, i, true, now_ms(), pushed),
            None => self.pr_check(run_id, true, now_ms(), pushed),
        };
        if let Err(e) = r {
            let detail = match issue {
                Some(i) => json!({"issue": i, "error": format!("{e:#}")}),
                None => json!(format!("{e:#}")),
            };
            self.run_event(run_id, None, "pr.error", detail);
        }
    }

    // ------------------------------------------------------------ update push

    /// `request_pr` on a run whose PR is open: a gate to push the new
    /// commits to that PR (no new PR), a force push with lease when the
    /// branch was rebased. `title` / `body` are ignored.
    pub(crate) fn request_pr_update(&self, rec: &AgentRecord, run: &TeamRun) -> Result<String> {
        let n = run.pr_number.ok_or_else(|| anyhow!("this run has no pull request"))?;
        let wt = &rec.worktree;
        if orch_git::commit::rebase_in_progress(wt) {
            bail!(
                "a rebase is in progress: finish it with mcp__orch__commit (or abort it with mcp__orch__rebase_base) first"
            );
        }
        let pr = run
            .pr
            .as_ref()
            // A checked head: a commit sha (it becomes the lease and git arguments).
            .filter(|p| orch_git::git::valid_sha(&p.head_sha))
            .ok_or_else(|| anyhow!("PR #{n} was not checked yet: try again in a minute"))?;
        let base_ref = format!("origin/{}", pr.base);
        let branch = orch_git::worktree::current_branch(wt)?;
        // The head the user approves is the one pushed (not what the branch holds by then).
        let head = gh::rev_sha(wt, "HEAD").ok_or_else(|| anyhow!("{branch} has no commit"))?;
        // Someone else pushed to the PR: its commits (not the app's) must
        // already be here or in the base (GitHub's "Update branch"), or the user decides.
        let pushed = pr.pushed_sha.clone().unwrap_or_else(|| pr.head_sha.clone());
        let foreign_replaced = if pr.head_sha != pushed {
            self.foreign_pushes(wt, pr, &pushed, &branch, &pr_label(n, None), &["HEAD"])?
        } else {
            0
        };
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
            json!({"action": "push_pr_update", "run": run.id, "issue": null, "branch": branch, "pr_number": n, "url": url,
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
            "Requested: decision {id} asks the user to {what}. You will be told the result; there is nothing else to do for it now."
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
        let issue = p.get("issue").and_then(Value::as_str).map(str::to_string);
        let label = pr_label(n, issue.as_deref());
        let msg = match &r {
            Ok(()) => {
                let mut ev = json!({"number": n, "commits": commits, "force": force});
                if let Some(i) = &issue {
                    ev["issue"] = json!(i);
                    self.issue_pr_pushed(p);
                }
                self.run_event(&run_id, None, "pr.pushed", ev);
                format!(
                    "ORCH: the user approved; pushed {commits} commit(s) to {label}. CI restarts; you will be told the result."
                )
            }
            // The lease refused (git says "stale info"); any other failure is not "the PR moved".
            Err(e) if force && format!("{e:#}").contains("stale info") => format!(
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
            self.pr_check_now(&run_id, issue.as_deref(), Some(head));
        }
    }

    /// Someone else pushed to a PR (its head `st.head_sha` is not `pushed`,
    /// what the app pushed): their commits must already be in the base, or
    /// in `also` (refs that hold them), else the request is refused, as
    /// pushing would drop them. GitHub's "Update branch" merge is fine. How
    /// many commits the push replaces.
    pub(crate) fn foreign_pushes(
        &self,
        wt: &Path,
        st: &PrStatus,
        pushed: &str,
        branch: &str,
        label: &str,
        also: &[&str],
    ) -> Result<u32> {
        // The watcher fetches every few minutes: fetch the PR (and its base)
        // now, unless a check is fetching already (then it lands soon).
        if gh::rev_sha(wt, &st.head_sha).is_none()
            && let Ok(_watch) = self.pr_watch.try_lock()
        {
            let _ = gh::fetch_branches(wt, &[&st.base, branch], FETCH_DEADLINE);
        }
        if gh::rev_sha(wt, &st.head_sha).is_none() {
            bail!("{label} has commits pushed outside the app that could not be fetched: try again in a minute");
        }
        let mut not = vec![format!("^{pushed}"), format!("^origin/{}", st.base)];
        not.extend(also.iter().map(|r| format!("^{r}")));
        let range = || std::iter::once(st.head_sha.as_str()).chain(not.iter().map(String::as_str));
        let mut args = vec!["log", "--no-merges", "--max-count=50", "--format=%h %s"];
        args.extend(range());
        args.push("--");
        let theirs = orch_git::git(wt, &args)?;
        if !theirs.trim().is_empty() {
            bail!(
                "{label} has commits pushed outside the app that {branch} lacks:\n{}\nAsk the user what to do with \
                 mcp__orch__ask_user (list these commits): pushing would drop them.",
                theirs.trim()
            );
        }
        // What the push replaces: the commits pushed outside the app that are not in the base.
        let mut args = vec!["rev-list", "--count"];
        args.extend(range().filter(|r| *r != "^HEAD"));
        args.push("--");
        Ok(orch_git::git(wt, &args)?.trim().parse::<u32>().unwrap_or(0))
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
    fn a_background_check_raises_the_feature_topic() {
        let fx = integrating();
        open(&fx);
        let got = Arc::new(Mutex::new(Vec::<Value>::new()));
        let g = got.clone();
        fx.rt.subscribe_now(Arc::new(move |v| g.lock().unwrap().push(v.clone())));
        fx.rt.now_pump(false);
        let fid = fx.rt.db.run(&fx.run).unwrap().unwrap().feature_id;
        // Only what the check itself raises: the events of the setup are behind us.
        got.lock().unwrap().clear();
        fx.rt.pr_tick(now_ms() + MIN);
        fx.rt.now_pump(false);
        let ev = got.lock().unwrap().last().cloned().expect("the background check pushed a now event");
        assert!(ev["topics"].as_array().unwrap().contains(&json!(format!("feature:{fid}"))), "{ev}");
    }

    #[test]
    fn the_network_step_checks_open_prs_every_minute() {
        // The step both orchd and the app run on their network thread (the
        // app once had a fetch-only loop: no PR was ever checked there).
        let fx = integrating();
        open(&fx);
        let views = || fx.hub.lock().unwrap().views;
        let (mut net, t0) = (crate::server::NetClock::default(), std::time::Instant::now());
        let v0 = views();
        net.step(&fx.rt, t0);
        assert_eq!(views(), v0 + 1, "the first step checks the open PR");
        net.step(&fx.rt, t0 + PR_EVERY / 2);
        assert_eq!(views(), v0 + 1, "not again before PR_EVERY");
        net.step(&fx.rt, t0 + PR_EVERY);
        assert_eq!(views(), v0 + 2, "again once PR_EVERY has passed");
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

    fn commit_as(rt: &Runtime, agent: &str, file: &str, body: &str) {
        let wt = rt.store.read(|s| s.agents[agent].worktree.clone());
        std::fs::write(wt.join(file), body).unwrap();
        ok(rt, agent, "commit", json!({"message": format!("edit {file}")}));
    }

    fn head_of(rt: &Runtime, agent: &str) -> String {
        let wt = rt.store.read(|s| s.agents[agent].worktree.clone());
        g(&wt, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn origin_head(fx: &Fx) -> String {
        g(&fx.hub.lock().unwrap().origin, &["rev-parse", "orch/fx/integration"]).trim().to_string()
    }

    /// Someone else pushes to the PR branch from `fx.other`: a commit, or with
    /// `merge_base` GitHub's "Update branch" (a merge of origin/main).
    fn foreign_push(fx: &Fx, merge_base: bool) {
        let o = &fx.other;
        let id = ["-c", "user.name=O", "-c", "user.email=o@example.invalid"];
        g(o, &["fetch", "-q", "origin"]);
        g(o, &["checkout", "-q", "-B", "pr", "origin/orch/fx/integration"]);
        if merge_base {
            g(o, &[&id[..], &["merge", "-q", "--no-ff", "-m", "Merge main into the PR", "origin/main"]].concat());
        } else {
            std::fs::write(o.join("theirs.txt"), "theirs").unwrap();
            g(o, &["add", "theirs.txt"]);
            g(o, &[&id[..], &["commit", "-q", "-m", "their fix"]].concat());
        }
        g(o, &["push", "-q", "origin", "pr:orch/fx/integration"]);
        g(o, &["checkout", "-q", "main"]);
    }

    /// The `back` task: started, committed, reported and merged by the lead.
    fn back_task_merged(rt: &Runtime) {
        let back = "fx-back-1";
        ok(rt, LEAD, "create_task", json!({"title": "API", "node": "back"}));
        rt.team_tick();
        commit_as(rt, back, "api.rs", "fn api() {}");
        let t = rt.store.read(|s| s.agents[back].team.as_ref().unwrap().task.unwrap());
        assert!(call(rt, LEAD, "rebase_base", json!({})).1.contains("still open: T-"), "a task in progress");
        ok(rt, back, "report_done", json!({"summary": "api"}));
        ok(rt, back, "report_done", json!({"summary": "api", "review": {"fixed": [], "unfixed": []}}));
        ok(rt, LEAD, "merge", json!({"branch": "orch/fx/back-1", "task": t}));
    }

    fn approve(rt: &Runtime) -> orch_store::InboxItem {
        let gate = pending_gate(rt).unwrap();
        rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
        gate
    }

    #[test]
    fn rebase_base_moves_the_branches_and_the_pr_is_force_pushed_with_its_lease() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        back_task_merged(rt);
        assert!(call(rt, "fx-back-1", "rebase_base", json!({})).1.contains("not one of your tools"));
        main_moves(&fx, "m1.txt");
        // An update gate waiting: no rebase under it.
        ok(rt, LEAD, "request_pr", json!({}));
        assert!(call(rt, LEAD, "rebase_base", json!({})).1.contains("waiting for the user"));
        rt.decide(pending_gate(rt).unwrap().id, InboxDecision::Reject, None, None).unwrap();

        let from = head_of(rt, LEAD);
        let text = ok(rt, LEAD, "rebase_base", json!({}));
        assert!(
            text.contains("Rebased orch/fx/integration onto origin/main") && text.contains("moved orch/fx/back-1"),
            "{text}"
        );
        let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
        let to = head_of(rt, LEAD);
        assert!(orch_git::cleanup::is_merged(&wt, "origin/main", "HEAD") && wt.join("api.rs").is_file());
        assert_eq!(head_of(rt, "fx-back-1"), to, "the merged specialist branch follows");
        let ev = events(&fx, "git.rebase");
        assert_eq!(
            ev[0],
            json!({"from": from, "to": to, "onto": "origin/main", "base": "main", "base_commits": 1,
                   "branches_moved": ["orch/fx/back-1"]})
        );
        assert!(rt.db.audit_log(Some(LEAD), 50).unwrap().iter().any(|a| a.action == "rebase" && a.decision == "ok"));
        assert!(ok(rt, LEAD, "rebase_base", json!({})).contains("nothing to rebase"));
        wait("the PR checked after the rebase", || run(&fx).pr.is_some_and(|p| p.rewritten));

        // The update is a force push with the lease on what the app pushed.
        let pushed = run(&fx).pr.unwrap().pushed_sha.unwrap();
        ok(rt, LEAD, "request_pr", json!({}));
        let gate = approve(rt);
        assert_eq!(gate.body, format!("{LEAD}: force-push PR #12 rebased onto main"));
        assert_eq!((gate.payload["force"].clone(), gate.payload["lease_sha"].clone()), (json!(true), json!(pushed)));
        assert_eq!(gate.payload["foreign_replaced"], 0);
        let subjects: Vec<&str> =
            gate.payload["commits"].as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap()).collect();
        assert!(subjects.contains(&"Merge orch/fx/back-1") && subjects.contains(&"spec"), "{subjects:?}");
        wait("the force push", || !events(&fx, "pr.pushed").is_empty());
        assert_eq!(origin_head(&fx), to);
        assert_eq!(events(&fx, "pr.pushed")[0]["force"], true);
        let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
        assert!(audit.iter().any(|a| a.action == "pr_update" && a.decision == "ok" && a.detail["force"] == true));
        wait("checked again", || run(&fx).pr.is_some_and(|p| !p.rewritten && p.pushed_sha.as_deref() == Some(&to)));
        assert_eq!(run(&fx).status, RunState::PrOpen);
    }

    #[test]
    fn a_conflicting_rebase_continues_through_commit_and_a_moved_remote_refuses_the_force_push() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        commit_as(rt, LEAD, "shared.txt", "lead");
        commit_file(&fx.other, "shared.txt", "main", "main edits shared");
        g(&fx.other, &["push", "-q", "origin", "main"]);
        let before = head_of(rt, LEAD);
        let text = ok(rt, LEAD, "rebase_base", json!({}));
        assert!(text.contains("Conflicts in shared.txt (commit 2 of 2)"), "{text}");
        // No task starts on the old history while the rebase is stopped.
        ok(rt, LEAD, "create_task", json!({"title": "Later", "node": "back"}));
        rt.team_tick();
        assert!(rt.store.read(|s| !s.agents.contains_key("fx-back-1")), "dispatch waits for the rebase");
        assert!(call(rt, LEAD, "request_pr", json!({})).1.contains("rebase is in progress"));
        assert!(ok(rt, LEAD, "rebase_base", json!({"abort": true})).contains("Aborted"));
        assert_eq!(head_of(rt, LEAD), before);
        assert!(events(&fx, "git.rebase").is_empty(), "nothing moved on an abort");

        ok(rt, LEAD, "rebase_base", json!({}));
        let wt = rt.store.read(|s| s.agents[LEAD].worktree.clone());
        std::fs::write(wt.join("shared.txt"), "main + lead").unwrap();
        let text = ok(rt, LEAD, "commit", json!({"message": "resolve"}));
        assert!(text.starts_with("Rebased orch/fx/integration onto origin/main"), "{text}");
        assert_eq!(events(&fx, "git.rebase").len(), 1);
        assert_eq!(events(&fx, "git.rebase")[0]["onto"], "origin/main", "the recorded target, not a guess");
        assert!(
            rt.db.audit_log(Some(LEAD), 50).unwrap().iter().any(|a| a.action == "rebase" && a.decision == "conflict")
        );
        wait("the PR checked after the rebase", || run(&fx).pr.is_some_and(|p| p.rewritten));

        // Asked, then someone pushes to the PR before the user approves: the lease refuses.
        ok(rt, LEAD, "request_pr", json!({}));
        foreign_push(&fx, false);
        let theirs = origin_head(&fx);
        approve(rt);
        wait("the refusal told", || told(rt, "the force push to PR #12 was refused") == 1);
        assert_eq!(origin_head(&fx), theirs, "their commit is kept");
        let audit = rt.db.audit_log(Some(LEAD), 50).unwrap();
        assert!(audit.iter().any(|a| a.action == "pr_update" && a.decision == "failed" && a.detail["force"] == true));
        assert_eq!(run(&fx).status, RunState::PrOpen);
        // Their commit is not here nor in main: the lead must ask the user.
        rt.pr_refresh("fx").unwrap();
        let (err, text) = call(rt, LEAD, "request_pr", json!({}));
        assert!(err && text.contains("pushed outside the app") && text.contains("their fix"), "{text}");
        assert!(text.contains("mcp__orch__ask_user"));
    }

    #[test]
    fn an_update_branch_merge_is_replaced_by_the_rebase() {
        let fx = integrating();
        let rt = &fx.rt;
        open(&fx);
        main_moves(&fx, "m1.txt");
        foreign_push(&fx, true);
        let merge = origin_head(&fx);
        rt.pr_refresh("fx").unwrap();
        let pr = run(&fx).pr.unwrap();
        assert_eq!(pr.head_sha, merge);
        assert_ne!(pr.pushed_sha.as_deref(), Some(merge.as_str()));
        ok(rt, LEAD, "rebase_base", json!({}));
        wait("checked", || run(&fx).pr.is_some_and(|p| p.rewritten));
        ok(rt, LEAD, "request_pr", json!({}));
        let gate = approve(rt);
        assert_eq!(
            (
                gate.payload["force"].clone(),
                gate.payload["lease_sha"].clone(),
                gate.payload["foreign_replaced"].clone()
            ),
            (json!(true), json!(merge), json!(1))
        );
        wait("the force push", || !events(&fx, "pr.pushed").is_empty());
        assert_eq!(origin_head(&fx), head_of(rt, LEAD));
    }

    #[test]
    fn a_pr_url_without_a_number_is_looked_up_or_sends_the_run_back() {
        let fx = integrating();
        let rt = &fx.rt;
        fx.hub.lock().unwrap().no_url = true;
        open(&fx);
        assert_eq!(
            (run(&fx).pr_number, run(&fx).pr_url.as_deref()),
            (Some(12), Some("https://github.com/o/r/pull/12"))
        );

        let fx = integrating();
        let rt2 = &fx.rt;
        {
            let mut h = fx.hub.lock().unwrap();
            (h.no_url, h.no_lookup) = (true, true);
        }
        ok(rt2, LEAD, "request_pr", json!({"title": "Fix X"}));
        rt2.decide(pending_gate(rt2).unwrap().id, InboxDecision::Approve, None, None).unwrap();
        wait("the run back to integrating", || run(&fx).status == RunState::Integrating);
        assert_eq!(told(rt2, "its number could not be read"), 1);
        assert_eq!(run(&fx).pr_number, None);
        let _ = rt;
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
