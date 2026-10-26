//! GitHub through `gh` (FR-106): open PRs with their CI rollup, workflow
//! runs per branch, and the gated push / PR creation the core performs with
//! the user's credentials after approval (ADR-009: agents never push).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::git::{git_deadline, run_with_deadline};

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

fn gh(dir: &Path, args: &[&str]) -> Result<String> {
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
    git_deadline(worktree, &["push", "--set-upstream", remote, &format!("{branch}:{branch}")], Duration::from_secs(120))?;
    Ok(())
}

/// Opens a PR and returns its URL. **Gated** like [`push`].
pub fn create_pr(worktree: &Path, base: &str, head: &str, title: &str, body: &str, draft: bool) -> Result<String> {
    let mut args = vec!["pr", "create", "--base", base, "--head", head, "--title", title, "--body", body];
    if draft {
        args.push("--draft");
    }
    Ok(gh(worktree, &args)?.trim().lines().last().unwrap_or("").to_string())
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

    #[test]
    fn push_refuses_option_injection() {
        assert!(push(Path::new("/nonexistent"), "--receive-pack=x", "b").is_err());
    }
}
