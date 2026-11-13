//! Intake sources (FR-1312, P4.5 spec §3.2): what a project's Ready to start
//! lane lists. Read-only towards every source (D3): `gh api` GETs and one
//! GraphQL query (Projects v2), Linear queries, `git ls-tree` / `cat-file`
//! on the local branch (D2). Issues labelled `orch` / `orch:*` are team
//! tasks, never intake (D4); pull requests are not issues.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use orch_store::{GithubIntakeFilter, IntakeUpsert};
use serde::Serialize;
use serde_json::{Value, json};

use crate::github::Gh;
use crate::linear::{GqlError, Graphql};

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

/// The source error of a failed `gh` call.
pub fn gh_error(e: &anyhow::Error) -> SourceError {
    let m = format!("{e:#}");
    let l = m.to_ascii_lowercase();
    let code = if l.contains("no such file") || l.contains("not found in path") || l.contains("program not found") {
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

/// An open GitHub issue as an item (`None` when closed or a team task).
/// REST and GraphQL name the URL and update time differently.
fn gh_issue(c: &Value, url_key: &str, updated_key: &str, reference: impl FnOnce(u64) -> String) -> Option<Fetched> {
    if !c["state"].as_str().unwrap_or("open").eq_ignore_ascii_case("open") {
        return None;
    }
    let labels = names(&c["labels"], "name");
    if labels.iter().any(|l| is_orch_label(l)) {
        return None;
    }
    let number = c["number"].as_u64()?;
    let (body, truncated) = clip(c["body"].as_str().unwrap_or(""));
    let updated = st(&c[updated_key]);
    let mut extra = serde_json::Map::new();
    extra.insert("number".into(), json!(number));
    extra.insert("assignees".into(), json!(names(&c["assignees"], "login")));
    extra.insert("milestone".into(), c["milestone"]["title"].clone());
    Some(Fetched {
        reference: reference(number),
        title: st(&c["title"]),
        url: c[url_key].as_str().map(str::to_string),
        body,
        truncated,
        updated_at_ms: ts_ms(&updated),
        rev: updated,
        size: Size::of_labels(&labels),
        labels,
        extra,
        ..Default::default()
    })
}

/// A REST issue as an item (`None` for a pull request too).
fn rest_issue(v: &Value) -> Option<Fetched> {
    if v.get("pull_request").is_some() {
        return None;
    }
    gh_issue(v, "html_url", "updated_at", |n| format!("#{n}"))
}

fn rest_issues(gh: &dyn Gh, repo: &str, query: &str) -> R<Listing> {
    // Every page (`gh` prints one JSON array per page, back to back), oldest
    // first (spec §3.2 order). A GET: no `-f` here, `gh` would POST.
    let path = format!("repos/{repo}/issues?state=open&{query}&sort=created&direction=asc&per_page=100");
    let out = gh_run(gh, &["api", "--paginate", &path])?;
    let not_json = || SourceError::new("failed", format!("gh api {path}: not JSON"));
    let mut items = Vec::new();
    let mut pages = 0;
    for page in serde_json::Deserializer::from_str(&out).into_iter::<Value>() {
        let Value::Array(a) = page.map_err(|_| not_json())? else { return Err(not_json()) };
        pages += 1;
        if pages > MAX_PAGES {
            let mut l = Listing::capped(items);
            l.partial = true;
            return Ok(l);
        }
        items.extend(a.iter().filter_map(rest_issue));
    }
    // No page at all is no answer, not an empty source.
    if pages == 0 {
        return Err(not_json());
    }
    Ok(Listing::capped(items))
}

const PROJECT_QUERY: &str = "query OrchIntakeProject($owner: String!, $number: Int!, $after: String) { repositoryOwner(login: $owner) { \
    ... on ProjectV2Owner { projectV2(number: $number) { title url items(first: 100, after: $after) { \
    pageInfo { hasNextPage endCursor } nodes { \
    fieldValueByName(name: \"Status\") { ... on ProjectV2ItemFieldSingleSelectValue { name } } \
    content { __typename ... on Issue { number title body url state updatedAt repository { nameWithOwner } \
    labels(first: 20) { nodes { name } } assignees(first: 5) { nodes { login } } milestone { title } } } } } } } } }";

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

fn project_items(gh: &dyn Gh, repo: &str, owner: &str, number: u64, status: Option<&str>) -> R<Listing> {
    let mut items = Vec::new();
    let mut url = None;
    let mut after: Option<String> = None;
    let mut partial = false;
    for page in 0..MAX_PAGES {
        let mut args = vec![
            "api".to_string(),
            "graphql".into(),
            "-f".into(),
            format!("query={PROJECT_QUERY}"),
            "-f".into(),
            format!("owner={owner}"),
            "-F".into(),
            format!("number={number}"),
        ];
        if let Some(c) = &after {
            args.extend(["-f".into(), format!("after={c}")]);
        }
        let v = gh_json(gh, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
        if let Some(e) = graphql_errors(&v) {
            return Err(e);
        }
        let project = &v["data"]["repositoryOwner"]["projectV2"];
        if project.is_null() {
            return Err(SourceError::new("not_found", format!("no GitHub project {owner}/{number}")));
        }
        items.extend(project["items"]["nodes"].as_array().into_iter().flatten().filter_map(|n| {
            if !status.is_none_or(|want| {
                n["fieldValueByName"]["name"].as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case(want.trim()))
            }) {
                return None;
            }
            let c = &n["content"];
            if c["__typename"] != "Issue" || c["state"].is_null() {
                return None;
            }
            let in_repo = c["repository"]["nameWithOwner"].as_str().unwrap_or(repo);
            let local = in_repo.eq_ignore_ascii_case(repo);
            let mut f =
                gh_issue(c, "url", "updatedAt", |n| if local { format!("#{n}") } else { format!("{in_repo}#{n}") })?;
            f.extra.insert("repo".into(), json!(in_repo));
            f.extra.insert("status".into(), n["fieldValueByName"]["name"].clone());
            Some(f)
        }));
        // The board's own link: `/orgs/…` or `/users/…` by who owns it.
        if page == 0 {
            url = project["url"].as_str().filter(|u| u.starts_with("https://")).map(str::to_string);
        }
        match next_cursor(&project["items"]["pageInfo"]) {
            Some(c) => after = Some(c),
            None => break,
        }
        partial = page + 1 == MAX_PAGES;
    }
    Ok(Listing { url, partial, ..Listing::capped(items) })
}

/// The open issues of `repo` that `filter` selects.
pub fn github(gh: &dyn Gh, repo: &str, filter: &GithubIntakeFilter) -> R<Listing> {
    match filter {
        GithubIntakeFilter::Label { names } => {
            let labels = names.iter().map(|n| enc(n)).collect::<Vec<_>>().join(",");
            rest_issues(gh, repo, &format!("labels={labels}"))
        }
        GithubIntakeFilter::Milestone { title } => {
            let ms = gh_json(gh, &["api", &format!("repos/{repo}/milestones?state=open&per_page=100")])?;
            let number = ms
                .as_array()
                .into_iter()
                .flatten()
                .find(|m| m["title"].as_str().is_some_and(|t| t.trim() == title.trim()))
                .and_then(|m| m["number"].as_u64())
                .ok_or_else(|| SourceError::new("not_found", format!("no open milestone {title:?} in {repo}")))?;
            rest_issues(gh, repo, &format!("milestone={number}"))
        }
        GithubIntakeFilter::Project { owner, number, status } => {
            project_items(gh, repo, owner, *number, status.as_deref())
        }
    }
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
        GqlError::Failed(m) => SourceError::new("failed", format!("Linear: {m}")),
    }
}

/// A team filter by id (a UUID) or key (`ENG`).
fn team_filter(team: &str) -> Value {
    let uuid = team.len() == 36 && team.chars().filter(|c| *c == '-').count() == 4;
    if uuid { json!({"id": {"eq": team}}) } else { json!({"key": {"eq": team}}) }
}

const LINEAR_QUERY: &str = "query OrchIntakeIssues($filter: IssueFilter, $after: String) { issues(first: 100, after: $after, filter: $filter) { \
    pageInfo { hasNextPage endCursor } nodes { id identifier title description url priority estimate sortOrder updatedAt team { key } \
    project { slugId } labels { nodes { name } } } } }";

/// What a Linear intake source lists.
pub struct LinearQuery<'a> {
    pub team: &'a str,
    pub project: Option<&'a str>,
    pub label: Option<&'a str>,
    pub include_backlog: bool,
}

/// Open issues of the team in the kept state types, priority then sort order.
pub fn linear(gql: &dyn Graphql, backoff: &Backoff, now: u64, q: &LinearQuery) -> R<Listing> {
    let mut types = vec!["unstarted"];
    if q.include_backlog {
        types.push("backlog");
    }
    let mut filter = json!({"team": team_filter(q.team), "state": {"type": {"in": types}}});
    if let Some(p) = q.project {
        filter["project"] = json!({"id": {"eq": p}});
    }
    if let Some(l) = q.label {
        filter["labels"] = json!({"name": {"eq": l}});
    }
    // Every page, each inside the back-off.
    let mut nodes: Vec<Value> = Vec::new();
    let mut after = Value::Null;
    let mut partial = false;
    for page in 0..MAX_PAGES {
        let mut d = backoff.call(now, || gql.call(LINEAR_QUERY, &json!({"filter": filter, "after": after})))?;
        if let Value::Array(a) = d["issues"]["nodes"].take() {
            nodes.extend(a);
        }
        match next_cursor(&d["issues"]["pageInfo"]) {
            Some(c) => after = json!(c),
            None => break,
        }
        partial = page + 1 == MAX_PAGES;
    }
    let mut rows: Vec<(u64, f64, Fetched)> = nodes
        .iter()
        .filter_map(|v| {
            let labels = names(&v["labels"], "name");
            if labels.iter().any(|l| is_orch_label(l)) {
                return None;
            }
            let (body, truncated) = clip(v["description"].as_str().unwrap_or(""));
            let updated = st(&v["updatedAt"]);
            let priority = v["priority"].as_u64().unwrap_or(0);
            let estimate = v["estimate"].as_f64();
            let mut extra = serde_json::Map::new();
            extra.insert("identifier".into(), v["identifier"].clone());
            extra.insert("priority".into(), json!(priority));
            extra.insert("estimate".into(), json!(estimate));
            let f = Fetched {
                reference: v["identifier"]
                    .as_str()
                    .filter(|i| !i.is_empty())
                    .map_or_else(|| st(&v["id"]), str::to_string),
                title: st(&v["title"]),
                url: v["url"].as_str().map(str::to_string),
                body,
                truncated,
                updated_at_ms: ts_ms(&updated),
                rev: updated,
                size: estimate.map(Size::of_points),
                labels,
                extra,
                ..Default::default()
            };
            // Priority 1 (urgent) … 4 (low); 0 (none) last.
            let rank = if priority == 0 { 5 } else { priority };
            Some((rank, v["sortOrder"].as_f64().unwrap_or(0.0), f))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let url = linear_view_url(&nodes, q.project.is_some());
    Ok(Listing { url, partial, ..Listing::capped(rows.into_iter().map(|r| r.2).collect()) })
}

/// The Linear view of the source: the project (`/project/<slugId>`) or the
/// team (`/team/<KEY>/active`), the workspace slug being the one in the
/// issues' own links. `None` while no issue says them.
fn linear_view_url(nodes: &[Value], by_project: bool) -> Option<String> {
    nodes.iter().find_map(|v| {
        let ws = v["url"].as_str()?.strip_prefix("https://linear.app/")?.split('/').next().filter(|w| !w.is_empty())?;
        Some(if by_project {
            format!("https://linear.app/{ws}/project/{}", v["project"]["slugId"].as_str().filter(|s| !s.is_empty())?)
        } else {
            format!("https://linear.app/{ws}/team/{}/active", v["team"]["key"].as_str().filter(|s| !s.is_empty())?)
        })
    })
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
        fn fail(&self, key: &str, e: &str) {
            self.answers.lock().unwrap().insert(key.into(), Err(e.into()));
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

    fn view(v: &Value, by_project: bool) -> Option<String> {
        linear_view_url(v.as_array().unwrap(), by_project)
    }

    fn issue(n: u64, labels: &[&str]) -> Value {
        json!({"number": n, "title": format!("Issue {n}"), "body": "x".repeat(5000), "state": "open",
               "html_url": format!("https://github.com/acme/shop/issues/{n}"), "updated_at": "2026-11-01T10:00:00Z",
               "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
               "assignees": [{"login": "ann"}], "milestone": {"title": "v2"}})
    }

    #[test]
    fn github_label_excludes_prs_and_team_tasks_and_caps() {
        let gh = FakeGh::default();
        let mut list = vec![issue(1, &["ready", "Size:L"]), issue(2, &["ready", "orch"]), issue(3, &["orch:review"])];
        let mut pr = issue(4, &["ready"]);
        pr["pull_request"] = json!({"url": "x"});
        list.push(pr);
        list.extend((10..70).map(|n| issue(n, &["ready"])));
        gh.answer("repos/acme/shop/issues?state=open&labels=ready,good%20first", json!(list));
        let l =
            github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["ready".into(), "good first".into()] })
                .unwrap();
        assert_eq!((l.items.len(), l.total, l.more()), (50, 61, 11));
        let first = &l.items[0];
        assert_eq!((first.reference.as_str(), first.size, first.truncated), ("#1", Some(Size::L), true));
        assert_eq!(first.body.chars().count(), BODY_MAX);
        assert_eq!(first.updated_at_ms, 1_793_527_200_000);
        assert!(!l.items.iter().any(|i| ["#2", "#3", "#4"].contains(&i.reference.as_str())));
        let up = first.upsert();
        assert_eq!((up.meta["assignees"][0].as_str(), up.meta["milestone"].as_str()), (Some("ann"), Some("v2")));
        let first = gh.calls.lock().unwrap()[0].clone();
        assert!(
            first.iter().any(|a| a == "--paginate") && first.last().unwrap().contains("sort=created&direction=asc")
        );
        assert_read_only(&gh.calls.lock().unwrap());
        gh.fail("repos/acme/shop/issues", "exec: No such file or directory (os error 2)");
        let e = github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["ready".into()] }).unwrap_err();
        assert_eq!((e.code, e.fix), ("gh_missing", Some("integrations")));
        // gh's answer is classified, not our arguments: a label named "not found".
        gh.fail("repos/acme/shop/issues", "HTTP 500: server error");
        let e = github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["not found".into()] }).unwrap_err();
        assert_eq!(e.code, "failed");
    }

    #[test]
    fn github_milestone_resolves_its_number() {
        let gh = FakeGh::default();
        gh.answer("repos/acme/shop/milestones", json!([{"title": "v1", "number": 1}, {"title": "v2.1", "number": 4}]));
        gh.answer("repos/acme/shop/issues?state=open&milestone=4", json!([issue(7, &[])]));
        let l = github(&gh, "acme/shop", &GithubIntakeFilter::Milestone { title: "v2.1".into() }).unwrap();
        assert_eq!(l.items[0].reference, "#7");
        let e = github(&gh, "acme/shop", &GithubIntakeFilter::Milestone { title: "v9".into() }).unwrap_err();
        assert_eq!(e.code, "not_found");
        assert_read_only(&gh.calls.lock().unwrap());
    }

    #[test]
    fn github_project_keeps_open_issues_of_the_status_in_board_order() {
        let gh = FakeGh::default();
        let node = |n: u64, status: &str, kind: &str, state: &str, repo: &str, labels: &[&str]| {
            json!({"fieldValueByName": {"name": status}, "content": {"__typename": kind, "number": n,
                "title": format!("P{n}"), "body": "b", "url": format!("https://github.com/{repo}/issues/{n}"),
                "state": state, "updatedAt": "2026-11-02T00:00:00Z", "repository": {"nameWithOwner": repo},
                "labels": {"nodes": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>()},
                "assignees": {"nodes": []}, "milestone": null}})
        };
        gh.answer(
            "graphql",
            json!({"data": {"repositoryOwner": {"projectV2": {"title": "Road", "items": {"nodes": [
                node(9, "Todo", "Issue", "OPEN", "acme/shop", &[]),
                node(3, "Done", "Issue", "OPEN", "acme/shop", &[]),
                node(5, "todo", "PullRequest", "OPEN", "acme/shop", &[]),
                node(6, "Todo", "Issue", "CLOSED", "acme/shop", &[]),
                node(7, "Todo", "Issue", "OPEN", "acme/shop", &["orch"]),
                node(2, "Todo", "Issue", "OPEN", "acme/api", &["size/S"]),
            ]}}}}}),
        );
        let f = GithubIntakeFilter::Project { owner: "acme".into(), number: 7, status: Some("Todo".into()) };
        let l = github(&gh, "acme/shop", &f).unwrap();
        let refs: Vec<&str> = l.items.iter().map(|i| i.reference.as_str()).collect();
        assert_eq!(refs, ["#9", "acme/api#2"]);
        assert_eq!(l.items[1].size, Some(Size::S));
        let all =
            github(&gh, "acme/shop", &GithubIntakeFilter::Project { owner: "acme".into(), number: 7, status: None })
                .unwrap();
        assert_eq!(all.items.len(), 3);
        let calls = gh.calls.lock().unwrap().clone();
        assert!(calls[0].iter().any(|a| a == "number=7") && calls[0].iter().any(|a| a == "owner=acme"));
        assert_read_only(&calls);
        // Scope missing, no such project.
        gh.answer("graphql", json!({"errors": [{"type": "INSUFFICIENT_SCOPES", "message": "needs read:project"}]}));
        assert_eq!(github(&gh, "acme/shop", &f).unwrap_err().fix, Some("gh_scope"));
        gh.answer("graphql", json!({"data": {"repositoryOwner": {"projectV2": null}}}));
        assert_eq!(github(&gh, "acme/shop", &f).unwrap_err().code, "not_found");
    }

    /// A `gh` serving a Projects v2 board of `n` items in pages of 100, by
    /// the `after` cursor it is sent, and REST pages for `--paginate`.
    struct PagedGh {
        calls: Mutex<Vec<Vec<String>>>,
        /// Every board item, in board order.
        board: Vec<Value>,
        /// The REST issues (one array per page, printed back to back).
        rest: Vec<Vec<Value>>,
    }

    impl Gh for PagedGh {
        fn run(&self, args: &[String]) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(args.to_vec());
            if args.iter().any(|a| a == "graphql") {
                let start = args
                    .iter()
                    .find_map(|a| a.strip_prefix("after="))
                    .map_or(0, |c| c.strip_prefix("c").unwrap().parse::<usize>().unwrap());
                let end = (start + 100).min(self.board.len());
                let next = end < self.board.len();
                return Ok(json!({"data": {"repositoryOwner": {"projectV2": {"title": "Road", "items": {
                    "pageInfo": {"hasNextPage": next, "endCursor": format!("c{end}")},
                    "nodes": self.board[start..end]}}}}})
                .to_string());
            }
            Ok(self.rest.iter().map(|p| json!(p).to_string()).collect::<Vec<_>>().join("\n"))
        }
    }

    fn board_node(n: u64, status: &str, labels: &[&str]) -> Value {
        json!({"fieldValueByName": {"name": status}, "content": {"__typename": "Issue", "number": n,
            "title": format!("P{n}"), "body": "b", "url": format!("https://github.com/acme/shop/issues/{n}"),
            "state": "OPEN", "updatedAt": "2026-11-02T00:00:00Z", "repository": {"nameWithOwner": "acme/shop"},
            "labels": {"nodes": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>()},
            "assignees": {"nodes": []}, "milestone": null}})
    }

    fn project(status: Option<&str>) -> GithubIntakeFilter {
        GithubIntakeFilter::Project { owner: "acme".into(), number: 7, status: status.map(str::to_string) }
    }

    #[test]
    fn a_project_board_is_read_page_after_page_in_board_order() {
        // 250 items over 3 pages; the 180th and 240th are in the Status, so is an `orch` one on page 3.
        let board = (1..=250)
            .map(|n| match n {
                180 | 240 => board_node(n, "Ready", &[]),
                245 => board_node(n, "Ready", &["orch"]),
                _ => board_node(n, "Done", &[]),
            })
            .collect();
        let gh = PagedGh { calls: Default::default(), board, rest: vec![] };
        let l = github(&gh, "acme/shop", &project(Some("Ready"))).unwrap();
        let refs: Vec<&str> = l.items.iter().map(|i| i.reference.as_str()).collect();
        assert_eq!((refs, l.total, l.partial), (vec!["#180", "#240"], 2, false));
        let calls = gh.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3);
        assert!(!calls[0].iter().any(|a| a.starts_with("after=")));
        assert!(calls[1].iter().any(|a| a == "after=c100") && calls[2].iter().any(|a| a == "after=c200"));
        assert_read_only(&calls);
        // No Status: every match counts, only CAP are listed.
        let l = github(&gh, "acme/shop", &project(None)).unwrap();
        assert_eq!((l.items.len(), l.total, l.more()), (50, 249, 199));
    }

    #[test]
    fn a_source_past_twenty_pages_is_partial() {
        let board = (1..=2150).map(|n| board_node(n, "Ready", &[])).collect();
        let gh = PagedGh { calls: Default::default(), board, rest: vec![] };
        let l = github(&gh, "acme/shop", &project(Some("Ready"))).unwrap();
        assert_eq!((l.total, l.partial, gh.calls.lock().unwrap().len()), (2000, true, MAX_PAGES));
        // Exactly 20 pages: the end was reached, nothing is cut.
        let board = (1..=2000).map(|n| board_node(n, "Ready", &[])).collect();
        let gh = PagedGh { calls: Default::default(), board, rest: vec![] };
        let l = github(&gh, "acme/shop", &project(Some("Ready"))).unwrap();
        assert_eq!((l.total, l.partial), (2000, false));
        // REST: 21 pages of 100 read 20.
        let rest = (0..21).map(|p| (0..100).map(|i| issue(p * 100 + i + 1, &["ready"])).collect()).collect();
        let gh = PagedGh { calls: Default::default(), board: vec![], rest };
        let l = github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["ready".into()] }).unwrap();
        assert_eq!((l.total, l.partial), (2000, true));
        // Linear: more pages forever.
        let gql = PagedLinear::new(3000);
        let q = LinearQuery { team: "ENG", project: None, label: None, include_backlog: false };
        let l = linear(&gql, &Backoff::default(), 1000, &q).unwrap();
        assert_eq!((l.total, l.partial, gql.calls.lock().unwrap().len()), (2000, true, MAX_PAGES));
    }

    #[test]
    fn label_issues_are_read_through_every_page_oldest_first() {
        let rest = vec![
            (1..=100).map(|n| issue(n, &["ready"])).collect(),
            (101..=130).map(|n| issue(n, &["ready"])).chain([issue(999, &["ready", "orch"])]).collect(),
        ];
        let gh = PagedGh { calls: Default::default(), board: vec![], rest };
        let l = github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["ready".into()] }).unwrap();
        assert_eq!((l.total, l.more(), l.partial), (130, 80, false));
        assert_eq!((l.items[0].reference.as_str(), l.items[49].reference.as_str()), ("#1", "#50"));
        let calls = gh.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][..2], ["api", "--paginate"]);
        assert!(calls[0][2].contains("sort=created&direction=asc&per_page=100"));
        assert_read_only(&calls);
    }

    /// Linear serving `n` issues in pages of 100, the last (lowest) priority first.
    struct PagedLinear {
        n: usize,
        calls: Mutex<Vec<Value>>,
    }

    impl PagedLinear {
        fn new(n: usize) -> PagedLinear {
            PagedLinear { n, calls: Default::default() }
        }
    }

    impl Graphql for PagedLinear {
        fn call(&self, _: &str, vars: &Value) -> Result<Value, GqlError> {
            self.calls.lock().unwrap().push(vars.clone());
            let start = vars["after"].as_str().map_or(0, |c| c.parse::<usize>().unwrap());
            let end = (start + 100).min(self.n);
            let nodes: Vec<Value> = (start..end)
                .map(|i| {
                    // The only urgent issue is the very last one read.
                    let prio = if i + 1 == self.n { 1 } else { 3 };
                    json!({"id": format!("u{i}"), "identifier": format!("ENG-{i}"), "title": "t", "description": "d",
                        "url": format!("https://linear.app/x/issue/ENG-{i}"), "priority": prio, "estimate": null,
                        "team": {"key": "ENG"}, "project": null, "sortOrder": i as f64, "updatedAt": "2026-11-03T00:00:00Z",
                        "labels": {"nodes": []}})
                })
                .collect();
            Ok(
                json!({"issues": {"pageInfo": {"hasNextPage": end < self.n, "endCursor": end.to_string()}, "nodes": nodes}}),
            )
        }
    }

    #[test]
    fn linear_is_sorted_over_every_page() {
        let gql = PagedLinear::new(150);
        let q = LinearQuery { team: "ENG", project: None, label: None, include_backlog: false };
        let l = linear(&gql, &Backoff::default(), 1000, &q).unwrap();
        assert_eq!((l.total, l.partial, l.items[0].reference.as_str()), (150, false, "ENG-149"));
        assert_eq!(l.items[1].reference, "ENG-0");
        let calls = gql.calls.lock().unwrap();
        assert_eq!((calls.len(), calls[0]["after"].clone(), calls[1]["after"].clone()), (2, Value::Null, json!("100")));
    }

    #[test]
    fn a_rate_limit_on_a_later_linear_page_backs_off() {
        struct Limited(Mutex<usize>);
        impl Graphql for Limited {
            fn call(&self, _: &str, _: &Value) -> Result<Value, GqlError> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                if *n == 2 {
                    return Err(GqlError::RateLimited { retry_after_s: Some(30) });
                }
                Ok(json!({"issues": {"pageInfo": {"hasNextPage": true, "endCursor": "c"}, "nodes": []}}))
            }
        }
        let b = Backoff::default();
        let q = LinearQuery { team: "ENG", project: None, label: None, include_backlog: false };
        let e = linear(&Limited(Mutex::new(0)), &b, 1000, &q).unwrap_err();
        assert_eq!((e.code, e.retry_at_ms), ("rate_limited", Some(31_000)));
    }

    #[test]
    fn a_project_board_links_to_where_github_says_it_is() {
        let gh = FakeGh::default();
        let board = |url: Value| json!({"data": {"repositoryOwner": {"projectV2": {"title": "Road", "url": url, "items": {"nodes": []}}}}});
        let f = |owner: &str| GithubIntakeFilter::Project { owner: owner.into(), number: 4, status: None };
        // A user-owned board is under /users/, an organization's under /orgs/.
        gh.answer("graphql", board(json!("https://github.com/users/me/projects/4")));
        assert_eq!(
            github(&gh, "me/shop", &f("me")).unwrap().url.as_deref(),
            Some("https://github.com/users/me/projects/4")
        );
        gh.answer("graphql", board(json!("https://github.com/orgs/acme/projects/4")));
        assert_eq!(
            github(&gh, "acme/shop", &f("acme")).unwrap().url.as_deref(),
            Some("https://github.com/orgs/acme/projects/4")
        );
        // No link in the answer: none is made up.
        gh.answer("graphql", board(Value::Null));
        assert_eq!(github(&gh, "acme/shop", &f("acme")).unwrap().url, None);
        // A label or milestone listing has no board.
        gh.answer("repos/acme/shop/issues", json!([]));
        assert_eq!(github(&gh, "acme/shop", &GithubIntakeFilter::Label { names: vec!["a".into()] }).unwrap().url, None);
    }

    #[test]
    fn a_linear_view_links_from_the_team_key_and_the_project_slug() {
        let nodes = |url: &str| {
            json!([{"url": url, "team": {"key": "ENG"}, "project": {"slugId": "ab12cd"}},
                   {"url": "https://linear.app/other/issue/X-1", "team": {"key": "X"}, "project": null}])
        };
        let url = "https://linear.app/acme/issue/ENG-42/refunds";
        assert_eq!(view(&nodes(url), false).as_deref(), Some("https://linear.app/acme/team/ENG/active"));
        assert_eq!(view(&nodes(url), true).as_deref(), Some("https://linear.app/acme/project/ab12cd"));
        // The first issue that says everything decides; none, none.
        let no_project = json!([{"url": url, "team": {"key": "ENG"}, "project": null},
                                {"url": url, "team": {"key": "ENG"}, "project": {"slugId": "zz"}}]);
        assert_eq!(view(&no_project, true).as_deref(), Some("https://linear.app/acme/project/zz"));
        assert_eq!(view(&json!([]), false), None);
        assert_eq!(view(&json!([{"url": "https://example.com/x", "team": {"key": "E"}}]), false), None);
        // Through the listing: the query asks for them, and the answer's become the source's link.
        let gql = FakeLinear::default();
        let q = LinearQuery { team: "ENG", project: None, label: None, include_backlog: false };
        let (query, _) = {
            linear(&gql, &Backoff::default(), 1000, &q).unwrap();
            gql.calls.lock().unwrap()[0].clone()
        };
        assert!(query.contains("team { key }") && query.contains("project { slugId }"));
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

    /// Linear answering issues by the filter it was sent, recording calls.
    #[derive(Default)]
    struct FakeLinear {
        calls: Mutex<Vec<(String, Value)>>,
        limited: Mutex<bool>,
    }

    impl Graphql for FakeLinear {
        fn call(&self, query: &str, vars: &Value) -> Result<Value, GqlError> {
            self.calls.lock().unwrap().push((query.to_string(), vars.clone()));
            if *self.limited.lock().unwrap() {
                return Err(GqlError::RateLimited { retry_after_s: Some(30) });
            }
            let types: Vec<String> = vars["filter"]["state"]["type"]["in"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            let issue = |id: &str, prio: u64, order: f64, est: Value, state: &str, labels: &[&str]| {
                json!({"id": format!("u-{id}"), "identifier": id, "title": format!("T {id}"), "description": "d",
                    "url": format!("https://linear.app/x/issue/{id}"), "priority": prio, "estimate": est,
                    "team": {"key": "ENG"}, "project": {"slugId": "ab12cd"},
                    "sortOrder": order, "updatedAt": "2026-11-03T00:00:00Z", "state": state,
                    "labels": {"nodes": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>()}})
            };
            let all = [
                issue("ENG-1", 0, 1.0, json!(null), "unstarted", &[]),
                issue("ENG-2", 2, 5.0, json!(1), "unstarted", &["next"]),
                issue("ENG-3", 2, 2.0, json!(8), "unstarted", &[]),
                issue("ENG-4", 1, 9.0, json!(3), "backlog", &[]),
                issue("ENG-5", 1, 0.0, json!(2), "unstarted", &["orch"]),
            ];
            let nodes: Vec<Value> =
                all.into_iter().filter(|i| types.iter().any(|t| i["state"] == t.as_str())).collect();
            Ok(json!({"issues": {"nodes": nodes}}))
        }
    }

    #[test]
    fn linear_lists_by_priority_with_the_backlog_flag_and_backs_off() {
        let gql = FakeLinear::default();
        let b = Backoff::default();
        let q = LinearQuery { team: "ENG", project: Some("pr-1"), label: Some("next"), include_backlog: false };
        let l = linear(&gql, &b, 1000, &q).unwrap();
        let refs: Vec<&str> = l.items.iter().map(|i| i.reference.as_str()).collect();
        assert_eq!(refs, ["ENG-3", "ENG-2", "ENG-1"], "priority, then sort order; none last; orch left out");
        assert_eq!(l.url.as_deref(), Some("https://linear.app/x/project/ab12cd"), "the project's view");
        assert_eq!((l.items[0].size, l.items[1].size, l.items[2].size), (Some(Size::L), Some(Size::S), None));
        assert_eq!((Size::of_points(3.0), Size::of_points(3.5), Size::of_points(0.5)), (Size::M, Size::L, Size::S));
        let (_, vars) = gql.calls.lock().unwrap()[0].clone();
        assert_eq!(vars["filter"]["team"], json!({"key": {"eq": "ENG"}}));
        assert_eq!(vars["filter"]["project"], json!({"id": {"eq": "pr-1"}}));
        assert_eq!(vars["filter"]["labels"], json!({"name": {"eq": "next"}}));
        let q2 = LinearQuery {
            team: "1b2c3d4e-0000-1111-2222-333344445555",
            project: None,
            label: None,
            include_backlog: true,
        };
        let l = linear(&gql, &b, 1000, &q2).unwrap();
        assert_eq!(l.items[0].reference, "ENG-4");
        assert_eq!(gql.calls.lock().unwrap()[1].1["filter"]["team"]["id"]["eq"], q2.team);
        for (query, _) in gql.calls.lock().unwrap().iter() {
            assert!(!query.contains("mutation"));
        }
        // 429: the error says when, and no call is made until then.
        *gql.limited.lock().unwrap() = true;
        let e = linear(&gql, &b, 2000, &q).unwrap_err();
        assert_eq!((e.code, e.message.contains("30 s"), e.retry_at_ms), ("rate_limited", true, Some(32_000)));
        assert_eq!(e.fix, None);
        *gql.limited.lock().unwrap() = false;
        let n = gql.calls.lock().unwrap().len();
        assert_eq!(linear(&gql, &b, 10_000, &q).unwrap_err().code, "rate_limited");
        assert_eq!(gql.calls.lock().unwrap().len(), n, "backing off");
        assert!(linear(&gql, &b, 32_001, &q).is_ok());
        assert_eq!(linear_error(&GqlError::Unauthorized("no Linear API key: x".into())).code, "linear_key");
        assert_eq!(linear_error(&GqlError::Unauthorized("401".into())).code, "linear_auth");
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
