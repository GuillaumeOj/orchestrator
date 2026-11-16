//! Building an issue branch (feature allow-multiple-prs §3.3): from the
//! integration branch, the first-parent commits of one issue plus the
//! shared ones, replayed onto the fork point with plumbing only (no
//! worktree, no hooks, never a push).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use orch_git::replay::{self, CommitInfo, Replay};
use orch_store::{Feature, RunPr, Task};
use serde_json::json;

use crate::runtime::Runtime;
use crate::team::RunCtx;

/// Which PR a commit of the integration branch belongs to: `Some(ref)` an
/// issue's, `None` every one's (shared). A task merge (`Orch-Task: n`)
/// follows task `n`'s issue as it is now; `Orch-Issue: r` names it; anything
/// else (lead docs, untagged tasks, unknown tasks) is shared.
pub fn attribute(c: &CommitInfo, task_issue: &dyn Fn(u64) -> Option<String>) -> Option<String> {
    if let Some(n) = c.trailer("Orch-Task").and_then(|v| v.trim().parse::<u64>().ok()) {
        return task_issue(n);
    }
    c.trailer("Orch-Issue").map(|r| r.trim().to_string()).filter(|r| !r.is_empty())
}

/// The commits that go into `issue`'s PR: its own and the shared ones.
pub fn of_issue<'a>(
    commits: &'a [CommitInfo],
    issue: &str,
    task_issue: &dyn Fn(u64) -> Option<String>,
) -> Vec<(&'a CommitInfo, bool)> {
    commits
        .iter()
        .filter_map(|c| match attribute(c, task_issue) {
            None => Some((c, false)),
            Some(r) if r == issue => Some((c, true)),
            Some(_) => None,
        })
        .collect()
}

/// How a successful build reaches the PR.
#[derive(Clone, Debug, PartialEq)]
pub enum Push {
    /// No PR yet: a new one from the branch.
    Open,
    /// One commit on top of the PR's pushed head (`lease`): a fast-forward.
    Update { lease: String },
    /// The fork point moved (`rebase_base`): the branch replaces the PR's, with its lease.
    Force { lease: String },
    /// The PR already has this content.
    UpToDate,
}

/// A build of an issue branch.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub issue: String,
    pub branch: String,
    pub fork_sha: String,
    pub integration_head: String,
    /// The head to push.
    pub head: String,
    /// Commits of the issue / shared replayed, and those left out (empty).
    pub commits: usize,
    pub shared: usize,
    pub skipped: usize,
    pub push: Push,
}

/// A build stopped on a commit that does not apply without another issue's work.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub commit: String,
    pub subject: String,
    /// The task merged by that commit: (handle, node).
    pub task: Option<(String, String)>,
    pub files: Vec<String>,
}

impl Conflict {
    /// The refusal the lead reads.
    pub fn refusal(&self, issue: &str) -> String {
        let what = match &self.task {
            Some((h, node)) => format!("task {h} (node {node}, commit {} \"{}\")", short(&self.commit), self.subject),
            None => format!("commit {} \"{}\"", short(&self.commit), self.subject),
        };
        format!(
            "the PR of issue {issue} can't be built: {what} conflicts in {} without the work of another issue. \
             Nothing was requested. Re-tag the tasks it builds on (mcp__orch__update_task issue), or ask the user \
             (mcp__orch__ask_user) to group these issues in one PR or to go back to one PR for the feature.",
            self.files.join(", ")
        )
    }
}

fn short(s: &str) -> &str {
    &s[..s.len().min(10)]
}

/// The fork point of the integration branch: where it leaves `origin/<base>`.
pub fn fork_point(wt: &Path, feature: &Feature) -> Result<String> {
    let integ = feature.integration_branch();
    let base = format!("origin/{}", feature.base);
    Ok(orch_git::git(wt, &["merge-base", &integ, &base])?.trim().to_string())
}

/// The issue of each task of a run, by id (`None`: untagged).
fn task_issues(tasks: &[Task]) -> BTreeMap<u64, Option<String>> {
    tasks.iter().map(|t| (t.id, t.issue.clone())).collect()
}

impl Runtime {
    /// Builds `issue`'s branch in the integration worktree `wt` (spec §3.3).
    /// `pr` is its open PR: the build becomes one update commit on the PR's
    /// pushed head, or a force push after `rebase_base` moved the fork point.
    /// The local `orch/<slug>/issue/<key>` is rewritten; nothing else moves.
    /// The `pr.build` event is on the run's timeline.
    pub(crate) fn build_issue(
        &self,
        cx: &RunCtx,
        wt: &Path,
        issue: &str,
        pr: Option<&RunPr>,
    ) -> Result<std::result::Result<Build, Conflict>> {
        let r = self.build_issue_inner(cx, wt, issue, pr);
        let detail = match &r {
            Ok(Ok(b)) => json!({"issue": issue, "ok": true, "commits": b.commits, "shared": b.shared, "skipped": b.skipped}),
            Ok(Err(c)) => json!({"issue": issue, "ok": false, "commits": 0, "shared": 0, "skipped": 0,
                                 "conflict": {"task": c.task.as_ref().map(|t| &t.0), "commit": c.commit, "files": c.files}}),
            Err(e) => json!({"issue": issue, "ok": false, "error": format!("{e:#}")}),
        };
        self.run_event(&cx.run.id, None, "pr.build", detail);
        r
    }

    fn build_issue_inner(
        &self,
        cx: &RunCtx,
        wt: &Path,
        issue: &str,
        pr: Option<&RunPr>,
    ) -> Result<std::result::Result<Build, Conflict>> {
        let f = &cx.feature;
        let integ = f.integration_branch();
        let head_of_integ = orch_git::gh::rev_sha(wt, &integ).ok_or_else(|| anyhow!("{integ} has no commit"))?;
        let fork = fork_point(wt, f)?;
        let commits = replay::first_parent_commits(wt, &fork, &head_of_integ)?;
        let tasks = self.db.tasks(None, None, Some(&cx.run.id))?;
        let issues = task_issues(&tasks);
        let task_issue = |n: u64| issues.get(&n).cloned().flatten();
        let picked = of_issue(&commits, issue, &task_issue);
        let list: Vec<CommitInfo> = picked.iter().map(|(c, _)| (*c).clone()).collect();
        let (head, replayed, skipped) = match replay::replay(wt, &fork, &list)? {
            Replay::Done { head, replayed, skipped } => (head, replayed, skipped),
            Replay::Conflict { commit, files } => {
                let c = commits.iter().find(|c| c.sha == commit);
                let task = c
                    .and_then(|c| c.trailer("Orch-Task"))
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .and_then(|n| tasks.iter().find(|t| t.id == n))
                    .map(|t| (t.handle(), t.node.clone().unwrap_or_default()));
                return Ok(Err(Conflict {
                    subject: c.map(|c| c.subject.clone()).unwrap_or_default(),
                    commit,
                    task,
                    files,
                }));
            }
        };
        let own = |sha: &String| picked.iter().any(|(c, mine)| *mine && &c.sha == sha);
        let n_own = replayed.iter().filter(|s| own(s)).count();
        let branch = f.issue_branch(issue);
        let lease = pr.map(|p| p.pr.as_ref().and_then(|s| s.pushed_sha.clone()).unwrap_or_else(|| p.built_sha.clone()));
        let (head, push) = match (pr, lease) {
            (Some(p), Some(lease)) if p.fork_sha == fork && orch_git::git::valid_sha(&lease) => {
                let tree = replay::tree_of(wt, &head)?;
                if replay::tree_of(wt, &lease)? == tree {
                    (lease, Push::UpToDate)
                } else {
                    let msg = update_message(issue, &commits, p, &task_issue, &tasks);
                    let author = commits.last().map(|c| c.author.clone()).unwrap_or_default();
                    let sha = replay::commit_tree(wt, &tree, &lease, &msg, &author)?;
                    (sha, Push::Update { lease })
                }
            }
            (Some(_), Some(lease)) => (head, Push::Force { lease }),
            _ => (head, Push::Open),
        };
        if push != Push::UpToDate {
            replay::set_branch(wt, &branch, &head)?;
        }
        Ok(Ok(Build {
            issue: issue.to_string(),
            branch,
            fork_sha: fork,
            integration_head: head_of_integ,
            head,
            commits: n_own,
            shared: replayed.len() - n_own,
            skipped: skipped.len(),
            push,
        }))
    }

    /// `PrStatus.unpushed` / `rewritten` of an issue PR (spec §3.5): the
    /// integration commits of the issue or shared since its last pushed
    /// build, and whether the fork point moved since.
    pub(crate) fn issue_pr_drift(&self, cx: &RunCtx, wt: &Path, p: &RunPr) -> (u32, bool) {
        let Ok(fork) = fork_point(wt, &cx.feature) else { return (0, false) };
        let rewritten = !p.fork_sha.is_empty() && p.fork_sha != fork;
        let integ = cx.feature.integration_branch();
        let from = if rewritten || p.integration_sha.is_empty() { fork.as_str() } else { p.integration_sha.as_str() };
        let Ok(commits) = replay::first_parent_commits(wt, from, &integ) else { return (0, rewritten) };
        let tasks = self.db.tasks(None, None, Some(&cx.run.id)).unwrap_or_default();
        let issues = task_issues(&tasks);
        let n = of_issue(&commits, &p.issue, &|n| issues.get(&n).cloned().flatten()).len();
        (n as u32, rewritten)
    }
}

/// `Update for <ref>: <task handles>`: the issue's tasks merged since the
/// PR's last pushed build (`shared changes` when only shared work moved).
fn update_message(
    issue: &str,
    commits: &[CommitInfo],
    p: &RunPr,
    task_issue: &dyn Fn(u64) -> Option<String>,
    tasks: &[Task],
) -> String {
    let since = commits.iter().position(|c| c.sha == p.integration_sha).map_or(0, |i| i + 1);
    let mut handles: Vec<String> = vec![];
    for c in &commits[since..] {
        let Some(n) = c.trailer("Orch-Task").and_then(|v| v.trim().parse::<u64>().ok()) else { continue };
        if task_issue(n).as_deref() == Some(issue)
            && let Some(t) = tasks.iter().find(|t| t.id == n)
            && !handles.contains(&t.handle())
        {
            handles.push(t.handle());
        }
    }
    let what = if handles.is_empty() { "shared changes".to_string() } else { handles.join(", ") };
    format!("Update for {issue}: {what}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_git::replay::Ident;

    fn c(sha: &str, trailers: &[(&str, &str)]) -> CommitInfo {
        CommitInfo {
            sha: sha.into(),
            parent: Some("p".into()),
            subject: sha.into(),
            message: sha.into(),
            author: Ident::default(),
            trailers: trailers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn commits_are_attributed_by_task_then_issue_trailer_else_shared() {
        let tasks: BTreeMap<u64, Option<String>> = [(1, Some("#1".into())), (2, Some("#2".into())), (3, None)].into();
        let ti = |n: u64| tasks.get(&n).cloned().flatten();
        let commits = [
            c("spec", &[]),
            c("t1", &[("Orch-Task", "1")]),
            c("t2", &[("Orch-Task", "2")]),
            c("t3", &[("Orch-Task", "3")]),
            c("unknown", &[("Orch-Task", "99")]),
            c("lead", &[("Orch-Issue", "#2")]),
            c("both", &[("Orch-Task", "1"), ("Orch-Issue", "#2")]),
        ];
        let got: Vec<Option<String>> = commits.iter().map(|x| attribute(x, &ti)).collect();
        let want = [None, Some("#1"), Some("#2"), None, None, Some("#2"), Some("#1")];
        assert_eq!(got, want.map(|w| w.map(String::from)));
        let mine: Vec<(&str, bool)> = of_issue(&commits, "#2", &ti).iter().map(|(c, m)| (c.sha.as_str(), *m)).collect();
        assert_eq!(mine, [("spec", false), ("t2", true), ("t3", false), ("unknown", false), ("lead", true)]);
        // Re-tagging a task changes the next build.
        let retagged: BTreeMap<u64, Option<String>> = [(1, Some("#2".into()))].into();
        let ti = |n: u64| retagged.get(&n).cloned().flatten();
        assert!(of_issue(&commits, "#2", &ti).iter().any(|(c, m)| c.sha == "t1" && *m));
    }
}
