//! GitHub Issues through `gh api` (the user's own `gh` login; agents never
//! call it, ADR-004/009). The board's model maps onto labels: `orch` marks
//! the issues the app manages, `orch:<status>` the column (a closed issue is
//! Done), `orch:node:<id>` the assignee node; dependencies are a
//! `Blocked by: #12, #13` line at the end of the body.
//!
//! Every open issue is read, with or without `orch` (Backlog until its first
//! status change makes it managed); reading never writes. After the first
//! sync only the issues changed since a cursor are read (`since`).

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use orch_team::TaskStatus;
use serde_json::Value;

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
    pub dir: PathBuf,
}

impl GhCli {
    pub fn new(dir: PathBuf) -> GhCli {
        GhCli { dir }
    }
}

impl Gh for GhCli {
    fn run(&self, args: &[String]) -> Result<String> {
        orch_git::gh::gh(&self.dir, &args.iter().map(String::as_str).collect::<Vec<_>>())
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
    /// Has the `orch` label.
    pub managed: bool,
}

/// What a listing leaves out, i.e. what an issue not in it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Only the issues changed since the cursor: one not listed did not change.
    ChangedSince,
    /// Every open issue and the managed closed ones: one not listed may be
    /// closed without `orch` (read it alone).
    OpenAndManaged,
}

/// The issues a sync reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub issues: Vec<Issue>,
    pub coverage: Coverage,
    /// The newest `updated_at` seen (never older than the `since` asked
    /// with), the next sync's `since`; `None` = the remote keeps no cursor.
    pub cursor: Option<String>,
}

/// Overlap of an incremental read, for clock skew and same-second writes.
const SINCE_OVERLAP_S: i64 = 60;

/// An incremental read's `since` (the last cursor, parsed) and the time it
/// reads from (a minute before). `None`: no cursor, or not a time.
pub(crate) fn since_window(since: Option<&str>) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
    let t = since?.parse::<jiff::Timestamp>().ok()?;
    Some((t, t - jiff::SignedDuration::from_secs(SINCE_OVERLAP_S)))
}

/// A listing whose cursor is the newest `updated_at` of `issues`, never
/// older than the `since` it was asked with.
pub(crate) fn listing(issues: Vec<Issue>, coverage: Coverage, since: Option<jiff::Timestamp>) -> Listing {
    let newest = issues.iter().filter_map(|i| i.updated_at.parse::<jiff::Timestamp>().ok()).max();
    Listing { cursor: newest.max(since).map(|t| t.to_string()), issues, coverage }
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

/// A label of the task backend (`orch`, `orch:*`).
pub fn managed(label: &str) -> bool {
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

/// Reads one issue from `gh api` output (REST, or `--json` field names).
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
    let is_managed = labels.iter().any(|l| l == LABEL);
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
        managed: is_managed,
    })
}

/// Which parts of an issue a write sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    /// Labels (managed + the issue's others) and open/closed.
    pub state: bool,
    /// Title and body (with its `Blocked by:` line).
    pub content: bool,
}

impl Fields {
    pub const ALL: Fields = Fields { state: true, content: true };
    pub const STATE: Fields = Fields { state: true, content: false };

    pub fn any(self) -> bool {
        self.state || self.content
    }
}

/// What to write to an issue.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueWrite<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub status: TaskStatus,
    pub node: Option<&'a str>,
    pub blocked_by: Vec<String>,
    /// What an update sends; a create sends everything.
    pub fields: Fields,
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

    /// The repository when it needs no `gh` call to know it.
    pub fn known_repo(&self) -> Option<String> {
        self.repo.get().cloned().or_else(|| self.configured.clone())
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
        let f = w.fields;
        let mut a: Vec<String> = Vec::new();
        if f.content {
            a.extend(["-f".into(), format!("title={}", w.title)]);
            a.extend(["-f".into(), format!("body={}", render_body(w.body, &w.blocked_by))]);
        }
        if f.state {
            for l in labels_for(w.status, w.node).into_iter().chain(keep.iter().cloned()) {
                a.extend(["-f".into(), format!("labels[]={l}")]);
            }
        }
        a
    }

    /// Opens an issue; returns it as GitHub stored it.
    pub fn create(&self, w: &IssueWrite) -> Result<Issue> {
        self.ensure_labels()?;
        let repo = self.repo()?;
        let mut args = vec!["api".to_string(), "-X".into(), "POST".into(), format!("repos/{repo}/issues")];
        args.extend(Self::fields(&IssueWrite { fields: Fields::ALL, ..w.clone() }, &[]));
        let mut issue = self.parse(&self.gh.run(&args)?)?;
        if w.status == TaskStatus::Done {
            issue = self.update(&issue.number, &IssueWrite { fields: Fields::STATE, ..w.clone() }, &[])?;
        }
        Ok(issue)
    }

    /// Sends what `w.fields` names: title and body, and/or state and the
    /// labels (managed + `keep`, the issue's other labels).
    pub fn update(&self, number: &str, w: &IssueWrite, keep: &[String]) -> Result<Issue> {
        if w.fields.state {
            self.ensure_labels()?;
        }
        let repo = self.repo()?;
        let mut args = vec!["api".to_string(), "-X".into(), "PATCH".into(), format!("repos/{repo}/issues/{number}")];
        args.extend(Self::fields(w, keep));
        if w.fields.state {
            args.push("-f".into());
            args.push(format!("state={}", if w.status == TaskStatus::Done { "closed" } else { "open" }));
        }
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

    /// The issues a sync reads, through the REST issues endpoint, paged.
    /// No `since`: every open issue and the closed `orch` ones.
    /// With `since` (the last cursor): every issue changed since a minute
    /// before it, open or closed. Pull requests are skipped.
    pub fn list(&self, since: Option<&str>) -> Result<Listing> {
        let repo = self.repo()?;
        let window = since_window(since);
        let (raw, coverage) = match window {
            Some((_, from)) => (self.pages(&repo, &format!("state=all&since={from}"))?, Coverage::ChangedSince),
            None => {
                let mut v = self.pages(&repo, "state=open")?;
                v.extend(self.pages(&repo, &format!("state=closed&labels={LABEL}"))?);
                (v, Coverage::OpenAndManaged)
            }
        };
        let issues = raw.iter().filter(|v| v.get("pull_request").is_none()).filter_map(parse_issue).collect();
        Ok(listing(issues, coverage, window.map(|(t, _)| t)))
    }

    /// Every issue of a paged GET of `repo`'s issues with `query` (`gh api
    /// --paginate` prints one JSON array per page, back to back). A GET: no
    /// `-f` here, `gh` would POST.
    fn pages(&self, repo: &str, query: &str) -> Result<Vec<Value>> {
        let path = format!("repos/{repo}/issues?{query}&per_page=100");
        let out = self.gh(&["api", "--paginate", &path])?;
        let mut items = Vec::new();
        for page in serde_json::Deserializer::from_str(&out).into_iter::<Value>() {
            match page.context("gh api: not JSON")? {
                Value::Array(a) => items.extend(a),
                v => return Err(anyhow!("gh api {path}: expected a list, got {v}")),
            }
        }
        Ok(items)
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
        assert!(parse_issue(&bare).unwrap().managed);
        let foreign = serde_json::json!({"number": 3, "state": "open", "labels": [{"name": "bug"}]});
        let f = parse_issue(&foreign).unwrap();
        assert_eq!((f.status, f.managed, f.other_labels), (TaskStatus::Backlog, false, vec!["bug".to_string()]));
    }

    struct Pages(String);
    impl Gh for Pages {
        fn run(&self, args: &[String]) -> Result<String> {
            assert!(!args.iter().any(|a| a == "-f" || a == "-F"), "a list is a GET: {args:?}");
            assert_eq!(args[..2], ["api", "--paginate"]);
            if args[2].contains("state=closed") { Ok("[]".into()) } else { Ok(self.0.clone()) }
        }
    }

    #[test]
    fn paged_lists_skip_pull_requests() {
        let out = r#"[{"number": 1, "state": "open", "updated_at": "2026-11-02T10:00:00Z"},
                      {"number": 2, "state": "open", "pull_request": {}, "updated_at": "2026-11-02T12:00:00Z"}]
                     [{"number": 3, "state": "open", "updated_at": "2026-11-02T11:00:00Z"}]"#;
        let gh = GithubIssues::new(Pages(out.into()), Some("me/shop".into()));
        let l = gh.list(None).unwrap();
        assert_eq!(l.issues.iter().map(|i| i.number.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        assert_eq!((l.coverage, l.cursor.as_deref()), (Coverage::OpenAndManaged, Some("2026-11-02T11:00:00Z")));
        let l = gh.list(Some("2026-11-02T11:00:00Z")).unwrap();
        assert_eq!(l.coverage, Coverage::ChangedSince);
    }
}
