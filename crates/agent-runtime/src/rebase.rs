//! The lead's `rebase_base` (feature improve-end-of-feature §3.4): brings
//! `origin/<base>` into the integration branch by a rebase run by the core
//! (hooks off, the lead's identity), continued by the lead's `commit` on a
//! conflict; once done, the specialists' merged branches follow it.
//! A feature on someone else's PR can also bring the PR head in (`onto:
//! "pr_head"`, allow-launch-team-on-update-pr §5.4).

use anyhow::{Result, anyhow, bail};
use governor::TurnKind;
use orch_git::commit::{self, RebaseOutcome};
use orch_git::gh;
use orch_team::{RunState, TaskStatus};
use serde_json::{Value, json};

use crate::pr::FETCH_DEADLINE;
use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::{RunCtx, effective};

impl Runtime {
    /// `rebase_base {abort?, onto?}` (lead only; checked by the caller).
    pub(crate) fn rebase_base(&self, cx: &RunCtx, rec: &AgentRecord, args: &Value) -> Result<(String, Value)> {
        let gitconfig = self.agent_gitconfig(&rec.id);
        if args.get("abort").and_then(Value::as_bool).unwrap_or(false) {
            let r = commit::rebase_abort(&rec.worktree, gitconfig.as_deref());
            return self.rebase_reply(cx, rec, r, &format!("origin/{}", cx.feature.base));
        }
        // The branch brought in: the base, or (a linked feature) the PR head.
        let pr_head = match args.get("onto").and_then(Value::as_str).unwrap_or("base") {
            "base" => false,
            "pr_head" => true,
            o => bail!("onto is \"base\" or \"pr_head\", not {o:?}"),
        };
        let target = match (&cx.feature.pr_link, pr_head) {
            (_, false) => cx.feature.base.clone(),
            (Some(l), true) => l.head_ref.clone(),
            (None, true) => {
                bail!("onto \"pr_head\" is for a feature on someone else's PR: this one has its own branch")
            }
        };
        let state = effective(&cx.run);
        if pr_head {
            // G4: a head moved while the tasks run can come in between them.
            if !matches!(state, RunState::Planning | RunState::Executing | RunState::Integrating | RunState::PrOpen) {
                bail!("the PR head is brought in once the spec is approved (this run is {})", state.label());
            }
        } else if !matches!(state, RunState::Integrating | RunState::PrOpen) {
            bail!("the base is brought in while integrating or once the PR is open (this run is {})", state.label());
        }
        if self.pending_pr_gate(&rec.id, Some(&cx.run.id)) {
            bail!("a pull request or update request is waiting for the user: the rebase would change what it pushes");
        }
        // A task in progress would be built on the old history.
        let busy: Vec<String> = self
            .db
            .tasks(None, None, Some(&cx.run.id))?
            .into_iter()
            .filter(|t| {
                !(t.status == TaskStatus::Done
                    || matches!(t.status, TaskStatus::Ready | TaskStatus::Backlog) && t.branch.is_none())
            })
            .map(|t| format!("{} ({})", t.handle(), t.status.label()))
            .collect();
        if !busy.is_empty() {
            bail!("every task must be merged first; still open: {}", busy.join(", "));
        }
        {
            // Not beside the PR watcher's own fetch in the same repo (ref locks).
            let _watch = self.pr_watch.lock().unwrap_or_else(|e| e.into_inner());
            // An explicit refspec: `origin/<head>` is updated even when the
            // remote's fetch refspec leaves the PR branch out.
            let spec = format!("+refs/heads/{target}:refs/remotes/origin/{target}");
            gh::fetch_branches(&rec.worktree, &[&spec], FETCH_DEADLINE)
                .map_err(|e| anyhow!("fetching origin/{target} failed: {e:#}"))?;
        }
        // Behind only (no team commit yet): git fast-forwards.
        let onto = format!("origin/{target}");
        let r = commit::rebase_worktree(&rec.worktree, &onto, gitconfig.as_deref());
        self.rebase_reply(cx, rec, r, &onto)
    }

    /// The lead's `commit` while its rebase is in progress: continues it.
    /// Any other agent's rebase (not one `rebase_base` started) is only
    /// continued: it never moves the run's branches.
    pub(crate) fn rebase_continue_for(&self, rec: &AgentRecord) -> Result<(String, Value)> {
        // Loaded before the rebase moves: a failure here leaves it untouched.
        let lead_cx = match rec.team.as_ref() {
            Some(seat) => Some(self.run_ctx(&seat.run_id)?).filter(|cx| cx.team.is_lead(&seat.node)),
            None => None,
        };
        // What the rebase replays onto: the PR head when it is that one's tip.
        let onto = lead_cx.as_ref().map(|cx| {
            let head = cx.feature.pr_link.as_ref().map(|l| format!("origin/{}", l.head_ref));
            match (head, commit::rebase_onto(&rec.worktree)) {
                // The PR head at the start, or an ancestor of it once fetched again.
                (Some(h), Some(sha))
                    if gh::rev_sha(&rec.worktree, &h)
                        .is_some_and(|tip| orch_git::cleanup::is_merged(&rec.worktree, &sha, &tip))
                        && !gh::rev_sha(&rec.worktree, &format!("origin/{}", cx.feature.base))
                            .is_some_and(|b| b == sha) =>
                {
                    h
                }
                _ => format!("origin/{}", cx.feature.base),
            }
        });
        let r = commit::rebase_continue(&rec.worktree, self.agent_gitconfig(&rec.id).as_deref());
        match lead_cx {
            Some(cx) => self.rebase_reply(&cx, rec, r, onto.as_deref().unwrap_or_default()),
            None => {
                let r = r?;
                Ok((format!("Rebase: {}", json!(r)), json!(r)))
            }
        }
    }

    /// Audits a rebase step and words its outcome for the lead; a finished
    /// rebase moves the specialists' branches ([`Runtime::after_rebase`]).
    fn rebase_reply(
        &self,
        cx: &RunCtx,
        rec: &AgentRecord,
        r: Result<RebaseOutcome>,
        base: &str,
    ) -> Result<(String, Value)> {
        let integ = cx.feature.integration_branch();
        let (decision, mut detail) = match &r {
            Ok(o) => (
                match o {
                    RebaseOutcome::Rebased { .. } => "ok",
                    RebaseOutcome::UpToDate => "up_to_date",
                    RebaseOutcome::Conflict { .. } => "conflict",
                    RebaseOutcome::Aborted => "aborted",
                },
                json!(o),
            ),
            Err(e) => ("failed", json!({"error": format!("{e:#}")})),
        };
        let moved = match &r {
            Ok(RebaseOutcome::Rebased { from, to }) => self.after_rebase(cx, rec, from, to, base),
            _ => vec![],
        };
        if !moved.is_empty() {
            detail["branches_moved"] = json!(moved);
        }
        let _ = self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "rebase",
            Some(&integ),
            decision,
            detail.clone(),
        );
        let text = match r? {
            RebaseOutcome::Rebased { to, .. } => format!(
                "Rebased {integ} onto {base} ({}){}. Run the tests, then call mcp__orch__request_pr to push.",
                &to[..to.len().min(12)],
                if moved.is_empty() { String::new() } else { format!("; moved {}", moved.join(", ")) }
            ),
            RebaseOutcome::UpToDate => format!("{integ} already contains {base}: nothing to rebase."),
            RebaseOutcome::Conflict { files, step, of } => format!(
                "Conflicts in {} (commit {step} of {of}). Fix them (remove the markers), then call mcp__orch__commit to \
                 continue the rebase; mcp__orch__rebase_base {{\"abort\": true}} gives up and restores the branch.",
                files.join(", ")
            ),
            RebaseOutcome::Aborted => format!("Aborted: {integ} is back where it was."),
        };
        Ok((text, detail))
    }

    /// After a finished rebase `from` → `to`: each specialist branch of the
    /// run whose work is all in `from` moves to `to` (its worktree clean),
    /// the others' owners are told to sync; `git.rebase` on the timeline; the
    /// open PR checked. Under the PR watcher's guard, so a check never
    /// interleaves. The moved branches.
    fn after_rebase(&self, cx: &RunCtx, lead: &AgentRecord, from: &str, to: &str, onto: &str) -> Vec<String> {
        let guard = self.pr_watch.lock().unwrap_or_else(|e| e.into_inner());
        let prefix = format!("orch/{}/", cx.feature.slug);
        let integ = cx.feature.integration_branch();
        let (mut moved, mut to_sync) = (vec![], vec![]);
        for a in self.run_agents(&cx.run.id).into_iter().filter(|a| a.id != lead.id) {
            let branch = self.branch_of(&a);
            // Only this run's specialist branches, never the integration branch.
            if !branch.starts_with(&prefix) || branch == integ {
                continue;
            }
            let merged = gh::rev_sha(&lead.worktree, &format!("refs/heads/{branch}"))
                .is_some_and(|h| orch_git::cleanup::is_merged(&lead.worktree, &h, from));
            // `reset_worktree` refuses a worktree on any other branch, or not clean.
            if merged
                && commit::reset_worktree(&a.worktree, &branch, to, self.agent_gitconfig(&a.id).as_deref()).is_ok()
            {
                moved.push(branch);
            } else {
                to_sync.push(a.id);
            }
        }
        let base_commits = orch_git::cleanup::ahead_behind(&lead.worktree, onto, from).0;
        self.run_event(
            &cx.run.id,
            Some(&lead.id),
            "git.rebase",
            json!({"from": from, "to": to, "onto": onto, "base": cx.feature.base, "base_commits": base_commits,
                   "branches_moved": moved}),
        );
        drop(guard);
        for id in to_sync {
            let _ = self.deliver_kind(
                &id,
                &format!(
                    "ORCH: {integ} was rebased onto {onto}. Call mcp__orch__sync_integration before your next task."
                ),
                TurnKind::Continue,
            );
        }
        // A linked PR is watched from launch: its foreign head clears now.
        let linked = cx.feature.pr_link.is_some() && cx.run.pr_number.is_some();
        if (effective(&cx.run) == RunState::PrOpen || linked)
            && let Some(me) = self.me.upgrade()
        {
            let run = cx.run.id.clone();
            // The single PR, or each open issue PR (now `rewritten`).
            let issues = if cx.run.pr_number.is_some() {
                vec![None]
            } else {
                crate::pr::open_issues(&cx.run).into_iter().map(Some).collect()
            };
            std::thread::spawn(move || {
                for i in issues {
                    me.pr_check_now(&run, i.as_deref(), None);
                }
            });
        }
        moved
    }
}
