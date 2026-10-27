//! GitHub Issues through `gh api` (the user's own `gh` login; agents never
//! call it, ADR-004/009). The board's model maps onto labels: `orch` marks
//! the issues the app manages, `orch:<status>` the column (a closed issue is
//! Done), `orch:node:<id>` the assignee node; dependencies are a
//! `Blocked by: #12, #13` line at the end of the body.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use orch_team::TaskStatus;
use serde_json::Value;

const GH_DEADLINE: Duration = Duration::from_secs(30);
pub const LABEL: &str = "orch";
const NODE_PREFIX: &str = "orch:node:";
const BLOCKED: &str = "Blocked by:";

/// Runs `gh` with arguments and returns its stdout. A trait so tests can
/// stand in for GitHub.
pub trait Gh: Send + Sync {
    fn run(&self, args: &[String]) -> Result<String>;
}

/// The real `gh`, run in the project's checkout (so `gh` finds the repo).
pub struct GhCli {
    pub program: PathBuf,
    pub dir: PathBuf,
}

impl GhCli {
    pub fn new(dir: PathBuf) -> GhCli {
        GhCli { program: PathBuf::from("gh"), dir }
    }
}

impl Gh for GhCli {
    fn run(&self, args: &[String]) -> Result<String> {
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(&self.dir).args(args).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
        let out =
            orch_git::git::run_with_deadline(cmd, GH_DEADLINE).with_context(|| format!("gh {}", args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "gh {}: {}",
                args.first().map(String::as_str).unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// An issue as GitHub has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub number: String,
    pub title: String,
    /// The body without the `Blocked by:` line.
    pub body: String,
    pub status: TaskStatus,
    pub node: Option<String>,
    /// Issue numbers.
    pub blocked_by: Vec<String>,
    /// Labels the app does not manage, kept on every update.
    pub other_labels: Vec<String>,
    pub updated_at: String,
}

pub fn status_label(s: TaskStatus) -> String {
    format!("orch:{}", s.as_str())
}

/// Labels for a task: `orch`, its status and its node.
pub fn labels_for(status: TaskStatus, node: Option<&str>) -> Vec<String> {
    let mut l = vec![LABEL.to_string(), status_label(status)];
    if let Some(n) = node {
        l.push(format!("{NODE_PREFIX}{n}"));
    }
    l
}

fn managed(label: &str) -> bool {
    label == LABEL || label.starts_with("orch:")
}

/// The body to send: the text plus the dependency line.
pub fn render_body(body: &str, blocked_by: &[String]) -> String {
    let body = body.trim_end();
    if blocked_by.is_empty() {
        return body.to_string();
    }
    let deps = blocked_by.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ");
    if body.is_empty() { format!("{BLOCKED} {deps}") } else { format!("{body}\n\n{BLOCKED} {deps}") }
}

/// Splits the dependency line off a body.
pub fn parse_body(body: &str) -> (String, Vec<String>) {
    let mut deps = Vec::new();
    let mut kept = Vec::new();
    for line in body.lines() {
        if let Some(rest) = line.trim().strip_prefix(BLOCKED) {
            deps.extend(
                rest.split([',', ' '])
                    .filter_map(|t| t.trim().strip_prefix('#'))
                    .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                    .map(str::to_string),
            );
        } else {
            kept.push(line);
        }
    }
    (kept.join("\n").trim_end().to_string(), deps)
}

/// Reads one issue from `gh issue list --json` or `gh api` output.
pub fn parse_issue(v: &Value) -> Option<Issue> {
    let number = match &v["number"] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let labels: Vec<String> = v["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.get("name").and_then(Value::as_str).or_else(|| l.as_str()).map(str::to_string))
        .collect();
    let closed = v["state"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("closed"));
    let status = if closed {
        TaskStatus::Done
    } else {
        labels
            .iter()
            .filter_map(|l| l.strip_prefix("orch:"))
            .filter(|l| !l.starts_with("node:"))
            .find_map(|l| l.parse::<TaskStatus>().ok())
            .filter(|s| *s != TaskStatus::Done)
            .unwrap_or(TaskStatus::Backlog)
    };
    let node = labels.iter().find_map(|l| l.strip_prefix(NODE_PREFIX)).map(str::to_string);
    let (body, blocked_by) = parse_body(v["body"].as_str().unwrap_or(""));
    Some(Issue {
        number,
        title: v["title"].as_str().unwrap_or("").to_string(),
        body,
        status,
        node,
        blocked_by,
        other_labels: labels.into_iter().filter(|l| !managed(l)).collect(),
        updated_at: v["updatedAt"].as_str().or_else(|| v["updated_at"].as_str()).unwrap_or("").to_string(),
    })
}

/// What to write to an issue.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueWrite<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub status: TaskStatus,
    pub node: Option<&'a str>,
    pub blocked_by: Vec<String>,
}

pub struct GithubIssues<G: Gh> {
    gh: G,
    repo: OnceLock<String>,
    configured: Option<String>,
    labels_ready: OnceLock<()>,
}

impl<G: Gh> GithubIssues<G> {
    /// `repo` = `owner/name`, or `None` for the checkout's own repository.
    pub fn new(gh: G, repo: Option<String>) -> Self {
        GithubIssues { gh, repo: OnceLock::new(), configured: repo, labels_ready: OnceLock::new() }
    }

    fn gh(&self, args: &[&str]) -> Result<String> {
        self.gh.run(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    pub fn repo(&self) -> Result<String> {
        if let Some(r) = self.repo.get() {
            return Ok(r.clone());
        }
        let r = match &self.configured {
            Some(r) => r.clone(),
            None => {
                let out = self.gh(&["repo", "view", "--json", "nameWithOwner"])?;
                let v: Value = serde_json::from_str(&out)?;
                v["nameWithOwner"].as_str().ok_or_else(|| anyhow!("gh repo view: no nameWithOwner"))?.to_string()
            }
        };
        Ok(self.repo.get_or_init(|| r).clone())
    }

    /// Creates the managed labels once per backend (an existing one is fine).
    fn ensure_labels(&self) -> Result<()> {
        if self.labels_ready.get().is_some() {
            return Ok(());
        }
        let repo = self.repo()?;
        let mut names = vec![LABEL.to_string()];
        names.extend(TaskStatus::ALL.iter().map(|s| status_label(*s)));
        for n in names {
            // 422 = already exists.
            if let Err(e) = self.gh(&[
                "api",
                "-X",
                "POST",
                &format!("repos/{repo}/labels"),
                "-f",
                &format!("name={n}"),
                "-f",
                "color=6E7681",
            ]) && !e.to_string().contains("already_exists")
                && !e.to_string().contains("422")
            {
                return Err(e);
            }
        }
        let _ = self.labels_ready.set(());
        Ok(())
    }

    fn fields(w: &IssueWrite, keep: &[String]) -> Vec<String> {
        let mut a = vec![
            "-f".into(),
            format!("title={}", w.title),
            "-f".into(),
            format!("body={}", render_body(w.body, &w.blocked_by)),
        ];
        for l in labels_for(w.status, w.node).into_iter().chain(keep.iter().cloned()) {
            a.push("-f".into());
            a.push(format!("labels[]={l}"));
        }
        a
    }

    /// Opens an issue; returns it as GitHub stored it.
    pub fn create(&self, w: &IssueWrite) -> Result<Issue> {
        self.ensure_labels()?;
        let repo = self.repo()?;
        let mut args = vec!["api".to_string(), "-X".into(), "POST".into(), format!("repos/{repo}/issues")];
        args.extend(Self::fields(w, &[]));
        let mut issue = self.parse(&self.gh.run(&args)?)?;
        if w.status == TaskStatus::Done {
            issue = self.update(&issue.number, w, &[])?;
        }
        Ok(issue)
    }

    /// Replaces title, body, state and the managed labels; `keep` are the
    /// issue's other labels.
    pub fn update(&self, number: &str, w: &IssueWrite, keep: &[String]) -> Result<Issue> {
        self.ensure_labels()?;
        let repo = self.repo()?;
        let mut args = vec!["api".to_string(), "-X".into(), "PATCH".into(), format!("repos/{repo}/issues/{number}")];
        args.extend(Self::fields(w, keep));
        args.push("-f".into());
        args.push(format!("state={}", if w.status == TaskStatus::Done { "closed" } else { "open" }));
        self.parse(&self.gh.run(&args)?)
    }

    pub fn get(&self, number: &str) -> Result<Issue> {
        let repo = self.repo()?;
        self.parse(&self.gh(&["api", &format!("repos/{repo}/issues/{number}")])?)
    }

    pub fn comment(&self, number: &str, text: &str) -> Result<()> {
        let repo = self.repo()?;
        self.gh(&[
            "api",
            "-X",
            "POST",
            &format!("repos/{repo}/issues/{number}/comments"),
            "-f",
            &format!("body={text}"),
        ])?;
        Ok(())
    }

    /// Every managed issue, open and closed.
    pub fn list(&self) -> Result<Vec<Issue>> {
        let repo = self.repo()?;
        let out = self.gh(&[
            "issue",
            "list",
            "--repo",
            &repo,
            "--label",
            LABEL,
            "--state",
            "all",
            "--limit",
            "500",
            "--json",
            "number,title,body,state,labels,updatedAt",
        ])?;
        let v: Value = serde_json::from_str(&out)?;
        Ok(v.as_array().into_iter().flatten().filter_map(parse_issue).collect())
    }

    fn parse(&self, out: &str) -> Result<Issue> {
        let v: Value = serde_json::from_str(out).context("gh api: not JSON")?;
        parse_issue(&v).ok_or_else(|| anyhow!("gh api: no issue in the answer"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_labels() {
        assert_eq!(render_body("Do it.\n", &["12".into(), "13".into()]), "Do it.\n\nBlocked by: #12, #13");
        assert_eq!(render_body("", &["4".into()]), "Blocked by: #4");
        assert_eq!(parse_body("Do it.\n\nBlocked by: #12, #13"), ("Do it.".into(), vec!["12".into(), "13".into()]));
        assert_eq!(parse_body("x"), ("x".into(), vec![]));
        assert_eq!(
            labels_for(TaskStatus::InProgress, Some("back")),
            vec!["orch", "orch:in_progress", "orch:node:back"]
        );
    }

    #[test]
    fn issues_from_gh_json() {
        let v = serde_json::json!({"number": 14, "title": "Session", "body": "b\n\nBlocked by: #11",
            "state": "OPEN", "updatedAt": "2026-10-27T10:00:00Z",
            "labels": [{"name": "orch"}, {"name": "orch:review"}, {"name": "orch:node:back"}, {"name": "bug"}]});
        let i = parse_issue(&v).unwrap();
        assert_eq!(i.number, "14");
        assert_eq!(i.status, TaskStatus::Review);
        assert_eq!(i.node.as_deref(), Some("back"));
        assert_eq!(i.blocked_by, vec!["11"]);
        assert_eq!(i.other_labels, vec!["bug"]);
        let closed = serde_json::json!({"number": 1, "state": "closed", "labels": ["orch:ready"], "updated_at": "t"});
        assert_eq!(parse_issue(&closed).unwrap().status, TaskStatus::Done);
        let bare = serde_json::json!({"number": 2, "state": "open", "labels": [{"name": "orch"}]});
        assert_eq!(parse_issue(&bare).unwrap().status, TaskStatus::Backlog);
    }
}
