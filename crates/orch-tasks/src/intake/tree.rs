//! The tracker tree (implement-improve-intake B2, contract §2): GitHub
//! milestones → issues → sub-issues, Linear initiatives → projects →
//! milestones → issues → sub-issues, Local backlog files (flat). Read-only
//! towards the sources (D3). Team tasks (`orch` / `orch:*` labels) are never
//! nodes: they are counted in their parent's `team_tasks` (F31).
//!
//! What a read covers (so Work's *Closed* and *All* filters have something to
//! show without reading a repository's whole history): every open issue, the
//! sub-issues of the loaded parents in every state, and the closed *root*
//! issues updated in the last [`CLOSED_WINDOW_DAYS`] days; Linear: the same
//! window for completed / canceled issues.

use serde_json::{Map, Value, json};

use orch_store::IntakeUpsert;

use super::{Backoff, Fetched, R, Size, SourceError, clip, gh_run, graphql_errors, names, next_cursor, st, ts_ms};
use crate::github::Gh;
use crate::linear::Graphql;

/// Closed items older than this are not read (see the module doc).
pub const CLOSED_WINDOW_DAYS: i64 = 90;
/// Issues per GraphQL page: with the nested connections below, well inside
/// GitHub's node limit.
const GH_PAGE: usize = 50;
/// A tree read stops at this many pages per query (`partial`).
const TREE_PAGES: usize = 40;
/// Parents whose sub-issues are fetched one by one in a refresh.
const PARENT_FILL_MAX: usize = 40;
/// Linear page sizes tried when a query is too complex: halved each time.
const LINEAR_PAGES: [usize; 4] = [100, 50, 25, 10];

/// One node of the tree as a source lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// `gh:owner/name#12`, `gh-milestone:owner/name/3`, `lin:ENG-42`, `lin-project:<id>`,
    /// `lin-milestone:<id>`, `lin-initiative:<id>`, `local:<path>`.
    pub id: String,
    /// `issue` | `milestone` | `linear_project` | `linear_milestone` | `initiative` | `local`.
    pub kind: &'static str,
    pub parent: Option<String>,
    pub f: Fetched,
}

impl Node {
    /// The cache row of the node.
    pub fn upsert(&self) -> IntakeUpsert {
        let mut u = self.f.upsert();
        u.node_id = self.id.clone();
        u.kind = self.kind.to_string();
        u.parent_id = self.parent.clone();
        u
    }
}

/// What a tree read returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    /// Parents before their children; siblings in the source's order.
    pub nodes: Vec<Node>,
    /// `gh_sub_issues`: GitHub did not return the hierarchy (F26).
    pub warning: Option<&'static str>,
    /// A query stopped at its page cap.
    pub partial: bool,
    /// The listing in the source's own UI.
    pub url: Option<String>,
    /// The listing is the whole tree (not an incremental read): a node
    /// missing from it is gone.
    pub full: bool,
}

/// Parents before children; siblings keep their order; a node whose parent
/// is not in the list is a root.
pub fn dfs_order(nodes: Vec<Node>) -> Vec<Node> {
    use std::collections::HashMap;
    let ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut kids: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        let p = n.parent.clone().filter(|p| ids.contains(p) && *p != n.id);
        kids.entry(p).or_default().push(i);
    }
    let mut out = Vec::with_capacity(nodes.len());
    let mut stack: Vec<usize> = kids.remove(&None).unwrap_or_default().into_iter().rev().collect();
    let mut seen = vec![false; nodes.len()];
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        out.push(i);
        if let Some(k) = kids.remove(&Some(nodes[i].id.clone())) {
            stack.extend(k.into_iter().rev());
        }
    }
    // A parent cycle (never from a tracker) would leave nodes out: keep them as roots.
    out.extend((0..nodes.len()).filter(|i| !seen[*i]));
    let mut nodes: Vec<Option<Node>> = nodes.into_iter().map(Some).collect();
    out.into_iter().filter_map(|i| nodes[i].take()).collect()
}

fn iso_days_ago(now_ms: u64, days: i64) -> String {
    let t = jiff::Timestamp::from_millisecond(now_ms as i64).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    t.checked_sub(jiff::SignedDuration::from_hours(24 * days)).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_string()
}

/// The `since` of an incremental read that follows a read started at `at_ms`: a minute earlier, so
/// nothing changed meanwhile is missed.
pub fn tree_since(at_ms: u64) -> String {
    let t =
        jiff::Timestamp::from_millisecond(at_ms.saturating_sub(60_000) as i64).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    t.to_string()
}

fn dep(id: Option<String>, reference: String, title: &str, url: Option<&str>, open: bool) -> Value {
    json!({"id": id, "ref": reference, "title": title, "url": url, "open": open})
}

// ------------------------------------------------------------------ GitHub

/// What a GitHub tree read is asked.
pub struct GithubTree<'a> {
    pub repo: &'a str,
    /// The Projects v2 board whose Status the nodes carry.
    pub board: Option<(&'a str, u64)>,
    pub group_by_milestone: bool,
    /// `Some`: an incremental read of what changed since (open or closed).
    pub since: Option<&'a str>,
    pub now_ms: u64,
}

const ISSUE_FIELDS_LITE: &str = "number title body url state stateReason updatedAt createdAt \
    labels(first: 30) { nodes { name } } assignees(first: 5) { nodes { login } } milestone { number title } \
    closedByPullRequestsReferences(first: 5, includeClosedPrs: true) { nodes { number title url state merged } } \
    projectItems(first: 5) { nodes { project { number owner { ... on Organization { login } ... on User { login } } } \
    fieldValueByName(name: \"Status\") { ... on ProjectV2ItemFieldSingleSelectValue { name } } } }";

/// The hierarchy fields: GitHub without sub-issues / dependencies / issue
/// types rejects the query, and the read falls back to the lite fields.
const ISSUE_FIELDS_TREE: &str = "issueType { name } parent { number } subIssuesSummary { total completed } \
    blockedBy(first: 20) { nodes { number title url state repository { nameWithOwner } } } \
    blocking(first: 20) { nodes { number title url state repository { nameWithOwner } } }";

fn issue_fields(tree: bool) -> String {
    if tree { format!("{ISSUE_FIELDS_LITE} {ISSUE_FIELDS_TREE}") } else { ISSUE_FIELDS_LITE.to_string() }
}

fn issues_query(tree: bool) -> String {
    format!(
        "query OrchTreeIssues($owner: String!, $name: String!, $first: Int!, $after: String, $states: [IssueState!], \
         $since: DateTime) {{ repository(owner: $owner, name: $name) {{ url \
         issues(first: $first, after: $after, states: $states, orderBy: {{field: CREATED_AT, direction: ASC}}, \
         filterBy: {{since: $since}}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ {} }} }} }} }}",
        issue_fields(tree)
    )
}

fn sub_issues_query(tree: bool) -> String {
    format!(
        "query OrchTreeSubIssues($owner: String!, $name: String!, $number: Int!, $after: String) {{ repository(owner: $owner, name: $name) \
         {{ issue(number: $number) {{ subIssues(first: 50, after: $after) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ {} }} }} }} }} }}",
        issue_fields(tree)
    )
}

const MILESTONES_QUERY: &str = "query OrchTreeMilestones($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { \
    milestones(first: 50, states: OPEN, orderBy: {field: DUE_DATE, direction: ASC}) { nodes { number title url dueOn updatedAt \
    open: issues(states: OPEN) { totalCount } closed: issues(states: CLOSED) { totalCount } } } } }";

fn gql(gh: &dyn Gh, query: &str, vars: &[(&str, String)]) -> R<Value> {
    let mut args = vec!["api".to_string(), "graphql".into(), "-f".into(), format!("query={query}")];
    for (k, v) in vars {
        // Numbers go as typed fields (`-F`), text as raw (`-f`).
        let typed = v.parse::<i64>().is_ok() && *k != "name" && *k != "owner";
        args.push(if typed { "-F".into() } else { "-f".into() });
        args.push(format!("{k}={v}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = gh_run(gh, &refs)?;
    let v: Value = serde_json::from_str(&out).map_err(|_| SourceError::new("failed", "gh api graphql: not JSON"))?;
    match graphql_errors(&v) {
        Some(e) => Err(e),
        None => Ok(v),
    }
}

/// A GraphQL error that says the hierarchy fields do not exist (GitHub
/// Enterprise, or the feature is off for the repository).
fn lacks_hierarchy(e: &SourceError) -> bool {
    let m = e.message.to_ascii_lowercase();
    ["subissues", "parent", "issuetype", "blockedby", "blocking", "doesn't exist on type", "undefinedfield"]
        .iter()
        .any(|k| m.contains(k))
}

/// The labels of an issue minus the ones the app manages, and whether any is
/// a team-task label.
fn labels_of(c: &Value) -> (Vec<String>, bool) {
    let all = names(&c["labels"], "name");
    let team = all.iter().any(|l| super::is_orch_label(l));
    (all.into_iter().filter(|l| !super::is_orch_label(l)).collect(), team)
}

struct Raw<'a> {
    c: &'a Value,
    team_task: bool,
}

fn gh_dep(repo: &str, d: &Value) -> Value {
    let number = d["number"].as_u64().unwrap_or(0);
    let other = d["repository"]["nameWithOwner"].as_str().filter(|r| !r.is_empty() && *r != repo);
    let open = d["state"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("open"));
    match other {
        Some(r) => dep(None, format!("{r}#{number}"), d["title"].as_str().unwrap_or(""), d["url"].as_str(), open),
        None => dep(
            Some(format!("gh:{repo}#{number}")),
            format!("#{number}"),
            d["title"].as_str().unwrap_or(""),
            d["url"].as_str(),
            open,
        ),
    }
}

fn gh_issue_node(raw: &Raw, o: &GithubTree, milestones: &std::collections::HashSet<u64>) -> Option<Node> {
    if raw.team_task {
        return None;
    }
    let c = raw.c;
    let number = c["number"].as_u64()?;
    let repo = o.repo;
    let (labels, _) = labels_of(c);
    let state = if c["state"].as_str().unwrap_or("OPEN").eq_ignore_ascii_case("open") { "open" } else { "closed" };
    let (body, truncated) = clip(c["body"].as_str().unwrap_or(""));
    let updated = st(&c["updatedAt"]);
    let parent = c["parent"]["number"].as_u64().map(|p| format!("gh:{repo}#{p}")).or_else(|| {
        let m = c["milestone"]["number"].as_u64().filter(|m| o.group_by_milestone && milestones.contains(m))?;
        Some(format!("gh-milestone:{repo}/{m}"))
    });
    let board_status = o.board.and_then(|(owner, number)| {
        c["projectItems"]["nodes"].as_array()?.iter().find_map(|i| {
            let p = &i["project"];
            let same = p["number"].as_u64() == Some(number)
                && p["owner"]["login"].as_str().is_some_and(|l| l.eq_ignore_ascii_case(owner));
            same.then(|| i["fieldValueByName"]["name"].as_str().map(str::to_string)).flatten()
        })
    });
    let prs: Vec<Value> = c["closedByPullRequestsReferences"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let state = if p["merged"] == true {
                "merged"
            } else if p["state"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("open")) {
                "open"
            } else {
                "closed"
            };
            json!({"number": p["number"], "title": p["title"], "url": p["url"], "state": state})
        })
        .collect();
    let deps =
        |k: &str| -> Vec<Value> { c[k]["nodes"].as_array().into_iter().flatten().map(|d| gh_dep(repo, d)).collect() };
    let mut extra = Map::new();
    extra.insert("number".into(), json!(number));
    extra.insert("state".into(), json!(state));
    extra.insert("state_type".into(), Value::Null);
    extra.insert(
        "closed_reason".into(),
        match (state, c["stateReason"].as_str()) {
            ("closed", Some(r)) if r.eq_ignore_ascii_case("not_planned") => json!("not_planned"),
            ("closed", _) => json!("completed"),
            _ => Value::Null,
        },
    );
    extra.insert("issue_type".into(), c["issueType"]["name"].clone());
    extra.insert("assignees".into(), json!(names(&c["assignees"], "login")));
    extra.insert("milestone".into(), c["milestone"]["title"].clone());
    extra.insert("board_status".into(), json!(board_status));
    extra.insert("blocked_by".into(), json!(deps("blockedBy")));
    extra.insert("blocking".into(), json!(deps("blocking")));
    extra.insert("linked_prs".into(), json!(prs));
    extra.insert("sub_total".into(), c["subIssuesSummary"]["total"].clone());
    extra.insert("sub_done".into(), c["subIssuesSummary"]["completed"].clone());
    Some(Node {
        id: format!("gh:{repo}#{number}"),
        kind: "issue",
        parent,
        f: Fetched {
            reference: format!("#{number}"),
            title: st(&c["title"]),
            url: c["url"].as_str().map(str::to_string),
            body,
            truncated,
            updated_at_ms: ts_ms(&updated),
            rev: updated,
            size: Size::of_labels(&labels),
            labels,
            extra,
            ..Default::default()
        },
    })
}

fn gh_milestone_node(repo: &str, m: &Value) -> Option<Node> {
    let number = m["number"].as_u64()?;
    let (open, closed) =
        (m["open"]["totalCount"].as_u64().unwrap_or(0), m["closed"]["totalCount"].as_u64().unwrap_or(0));
    let title = st(&m["title"]);
    let mut extra = Map::new();
    extra.insert("number".into(), json!(number));
    extra.insert("state".into(), json!("open"));
    extra.insert("due_on".into(), m["dueOn"].as_str().map_or(Value::Null, |d| json!(d.get(..10).unwrap_or(d))));
    extra.insert("milestone".into(), json!(title));
    extra.insert("sub_total".into(), json!(open + closed));
    extra.insert("sub_done".into(), json!(closed));
    let updated = st(&m["updatedAt"]);
    Some(Node {
        id: format!("gh-milestone:{repo}/{number}"),
        kind: "milestone",
        parent: None,
        f: Fetched {
            reference: title.clone(),
            title,
            url: m["url"].as_str().map(str::to_string),
            updated_at_ms: ts_ms(&updated),
            rev: format!("{updated}|{open}|{closed}"),
            extra,
            ..Default::default()
        },
    })
}

/// The "Sub-task of #13" line of a team task GitHub refused to attach as a sub-issue (contract §5).
fn sub_task_of(body: &str) -> Option<u64> {
    body.lines().find_map(|l| {
        l.trim().strip_prefix("Sub-task of #")?.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    })
}

struct Read {
    nodes: Vec<Value>,
    partial: bool,
    url: Option<String>,
}

fn read_issues(gh: &dyn Gh, o: &GithubTree, states: &str, since: Option<&str>, tree: bool) -> R<Read> {
    let (owner, name) = o.repo.split_once('/').unwrap_or((o.repo, ""));
    let query = issues_query(tree);
    let (mut nodes, mut after, mut url, mut partial) = (vec![], None::<String>, None, false);
    for page in 0..TREE_PAGES {
        let mut vars = vec![("owner", owner.to_string()), ("name", name.to_string()), ("first", GH_PAGE.to_string())];
        if let Some(a) = &after {
            vars.push(("after", a.clone()));
        }
        // `states` is a list: a repeated `-f states[]=`.
        let mut args_vars = vars.clone();
        args_vars.extend(states.split(',').map(|s| ("states[]", s.to_string())));
        if let Some(s) = since {
            args_vars.push(("since", s.to_string()));
        }
        let v = gql(gh, &query, &args_vars)?;
        let r = &v["data"]["repository"];
        if r.is_null() {
            return Err(SourceError::new("not_found", format!("no GitHub repository {}", o.repo)));
        }
        url = url.or_else(|| r["url"].as_str().map(str::to_string));
        if let Value::Array(a) = &r["issues"]["nodes"] {
            nodes.extend(a.iter().cloned());
        }
        match next_cursor(&r["issues"]["pageInfo"]) {
            Some(c) => after = Some(c),
            None => break,
        }
        partial = page + 1 == TREE_PAGES;
    }
    Ok(Read { nodes, partial, url })
}

/// Every sub-issue of issue `parent`, in every state, page after page (each with its parent set).
fn read_sub_issues(gh: &dyn Gh, o: &GithubTree, parent: u64) -> R<Vec<Value>> {
    let (owner, name) = o.repo.split_once('/').unwrap_or((o.repo, ""));
    let query = sub_issues_query(true);
    let (mut out, mut after) = (vec![], None::<String>);
    for _ in 0..TREE_PAGES {
        let mut vars = vec![("owner", owner.to_string()), ("name", name.to_string()), ("number", parent.to_string())];
        if let Some(a) = &after {
            vars.push(("after", a.clone()));
        }
        let v = gql(gh, &query, &vars)?;
        let subs = &v["data"]["repository"]["issue"]["subIssues"];
        for c in subs["nodes"].as_array().into_iter().flatten() {
            let mut c = c.clone();
            c["parent"] = json!({"number": parent});
            out.push(c);
        }
        match next_cursor(&subs["pageInfo"]) {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    Ok(out)
}

/// The sub-issues of one GitHub issue as nodes, every state, no cap short of
/// [`TREE_PAGES`] pages (the lazy children of Work). Team tasks are not nodes.
pub fn github_children(gh: &dyn Gh, o: &GithubTree, parent: u64) -> R<Vec<Node>> {
    let none = std::collections::HashSet::new();
    Ok(read_sub_issues(gh, o, parent)?
        .iter()
        .filter_map(|c| {
            let (_, team_task) = labels_of(c);
            gh_issue_node(&Raw { c, team_task }, o, &none)
        })
        .collect())
}

/// Reads the GitHub tree of `o.repo` (module doc for what it covers).
pub fn github_tree(gh: &dyn Gh, o: &GithubTree) -> R<Tree> {
    let since_closed = iso_days_ago(o.now_ms, CLOSED_WINDOW_DAYS);
    let (mut warning, mut tree) = (None, true);
    let (owner, name) = o.repo.split_once('/').unwrap_or((o.repo, ""));
    let read = |tree: bool| -> R<(Read, Read)> {
        match o.since {
            // Open and closed alike, whatever changed.
            Some(s) => Ok((
                read_issues(gh, o, "OPEN,CLOSED", Some(s), tree)?,
                Read { nodes: vec![], partial: false, url: None },
            )),
            None => {
                Ok((read_issues(gh, o, "OPEN", None, tree)?, read_issues(gh, o, "CLOSED", Some(&since_closed), tree)?))
            }
        }
    };
    let (open, closed) = match read(true) {
        Err(e) if lacks_hierarchy(&e) => {
            // F26: no sub-issues API here: issues without hierarchy.
            (warning, tree) = (Some("gh_sub_issues"), false);
            read(false)?
        }
        r => r?,
    };
    let partial = open.partial || closed.partial;
    let url = open.url.clone();
    // Milestones: one call; the issues of an open milestone sit under it.
    let mut nodes: Vec<Node> = vec![];
    let mut milestones = std::collections::HashSet::new();
    if o.group_by_milestone {
        let v = gql(gh, MILESTONES_QUERY, &[("owner", owner.to_string()), ("name", name.to_string())])?;
        for m in v["data"]["repository"]["milestones"]["nodes"].as_array().into_iter().flatten() {
            if let Some(n) = gh_milestone_node(o.repo, m) {
                milestones.insert(m["number"].as_u64().unwrap_or(0));
                nodes.push(n);
            }
        }
    }
    let mut raw: Vec<Value> = open.nodes;
    // Open issues are always read; only closed ones depend on their parent being loaded.
    let open_numbers: std::collections::HashSet<u64> = raw.iter().filter_map(|c| c["number"].as_u64()).collect();
    let mut have: std::collections::HashSet<u64> = raw.iter().filter_map(|c| c["number"].as_u64()).collect();
    // Closed ones: roots, and the children of a loaded parent. (Incremental: all that changed.)
    let loaded_parent = |c: &Value, have: &std::collections::HashSet<u64>| {
        c["parent"]["number"].as_u64().is_none_or(|p| have.contains(&p))
    };
    for c in closed.nodes {
        if let Some(n) = c["number"].as_u64()
            && have.insert(n)
        {
            raw.push(c);
        }
    }
    if o.since.is_none() {
        // A closed child of a closed root older than the window is not loaded; its own parent is.
        raw.retain(|c| c["number"].as_u64().is_some_and(|n| open_numbers.contains(&n)) || loaded_parent(c, &have));
    }
    // Sub-issues of loaded parents in every state (module doc), one parent at a time.
    if tree && o.since.is_none() {
        let loaded: std::collections::HashMap<u64, usize> = raw.iter().fold(Default::default(), |mut m, c| {
            if let Some(p) = c["parent"]["number"].as_u64() {
                *m.entry(p).or_insert(0) += 1;
            }
            m
        });
        let missing: Vec<u64> = raw
            .iter()
            .filter(|c| {
                c["subIssuesSummary"]["total"].as_u64().unwrap_or(0) as usize
                    > loaded.get(&c["number"].as_u64().unwrap_or(0)).copied().unwrap_or(0)
            })
            .filter_map(|c| c["number"].as_u64())
            .take(PARENT_FILL_MAX)
            .collect();
        for p in missing {
            let Ok(kids) = read_sub_issues(gh, o, p) else { continue };
            for c in kids {
                if let Some(n) = c["number"].as_u64()
                    && have.insert(n)
                {
                    raw.push(c);
                }
            }
        }
    }
    // Team tasks: counted in their parent, never nodes.
    let mut team_tasks: std::collections::HashMap<String, u64> = Default::default();
    for c in &raw {
        let (_, team) = labels_of(c);
        if !team {
            continue;
        }
        let parent = c["parent"]["number"].as_u64().or_else(|| sub_task_of(c["body"].as_str().unwrap_or("")));
        if let Some(p) = parent {
            *team_tasks.entry(format!("gh:{}#{p}", o.repo)).or_insert(0) += 1;
        }
    }
    for c in &raw {
        let (_, team_task) = labels_of(c);
        if let Some(mut n) = gh_issue_node(&Raw { c, team_task }, o, &milestones) {
            if let Some(t) = team_tasks.get(&n.id) {
                n.f.extra.insert("team_tasks".into(), json!(t));
            }
            nodes.push(n);
        }
    }
    let nodes = if o.since.is_none() { dfs_order(nodes) } else { nodes };
    Ok(Tree { nodes, warning, partial, url, full: o.since.is_none() })
}

// ------------------------------------------------------------------ Linear

/// What a Linear tree read is asked.
pub struct LinearTree<'a> {
    /// Team keys (or ids).
    pub teams: &'a [String],
    /// Project ids; empty = every project of the teams.
    pub projects: &'a [String],
    pub since: Option<&'a str>,
    pub now_ms: u64,
}

const LINEAR_PROJECTS: &str = "query OrchTreeProjects($filter: ProjectFilter, $after: String) { projects(first: 50, after: $after, filter: $filter) { \
    pageInfo { hasNextPage endCursor } nodes { id name slugId url state targetDate updatedAt \
    projectMilestones { nodes { id name targetDate } } initiatives { nodes { id name url } } } } }";

const LINEAR_ISSUE_FIELDS: &str = "id identifier title description url priority estimate sortOrder updatedAt dueDate \
    state { name type } cycle { name number startsAt endsAt } project { id } projectMilestone { id name } parent { identifier } \
    team { key } labels { nodes { name } } assignee { name } children { nodes { identifier } } \
    relations { nodes { type relatedIssue { identifier title url state { type } } } } \
    inverseRelations { nodes { type issue { identifier title url state { type } } } }";

fn linear_issues_query() -> String {
    format!(
        "query OrchTreeIssues($filter: IssueFilter, $first: Int, $after: String) {{ issues(first: $first, after: $after, filter: $filter) {{ \
         pageInfo {{ hasNextPage endCursor }} nodes {{ {LINEAR_ISSUE_FIELDS} }} }} }}"
    )
}

fn team_in(teams: &[String]) -> Value {
    let (ids, keys): (Vec<&String>, Vec<&String>) =
        teams.iter().partition(|t| t.len() == 36 && t.chars().filter(|c| *c == '-').count() == 4);
    match (ids.is_empty(), keys.is_empty()) {
        (_, true) => json!({"id": {"in": ids}}),
        (true, _) => json!({"key": {"in": keys}}),
        _ => json!({"or": [{"id": {"in": ids}}, {"key": {"in": keys}}]}),
    }
}

/// One paged Linear query, inside the back-off. A "too complex" answer
/// retries the page with a smaller one, down to the last size: then
/// `linear_complexity` (F27).
fn linear_pages(
    gql: &dyn Graphql,
    backoff: &Backoff,
    now: u64,
    query: &str,
    filter: &Value,
    root: &str,
) -> R<(Vec<Value>, bool)> {
    let (mut nodes, mut after, mut size, mut partial) = (vec![], Value::Null, 0, false);
    for page in 0..TREE_PAGES {
        let vars = json!({"filter": filter, "first": LINEAR_PAGES[size], "after": after});
        let mut d = match backoff.call(now, || gql.call(query, &vars)) {
            Err(e) if e.code == "linear_complexity" && size + 1 < LINEAR_PAGES.len() => {
                size += 1;
                continue;
            }
            r => r?,
        };
        if let Value::Array(a) = d[root]["nodes"].take() {
            nodes.extend(a);
        }
        match next_cursor(&d[root]["pageInfo"]) {
            Some(c) => after = json!(c),
            None => return Ok((nodes, false)),
        }
        partial = page + 1 == TREE_PAGES;
    }
    Ok((nodes, partial))
}

fn project_state(s: &str) -> (&'static str, &'static str) {
    match s {
        "backlog" => ("backlog", "backlog"),
        "planned" => ("planned", "unstarted"),
        "started" => ("started", "started"),
        "paused" => ("paused", "started"),
        "completed" => ("completed", "completed"),
        "canceled" => ("canceled", "canceled"),
        _ => ("started", "started"),
    }
}

fn lin_dep(d: &Value) -> Value {
    let ident = d["identifier"].as_str().unwrap_or("");
    let open = !matches!(d["state"]["type"].as_str(), Some("completed" | "canceled"));
    dep(Some(format!("lin:{ident}")), ident.to_string(), d["title"].as_str().unwrap_or(""), d["url"].as_str(), open)
}

/// `current` / `next` of a cycle by its dates (now between them / the next one to start).
fn cycle_kind(c: &Value, now_ms: u64, next_start: Option<u64>) -> Value {
    let (s, e) = (ts_ms(c["startsAt"].as_str().unwrap_or("")), ts_ms(c["endsAt"].as_str().unwrap_or("")));
    if s == 0 {
        Value::Null
    } else if s <= now_ms && now_ms < e {
        json!("current")
    } else if s > now_ms && Some(s) == next_start {
        json!("next")
    } else {
        Value::Null
    }
}

fn linear_issue_node(
    v: &Value,
    now_ms: u64,
    next_start: &std::collections::HashMap<String, u64>,
) -> Option<(Node, bool)> {
    let ident = v["identifier"].as_str().filter(|i| !i.is_empty())?;
    let labels = names(&v["labels"], "name");
    let team_task = labels.iter().any(|l| super::is_orch_label(l));
    let labels: Vec<String> = labels.into_iter().filter(|l| !super::is_orch_label(l)).collect();
    let (body, truncated) = clip(v["description"].as_str().unwrap_or(""));
    let updated = st(&v["updatedAt"]);
    let priority = v["priority"].as_u64().unwrap_or(0);
    let estimate = v["estimate"].as_f64();
    let state_type = v["state"]["type"].as_str().unwrap_or("");
    let parent = v["parent"]["identifier"]
        .as_str()
        .map(|p| format!("lin:{p}"))
        .or_else(|| v["projectMilestone"]["id"].as_str().map(|m| format!("lin-milestone:{m}")))
        .or_else(|| v["project"]["id"].as_str().map(|p| format!("lin-project:{p}")));
    let team = v["team"]["key"].as_str().unwrap_or("");
    let mut extra = Map::new();
    extra.insert("identifier".into(), json!(ident));
    extra.insert("state".into(), v["state"]["name"].clone());
    extra.insert("state_type".into(), json!(state_type));
    extra.insert(
        "closed_reason".into(),
        if state_type == "canceled" {
            json!("not_planned")
        } else if state_type == "completed" {
            json!("completed")
        } else {
            Value::Null
        },
    );
    extra.insert("assignees".into(), v["assignee"]["name"].as_str().map_or(json!([]), |n| json!([n])));
    extra.insert("milestone".into(), v["projectMilestone"]["name"].clone());
    extra.insert("project_id".into(), v["project"]["id"].clone());
    extra.insert("cycle".into(), v["cycle"]["name"].clone());
    extra.insert("cycle_kind".into(), cycle_kind(&v["cycle"], now_ms, next_start.get(team).copied()));
    extra.insert("priority".into(), json!(priority));
    extra.insert("estimate".into(), json!(estimate));
    extra.insert("target_date".into(), v["dueDate"].clone());
    let rel = |k: &str, side: &str| -> Vec<Value> {
        v[k]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["type"] == "blocks")
            .map(|r| lin_dep(&r[side]))
            .collect()
    };
    extra.insert("blocking".into(), json!(rel("relations", "relatedIssue")));
    extra.insert("blocked_by".into(), json!(rel("inverseRelations", "issue")));
    extra.insert("sub_total".into(), json!(v["children"]["nodes"].as_array().map_or(0, Vec::len)));
    extra.insert("team".into(), json!(team));
    let node = Node {
        id: format!("lin:{ident}"),
        kind: "issue",
        parent,
        f: Fetched {
            reference: ident.to_string(),
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
        },
    };
    Some((node, team_task))
}

/// The sub-issues of one Linear issue as nodes, every state (the lazy children
/// of Work). `parent` is its identifier (`ENG-12`, what a node carries: the
/// filter's `id` wants the uuid, which the cache does not keep) or its id.
/// Team tasks are not nodes.
pub fn linear_children(gql: &dyn Graphql, backoff: &Backoff, now_ms: u64, parent: &str) -> R<Vec<Node>> {
    let filter = match parent.rsplit_once('-').filter(|(k, n)| !k.is_empty() && n.parse::<u64>().is_ok()) {
        Some((key, n)) if parent.len() != 36 => json!({"parent": {
            "team": {"key": {"eq": key}}, "number": {"eq": n.parse::<u64>().unwrap_or(0)}}}),
        _ => json!({"parent": {"id": {"eq": parent}}}),
    };
    let (raw, _) = linear_pages(gql, backoff, now_ms, &linear_issues_query(), &filter, "issues")?;
    let none = std::collections::HashMap::new();
    Ok(raw
        .iter()
        .filter_map(|v| linear_issue_node(v, now_ms, &none))
        .filter(|(_, team)| !team)
        .map(|(n, _)| n)
        .collect())
}

/// Reads the Linear tree (module doc for what it covers).
pub fn linear_tree(gql: &dyn Graphql, backoff: &Backoff, q: &LinearTree) -> R<Tree> {
    let now = q.now_ms;
    let mut nodes: Vec<Node> = vec![];
    let mut slugs: std::collections::HashMap<String, String> = Default::default();
    // Initiatives, projects and their milestones.
    if q.since.is_none() {
        let mut pf = json!({"accessibleTeams": {"some": team_in(q.teams)}});
        if !q.projects.is_empty() {
            pf = json!({"and": [pf, {"id": {"in": q.projects}}]});
        }
        let (projects, _) = linear_pages(gql, backoff, now, LINEAR_PROJECTS, &pf, "projects")?;
        let mut seen_initiatives = std::collections::HashSet::new();
        for p in &projects {
            let pid = st(&p["id"]);
            slugs.insert(pid.clone(), st(&p["slugId"]));
            let initiative = p["initiatives"]["nodes"].as_array().and_then(|a| a.first());
            if let Some(i) = initiative {
                let iid = st(&i["id"]);
                if seen_initiatives.insert(iid.clone()) {
                    nodes.push(Node {
                        id: format!("lin-initiative:{iid}"),
                        kind: "initiative",
                        parent: None,
                        f: Fetched {
                            reference: st(&i["name"]),
                            title: st(&i["name"]),
                            url: i["url"].as_str().map(str::to_string),
                            rev: st(&i["id"]),
                            extra: Map::from_iter([
                                ("state".to_string(), json!("open")),
                                ("state_type".into(), Value::Null),
                            ]),
                            ..Default::default()
                        },
                    });
                }
            }
            let (state, state_type) = project_state(p["state"].as_str().unwrap_or(""));
            let mut extra = Map::new();
            extra.insert("state".into(), json!(state));
            extra.insert("state_type".into(), json!(state_type));
            extra.insert("target_date".into(), p["targetDate"].clone());
            extra.insert("project_id".into(), json!(pid));
            nodes.push(Node {
                id: format!("lin-project:{pid}"),
                kind: "linear_project",
                parent: initiative.map(|i| format!("lin-initiative:{}", st(&i["id"]))),
                f: Fetched {
                    reference: st(&p["name"]),
                    title: st(&p["name"]),
                    url: p["url"].as_str().map(str::to_string),
                    rev: st(&p["updatedAt"]),
                    updated_at_ms: ts_ms(&st(&p["updatedAt"])),
                    extra,
                    ..Default::default()
                },
            });
            for m in p["projectMilestones"]["nodes"].as_array().into_iter().flatten() {
                let mut extra = Map::new();
                extra.insert("state".into(), json!("open"));
                extra.insert("state_type".into(), Value::Null);
                extra.insert("target_date".into(), m["targetDate"].clone());
                nodes.push(Node {
                    id: format!("lin-milestone:{}", st(&m["id"])),
                    kind: "linear_milestone",
                    parent: Some(format!("lin-project:{pid}")),
                    f: Fetched {
                        reference: st(&m["name"]),
                        title: st(&m["name"]),
                        rev: format!("{}|{}", st(&m["name"]), st(&m["targetDate"])),
                        extra,
                        ..Default::default()
                    },
                });
            }
        }
    }
    // Issues: not completed / canceled, and the ones that did within the window (or changed since).
    let mut filter = json!({"team": team_in(q.teams)});
    if !q.projects.is_empty() {
        filter["project"] = json!({"id": {"in": q.projects}});
    }
    match q.since {
        Some(s) => filter["updatedAt"] = json!({"gt": s}),
        None => {
            filter["or"] = json!([
                {"state": {"type": {"nin": ["completed", "canceled"]}}},
                {"updatedAt": {"gt": iso_days_ago(now, CLOSED_WINDOW_DAYS)}},
            ]);
        }
    }
    let query = linear_issues_query();
    let (mut raw, partial) = linear_pages(gql, backoff, now, &query, &filter, "issues")?;
    // Sub-issues of loaded parents in every state, one parent at a time.
    if q.since.is_none() {
        let have: std::collections::HashSet<String> =
            raw.iter().filter_map(|v| v["identifier"].as_str().map(str::to_string)).collect();
        let mut loaded: std::collections::HashMap<&str, usize> = Default::default();
        for p in raw.iter().filter_map(|v| v["parent"]["identifier"].as_str()) {
            *loaded.entry(p).or_default() += 1;
        }
        let loaded_children = |ident: &str| loaded.get(ident).copied().unwrap_or(0);
        let missing: Vec<(String, String)> = raw
            .iter()
            .filter_map(|v| {
                let id = v["identifier"].as_str()?;
                let n = v["children"]["nodes"].as_array().map_or(0, Vec::len);
                (n > loaded_children(id)).then(|| (v["id"].as_str().unwrap_or("").to_string(), id.to_string()))
            })
            .take(PARENT_FILL_MAX)
            .collect();
        let mut seen = have;
        for (id, _) in missing {
            let f = json!({"parent": {"id": {"eq": id}}});
            let Ok((kids, _)) = linear_pages(gql, backoff, now, &query, &f, "issues") else { continue };
            for k in kids {
                if k["identifier"].as_str().is_some_and(|i| seen.insert(i.to_string())) {
                    raw.push(k);
                }
            }
        }
    }
    // The next cycle of a team is the first one that has not started.
    let mut next_start: std::collections::HashMap<String, u64> = Default::default();
    for v in &raw {
        let s = ts_ms(v["cycle"]["startsAt"].as_str().unwrap_or(""));
        if s > now {
            let e = next_start.entry(v["team"]["key"].as_str().unwrap_or("").to_string()).or_insert(s);
            *e = (*e).min(s);
        }
    }
    let mut team_tasks: std::collections::HashMap<String, u64> = Default::default();
    let mut issues: Vec<Node> = vec![];
    for v in &raw {
        let Some((n, team_task)) = linear_issue_node(v, now, &next_start) else { continue };
        if team_task {
            if let Some(p) = &n.parent {
                *team_tasks.entry(p.clone()).or_insert(0) += 1;
            }
        } else {
            issues.push(n);
        }
    }
    // Priority first (urgent … low, none last), then the board order.
    let order: std::collections::HashMap<String, (u64, f64)> = raw
        .iter()
        .filter_map(|v| {
            let p = v["priority"].as_u64().unwrap_or(0);
            Some((
                format!("lin:{}", v["identifier"].as_str()?),
                (if p == 0 { 5 } else { p }, v["sortOrder"].as_f64().unwrap_or(0.0)),
            ))
        })
        .collect();
    issues.sort_by(|a, b| {
        let (x, y) = (order.get(&a.id).copied().unwrap_or((5, 0.0)), order.get(&b.id).copied().unwrap_or((5, 0.0)));
        x.0.cmp(&y.0).then(x.1.total_cmp(&y.1))
    });
    for mut n in issues {
        if let Some(t) = team_tasks.get(&n.id) {
            n.f.extra.insert("team_tasks".into(), json!(t));
        }
        nodes.push(n);
    }
    // The view in Linear: the project or the team, the workspace slug being the one in the issues' own links.
    let ws = raw.iter().find_map(|v| {
        v["url"]
            .as_str()?
            .strip_prefix("https://linear.app/")?
            .split('/')
            .next()
            .filter(|w| !w.is_empty())
            .map(str::to_string)
    });
    let url = ws.and_then(|ws| match (q.projects, q.teams) {
        ([p], _) => slugs.get(p).filter(|s| !s.is_empty()).map(|s| format!("https://linear.app/{ws}/project/{s}")),
        ([], [t]) => Some(format!("https://linear.app/{ws}/team/{t}/active")),
        _ => None,
    });
    let nodes = if q.since.is_none() { dfs_order(nodes) } else { nodes };
    Ok(Tree { nodes, warning: None, partial, url, full: q.since.is_none() })
}

// ------------------------------------------------------------------ Local

/// The backlog files as flat nodes (`local:<path>`), and the tip read.
pub fn local_tree(dir: &std::path::Path, glob: &str, branch: &str) -> R<(Tree, String)> {
    let (l, tip) = super::repo_docs(dir, glob, branch)?;
    let nodes = l
        .items
        .into_iter()
        .map(|f| Node { id: format!("local:{}", f.reference), kind: "local", parent: None, f })
        .collect();
    Ok((Tree { nodes, warning: None, partial: l.partial, url: None, full: true }, tip))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// 2026-11-13T12:00:00Z.
    const NOW: u64 = 1_794_571_200_000;

    type GhAnswer = Box<dyn Fn(&str, &[String]) -> Result<Value, String> + Send + Sync>;
    type LinAnswer = Box<dyn Fn(&str, &Value) -> Result<Value, crate::linear::GqlError> + Send + Sync>;

    /// `gh` answering a GraphQL query by its operation and arguments, recording the calls.
    struct FakeGh {
        calls: Mutex<Vec<Vec<String>>>,
        answer: GhAnswer,
    }

    impl FakeGh {
        fn new(f: impl Fn(&str, &[String]) -> Result<Value, String> + Send + Sync + 'static) -> FakeGh {
            FakeGh { calls: Mutex::default(), answer: Box::new(f) }
        }
        fn ops(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().map(|a| op_of(a)).collect()
        }
    }

    fn op_of(args: &[String]) -> String {
        let q = args.iter().find_map(|a| a.strip_prefix("query=")).unwrap_or("");
        q.split_whitespace().nth(1).unwrap_or("").split('(').next().unwrap_or("").to_string()
    }

    impl Gh for FakeGh {
        fn run(&self, args: &[String]) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(args.to_vec());
            match (self.answer)(&op_of(args), args) {
                Ok(v) => Ok(v.to_string()),
                Err(e) => Err(anyhow::anyhow!("gh {}: {e}", args.join(" "))),
            }
        }
    }

    fn has(args: &[String], arg: &str) -> bool {
        args.iter().any(|a| a == arg)
    }

    fn issue(n: u64, state: &str, extra: Value) -> Value {
        let mut v = json!({"number": n, "title": format!("Issue {n}"), "body": format!("Body {n}"),
            "url": format!("https://github.com/acme/shop/issues/{n}"), "state": state,
            "updatedAt": "2026-11-10T00:00:00Z", "labels": {"nodes": []}, "assignees": {"nodes": []},
            "milestone": null, "parent": null, "subIssuesSummary": {"total": 0, "completed": 0}});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    }

    fn issues_page(nodes: Vec<Value>, next: Option<&str>) -> Value {
        json!({"data": {"repository": {"url": "https://github.com/acme/shop", "issues": {
            "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}, "nodes": nodes}}}})
    }

    fn opts(since: Option<&'static str>, group: bool) -> GithubTree<'static> {
        GithubTree { repo: "acme/shop", board: Some(("acme", 4)), group_by_milestone: group, since, now_ms: NOW }
    }

    fn ids(t: &Tree) -> Vec<(&str, Option<&str>)> {
        t.nodes.iter().map(|n| (n.id.as_str(), n.parent.as_deref())).collect()
    }

    fn milestones() -> Value {
        json!({"data": {"repository": {"milestones": {"nodes": [
            {"number": 3, "title": "v2", "url": "https://github.com/acme/shop/milestone/3", "dueOn": "2026-12-01T00:00:00Z",
             "updatedAt": "2026-11-09T00:00:00Z", "open": {"totalCount": 2}, "closed": {"totalCount": 1}}]}}}})
    }

    fn the_repo() -> FakeGh {
        FakeGh::new(|op, args| match op {
            "OrchTreeMilestones" => Ok(milestones()),
            "OrchTreeIssues" if has(args, "states[]=CLOSED") => Ok(issues_page(
                vec![
                    issue(6, "CLOSED", json!({"stateReason": "NOT_PLANNED"})),
                    // A closed child of a parent that is not loaded: not read.
                    issue(7, "CLOSED", json!({"parent": {"number": 99}})),
                ],
                None,
            )),
            "OrchTreeIssues" => Ok(issues_page(
                vec![
                    issue(
                        1,
                        "OPEN",
                        json!({"milestone": {"number": 3, "title": "v2"}, "issueType": {"name": "Epic"},
                               "labels": {"nodes": [{"name": "ready"}, {"name": "size:L"}]},
                               "subIssuesSummary": {"total": 3, "completed": 1},
                               "blockedBy": {"nodes": [{"number": 5, "title": "Issue 5", "state": "OPEN",
                                   "url": "u5", "repository": {"nameWithOwner": "acme/shop"}},
                                   {"number": 4, "title": "Outside", "state": "CLOSED", "url": "u4",
                                    "repository": {"nameWithOwner": "acme/api"}}]},
                               "closedByPullRequestsReferences": {"nodes": [
                                   {"number": 61, "title": "Fix", "url": "p", "state": "MERGED", "merged": true}]},
                               "projectItems": {"nodes": [{"project": {"number": 4, "owner": {"login": "acme"}},
                                   "fieldValueByName": {"name": "Todo"}}]}}),
                    ),
                    issue(2, "OPEN", json!({"parent": {"number": 1}, "milestone": {"number": 3, "title": "v2"}})),
                    // A team task, sub-issue of #1, and one that says so in its body.
                    issue(8, "OPEN", json!({"parent": {"number": 1}, "labels": {"nodes": [{"name": "orch"}]}})),
                    issue(
                        9,
                        "OPEN",
                        json!({"body": "Sub-task of #1\n", "labels": {"nodes": [{"name": "orch:ready"}]}}),
                    ),
                    issue(5, "OPEN", json!({})),
                ],
                None,
            )),
            "OrchTreeSubIssues" => Ok(json!({"data": {"repository": {"issue": {"subIssues": {"nodes": [
                issue(2, "OPEN", json!({})),
                issue(3, "CLOSED", json!({"stateReason": "COMPLETED"}))]}}}}})),
            other => Err(format!("unexpected {other}")),
        })
    }

    #[test]
    fn github_tree_has_milestones_issues_and_sub_issues_with_their_facts() {
        let gh = the_repo();
        let t = github_tree(&gh, &opts(None, true)).unwrap();
        assert_eq!(
            ids(&t),
            [
                ("gh-milestone:acme/shop/3", None),
                ("gh:acme/shop#1", Some("gh-milestone:acme/shop/3")),
                ("gh:acme/shop#2", Some("gh:acme/shop#1")),
                ("gh:acme/shop#3", Some("gh:acme/shop#1")),
                ("gh:acme/shop#5", None),
                ("gh:acme/shop#6", None),
            ],
            "parents first; a closed child of an unloaded parent, and the team tasks, are not nodes"
        );
        assert!(t.full && t.warning.is_none() && !t.partial);
        let meta = |id: &str| t.nodes.iter().find(|n| n.id == id).unwrap().upsert().meta;
        let one = meta("gh:acme/shop#1");
        assert_eq!(
            (one["state"].as_str(), one["issue_type"].as_str(), one["board_status"].as_str()),
            (Some("open"), Some("Epic"), Some("Todo"))
        );
        assert_eq!(one["labels"], json!(["ready", "size:L"]));
        assert_eq!(one["size"], "L");
        assert_eq!(one["team_tasks"], 2, "sub-issue and `Sub-task of #1` line");
        assert_eq!((one["sub_total"].as_u64(), one["sub_done"].as_u64()), (Some(3), Some(1)));
        assert_eq!(
            one["blocked_by"],
            json!([{"id": "gh:acme/shop#5", "ref": "#5", "title": "Issue 5", "url": "u5", "open": true},
                   {"id": null, "ref": "acme/api#4", "title": "Outside", "url": "u4", "open": false}])
        );
        assert_eq!(one["linked_prs"][0], json!({"number": 61, "title": "Fix", "url": "p", "state": "merged"}));
        assert_eq!(meta("gh:acme/shop#3")["state"], "closed");
        assert_eq!(meta("gh:acme/shop#3")["closed_reason"], "completed");
        assert_eq!(meta("gh:acme/shop#6")["closed_reason"], "not_planned");
        let m = meta("gh-milestone:acme/shop/3");
        assert_eq!(
            (m["due_on"].as_str(), m["sub_total"].as_u64(), m["sub_done"].as_u64()),
            (Some("2026-12-01"), Some(3), Some(1))
        );
        // The closed ones are asked for inside the window; the sub-issues of #1 (3 > 1 loaded) one by one.
        let ops = gh.ops();
        let calls = gh.calls.lock().unwrap();
        let closed = calls.iter().find(|a| has(a, "states[]=CLOSED")).unwrap();
        assert!(closed.iter().any(|a| a == "since=2026-08-15T12:00:00Z"), "{closed:?}");
        assert_eq!(ops.iter().filter(|o| *o == "OrchTreeSubIssues").count(), 1);
        // Read-only: no write, nothing but queries.
        assert!(calls.iter().all(|a| !has(a, "-X") && a.iter().all(|x| !x.contains("mutation"))));
    }

    #[test]
    fn without_milestone_grouping_issues_sit_at_the_root() {
        let t = github_tree(&the_repo(), &opts(None, false)).unwrap();
        assert!(t.nodes.iter().all(|n| n.kind == "issue"));
        let one = t.nodes.iter().find(|n| n.id == "gh:acme/shop#1").unwrap();
        assert_eq!(one.parent, None);
    }

    #[test]
    fn github_without_the_sub_issues_api_reads_a_flat_tree_and_says_so() {
        let gh = FakeGh::new(|op, args| match op {
            "OrchTreeMilestones" => Ok(json!({"data": {"repository": {"milestones": {"nodes": []}}}})),
            "OrchTreeIssues" if args.iter().any(|a| a.contains("subIssuesSummary")) => {
                Ok(json!({"errors": [{"message": "Field 'subIssuesSummary' doesn't exist on type 'Issue'"}]}))
            }
            "OrchTreeIssues" => Ok(issues_page(vec![issue(1, "OPEN", json!({}))], None)),
            other => Err(format!("unexpected {other}")),
        });
        let t = github_tree(&gh, &opts(None, true)).unwrap();
        assert_eq!(t.warning, Some("gh_sub_issues"));
        assert_eq!(ids(&t), [("gh:acme/shop#1", None)]);
        // Any other GraphQL error is the source's, not a fallback.
        let gh = FakeGh::new(|_, _| Ok(json!({"errors": [{"message": "Bad credentials", "type": "FORBIDDEN"}]})));
        assert_eq!(github_tree(&gh, &opts(None, false)).unwrap_err().code, "gh_auth");
    }

    #[test]
    fn github_pages_and_incremental_reads() {
        let gh = FakeGh::new(|op, args| match op {
            "OrchTreeMilestones" => Ok(json!({"data": {"repository": {"milestones": {"nodes": []}}}})),
            "OrchTreeIssues" if has(args, "after=c1") => Ok(issues_page(vec![issue(2, "OPEN", json!({}))], None)),
            "OrchTreeIssues" if has(args, "states[]=CLOSED") && !has(args, "states[]=OPEN") => {
                Ok(issues_page(vec![], None))
            }
            "OrchTreeIssues" => Ok(issues_page(vec![issue(1, "OPEN", json!({}))], Some("c1"))),
            other => Err(format!("unexpected {other}")),
        });
        let t = github_tree(&gh, &opts(None, false)).unwrap();
        assert_eq!(ids(&t), [("gh:acme/shop#1", None), ("gh:acme/shop#2", None)], "every page");
        // Incremental: one query, open and closed alike, since the cursor; the result is not the whole tree.
        let gh2 = FakeGh::new(|op, args| match op {
            "OrchTreeIssues" => {
                assert!(has(args, "states[]=OPEN") && has(args, "states[]=CLOSED"), "{args:?}");
                assert!(has(args, "since=2026-11-13T11:59:00Z"), "{args:?}");
                Ok(issues_page(vec![issue(4, "CLOSED", json!({"parent": {"number": 1}}))], None))
            }
            other => Err(format!("unexpected {other}")),
        });
        let t = github_tree(&gh2, &opts(Some("2026-11-13T11:59:00Z"), false)).unwrap();
        assert!(!t.full);
        assert_eq!(ids(&t), [("gh:acme/shop#4", Some("gh:acme/shop#1"))], "a changed closed child is kept");
        assert_eq!(tree_since(NOW), "2026-11-13T11:59:00Z");
    }

    #[test]
    fn dfs_puts_parents_first_and_keeps_orphans_and_cycles() {
        let n = |id: &str, p: Option<&str>| Node {
            id: id.into(),
            kind: "issue",
            parent: p.map(str::to_string),
            f: Fetched::default(),
        };
        let out = dfs_order(vec![
            n("c", Some("b")),
            n("b", Some("a")),
            n("x", Some("gone")),
            n("a", None),
            n("d", Some("a")),
        ]);
        assert_eq!(out.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["x", "a", "b", "c", "d"]);
        let cyc = dfs_order(vec![n("p", Some("q")), n("q", Some("p"))]);
        assert_eq!(cyc.len(), 2);
    }

    // ------------------------------------------------------------ Linear

    struct FakeLinear {
        queries: Mutex<Vec<(String, Value)>>,
        answer: LinAnswer,
    }

    impl Graphql for FakeLinear {
        fn call(&self, query: &str, vars: &Value) -> Result<Value, crate::linear::GqlError> {
            let op = query.split_whitespace().nth(1).unwrap_or("").split('(').next().unwrap_or("").to_string();
            self.queries.lock().unwrap().push((op.clone(), vars.clone()));
            (self.answer)(&op, vars)
        }
    }

    fn lin_issue(ident: &str, extra: Value) -> Value {
        let mut v = json!({"id": format!("id-{ident}"), "identifier": ident, "title": format!("T {ident}"),
            "description": "d", "url": format!("https://linear.app/acme/issue/{ident}/x"), "priority": 0,
            "estimate": null, "sortOrder": 1.0, "updatedAt": "2026-11-10T00:00:00Z", "dueDate": null,
            "state": {"name": "Todo", "type": "unstarted"}, "cycle": null, "project": null, "projectMilestone": null,
            "parent": null, "team": {"key": "ENG"}, "labels": {"nodes": []}, "assignee": null,
            "children": {"nodes": []}, "relations": {"nodes": []}, "inverseRelations": {"nodes": []}});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    }

    fn lin(
        answer: impl Fn(&str, &Value) -> Result<Value, crate::linear::GqlError> + Send + Sync + 'static,
    ) -> FakeLinear {
        FakeLinear { queries: Mutex::default(), answer: Box::new(answer) }
    }

    fn teams() -> Vec<String> {
        vec!["ENG".to_string()]
    }

    #[test]
    fn linear_tree_has_initiatives_projects_milestones_issues_and_sub_issues() {
        let f = lin(|op, vars| match op {
            "OrchTreeProjects" => Ok(json!({"projects": {"pageInfo": {"hasNextPage": false}, "nodes": [
                {"id": "pr1", "name": "Checkout", "slugId": "co", "url": "https://linear.app/acme/project/co",
                 "state": "started", "targetDate": "2026-12-01", "updatedAt": "2026-11-01T00:00:00Z",
                 "projectMilestones": {"nodes": [{"id": "m1", "name": "Beta", "targetDate": "2026-11-30"}]},
                 "initiatives": {"nodes": [{"id": "i1", "name": "Growth", "url": "https://linear.app/acme/initiative/g"}]}}]}})),
            "OrchTreeIssues" if vars["filter"]["parent"].is_object() => {
                Ok(json!({"issues": {"pageInfo": {"hasNextPage": false},
                "nodes": [lin_issue("ENG-3", json!({"parent": {"identifier": "ENG-1"}, "state": {"name": "Done", "type": "completed"}}))]}}))
            }
            "OrchTreeIssues" => Ok(json!({"issues": {"pageInfo": {"hasNextPage": false}, "nodes": [
                lin_issue("ENG-1", json!({"projectMilestone": {"id": "m1", "name": "Beta"}, "project": {"id": "pr1"}, "priority": 3,
                    "children": {"nodes": [{"identifier": "ENG-2"}, {"identifier": "ENG-3"}, {"identifier": "ENG-9"}]},
                    "cycle": {"name": "Cycle 14", "number": 14, "startsAt": "2026-11-09T00:00:00Z", "endsAt": "2026-11-23T00:00:00Z"},
                    "relations": {"nodes": [{"type": "blocks", "relatedIssue": {"identifier": "ENG-5", "title": "Five", "url": "u", "state": {"type": "started"}}}]},
                    "labels": {"nodes": [{"name": "next"}]}, "estimate": 5.0})),
                lin_issue("ENG-2", json!({"parent": {"identifier": "ENG-1"}, "project": {"id": "pr1"}, "priority": 1,
                    "cycle": {"name": "Cycle 15", "number": 15, "startsAt": "2026-11-23T00:00:00Z", "endsAt": "2026-12-07T00:00:00Z"},
                    "inverseRelations": {"nodes": [{"type": "blocks", "issue": {"identifier": "ENG-1", "title": "One", "url": "u1", "state": {"type": "unstarted"}}}]}})),
                // A team task: counted in its parent.
                lin_issue("ENG-9", json!({"parent": {"identifier": "ENG-1"}, "labels": {"nodes": [{"name": "orch"}]}})),
                lin_issue("ENG-4", json!({"priority": 1})),
            ]}})),
            other => panic!("unexpected {other}"),
        });
        let q = LinearTree { teams: &teams(), projects: &[], since: None, now_ms: NOW };
        let t = linear_tree(&f, &Backoff::default(), &q).unwrap();
        assert_eq!(
            ids(&t),
            [
                ("lin-initiative:i1", None),
                ("lin-project:pr1", Some("lin-initiative:i1")),
                ("lin-milestone:m1", Some("lin-project:pr1")),
                ("lin:ENG-1", Some("lin-milestone:m1")),
                ("lin:ENG-2", Some("lin:ENG-1")),
                ("lin:ENG-3", Some("lin:ENG-1")),
                ("lin:ENG-4", None),
            ]
        );
        let kinds: Vec<&str> = t.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds[..4], ["initiative", "linear_project", "linear_milestone", "issue"]);
        let meta = |id: &str| t.nodes.iter().find(|n| n.id == id).unwrap().upsert().meta;
        let one = meta("lin:ENG-1");
        assert_eq!(
            (one["state"].as_str(), one["state_type"].as_str(), one["cycle"].as_str(), one["cycle_kind"].as_str()),
            (Some("Todo"), Some("unstarted"), Some("Cycle 14"), Some("current"))
        );
        assert_eq!(one["team_tasks"], 1);
        assert_eq!(one["size"], "L");
        assert_eq!(one["project_id"], "pr1");
        assert_eq!(one["blocking"][0]["id"], "lin:ENG-5");
        assert_eq!(
            meta("lin:ENG-2")["blocked_by"][0],
            json!({"id": "lin:ENG-1", "ref": "ENG-1", "title": "One", "url": "u1", "open": true})
        );
        assert_eq!(meta("lin:ENG-2")["cycle_kind"], "next");
        assert_eq!(meta("lin:ENG-3")["state_type"], "completed");
        assert_eq!(meta("lin:ENG-3")["closed_reason"], "completed");
        assert_eq!(meta("lin-project:pr1")["state_type"], "started");
        assert_eq!(t.url.as_deref(), Some("https://linear.app/acme/team/ENG/active"));
        // The closed window and the sub-issues of loaded parents are asked for.
        let qs = f.queries.lock().unwrap();
        let first = qs.iter().find(|(o, v)| o == "OrchTreeIssues" && v["filter"]["or"].is_array()).unwrap();
        assert_eq!(first.1["filter"]["or"][1]["updatedAt"]["gt"], "2026-08-15T12:00:00Z");
        assert!(qs.iter().any(|(_, v)| v["filter"]["parent"]["id"]["eq"] == "id-ENG-1"));
        // Priority first (urgent … none last): ENG-4 (1) before nothing else at the root.
    }

    #[test]
    fn linear_halves_its_pages_when_a_query_is_too_complex_then_gives_up() {
        use crate::linear::GqlError;
        let f = lin(|op, vars| match op {
            "OrchTreeProjects" => Ok(json!({"projects": {"pageInfo": {"hasNextPage": false}, "nodes": []}})),
            _ if vars["first"].as_u64().unwrap_or(0) > 25 => Err(GqlError::Failed("Query too complex".into())),
            _ => Ok(json!({"issues": {"pageInfo": {"hasNextPage": false}, "nodes": [lin_issue("ENG-1", json!({}))]}})),
        });
        let q = LinearTree { teams: &teams(), projects: &[], since: None, now_ms: NOW };
        let t = linear_tree(&f, &Backoff::default(), &q).unwrap();
        assert_eq!(t.nodes.len(), 1);
        let firsts: Vec<u64> = f
            .queries
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _)| o == "OrchTreeIssues")
            .filter_map(|(_, v)| v["first"].as_u64())
            .collect();
        assert_eq!(firsts, [100, 50, 25]);
        let always = lin(|op, _| match op {
            "OrchTreeProjects" => Ok(json!({"projects": {"pageInfo": {"hasNextPage": false}, "nodes": []}})),
            _ => Err(GqlError::Failed("Query too complex".into())),
        });
        let e = linear_tree(&always, &Backoff::default(), &q).unwrap_err();
        assert_eq!(e.code, "linear_complexity");
    }

    #[test]
    fn linear_incremental_reads_what_changed_and_projects_narrow_the_filter() {
        let f = lin(|op, vars| match op {
            "OrchTreeIssues" => {
                assert_eq!(vars["filter"]["updatedAt"]["gt"], "2026-11-13T11:59:00Z");
                assert!(vars["filter"].get("or").is_none(), "every state");
                assert_eq!(vars["filter"]["project"]["id"]["in"], json!(["pr1"]));
                Ok(json!({"issues": {"pageInfo": {"hasNextPage": false}, "nodes": [lin_issue("ENG-1", json!({}))]}}))
            }
            other => panic!("an incremental read asks no {other}"),
        });
        let (t, p) = (teams(), vec!["pr1".to_string()]);
        let q = LinearTree { teams: &t, projects: &p, since: Some("2026-11-13T11:59:00Z"), now_ms: NOW };
        let tree = linear_tree(&f, &Backoff::default(), &q).unwrap();
        assert!(!tree.full);
        assert_eq!(tree.nodes.len(), 1);
    }
}
