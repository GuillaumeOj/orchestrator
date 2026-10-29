//! GitHub through `gh` (FR-106): open PRs with their CI rollup, workflow
//! runs per branch, one PR's status for the team run's watcher, and the gated
//! push / PR creation the core performs with the user's credentials after
//! approval (ADR-009: agents never push).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use orch_store::pr_number_from_url;
use orch_store::{CiJob, CiSummary, PrPosition};

use crate::git::{git, git_deadline, run_with_deadline};

const GH_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChecksSummary {
    pub passing: u32,
    pub failing: u32,
    pub pending: u32,
    pub skipped: u32,
}

impl ChecksSummary {
    /// `passing` / `failing` / `pending` / `none`.
    pub fn overall(&self) -> &'static str {
        if self.failing > 0 {
            "failing"
        } else if self.pending > 0 {
            "pending"
        } else if self.passing > 0 {
            "passing"
        } else {
            "none"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub state: String,
    pub url: String,
    pub is_draft: bool,
    pub review_decision: Option<String>,
    pub checks: ChecksSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub head_sha: String,
    pub created_at: String,
}

/// Summarizes `statusCheckRollup` (CheckRun and StatusContext entries).
pub fn summarize_rollup(v: &Value) -> ChecksSummary {
    let mut s = ChecksSummary::default();
    for c in v.as_array().into_iter().flatten() {
        let status = c.get("status").and_then(Value::as_str).unwrap_or("COMPLETED");
        let outcome = c.get("conclusion").or_else(|| c.get("state")).and_then(Value::as_str).unwrap_or("");
        match (status, outcome) {
            (s2, _) if s2 != "COMPLETED" => s.pending += 1,
            (_, "SUCCESS" | "NEUTRAL") => s.passing += 1,
            (_, "SKIPPED") => s.skipped += 1,
            (_, "PENDING" | "EXPECTED" | "") => s.pending += 1,
            _ => s.failing += 1,
        }
    }
    s
}

/// Job states that end a job badly: any of them turns the CI dot red.
const FAILED: &[&str] = &["failure", "cancelled", "timed_out", "action_required", "startup_failure", "stale", "error"];
/// Job states that end a job well.
const PASSED: &[&str] = &["success", "neutral", "skipped"];
/// The most jobs kept on a PR status.
const MAX_JOBS: usize = 100;

/// A finished job that failed (the CI dot is red).
pub fn job_failed(state: &str) -> bool {
    FAILED.contains(&state)
}

fn job_finished(state: &str) -> bool {
    FAILED.contains(&state) || PASSED.contains(&state)
}

/// Sort group: failed jobs first, then running, waiting, the rest.
fn job_group(state: &str) -> u8 {
    match state {
        s if job_failed(s) => 0,
        "in_progress" => 1,
        s if !job_finished(s) => 2,
        _ => 3,
    }
}

/// One job of `statusCheckRollup`: a CheckRun (`status`, `conclusion`) or a
/// StatusContext (`context`, `state`), its state lower-cased.
fn job(c: &Value) -> CiJob {
    let s = |k: &str| c.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
    let low = |v: &str| v.to_ascii_lowercase();
    if c.get("__typename").and_then(Value::as_str) == Some("StatusContext") || c.get("context").is_some() {
        return CiJob {
            name: s("context").unwrap_or("").into(),
            workflow: None,
            state: s("state").map(low).unwrap_or_else(|| "pending".into()),
            url: s("targetUrl").map(str::to_string),
        };
    }
    let state = match (s("status"), s("conclusion")) {
        (Some(st), _) if st != "COMPLETED" => low(st),
        (_, Some(c)) => low(c),
        _ => "pending".into(),
    };
    CiJob {
        name: s("name").unwrap_or("").into(),
        workflow: s("workflowName").map(str::to_string),
        state,
        url: s("detailsUrl").map(str::to_string),
    }
}

/// `statusCheckRollup` as jobs (spec §3.2): `done` counts finished jobs
/// whatever their outcome; `overall` is the dot's color, `error` before
/// `running` before `waiting` before `ok`, `none` with no job.
pub fn summarize_jobs(v: &Value) -> CiSummary {
    // `gh` flattens the rollup into an array; a paged GraphQL one is
    // `{totalCount, nodes | contexts}`: `total` counts every job then.
    let list = v.as_array().or_else(|| v.get("nodes").or_else(|| v.get("contexts")).and_then(Value::as_array));
    let mut jobs: Vec<CiJob> = list.into_iter().flatten().map(job).collect();
    let paged = v.get("totalCount").and_then(Value::as_u64).unwrap_or(0) as u32;
    let total = paged.max(jobs.len() as u32);
    let done = jobs.iter().filter(|j| job_finished(&j.state)).count() as u32;
    let overall = if jobs.iter().any(|j| job_failed(&j.state)) {
        "error"
    } else if jobs.iter().any(|j| j.state == "in_progress") {
        "running"
    } else if done < jobs.len() as u32 || total > jobs.len() as u32 {
        "waiting"
    } else if total > 0 {
        "ok"
    } else {
        "none"
    };
    jobs.sort_by(|a, b| (job_group(&a.state), &a.name).cmp(&(job_group(&b.state), &b.name)));
    jobs.truncate(MAX_JOBS);
    CiSummary { overall: overall.into(), done, total, jobs }
}

/// One PR as `gh pr view` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct PrView {
    pub url: String,
    /// `open` / `merged` / `closed`.
    pub state: String,
    pub head_sha: String,
    pub base: String,
    /// `MERGEABLE` / `CONFLICTING` / `UNKNOWN`.
    pub mergeable: String,
    /// `CLEAN`, `BEHIND`, `DIRTY`, `BLOCKED`, `UNKNOWN`…
    pub merge_state: String,
    pub ci: CiSummary,
}

pub const PR_VIEW_FIELDS: &str = "number,url,state,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup";

pub fn parse_pr_view(json: &str) -> Result<PrView> {
    let p: Value = serde_json::from_str(json).context("gh pr view: not JSON")?;
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if p.get("number").and_then(Value::as_u64).is_none() {
        bail!("gh pr view: no PR in {json:.200}");
    }
    Ok(PrView {
        url: s("url"),
        state: s("state").to_ascii_lowercase(),
        head_sha: s("headRefOid"),
        base: s("baseRefName"),
        mergeable: s("mergeable"),
        merge_state: s("mergeStateStatus"),
        ci: summarize_jobs(&p["statusCheckRollup"]),
    })
}

/// The `gh` arguments of [`pr_lookup`].
pub fn pr_lookup_args(branch: &str) -> Vec<String> {
    ["pr", "view", branch, "--json", "number,url"].map(str::to_string).to_vec()
}

/// `(number, url)` from `gh pr view <branch> --json number,url`.
pub fn parse_pr_lookup(json: &str) -> Result<(u64, String)> {
    let p: Value = serde_json::from_str(json).context("gh pr view: not JSON")?;
    let n = p.get("number").and_then(Value::as_u64).context("gh pr view: no PR number")?;
    Ok((n, p.get("url").and_then(Value::as_str).unwrap_or("").to_string()))
}

/// The open PR of `branch` (its number and URL), from a checkout of its repo.
pub fn pr_lookup(dir: &Path, branch: &str) -> Result<(u64, String)> {
    if branch.starts_with('-') {
        bail!("invalid branch {branch:?}");
    }
    let args = pr_lookup_args(branch);
    parse_pr_lookup(&gh(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?)
}

/// The `gh` arguments of [`pr_view`].
pub fn pr_view_args(number: u64) -> Vec<String> {
    ["pr", "view", &number.to_string(), "--json", PR_VIEW_FIELDS].map(str::to_string).to_vec()
}

/// One PR's state, CI and mergeability, from a checkout of its repo.
pub fn pr_view(dir: &Path, number: u64) -> Result<PrView> {
    let args = pr_view_args(number);
    parse_pr_view(&gh(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?)
}

/// The PR head against its base (spec §3.2): `conflicts` when GitHub says so,
/// `unknown` while it computes mergeability or without the local counts
/// (`(behind, ahead)`, `None` when the base was never fetched).
pub fn position(
    mergeable: &str,
    merge_state: &str,
    counts: Option<(u32, u32)>,
    base_sha: Option<String>,
) -> PrPosition {
    let (behind, ahead) = counts.unwrap_or((0, 0));
    let status = if mergeable == "CONFLICTING" || merge_state == "DIRTY" {
        "conflicts"
    } else if counts.is_none() || mergeable == "UNKNOWN" {
        "unknown"
    } else if behind > 0 {
        "behind"
    } else {
        "up_to_date"
    };
    PrPosition { status: status.into(), behind, ahead, base_sha }
}

/// `git fetch origin <branch>…` (updates `origin/<branch>`: the PR's base and
/// head, so both are local for the counts), with a deadline.
pub fn fetch_branches(dir: &Path, branches: &[&str], deadline: Duration) -> Result<()> {
    if branches.iter().any(|b| b.is_empty() || b.starts_with('-')) {
        bail!("invalid branch in {branches:?}");
    }
    let mut args = vec!["fetch", "--quiet", "--no-write-fetch-head", "origin"];
    args.extend_from_slice(branches);
    git_deadline(dir, &args, deadline)?;
    Ok(())
}

/// The commit `rev` names in `dir`, if any.
pub fn rev_sha(dir: &Path, rev: &str) -> Option<String> {
    let out = git(dir, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]).ok()?;
    Some(out.trim().to_string()).filter(|s| !s.is_empty())
}

/// The commits of `HEAD` not in `sha`, newest first: `(sha, subject)`.
pub fn commits_since(dir: &Path, sha: &str) -> Result<Vec<(String, String)>> {
    let out = git(dir, &["log", "--max-count=200", "--format=%H%x1f%s", &format!("{sha}..HEAD")])?;
    Ok(out.lines().filter_map(|l| l.split_once('\x1f')).map(|(h, s)| (h.to_string(), s.to_string())).collect())
}

pub fn parse_pr_list(json: &str) -> Result<Vec<PullRequest>> {
    let v: Value = serde_json::from_str(json)?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|p| PullRequest {
            number: p["number"].as_u64().unwrap_or(0),
            title: p["title"].as_str().unwrap_or("").into(),
            head_ref: p["headRefName"].as_str().unwrap_or("").into(),
            base_ref: p["baseRefName"].as_str().unwrap_or("").into(),
            state: p["state"].as_str().unwrap_or("").into(),
            url: p["url"].as_str().unwrap_or("").into(),
            is_draft: p["isDraft"].as_bool().unwrap_or(false),
            review_decision: p["reviewDecision"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            checks: summarize_rollup(&p["statusCheckRollup"]),
        })
        .collect())
}

/// Runs `gh` in `dir` (no prompts, no color, a deadline); stdout on success.
pub fn gh(dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(dir).args(args).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    let out = run_with_deadline(cmd, GH_DEADLINE).with_context(|| format!("gh {}", args.join(" ")))?;
    if !out.status.success() {
        bail!("gh {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `gh auth status` succeeds (FR-1203 GitHub status).
pub fn available(dir: &Path) -> bool {
    gh(dir, &["auth", "status"]).is_ok()
}

/// Open PRs with their checks.
pub fn prs(repo: &Path) -> Result<Vec<PullRequest>> {
    parse_pr_list(&gh(
        repo,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,headRefName,baseRefName,state,url,isDraft,reviewDecision,statusCheckRollup",
        ],
    )?)
}

/// Recent workflow runs of a branch.
pub fn runs(repo: &Path, branch: &str) -> Result<Vec<WorkflowRun>> {
    let out = gh(
        repo,
        &["run", "list", "--branch", branch, "--limit", "10", "--json", "name,status,conclusion,url,headSha,createdAt"],
    )?;
    let v: Value = serde_json::from_str(&out)?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|r| WorkflowRun {
            name: r["name"].as_str().unwrap_or("").into(),
            status: r["status"].as_str().unwrap_or("").into(),
            conclusion: r["conclusion"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            url: r["url"].as_str().unwrap_or("").into(),
            head_sha: r["headSha"].as_str().unwrap_or("").into(),
            created_at: r["createdAt"].as_str().unwrap_or("").into(),
        })
        .collect())
}

/// Pushes `branch` from a worktree with the user's credentials. **Gated**:
/// only call after the user approved it (PR gate, ADR-009).
pub fn push(worktree: &Path, remote: &str, branch: &str) -> Result<()> {
    if remote.starts_with('-') || branch.starts_with('-') {
        bail!("invalid remote or branch");
    }
    git_deadline(
        worktree,
        &["push", "--set-upstream", remote, &format!("{branch}:{branch}")],
        Duration::from_secs(120),
    )?;
    Ok(())
}

/// Pushes the commit `rev` to `<remote>`'s `branch` (a fast-forward): what
/// was approved, not whatever the branch holds by then. **Gated** like [`push`].
pub fn push_rev(worktree: &Path, remote: &str, rev: &str, branch: &str) -> Result<()> {
    if remote.starts_with('-') || rev.starts_with('-') || branch.starts_with('-') || rev.is_empty() {
        bail!("invalid remote, commit or branch");
    }
    git_deadline(worktree, &["push", remote, &format!("{rev}:refs/heads/{branch}")], Duration::from_secs(120))?;
    Ok(())
}

/// Force-pushes the commit `rev` to `<remote>`'s `branch` **only while the
/// remote branch is still at `lease`** (`--force-with-lease`, never a bare
/// `--force`): someone else's push since makes it fail. **Gated** like [`push`].
pub fn push_with_lease(worktree: &Path, remote: &str, rev: &str, branch: &str, lease: &str) -> Result<()> {
    if remote.starts_with('-') || rev.starts_with('-') || branch.starts_with('-') || rev.is_empty() {
        bail!("invalid remote, commit or branch");
    }
    if lease.len() < 7 || !lease.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid lease {lease:?}: a commit sha");
    }
    git_deadline(
        worktree,
        &[
            "push",
            &format!("--force-with-lease=refs/heads/{branch}:{lease}"),
            remote,
            &format!("{rev}:refs/heads/{branch}"),
        ],
        Duration::from_secs(120),
    )?;
    Ok(())
}

/// The `gh` arguments of [`create_pr`].
pub fn create_pr_args(base: &str, head: &str, title: &str, body: &str, draft: bool) -> Vec<String> {
    let mut args =
        ["pr", "create", "--base", base, "--head", head, "--title", title, "--body", body].map(str::to_string).to_vec();
    if draft {
        args.push("--draft".into());
    }
    args
}

/// The PR URL in `gh pr create`'s output (its last line).
pub fn created_pr_url(out: &str) -> String {
    out.trim().lines().last().unwrap_or("").to_string()
}

/// Opens a PR and returns its URL. **Gated** like [`push`].
pub fn create_pr(worktree: &Path, base: &str, head: &str, title: &str, body: &str, draft: bool) -> Result<String> {
    let args = create_pr_args(base, head, title, body, draft);
    Ok(created_pr_url(&gh(worktree, &args.iter().map(String::as_str).collect::<Vec<_>>())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prs_and_rollups() {
        let json = r#"[{"number":12,"title":"Fix","headRefName":"orch/solo/fix","baseRefName":"main","state":"OPEN",
            "url":"https://github.com/o/r/pull/12","isDraft":false,"reviewDecision":"",
            "statusCheckRollup":[
              {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"},
              {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":""},
              {"__typename":"CheckRun","name":"e2e","status":"COMPLETED","conclusion":"FAILURE"},
              {"__typename":"StatusContext","context":"ci","state":"SUCCESS"},
              {"__typename":"CheckRun","name":"opt","status":"COMPLETED","conclusion":"SKIPPED"}]}]"#;
        let prs = parse_pr_list(json).unwrap();
        assert_eq!(prs[0].number, 12);
        assert_eq!(prs[0].review_decision, None);
        assert_eq!(prs[0].checks, ChecksSummary { passing: 2, failing: 1, pending: 1, skipped: 1 });
        assert_eq!(prs[0].checks.overall(), "failing");
        assert_eq!(parse_pr_list("[]").unwrap(), vec![]);
        assert_eq!(ChecksSummary::default().overall(), "none");
    }

    fn states(c: &CiSummary) -> Vec<&str> {
        c.jobs.iter().map(|j| j.state.as_str()).collect()
    }

    fn run(name: &str, status: &str, conclusion: &str) -> Value {
        serde_json::json!({"__typename": "CheckRun", "name": name, "workflowName": "ci", "status": status,
                           "conclusion": conclusion, "detailsUrl": format!("https://x/{name}")})
    }

    #[test]
    fn jobs_count_done_and_sort_failures_first() {
        let v = serde_json::json!([
            run("lint", "COMPLETED", "SUCCESS"),
            run("e2e", "IN_PROGRESS", ""),
            run("unit", "COMPLETED", "FAILURE"),
            run("docs", "QUEUED", ""),
            run("opt", "COMPLETED", "SKIPPED"),
            {"__typename": "StatusContext", "context": "deploy", "state": "PENDING", "targetUrl": ""},
            run("build", "COMPLETED", "CANCELLED"),
        ]);
        let c = summarize_jobs(&v);
        assert_eq!((c.overall.as_str(), c.done, c.total), ("error", 4, 7));
        let names: Vec<&str> = c.jobs.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["build", "unit", "e2e", "deploy", "docs", "lint", "opt"]);
        assert_eq!(states(&c), ["cancelled", "failure", "in_progress", "pending", "queued", "success", "skipped"]);
        assert_eq!(c.jobs[1].workflow.as_deref(), Some("ci"));
        assert_eq!(c.jobs[1].url.as_deref(), Some("https://x/unit"));
        assert_eq!((c.jobs[3].workflow.as_deref(), c.jobs[3].url.as_deref()), (None, None));
    }

    #[test]
    fn overall_precedence() {
        let o = |v: Value| summarize_jobs(&v).overall;
        assert_eq!(o(serde_json::json!([])), "none");
        assert_eq!(o(Value::Null), "none");
        for bad in ["FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE", "STALE"] {
            assert_eq!(
                o(serde_json::json!([run("a", "COMPLETED", bad), run("b", "IN_PROGRESS", "")])),
                "error",
                "{bad}"
            );
        }
        assert_eq!(o(serde_json::json!([{"context": "c", "state": "ERROR"}])), "error");
        assert_eq!(o(serde_json::json!([run("a", "IN_PROGRESS", ""), run("b", "QUEUED", "")])), "running");
        for wait in ["QUEUED", "WAITING", "REQUESTED", "PENDING"] {
            assert_eq!(
                o(serde_json::json!([run("a", wait, ""), run("b", "COMPLETED", "SUCCESS")])),
                "waiting",
                "{wait}"
            );
        }
        assert_eq!(o(serde_json::json!([{"context": "c", "state": "EXPECTED"}])), "waiting");
        assert_eq!(
            o(serde_json::json!([
                run("a", "COMPLETED", "SUCCESS"),
                run("b", "COMPLETED", "NEUTRAL"),
                run("c", "COMPLETED", "SKIPPED")
            ])),
            "ok"
        );
        assert_eq!(o(serde_json::json!([run("a", "COMPLETED", "SURPRISE")])), "waiting", "unknown: never ok");
        let paged =
            summarize_jobs(&serde_json::json!({"totalCount": 120, "nodes": [run("a", "COMPLETED", "SUCCESS")]}));
        assert_eq!((paged.overall.as_str(), paged.done, paged.total, paged.jobs.len()), ("waiting", 1, 120, 1));
        let many: Vec<Value> = (0..150).map(|i| run(&format!("j{i:03}"), "COMPLETED", "SUCCESS")).collect();
        let c = summarize_jobs(&Value::Array(many));
        assert_eq!((c.total, c.done, c.jobs.len()), (150, 150, 100));
    }

    #[test]
    fn parses_pr_view_and_positions() {
        let json = r#"{"number":12,"url":"https://github.com/o/r/pull/12","state":"MERGED","headRefOid":"abc",
            "baseRefName":"main","mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","statusCheckRollup":[]}"#;
        let p = parse_pr_view(json).unwrap();
        assert_eq!((p.state.as_str(), p.head_sha.as_str(), p.base.as_str()), ("merged", "abc", "main"));
        assert_eq!(p.ci.overall, "none");
        assert_eq!(parse_pr_view(&json.replace("MERGED", "CLOSED")).unwrap().state, "closed");
        assert!(parse_pr_view("{}").is_err() && parse_pr_view("nope").is_err());
        assert_eq!(pr_view_args(7)[..3], ["pr", "view", "7"]);

        let pos = |m: &str, ms: &str, c| {
            let p = position(m, ms, c, None);
            (p.status, p.behind, p.ahead)
        };
        assert_eq!(pos("CONFLICTING", "DIRTY", Some((2, 3))), ("conflicts".into(), 2, 3));
        assert_eq!(pos("MERGEABLE", "DIRTY", None).0, "conflicts");
        assert_eq!(pos("MERGEABLE", "BEHIND", Some((2, 3))), ("behind".into(), 2, 3));
        assert_eq!(pos("MERGEABLE", "BEHIND", None).0, "unknown", "the base was never fetched");
        assert_eq!(pos("UNKNOWN", "UNKNOWN", Some((0, 3))).0, "unknown");
        assert_eq!(pos("MERGEABLE", "CLEAN", Some((0, 3))), ("up_to_date".into(), 0, 3));
        assert_eq!(pr_number_from_url("https://github.com/o/r/pull/42\n"), Some(42));
        assert_eq!(pr_number_from_url("https://github.com/o/r/pull/42/"), Some(42));
        assert_eq!(pr_number_from_url("https://github.com/o/r"), None);
    }

    #[test]
    fn local_counts_against_a_fetched_base() {
        use crate::fixtures::{commit_file, git as g, scratch_repo};
        let (d, origin_src) = scratch_repo();
        let bare = d.path().join("origin.git");
        g(d.path(), &["clone", "-q", "--bare", origin_src.to_str().unwrap(), bare.to_str().unwrap()]);
        let wt = d.path().join("wt");
        g(d.path(), &["clone", "-q", bare.to_str().unwrap(), wt.to_str().unwrap()]);
        g(&wt, &["checkout", "-qb", "feat"]);
        commit_file(&wt, "a.txt", "a", "feat a");
        let head = rev_sha(&wt, "HEAD").unwrap();
        commit_file(&wt, "b.txt", "b", "feat b");
        assert_eq!(commits_since(&wt, &head).unwrap().iter().map(|c| c.1.as_str()).collect::<Vec<_>>(), ["feat b"]);
        // main moves on the remote.
        let other = d.path().join("other");
        g(d.path(), &["clone", "-q", bare.to_str().unwrap(), other.to_str().unwrap()]);
        commit_file(&other, "m.txt", "m", "main moves");
        g(&other, &["push", "-q", "origin", "main"]);
        assert_eq!(crate::cleanup::try_ahead_behind(&wt, &head, "origin/main"), Some((1, 0)));
        fetch_branches(&wt, &["main"], Duration::from_secs(30)).unwrap();
        assert_eq!(crate::cleanup::try_ahead_behind(&wt, &head, "origin/main"), Some((1, 1)));
        assert_eq!(rev_sha(&wt, "origin/main"), rev_sha(&other, "HEAD"));
        assert!(fetch_branches(&wt, &["main", "--upload-pack=x"], Duration::from_secs(5)).is_err());
        assert_eq!(rev_sha(&wt, "origin/nope"), None);
    }

    #[test]
    fn a_force_push_holds_only_its_lease() {
        use crate::fixtures::{commit_file, git as g, scratch_repo};
        let (d, src) = scratch_repo();
        let bare = d.path().join("origin.git");
        g(d.path(), &["clone", "-q", "--bare", src.to_str().unwrap(), bare.to_str().unwrap()]);
        let wt = d.path().join("wt");
        g(d.path(), &["clone", "-q", bare.to_str().unwrap(), wt.to_str().unwrap()]);
        g(&wt, &["checkout", "-qb", "feat"]);
        commit_file(&wt, "a.txt", "a", "feat a");
        g(&wt, &["push", "-q", "origin", "feat"]);
        let pushed = rev_sha(&wt, "HEAD").unwrap();
        // Rewritten locally (amended): a plain push is refused, the lease one goes.
        g(
            &wt,
            &["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-q", "--amend", "-m", "feat a'"],
        );
        let new = rev_sha(&wt, "HEAD").unwrap();
        assert!(push_rev(&wt, "origin", &new, "feat").is_err(), "not a fast-forward");
        push_with_lease(&wt, "origin", &new, "feat", &pushed).unwrap();
        assert_eq!(g(&bare, &["rev-parse", "feat"]).trim(), new);
        // Someone pushes to the branch: the next lease (on what the app pushed) is refused.
        let other = d.path().join("other");
        g(d.path(), &["clone", "-q", "-b", "feat", bare.to_str().unwrap(), other.to_str().unwrap()]);
        commit_file(&other, "b.txt", "b", "someone else");
        g(&other, &["push", "-q", "origin", "feat"]);
        g(
            &wt,
            &["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-q", "--amend", "-m", "feat a''"],
        );
        let newer = rev_sha(&wt, "HEAD").unwrap();
        assert!(push_with_lease(&wt, "origin", &newer, "feat", &new).is_err(), "the remote moved");
        assert_ne!(g(&bare, &["rev-parse", "feat"]).trim(), newer);
        assert!(push_with_lease(&wt, "origin", &newer, "feat", "--force").is_err());
        assert!(push_with_lease(&wt, "--receive-pack=x", &newer, "feat", &new).is_err());
    }

    #[test]
    fn push_refuses_option_injection() {
        assert!(push(Path::new("/nonexistent"), "--receive-pack=x", "b").is_err());
    }
}
