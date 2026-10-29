//! The lead's `rebase_base` (feature improve-end-of-feature §3.4): brings
//! `origin/<base>` into the integration branch by a rebase run by the core
//! (hooks off, the lead's identity), continued by the lead's `commit` on a
//! conflict; once done, the specialists' merged branches follow it.

use anyhow::{Result, anyhow, bail};
use governor::TurnKind;
use orch_git::commit::{self, RebaseOutcome};
use orch_git::gh;
use orch_team::{RunState, TaskStatus};
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::{RunCtx, effective};

const FETCH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

impl Runtime {
    /// `rebase_base {abort?}` (lead only; checked by the caller).
    pub(crate) fn rebase_base(&self, cx: &RunCtx, rec: &AgentRecord, args: &Value) -> Result<(String, Value)> {
        let gitconfig = self.agent_gitconfig(&rec.id);
        if args.get("abort").and_then(Value::as_bool).unwrap_or(false) {
            let r = commit::rebase_abort(&rec.worktree, gitconfig.as_deref());
            return self.rebase_reply(cx, rec, r);
        }
        let state = effective(&cx.run);
        if !matches!(state, RunState::Integrating | RunState::PrOpen) {
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
        let base = &cx.feature.base;
        {
            // Not beside the PR watcher's own fetch in the same repo (ref locks).
            let _watch = self.pr_watch.lock().unwrap_or_else(|e| e.into_inner());
            gh::fetch_branches(&rec.worktree, &[base], FETCH_DEADLINE)
                .map_err(|e| anyhow!("fetching origin/{base} failed: {e:#}"))?;
        }
        let r = commit::rebase_worktree(&rec.worktree, &format!("origin/{base}"), gitconfig.as_deref());
        self.rebase_reply(cx, rec, r)
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
        let r = commit::rebase_continue(&rec.worktree, self.agent_gitconfig(&rec.id).as_deref());
        match lead_cx {
            Some(cx) => self.rebase_reply(&cx, rec, r),
            None => {
                let r = r?;
                Ok((format!("Rebase: {}", json!(r)), json!(r)))
            }
        }
    }

    /// Audits a rebase step and words its outcome for the lead; a finished
    /// rebase moves the specialists' branches ([`Runtime::after_rebase`]).
    fn rebase_reply(&self, cx: &RunCtx, rec: &AgentRecord, r: Result<RebaseOutcome>) -> Result<(String, Value)> {
        let base = format!("origin/{}", cx.feature.base);
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
            Ok(RebaseOutcome::Rebased { from, to }) => self.after_rebase(cx, rec, from, to, &base),
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
        let mut moved = vec![];
        for a in self.run_agents(&cx.run.id).into_iter().filter(|a| a.id != lead.id) {
            let branch = self.branch_of(&a);
            // Only this run's specialist branches, never the integration branch.
            if !branch.starts_with(&prefix) || branch == integ {
                continue;
            }
            let merged = gh::rev_sha(&lead.worktree, &format!("refs/heads/{branch}"))
                .is_some_and(|h| orch_git::cleanup::is_merged(&lead.worktree, &h, from));
            let checked_out = orch_git::worktree::current_branch(&a.worktree).is_ok_and(|b| b == branch);
            let reset = merged
                && checked_out
                && commit::reset_worktree(&a.worktree, &branch, to, self.agent_gitconfig(&a.id).as_deref()).is_ok();
            if reset {
                moved.push(branch);
            } else {
                let _ = self.deliver_kind(
                    &a.id,
                    &format!(
                        "ORCH: {integ} was rebased onto {onto}. Call mcp__orch__sync_integration before your next task."
                    ),
                    TurnKind::Continue,
                );
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
        if effective(&cx.run) == RunState::PrOpen
            && let Some(me) = self.me.upgrade()
        {
            let run = cx.run.id.clone();
            std::thread::spawn(move || me.pr_check_now(&run, None));
        }
        moved
    }
}
