//! One PR per issue (feature allow-multiple-prs §3.4–3.5): `request_pr
//! {issue}` builds the issue branch and raises that issue's gate; the
//! approved gate opens or updates that issue's PR; the run's state follows
//! all its PRs.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use orch_git::gh;
use orch_protocol::now_ms;
use orch_store::{Feature, PrMode, RunPr, TeamRun};
use orch_team::RunState;
use serde_json::{Value, json};

use crate::inbox::{PRIORITY_ALERT, PRIORITY_GATE, diff_summary};
use crate::issue_build::{Build, Push};
use crate::pr::latest_prs;
use crate::runtime::Runtime;
use crate::store::AgentRecord;
use crate::team::{RunCtx, effective};

/// The state a `per_issue` run is in from its PRs (spec §3.5): `issues`
/// the feature's, `prs` the run's, `gated` the issues whose `push_pr` gate
/// waits. `done` once every issue has a merged PR; `pr_open` while one is
/// open (or not checked yet); `pr_gate` while a PR gate waits; else
/// `integrating`.
pub fn per_issue_run_state(issues: &[String], prs: &[RunPr], gated: &[String]) -> (RunState, &'static str) {
    let latest = |i: &str| prs.iter().rev().find(|p| p.issue == i);
    let state = |p: &RunPr| p.pr.as_ref().map_or("open", |s| s.state.as_str()).to_string();
    let merged = |i: &str| prs.iter().any(|p| p.issue == i && p.pr.as_ref().is_some_and(|s| s.state == "merged"));
    if !issues.is_empty() && issues.iter().all(|i| merged(i)) {
        return (RunState::Done, "every issue PR merged");
    }
    if issues.iter().filter_map(|i| latest(i)).any(|p| state(p) == "open") {
        return (RunState::PrOpen, "pull request opened");
    }
    if !gated.is_empty() {
        return (RunState::PrGate, "pull request asked");
    }
    (RunState::Integrating, "no PR open")
}

fn s(p: &Value, k: &str) -> String {
    p.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Runtime {
    /// Moves a `per_issue` run to the state its PRs give
    /// ([`per_issue_run_state`]); `except` is a gate being decided (not
    /// waiting any more). At `done`, the one "Feature merged" item: the move
    /// happens once, from the stored state.
    pub(crate) fn reevaluate_run_prs(&self, run_id: &str, except: Option<u64>) -> Result<()> {
        let run = self.db.run(run_id)?.ok_or_else(|| anyhow!("no team run {run_id}"))?;
        let f = self.db.feature(&run.feature_id)?.ok_or_else(|| anyhow!("no feature {}", run.feature_id))?;
        let from = effective(&run);
        if f.pr_mode != PrMode::PerIssue || !matches!(from, RunState::Integrating | RunState::PrGate | RunState::PrOpen)
        {
            return Ok(());
        }
        let gated: Vec<String> = self
            .run_pr_gates(run_id)
            .into_iter()
            .filter(|g| g.action == "push_pr" && Some(g.id) != except)
            .filter_map(|g| g.issue)
            .collect();
        let (to, why) = per_issue_run_state(&f.issue_refs(), &run.prs, &gated);
        if to == from {
            return Ok(());
        }
        if to == RunState::Done {
            // A merge is not held back by a pause; done from any PR state.
            let mut r = run.clone();
            r.status = RunState::Done;
            r.resume_status = None;
            r.ended_at_ms = Some(now_ms());
            self.db.upsert_run(&r)?;
            self.run_event(run_id, None, "run.state", json!({"from": run.status, "to": RunState::Done, "why": why}));
            let last = latest_prs(&run).into_iter().rev().find(|p| p.pr.as_ref().is_some_and(|s| s.state == "merged"));
            let _ = self.add_item(
                None,
                "info",
                format!("Feature {} merged", f.slug),
                json!({"action": "pr_merged", "run": run_id, "feature": f.id, "issue": null,
                       "pr_number": last.map(|p| p.number), "url": last.map(|p| p.url.clone())}),
                PRIORITY_ALERT,
            );
            return Ok(());
        }
        // pr_open → pr_gate is not a move of §6.2: through integrating.
        if from == RunState::PrOpen && to == RunState::PrGate {
            self.advance_run(run_id, RunState::Integrating, "no PR open")?;
        }
        self.advance_run(run_id, to, why)?;
        Ok(())
    }

    /// An issue PR merged or closed (the run's PRs already stored): the run
    /// re-evaluated, and the lead's message.
    pub(crate) fn issue_pr_ended(&self, run: &TeamRun, f: &Feature, label: &str, merged: bool) -> String {
        if let Err(e) = self.reevaluate_run_prs(&run.id, None) {
            self.run_event(&run.id, None, "team_error", json!(format!("{e:#}")));
        }
        let run = self.db.run(&run.id).ok().flatten().unwrap_or_else(|| run.clone());
        let issue = label.split("(issue ").nth(1).map(|s| s.trim_end_matches(')')).unwrap_or("");
        if !merged {
            return format!(
                "ORCH: {label} was closed without being merged. Ask the user what they want with \
                 mcp__orch__ask_user; mcp__orch__request_pr {{\"issue\": \"{issue}\"}} opens a new pull request for it."
            );
        }
        let left: Vec<String> = f
            .issue_refs()
            .into_iter()
            .filter(|i| !run.prs.iter().any(|p| &p.issue == i && p.pr.as_ref().is_some_and(|s| s.state == "merged")))
            .collect();
        if left.is_empty() {
            return format!("ORCH: {label} was merged; every issue PR is merged. The feature is done; end your turn.");
        }
        let next = if effective(&run) == RunState::PrOpen {
            String::new()
        } else {
            " No PR is open: finish their work, then call mcp__orch__request_pr {\"issue\": …} for each.".into()
        };
        format!("ORCH: {label} was merged; {} issue(s) left: {}.{next}", left.len(), left.join(", "))
    }

    /// A gate of `issue` waits for the user in `run`.
    fn pending_issue_gate(&self, run: &str, issue: &str) -> bool {
        self.run_pr_gates(run).iter().any(|g| g.issue.as_deref() == Some(issue))
    }

    /// `request_pr {issue, title, body, draft?}` in `per_issue` mode (spec §3.4).
    pub(crate) fn request_pr_issue(&self, rec: &AgentRecord, cx: &RunCtx, args: &Value) -> Result<String> {
        let issue = args.get("issue").and_then(Value::as_str).map(str::trim).filter(|i| !i.is_empty());
        let refs = cx.feature.issue_refs();
        let issue = issue.ok_or_else(|| {
            anyhow!("`issue` is required: this feature has one PR per issue ({})", refs.join(", "))
        })?;
        // As typed (`12`, `#12`, `eng-7`): the feature's canonical ref.
        let issue = refs
            .iter()
            .find(|r| r.eq_ignore_ascii_case(issue) || r.trim_start_matches('#') == issue.trim_start_matches('#'))
            .cloned()
            .ok_or_else(|| anyhow!("{issue} is not an issue of this feature ({})", refs.join(", ")))?;
        if self.pending_issue_gate(&cx.run.id, &issue) {
            bail!("a pull request request for issue {issue} is already waiting for the user");
        }
        let wt = cx.feature.integration_worktree.clone().unwrap_or_else(|| rec.worktree.clone());
        if orch_git::commit::rebase_in_progress(&wt) {
            bail!(
                "a rebase is in progress: finish it with mcp__orch__commit (or abort it with mcp__orch__rebase_base) first"
            );
        }
        if let Some(m) = latest_prs(&cx.run)
            .into_iter()
            .find(|p| p.issue == issue && p.pr.as_ref().is_some_and(|s| s.state == "merged"))
        {
            bail!("PR #{} (issue {issue}) was merged: that issue is done", m.number);
        }
        let open = latest_prs(&cx.run)
            .into_iter()
            .find(|p| p.issue == issue && p.pr.as_ref().is_none_or(|s| s.state == "open"))
            .cloned();
        if let Some(p) = &open
            && let Some(st) = &p.pr
            && st.pushed_sha.as_deref().is_some_and(|ps| ps != st.head_sha)
            && !orch_git::cleanup::is_merged(&wt, &st.head_sha, &format!("origin/{}", st.base))
        {
            bail!(
                "PR #{} (issue {issue}) has commits pushed outside the app: pushing would drop them. Ask the user \
                 what to do with mcp__orch__ask_user.",
                p.number
            );
        }
        let b = match self.build_issue(cx, &wt, &issue, open.as_ref())? {
            Ok(b) => b,
            Err(c) => bail!("{}", c.refusal(&issue)),
        };
        let base = cx.feature.base.clone();
        let common = json!({"run": cx.run.id, "issue": issue, "branch": b.branch, "base": base, "worktree": wt,
                            "head": b.head, "fork_sha": b.fork_sha, "integration_head": b.integration_head,
                            "integration": cx.feature.integration_branch(), "diff_base": format!("origin/{base}")});
        let Some(p) = open else { return self.issue_open_gate(rec, args, &b, common) };
        let lease = match &b.push {
            Push::UpToDate => bail!("PR #{} (issue {issue}) is up to date: nothing to push", p.number),
            Push::Update { lease } | Push::Force { lease } => lease.clone(),
            Push::Open => unreachable!("an open PR is updated"),
        };
        let force = matches!(b.push, Push::Force { .. });
        let range = if force { format!("origin/{base}..{}", b.head) } else { format!("{lease}..{}", b.head) };
        let log = orch_git::git(&wt, &["log", "--max-count=200", "--format=%H%x1f%s", &range, "--"])?;
        let commits: Vec<Value> = log
            .lines()
            .filter_map(|l| l.split_once('\x1f'))
            .map(|(sha, subject)| json!({"sha": sha, "subject": subject}))
            .collect();
        let n = p.number;
        let title = if force {
            format!("{}: force-push PR #{n} (issue {issue}) rebased onto {base}", rec.id)
        } else {
            format!("{}: push {} commit(s) to PR #{n} (issue {issue}, {})", rec.id, commits.len(), b.branch)
        };
        let mut payload = common;
        let extra = json!({"action": "push_pr_update", "pr_number": n, "url": p.url, "commits": commits,
                           "force": force, "lease_sha": lease, "foreign_replaced": 0});
        merge(&mut payload, extra);
        let id = self.add_item(Some(&rec.id), "gate", title, payload, PRIORITY_GATE)?;
        self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "pr_update_requested",
            Some(&b.branch),
            "pending",
            json!({"item": id, "pr_number": n, "issue": issue, "commits": commits.len(), "force": force}),
        )?;
        let what = if force {
            format!("force-push {} (rebuilt on {base}) to PR #{n}", b.branch)
        } else {
            format!("push {} commit(s) of {} to PR #{n}", commits.len(), b.branch)
        };
        Ok(format!(
            "Requested: decision {id} asks the user to {what} (issue {issue}). You will be told the result; there is nothing else to do for it now."
        ))
    }

    /// The `push_pr` gate of an issue with no open PR.
    fn issue_open_gate(&self, rec: &AgentRecord, args: &Value, b: &Build, common: Value) -> Result<String> {
        let title = args.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        let title = title.ok_or_else(|| anyhow!("title is required"))?.chars().take(256).collect::<String>();
        let body = args.get("body").and_then(Value::as_str).unwrap_or("").chars().take(20_000).collect::<String>();
        let body = crate::issues::with_closes(&body, std::slice::from_ref(&b.issue));
        let draft = args.get("draft").and_then(Value::as_bool).unwrap_or(false);
        let wt = PathBuf::from(s(&common, "worktree"));
        let base = s(&common, "base");
        let mut payload = common;
        let diff = diff_summary(&wt, &format!("origin/{base}"), &b.branch);
        merge(&mut payload, json!({"action": "push_pr", "title": title, "body": body, "draft": draft, "diff": diff}));
        let id = self.add_item(
            Some(&rec.id),
            "gate",
            format!("{}: open a PR \"{title}\" for issue {} ({} → {base})", rec.id, b.issue, b.branch),
            payload,
            PRIORITY_GATE,
        )?;
        self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "pr_requested",
            Some(&b.branch),
            "pending",
            json!({"item": id, "issue": b.issue, "commits": b.commits, "shared": b.shared}),
        )?;
        Ok(format!(
            "Requested: decision {id} asks the user to push {} ({} commit(s) of issue {}, {} shared) and open a PR \
             into {base} that closes {}. You will be told the result; there is nothing else to do for it now.",
            b.branch, b.commits, b.issue, b.shared, b.issue
        ))
    }

    /// A per-issue gate whose integration head or built head moved since it
    /// was raised (`None` for other gates).
    pub(crate) fn issue_gate_moved(&self, p: &Value) -> Option<String> {
        p.get("issue").and_then(Value::as_str)?;
        let wt = PathBuf::from(p.get("worktree").and_then(Value::as_str)?);
        for (branch, was) in [(s(p, "integration"), s(p, "integration_head")), (s(p, "branch"), s(p, "head"))] {
            if branch.is_empty() || was.is_empty() {
                continue;
            }
            let now = gh::rev_sha(&wt, &branch).unwrap_or_default();
            if now != was {
                return Some(format!("{branch} is at {} now, not {}", &now[..now.len().min(12)], &was[..was.len().min(12)]));
            }
        }
        None
    }

    /// An approved issue PR gate opened its PR at `url`: recorded in the
    /// run's `prs` (replacing a closed one), the run `pr_open`. The run to
    /// check at once, once the lead is told.
    pub(crate) fn issue_pr_opened(&self, agent: &str, p: &Value, url: Option<&str>) -> Option<String> {
        let run_id = s(p, "run");
        let issue = s(p, "issue");
        let branch = s(p, "branch");
        let Some(u) = url else {
            let _ = self.reevaluate_run_prs(&run_id, None);
            return None;
        };
        let found = match gh::pr_number_from_url(u) {
            Some(n) => Some((n, u.to_string())),
            None => self
                .gh_pr_lookup(&PathBuf::from(s(p, "worktree")), &branch)
                .ok()
                .map(|(n, url)| (n, if url.is_empty() { u.to_string() } else { url })),
        };
        let Some((number, url)) = found else {
            let _ = self.reevaluate_run_prs(&run_id, None);
            let _ = self.deliver_user(
                agent,
                &format!(
                    "ORCH: the pull request of issue {issue} was opened ({u}) but its number could not be read, so it \
                     can't be followed. Check it on GitHub and ask the user what to do with mcp__orch__ask_user."
                ),
            );
            return None;
        };
        let entry = RunPr {
            issue: issue.clone(),
            branch,
            number,
            url: url.clone(),
            pr: None,
            fork_sha: s(p, "fork_sha"),
            built_sha: s(p, "head"),
            integration_sha: s(p, "integration_head"),
        };
        let _ = self.db.update_run_prs(&run_id, move |prs| {
            prs.retain(|e| e.issue != entry.issue);
            prs.push(entry);
        });
        let _ = self.advance_run(&run_id, RunState::PrOpen, "pull request opened");
        self.run_event(&run_id, None, "pr.opened", json!({"number": number, "url": url, "issue": issue}));
        Some(run_id)
    }

    /// An issue PR update pushed: its build is now the PR's.
    pub(crate) fn issue_pr_pushed(&self, p: &Value) {
        let n = p["pr_number"].as_u64().unwrap_or(0);
        let (fork, head, integ) = (s(p, "fork_sha"), s(p, "head"), s(p, "integration_head"));
        let _ = self.db.update_run_prs(&s(p, "run"), move |prs| {
            if let Some(e) = prs.iter_mut().rev().find(|e| e.number == n) {
                (e.fork_sha, e.built_sha, e.integration_sha) = (fork, head, integ);
            }
        });
    }

    /// Checks an issue PR now (just opened or pushed to).
    pub(crate) fn pr_check_issue_now(&self, run_id: &str, issue: &str, pushed: Option<String>) {
        if let Err(e) = self.pr_check_issue(run_id, issue, true, now_ms(), pushed) {
            self.run_event(run_id, None, "pr.error", json!({"issue": issue, "error": format!("{e:#}")}));
        }
    }
}

fn merge(into: &mut Value, extra: Value) {
    if let (Some(o), Value::Object(e)) = (into.as_object_mut(), extra) {
        o.extend(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_store::PrStatus;

    fn pr(issue: &str, state: Option<&str>) -> RunPr {
        RunPr {
            issue: issue.into(),
            number: 1,
            pr: state.map(|s| PrStatus { state: s.into(), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn the_run_state_follows_every_issue_pr() {
        let issues = ["#1".to_string(), "#2".to_string()];
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        use RunState::*;
        let cases: Vec<(Vec<RunPr>, Vec<String>, RunState, &str)> = vec![
            (vec![], g(&[]), Integrating, "no PR open"),
            (vec![], g(&["#1"]), PrGate, "pull request asked"),
            (vec![pr("#1", None)], g(&[]), PrOpen, "pull request opened"),
            (vec![pr("#1", Some("open"))], g(&["#2"]), PrOpen, "pull request opened"),
            (vec![pr("#1", Some("merged"))], g(&[]), Integrating, "no PR open"),
            (vec![pr("#1", Some("merged"))], g(&["#2"]), PrGate, "pull request asked"),
            (vec![pr("#1", Some("merged")), pr("#2", Some("open"))], g(&[]), PrOpen, "pull request opened"),
            (vec![pr("#1", Some("merged")), pr("#2", Some("merged"))], g(&[]), Done, "every issue PR merged"),
            (vec![pr("#1", Some("closed")), pr("#2", Some("open"))], g(&[]), PrOpen, "pull request opened"),
            (vec![pr("#1", Some("closed"))], g(&[]), Integrating, "no PR open"),
            // A closed PR replaced by a new one that merged.
            (vec![pr("#1", Some("closed")), pr("#2", Some("merged")), pr("#1", Some("merged"))], g(&[]), Done, "every issue PR merged"),
            // A PR of an issue no longer on the feature does not count.
            (vec![pr("#3", Some("open")), pr("#1", Some("merged")), pr("#2", Some("merged"))], g(&[]), Done, "every issue PR merged"),
        ];
        for (prs, gated, want, why) in cases {
            assert_eq!(per_issue_run_state(&issues, &prs, &gated), (want, why), "{prs:?} {gated:?}");
        }
        assert_eq!(per_issue_run_state(&issues[..1], &[pr("#1", Some("merged"))], &[]).0, Done);
    }
}

#[cfg(test)]
#[path = "pr_issue_tests.rs"]
mod e2e;
