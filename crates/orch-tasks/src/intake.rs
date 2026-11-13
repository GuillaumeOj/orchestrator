//! Intake sources (FR-1312, P4.5 spec §3.2): what a project's Ready to start
//! lane lists. Read-only towards every source (D3): `gh api` GETs and one
//! GraphQL query (Projects v2), Linear queries, `git ls-tree` / `cat-file`
//! on the local branch (D2). Issues labelled `orch` / `orch:*` are team
//! tasks, never intake (D4); pull requests are not issues.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use orch_store::IntakeUpsert;
use serde::Serialize;
use serde_json::{Value, json};

use crate::github::Gh;
use crate::linear::{GqlError, Graphql};

mod tree;
pub use tree::{
    CLOSED_WINDOW_DAYS, GithubTree, LinearTree, Node, Tree, dfs_order, github_children, github_tree, is_team_id,
    linear_children, linear_tree, local_tree, tree_since,
};

/// Items per project in the lane (more → "+ n more in <source>").
pub const CAP: usize = 50;
/// An item's body is kept to this many chars.
pub const BODY_MAX: usize = 4000;
/// A source is read in at most this many pages of 100 per refresh
/// (2,000 raw items); past it the listing is `partial`.
pub const MAX_PAGES: usize = 20;
/// Repo docs: at most this many matching files are read per refresh.
const DOCS_MAX: usize = 500;

/// A size hint (S × 0.5, M × 1, L × 2 of the estimate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Size {
    S,
    M,
    L,
}

impl Size {
    pub fn parse(s: &str) -> Option<Size> {
        match s.trim().to_ascii_uppercase().as_str() {
            "S" => Some(Size::S),
            "M" => Some(Size::M),
            "L" => Some(Size::L),
            _ => None,
        }
    }

    /// `size:S` / `size/S` labels, case-insensitive.
    pub fn of_labels(labels: &[String]) -> Option<Size> {
        labels.iter().find_map(|l| {
            let l = l.to_ascii_lowercase();
            l.strip_prefix("size:").or_else(|| l.strip_prefix("size/")).and_then(Size::parse)
        })
    }

    /// Linear points: ≤ 1 S, ≤ 3 M, more L (api-contract F22).
    pub fn of_points(p: f64) -> Size {
        if p <= 1.0 {
            Size::S
        } else if p <= 3.0 {
            Size::M
        } else {
            Size::L
        }
    }
}

/// One item as a source lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetched {
    pub reference: String,
    pub title: String,
    pub url: Option<String>,
    pub body: String,
    pub truncated: bool,
    /// The revision upstream (`updated_at`, a blob id).
    pub rev: String,
    pub updated_at_ms: u64,
    pub labels: Vec<String>,
    pub size: Option<Size>,
    /// `team` | `solo` from a doc's front matter.
    pub mode: Option<String>,
    /// A team id from a doc's front matter.
    pub team: Option<String>,
    /// Source-specific facts (number, assignees, milestone, priority…).
    pub extra: serde_json::Map<String, Value>,
}

impl Fetched {
    /// The cache row of the item.
    pub fn upsert(&self) -> IntakeUpsert {
        let mut meta = json!({
            "labels": self.labels,
            "size": self.size,
            "mode": self.mode,
            "team": self.team,
            "updated_at_ms": self.updated_at_ms,
            "truncated": self.truncated,
        });
        for (k, v) in &self.extra {
            meta[k] = v.clone();
        }
        IntakeUpsert {
            reference: self.reference.clone(),
            node_id: String::new(),
            kind: String::new(),
            parent_id: None,
            title: self.title.clone(),
            url: self.url.clone(),
            body: self.body.clone(),
            meta,
            rev: self.rev.clone(),
        }
    }
}

/// What a source listed: at most [`CAP`] items, in the source's order, and
/// how many it had in all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub items: Vec<Fetched>,
    pub total: usize,
    /// The list in the source's own UI when the source said where it is
    /// (a Projects v2 board, a Linear team or project view).
    pub url: Option<String>,
    /// The read stopped at [`MAX_PAGES`] before the end of the source:
    /// `total` counts only what was read.
    pub partial: bool,
}

impl Listing {
    fn capped(mut items: Vec<Fetched>) -> Listing {
        let total = items.len();
        items.truncate(CAP);
        Listing { items, total, url: None, partial: false }
    }

    pub fn more(&self) -> usize {
        self.total.saturating_sub(self.items.len())
    }
}

/// The longest error message (api-contract F3).
const MESSAGE_MAX: usize = 200;

/// A refresh's failure (api-contract §2.1 codes, F1–F3); `fix` names where
/// the user mends it: `settings` | `integrations` | `gh_scope` | none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceError {
    pub code: &'static str,
    pub message: String,
    pub fix: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
}

impl SourceError {
    pub fn new(code: &'static str, message: impl Into<String>) -> SourceError {
        let fix = match code {
            "gh_missing" | "gh_auth" | "linear_key" | "linear_auth" => Some("integrations"),
            "gh_scope" => Some("gh_scope"),
            "not_found" | "bad_glob" | "branch_missing" => Some("settings"),
            _ => None,
        };
        let message: String = message.into();
        let message = match message.char_indices().nth(MESSAGE_MAX - 1) {
            Some((at, _)) => format!("{}…", &message[..at]),
            None => message,
        };
        SourceError { code, message, fix, retry_at_ms: None }
    }

    pub fn json(&self) -> Value {
        json!(self)
    }
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

type R<T> = Result<T, SourceError>;

/// The first [`BODY_MAX`] chars of `s`, and whether it was cut.
pub fn clip(s: &str) -> (String, bool) {
    match s.char_indices().nth(BODY_MAX) {
        Some((at, _)) => (s[..at].to_string(), true),
        None => (s.to_string(), false),
    }
}

pub use crate::github::managed as is_orch_label;

fn ts_ms(s: &str) -> u64 {
    s.parse::<jiff::Timestamp>().map(|t| t.as_millisecond().max(0) as u64).unwrap_or(0)
}

fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn names(v: &Value, key: &str) -> Vec<String> {
    let arr = v.get("nodes").unwrap_or(v);
    arr.as_array().into_iter().flatten().filter_map(|l| l[key].as_str().or(l.as_str()).map(str::to_string)).collect()
}

/// Percent-encodes a query parameter value.
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ------------------------------------------------------------------ GitHub

/// The text of a failure that says the network is not reachable.
fn is_offline_text(lower: &str) -> bool {
    [
        "could not resolve host",
        "network is unreachable",
        "no route to host",
        "connection refused",
        "dial tcp",
        "network unreachable",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// The source error of a failed `gh` call.
pub fn gh_error(e: &anyhow::Error) -> SourceError {
    // (an unreachable network is `offline`: the cache is shown, F15)
    let m = format!("{e:#}");
    let l = m.to_ascii_lowercase();
    let code = if is_offline_text(&l) {
        "offline"
    } else if l.contains("no such file") || l.contains("not found in path") || l.contains("program not found") {
        "gh_missing"
    } else if l.contains("read:project") || l.contains("insufficient_scopes") || l.contains("required scopes") {
        "gh_scope"
    } else if l.contains("gh auth login")
        || l.contains("not logged")
        || l.contains("http 401")
        || l.contains("bad credentials")
    {
        "gh_auth"
    } else if l.contains("rate limit") {
        "rate_limited"
    } else if l.contains("http 404") || l.contains("not found") || l.contains("could not resolve") {
        "not_found"
    } else {
        "failed"
    };
    let message = match code {
        "gh_missing" => "the GitHub CLI (gh) is not installed".to_string(),
        "gh_auth" => "gh is not logged in: run `gh auth login`".to_string(),
        "gh_scope" => "gh needs the read:project scope: run `gh auth refresh -s read:project`".to_string(),
        _ => m,
    };
    SourceError::new(code, message)
}

fn gh_run(gh: &dyn Gh, args: &[&str]) -> R<String> {
    gh.run(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).map_err(|e| {
        // Classify gh's answer, not our own arguments (a label named "not found").
        let m = format!("{e:#}");
        let prefix = format!("gh {}: ", args.join(" "));
        gh_error(&anyhow::anyhow!("{}", m.strip_prefix(&prefix).unwrap_or(&m)))
    })
}

fn gh_json(gh: &dyn Gh, args: &[&str]) -> R<Value> {
    let out = gh_run(gh, args)?;
    serde_json::from_str(&out).map_err(|_| SourceError::new("failed", format!("gh {}: not JSON", args.join(" "))))
}

/// The cursor of the next page of a GraphQL `pageInfo`, `None` at the end.
fn next_cursor(info: &Value) -> Option<String> {
    info["endCursor"].as_str().filter(|_| info["hasNextPage"] == true).map(str::to_string)
}

/// GraphQL errors in a `gh api graphql` answer, as a source error.
fn graphql_errors(v: &Value) -> Option<SourceError> {
    let errs = v["errors"].as_array().filter(|e| !e.is_empty())?;
    let msg = errs.iter().filter_map(|e| e["message"].as_str()).collect::<Vec<_>>().join("; ");
    let scope = errs.iter().any(|e| e["type"] == "INSUFFICIENT_SCOPES");
    Some(gh_error(&anyhow::anyhow!(if scope { format!("INSUFFICIENT_SCOPES {msg}") } else { msg })))
}

const PROJECTS_QUERY: &str = "query OrchIntakeProjects($owner: String!) { repositoryOwner(login: $owner) { \
    ... on ProjectV2Owner { projectsV2(first: 20) { nodes { number title closed \
    field(name: \"Status\") { ... on ProjectV2SingleSelectField { options { name } } } } } } } }";

/// The open Projects v2 boards of `owner` with their *Status* values
/// (api-contract F19): `Err` with the source error (e.g. `gh_scope`).
fn github_projects(gh: &dyn Gh, owner: &str) -> R<Vec<Value>> {
    let v =
        gh_json(gh, &["api", "graphql", "-f", &format!("query={PROJECTS_QUERY}"), "-f", &format!("owner={owner}")])?;
    if let Some(e) = graphql_errors(&v) {
        return Err(e);
    }
    Ok(v["data"]["repositoryOwner"]["projectsV2"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["closed"] != true)
        .map(|p| {
            json!({"owner": owner, "number": p["number"], "title": p["title"],
                   "statuses": names(&p["field"]["options"], "name")})
        })
        .collect())
}

/// `intake_options` for GitHub: the repository's labels, open milestones
/// and its owner's projects (a projects failure is in `error`, F19).
pub fn github_options(gh: &dyn Gh, repo: &str) -> R<Value> {
    let labels = gh_json(gh, &["api", &format!("repos/{repo}/labels?per_page=100")])?;
    let labels: Vec<String> = names(&labels, "name").into_iter().filter(|l| !is_orch_label(l)).collect();
    let ms = gh_json(gh, &["api", &format!("repos/{repo}/milestones?state=open&per_page=100")])?;
    let milestones: Vec<Value> = ms
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| json!({"title": m["title"], "open_issues": m["open_issues"].as_u64().unwrap_or(0)}))
        .collect();
    let owner = repo.split('/').next().unwrap_or(repo);
    let (projects, error) = match github_projects(gh, owner) {
        Ok(p) => (p, Value::Null),
        Err(e) => (vec![], e.json()),
    };
    Ok(json!({"labels": labels, "milestones": milestones, "projects": projects, "error": error}))
}

const ISSUE_TYPES_QUERY: &str = "query OrchIssueTypes($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { \
    issueTypes(first: 50) { nodes { name } } } }";

/// The issue types of `repo` (none when GitHub has none or does not say:
/// the types are an option, not a requirement).
fn github_issue_types(gh: &dyn Gh, repo: &str) -> Vec<String> {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
    let v = gh_json(
        gh,
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={ISSUE_TYPES_QUERY}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("name={name}"),
        ],
    );
    match v {
        Ok(v) if graphql_errors(&v).is_none() => names(&v["data"]["repository"]["issueTypes"], "name"),
        _ => vec![],
    }
}

/// `tracker_options` for GitHub (contract §1): `intake_options` with the
/// repository's issue types and the owner's Projects v2 boards (with their
/// *Status* options) as `boards`; a boards failure is in `error`.
pub fn github_tracker_options(gh: &dyn Gh, repo: &str) -> R<Value> {
    let mut o = github_options(gh, repo)?;
    o["boards"] = o["projects"].take();
    o.as_object_mut().map(|m| m.remove("projects"));
    o["issue_types"] = json!(github_issue_types(gh, repo));
    Ok(o)
}

/// Whether `repo` has a label named `name`, by asking for it (not by reading
/// its first 100 labels).
pub fn github_has_label(gh: &dyn Gh, repo: &str, name: &str) -> R<bool> {
    match gh_run(gh, &["api", &format!("repos/{repo}/labels/{}", enc(name))]) {
        Ok(_) => Ok(true),
        Err(e) if e.code == "not_found" => Ok(false),
        Err(e) => Err(e),
    }
}

/// The checkout's repository by `gh repo view` (when no remote names it).
pub fn gh_repo(gh: &dyn Gh) -> R<String> {
    let v = gh_json(gh, &["repo", "view", "--json", "nameWithOwner"])?;
    v["nameWithOwner"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| SourceError::new("not_found", "no GitHub repository for this checkout"))
}

// ------------------------------------------------------------------ Linear

/// A per-source rate-limit back-off (Retry-After, else 60 s doubling up to
/// an hour), as the Linear task backend's.
#[derive(Default)]
pub struct Backoff(Mutex<(u64, u64)>);

impl Backoff {
    fn call<T>(&self, now: u64, f: impl FnOnce() -> Result<T, GqlError>) -> R<T> {
        let until = self.0.lock().unwrap_or_else(|e| e.into_inner()).0;
        if now < until {
            return Err(rate_limited(now, (until - now).div_ceil(1000)));
        }
        match f() {
            Ok(v) => {
                *self.0.lock().unwrap_or_else(|e| e.into_inner()) = (0, 0);
                Ok(v)
            }
            Err(GqlError::RateLimited { retry_after_s }) => {
                let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
                let step = retry_after_s.unwrap_or_else(|| (b.1 * 2).clamp(60, 3600));
                *b = (now + step * 1000, step);
                Err(rate_limited(now, step))
            }
            Err(e) => Err(linear_error(&e)),
        }
    }
}

fn rate_limited(now: u64, s: u64) -> SourceError {
    let mut e = SourceError::new("rate_limited", format!("rate limit reached: retrying in {s} s"));
    e.retry_at_ms = Some(now + s * 1000);
    e
}

pub fn linear_error(e: &GqlError) -> SourceError {
    match e {
        GqlError::RateLimited { retry_after_s } => rate_limited(orch_store::now_ms(), retry_after_s.unwrap_or(60)),
        GqlError::Unauthorized(m) if m.contains("no Linear API key") || m.contains("no Linear transport") => {
            SourceError::new("linear_key", "no Linear API key: add it in Settings › Integrations")
        }
        GqlError::Unauthorized(m) => SourceError::new("linear_auth", format!("Linear refused the API key ({m})")),
        GqlError::Failed(m) if is_offline_text(&m.to_ascii_lowercase()) => {
            SourceError::new("offline", format!("Linear is not reachable ({m})"))
        }
        GqlError::Failed(m) if m.to_ascii_lowercase().contains("complex") => {
            SourceError::new("linear_complexity", "Linear said the request was too large")
        }
        GqlError::Failed(m) => SourceError::new("failed", format!("Linear: {m}")),
    }
}

/// A team filter by id (a UUID) or key (`ENG`).
fn team_filter(team: &str) -> Value {
    let uuid = team.len() == 36 && team.chars().filter(|c| *c == '-').count() == 4;
    if uuid { json!({"id": {"eq": team}}) } else { json!({"key": {"eq": team}}) }
}

/// `intake_options` for Linear: the team's labels (but `orch`'s).
pub fn linear_options(gql: &dyn Graphql, backoff: &Backoff, now: u64, team: &str) -> R<Value> {
    let d = backoff.call(now, || {
        gql.call(
            "query OrchIntakeLabels($filter: TeamFilter) { teams(filter: $filter) { nodes { labels(first: 100) { nodes { name } } } } }",
            &json!({"filter": team_filter(team)}),
        )
    })?;
    let t = d["teams"]["nodes"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| SourceError::new("not_found", format!("no Linear team {team}")))?;
    let labels: Vec<String> = names(&t["labels"], "name").into_iter().filter(|l| !is_orch_label(l)).collect();
    Ok(json!({"labels": labels}))
}

const LINEAR_TEAMS_QUERY: &str = "query OrchTrackerTeams { teams(first: 100) { nodes { id key name } } }";

const LINEAR_TRACKER_QUERY: &str = "query OrchTrackerOptions($filter: TeamFilter) { teams(first: 100, filter: $filter) { nodes { key \
    labels(first: 100) { nodes { name } } states(first: 100) { nodes { id name type position } } \
    projects(first: 100) { nodes { id name } } } } }";

/// `tracker_options` for Linear (contract §1, F17): the workspace's teams,
/// and, for the `teams` (keys or ids, as the source filters take them)
/// already picked, their projects, labels and workflow states.
pub fn linear_tracker_options(gql: &dyn Graphql, backoff: &Backoff, now: u64, teams: &[String]) -> R<Value> {
    let d = backoff.call(now, || gql.call(LINEAR_TEAMS_QUERY, &json!({})))?;
    let all: Vec<Value> = d["teams"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| json!({"id": t["id"], "key": t["key"], "name": t["name"]}))
        .collect();
    let (mut projects, mut labels, mut states) = (vec![], vec![], vec![]);
    if !teams.is_empty() {
        let d = backoff.call(now, || {
            gql.call(
                LINEAR_TRACKER_QUERY,
                &json!({"filter": {"or": teams.iter().map(|t| team_filter(t)).collect::<Vec<_>>()}}),
            )
        })?;
        for t in d["teams"]["nodes"].as_array().into_iter().flatten() {
            for p in t["projects"]["nodes"].as_array().into_iter().flatten() {
                projects.push(json!({"id": p["id"], "name": p["name"], "team": t["key"]}));
            }
            for l in names(&t["labels"], "name") {
                if !is_orch_label(&l) && !labels.contains(&l) {
                    labels.push(l);
                }
            }
            let mut st: Vec<&Value> = t["states"]["nodes"].as_array().into_iter().flatten().collect();
            st.sort_by(|a, b| a["position"].as_f64().unwrap_or(0.0).total_cmp(&b["position"].as_f64().unwrap_or(0.0)));
            states.extend(st.into_iter().map(|s| {
                let (kind, name) = (s["type"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""));
                // The column a state maps to without a project mapping (the form shows it as the default).
                json!({"id": s["id"], "name": s["name"], "type": s["type"],
                       "default_column": crate::linear::default_column(kind, name)})
            }));
        }
    }
    Ok(json!({"teams": all, "projects": projects, "labels": labels, "states": states}))
}

// ------------------------------------------------------------------ repo docs

/// Compiles a repo-docs glob: `*` stays within a folder, `**` crosses them.
pub fn compile_glob(glob: &str) -> Result<globset::GlobMatcher, String> {
    globset::GlobBuilder::new(glob.trim())
        .literal_separator(true)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| e.to_string())
}

/// The commit at the tip of local `branch`, if it exists (no network).
pub fn branch_tip(dir: &Path, branch: &str) -> Option<String> {
    if branch.starts_with('-') {
        return None;
    }
    orch_git::gh::rev_sha(dir, &format!("refs/heads/{branch}"))
}

/// YAML-ish front matter (`key: value` lines between `---` fences) and the
/// text after it.
pub fn front_matter(text: &str) -> (BTreeMap<String, String>, &str) {
    let mut keys = BTreeMap::new();
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return (keys, text);
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        let l = line.trim_end();
        if l == "---" || l == "..." {
            for kv in rest[..at - line.len()].lines() {
                if let Some((k, v)) = kv.split_once(':') {
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
                    keys.insert(k.trim().to_ascii_lowercase(), v);
                }
            }
            return (keys, &rest[at..]);
        }
    }
    (keys, text)
}

fn doc_item(path: &str, oid: &str, text: &str, updated_at_ms: u64) -> Option<Fetched> {
    let (fm, body) = front_matter(text);
    if fm.get("status").is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "done" | "started")) {
        return None;
    }
    let heading = body.lines().find_map(|l| l.strip_prefix("# ")).map(|h| h.trim().to_string());
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.into());
    let title = fm.get("title").filter(|t| !t.is_empty()).cloned().or(heading).unwrap_or(stem);
    let (body, truncated) = clip(body.trim());
    let mode = fm.get("mode").map(|m| m.to_ascii_lowercase()).filter(|m| m == "team" || m == "solo");
    Some(Fetched {
        reference: path.to_string(),
        title,
        url: None,
        body,
        truncated,
        rev: oid.to_string(),
        updated_at_ms,
        size: fm.get("size").and_then(|s| Size::parse(s)),
        mode,
        team: fm.get("team").filter(|t| !t.is_empty()).cloned(),
        ..Default::default()
    })
}

/// The blobs of `tip` matching `glob`: (path, oid), in path order. Symlinks
/// and submodules are left out; paths come from git's tree, so none leaves
/// the repository.
fn matching_blobs(dir: &Path, tip: &str, glob: &str) -> R<Vec<(String, String)>> {
    let m = compile_glob(glob).map_err(|e| SourceError::new("bad_glob", format!("bad glob {glob:?}: {e}")))?;
    let out = orch_git::git::git(dir, &["ls-tree", "-r", "-z", "--full-tree", tip])
        .map_err(|e| SourceError::new("failed", format!("{e:#}")))?;
    let mut v: Vec<(String, String)> = out
        .split('\0')
        .filter_map(|e| {
            let (head, path) = e.split_once('\t')?;
            let mut f = head.split(' ');
            let (mode, kind, oid) = (f.next()?, f.next()?, f.next()?);
            (kind == "blob" && (mode == "100644" || mode == "100755") && m.is_match(path))
                .then(|| (path.to_string(), oid.to_string()))
        })
        .collect();
    v.sort();
    Ok(v)
}

fn resolve_tip(dir: &Path, branch: &str) -> R<String> {
    branch_tip(dir, branch).ok_or_else(|| SourceError::new("branch_missing", format!("no local branch {branch}")))
}

/// The docs matching `glob` on local `branch`: the listing and the tip it was read at.
pub fn repo_docs(dir: &Path, glob: &str, branch: &str) -> R<(Listing, String)> {
    let tip = resolve_tip(dir, branch)?;
    let blobs = matching_blobs(dir, &tip, glob)?;
    let at = orch_git::git::git(dir, &["show", "-s", "--format=%ct", &tip])
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(0, |s| s * 1000);
    let mut items = Vec::new();
    for (path, oid) in blobs.iter().take(DOCS_MAX) {
        let text = orch_git::git::git(dir, &["cat-file", "blob", oid])
            .map_err(|e| SourceError::new("failed", format!("{e:#}")))?;
        items.extend(doc_item(path, oid, &text, at));
    }
    Ok((Listing::capped(items), tip))
}

/// The files the glob matches on `branch` (paths), for the Local options and preview.
pub fn repo_docs_paths(dir: &Path, glob: &str, branch: &str) -> R<Vec<String>> {
    let tip = resolve_tip(dir, branch)?;
    Ok(matching_blobs(dir, &tip, glob)?.into_iter().map(|(p, _)| p).collect())
}

/// `intake_options` for repo docs: how many files the glob matches.
pub fn repo_docs_matches(dir: &Path, glob: &str, branch: &str) -> R<usize> {
    let tip = resolve_tip(dir, branch)?;
    Ok(matching_blobs(dir, &tip, glob)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A `gh` answering by the API path, recording every call.
    #[derive(Clone, Default)]
    pub(crate) struct FakeGh {
        pub answers: Arc<Mutex<BTreeMap<String, Result<String, String>>>>,
        pub calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl FakeGh {
        fn answer(&self, key: &str, v: Value) {
            self.answers.lock().unwrap().insert(key.into(), Ok(v.to_string()));
        }
    }

    impl Gh for FakeGh {
        fn run(&self, args: &[String]) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(args.to_vec());
            // The first argument after `api` that is not a flag (`--paginate`).
            let key = args[1..].iter().find(|a| !a.starts_with('-')).cloned().unwrap();
            let a = self.answers.lock().unwrap();
            let hit = a.iter().find(|(k, _)| key.starts_with(k.as_str())).map(|(_, v)| v.clone());
            match hit {
                Some(Ok(s)) => Ok(s),
                Some(Err(e)) => Err(anyhow::anyhow!("gh {}: {e}", args.join(" "))),
                None => Err(anyhow::anyhow!("gh {}: HTTP 404: Not Found", args.join(" "))),
            }
        }
    }

    /// Read-only: every call is a GET (no `-X`, no `-f`/`-F` but for the
    /// GraphQL query, which holds no mutation).
    pub(crate) fn assert_read_only(calls: &[Vec<String>]) {
        for c in calls {
            assert!(!c.iter().any(|a| a == "-X" || a == "--method"), "not a GET: {c:?}");
            let graphql = c.iter().any(|a| a == "graphql");
            if !graphql {
                assert!(!c.iter().any(|a| a == "-f" || a == "-F" || a.starts_with("--field")), "a field POSTs: {c:?}");
            }
            assert!(!c.iter().any(|a| a.contains("mutation")), "a mutation: {c:?}");
        }
    }

    #[test]
    fn gh_errors_have_their_codes() {
        let code = |m: &str| gh_error(&anyhow::anyhow!("gh api x: {m}")).code;
        assert_eq!(code("No such file or directory (os error 2)"), "gh_missing");
        assert_eq!(code("To get started with GitHub CLI, please run:  gh auth login"), "gh_auth");
        assert_eq!(code("HTTP 404: Not Found (https://api.github.com/repos/a/b/issues)"), "not_found");
        assert_eq!(code("API rate limit exceeded for user"), "rate_limited");
        assert_eq!(code("Your token has not been granted the required scopes: read:project"), "gh_scope");
        assert_eq!(code("boom"), "failed");
        assert_eq!(SourceError::new("gh_auth", "x").fix, Some("integrations"));
        assert_eq!(SourceError::new("bad_glob", "x").fix, Some("settings"));
        assert_eq!(SourceError::new("failed", "x").fix, None);
        let long = SourceError::new("failed", "y".repeat(500));
        assert_eq!(long.message.chars().count(), 200);
    }

    #[test]
    fn github_options_list_labels_and_milestones() {
        let gh = FakeGh::default();
        gh.answer("repos/a/b/labels", json!([{"name": "ready"}, {"name": "orch"}, {"name": "orch:ready"}]));
        gh.answer("repos/a/b/milestones", json!([{"title": "v2", "number": 2, "open_issues": 4}]));
        gh.answer(
            "graphql",
            json!({"data": {"repositoryOwner": {"projectsV2": {"nodes": [
            {"number": 3, "title": "Road", "closed": false, "field": {"options": [{"name": "Todo"}, {"name": "Done"}]}},
            {"number": 1, "title": "Old", "closed": true, "field": null}]}}}}),
        );
        let o = github_options(&gh, "a/b").unwrap();
        assert_eq!(
            o,
            json!({"labels": ["ready"], "milestones": [{"title": "v2", "open_issues": 4}],
                   "projects": [{"owner": "a", "number": 3, "title": "Road", "statuses": ["Todo", "Done"]}],
                   "error": null})
        );
        gh.answer("graphql", json!({"errors": [{"type": "INSUFFICIENT_SCOPES", "message": "read:project"}]}));
        let o = github_options(&gh, "a/b").unwrap();
        assert_eq!((o["projects"].as_array().unwrap().len(), &o["error"]["code"]), (0, &json!("gh_scope")));
        assert_read_only(&gh.calls.lock().unwrap());
    }

    /// Linear answering by the operation: the teams, then a team's projects, labels and states.
    struct FakeLinear {
        limited: Mutex<bool>,
    }

    impl Graphql for FakeLinear {
        fn call(&self, query: &str, _vars: &Value) -> Result<Value, GqlError> {
            if *self.limited.lock().unwrap() {
                return Err(GqlError::RateLimited { retry_after_s: Some(30) });
            }
            Ok(if query.contains("OrchTrackerTeams") {
                json!({"teams": {"nodes": [{"id": "t1", "key": "ENG", "name": "Engineering"}]}})
            } else {
                json!({"teams": {"nodes": [{"key": "ENG",
                    "labels": {"nodes": [{"name": "next"}, {"name": "orch"}]},
                    "states": {"nodes": [
                        {"id": "s2", "name": "In Review", "type": "started", "position": 2.0},
                        {"id": "s1", "name": "Todo", "type": "unstarted", "position": 1.0},
                        {"id": "s3", "name": "Done", "type": "completed", "position": 3.0},
                        {"id": "s0", "name": "Triage", "type": "triage", "position": 0.0}]},
                    "projects": {"nodes": [{"id": "p1", "name": "Checkout"}]}}]}})
            })
        }
    }

    #[test]
    fn linear_tracker_options_list_teams_and_the_picked_teams_states_with_their_default_column() {
        let (f, b) = (FakeLinear { limited: Mutex::new(false) }, Backoff::default());
        let none = linear_tracker_options(&f, &b, 1, &[]).unwrap();
        assert_eq!(none["teams"][0], json!({"id": "t1", "key": "ENG", "name": "Engineering"}));
        assert_eq!((none["states"].as_array().unwrap().len(), none["projects"].as_array().unwrap().len()), (0, 0));
        let o = linear_tracker_options(&f, &b, 1, &["ENG".to_string()]).unwrap();
        assert_eq!(o["labels"], json!(["next"]), "the orch label is not offered");
        assert_eq!(o["projects"][0], json!({"id": "p1", "name": "Checkout", "team": "ENG"}));
        let states: Vec<(&str, &str)> = o["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["default_column"].as_str().unwrap()))
            .collect();
        assert_eq!(
            states,
            [("Triage", "backlog"), ("Todo", "ready"), ("In Review", "review"), ("Done", "done")],
            "in the team's order, each with the column it maps to by default"
        );
        // A rate limit backs off: the next call does not reach Linear.
        *f.limited.lock().unwrap() = true;
        let e = linear_tracker_options(&f, &b, 1, &[]).unwrap_err();
        assert_eq!(e.code, "rate_limited");
        *f.limited.lock().unwrap() = false;
        assert_eq!(linear_tracker_options(&f, &b, 2, &[]).unwrap_err().code, "rate_limited", "still backing off");
        assert!(linear_tracker_options(&f, &b, 1 + 31_000, &[]).is_ok());
    }

    #[test]
    fn front_matter_and_titles() {
        let (fm, body) = front_matter("---\ntitle: \"Checkout v2\"\nsize: l\n---\n# Heading\nText\n");
        assert_eq!((fm["title"].as_str(), fm["size"].as_str(), body), ("Checkout v2", "l", "# Heading\nText\n"));
        let (fm, body) = front_matter("# Only\n");
        assert!(fm.is_empty() && body == "# Only\n");
        assert_eq!(doc_item("b/x.md", "o", "# Heading\nText", 0).unwrap().title, "Heading");
        assert_eq!(doc_item("b/01-x.md", "o", "plain", 0).unwrap().title, "01-x");
        assert!(doc_item("b/x.md", "o", "---\nstatus: Done\n---\n", 0).is_none());
        assert!(doc_item("b/x.md", "o", "---\nstatus: started\n---\n", 0).is_none());
        let d = doc_item("b/x.md", "o", "---\nmode: Solo\nteam: web\nsize: S\n---\nbody", 0).unwrap();
        assert_eq!((d.mode.as_deref(), d.team.as_deref(), d.size), (Some("solo"), Some("web"), Some(Size::S)));
    }

    #[test]
    fn repo_docs_read_the_local_branch_by_glob() {
        use orch_git::fixtures::{commit_file, git, scratch_repo};
        let (_d, repo) = scratch_repo();
        std::fs::create_dir_all(repo.join(".orchestrator/backlog/sub")).unwrap();
        commit_file(&repo, ".orchestrator/backlog/02-b.md", "---\ntitle: Bee\nsize: L\n---\nbody b", "b");
        commit_file(&repo, ".orchestrator/backlog/01-a.md", "# Ay\n", "a");
        commit_file(&repo, ".orchestrator/backlog/03-done.md", "---\nstatus: done\n---\n", "c");
        commit_file(&repo, ".orchestrator/backlog/sub/04-deep.md", "# Deep\n", "d");
        commit_file(&repo, ".orchestrator/backlog/notes.txt", "x", "e");
        let glob = orch_store::DEFAULT_INTAKE_GLOB;
        let (l, tip) = repo_docs(&repo, glob, "main").unwrap();
        let refs: Vec<&str> = l.items.iter().map(|i| i.reference.as_str()).collect();
        assert_eq!(refs, [".orchestrator/backlog/01-a.md", ".orchestrator/backlog/02-b.md"]);
        assert_eq!((l.items[0].title.as_str(), l.items[1].title.as_str()), ("Ay", "Bee"));
        assert_eq!(l.items[1].size, Some(Size::L));
        assert!(l.items[0].url.is_none() && l.items[0].updated_at_ms > 0);
        assert_eq!(Some(tip.clone()), branch_tip(&repo, "main"));
        assert_eq!(repo_docs_matches(&repo, glob, "main").unwrap(), 3);
        assert_eq!(repo_docs_matches(&repo, ".orchestrator/backlog/**/*.md", "main").unwrap(), 4);
        // An edit changes the blob id (the revision) and the tip.
        let rev = l.items[0].rev.clone();
        commit_file(&repo, ".orchestrator/backlog/01-a.md", "# Ay 2\n", "a2");
        let (l2, tip2) = repo_docs(&repo, glob, "main").unwrap();
        assert!(tip2 != tip && l2.items[0].rev != rev);
        // The working tree is not read: uncommitted files are not items.
        std::fs::write(repo.join(".orchestrator/backlog/00-wip.md"), "# Wip").unwrap();
        assert_eq!(repo_docs(&repo, glob, "main").unwrap().0.items.len(), 2);
        // A symlink in the tree is not followed.
        std::os::unix::fs::symlink("/etc/hosts", repo.join(".orchestrator/backlog/05-link.md")).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["-c", "user.name=T", "-c", "user.email=t@example.invalid", "commit", "-qm", "link"]);
        assert!(!repo_docs(&repo, glob, "main").unwrap().0.items.iter().any(|i| i.reference.contains("link")));
        assert_eq!(repo_docs(&repo, glob, "nope").unwrap_err().code, "branch_missing");
        assert_eq!(repo_docs(&repo, "a/[", "main").unwrap_err().code, "bad_glob");
    }
}
