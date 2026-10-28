//! Linear issues through its GraphQL API (FR-501, ADR-004), called by the
//! core only. The board's model maps onto Linear like GitHub's: the `orch`
//! label marks the issues the app manages, the workflow state is the column
//! (by state type, or the project's own mapping), an `orch:node:<id>` label
//! the assignee node, and dependencies are `blocks` issue relations. Notes
//! are comments. The transport ([`Graphql`]) is a trait: the app's sends the
//! Keychain's API key through curl; tests use [`fake::FakeLinear`].
//!
//! Rate limits: a 429 or a `RATELIMITED` error stops every call of this
//! backend for a while (Retry-After, else 60 s doubling up to an hour).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use orch_store::now_ms;
use orch_team::TaskStatus;
use serde::Serialize;
use serde_json::{Value, json};

use crate::github::{Issue, IssueWrite, LABEL};

pub const URL: &str = "https://api.linear.app/graphql";
const NODE_PREFIX: &str = "orch:node:";

/// What a GraphQL call can fail with.
#[derive(Clone, Debug, PartialEq)]
pub enum GqlError {
    /// HTTP 429 or a `RATELIMITED` error; `retry_after_s` from the answer.
    RateLimited {
        retry_after_s: Option<u64>,
    },
    /// 401/403, an authentication error, or no API key stored.
    Unauthorized(String),
    Failed(String),
}

impl std::fmt::Display for GqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GqlError::RateLimited { .. } => f.write_str("Linear rate limit reached"),
            GqlError::Unauthorized(m) => write!(f, "Linear: not authorized ({m})"),
            GqlError::Failed(m) => write!(f, "Linear: {m}"),
        }
    }
}

/// One GraphQL request; returns the answer's `data`.
pub trait Graphql: Send + Sync {
    fn call(&self, query: &str, variables: &Value) -> Result<Value, GqlError>;
}

impl Graphql for Box<dyn Graphql> {
    fn call(&self, query: &str, variables: &Value) -> Result<Value, GqlError> {
        (**self).call(query, variables)
    }
}

/// A Linear answer (`{data, errors}`) as the transport returns it.
pub fn parse_answer(status: u16, retry_after: Option<&str>, body: &str) -> Result<Value, GqlError> {
    let retry = retry_after.and_then(|s| s.trim().parse().ok());
    if status == 429 {
        return Err(GqlError::RateLimited { retry_after_s: retry });
    }
    let v: Value = serde_json::from_str(body).map_err(|_| match status {
        401 | 403 => GqlError::Unauthorized(format!("{status} from api.linear.app")),
        _ => GqlError::Failed(format!("{status} from api.linear.app")),
    })?;
    if let Some(errs) = v["errors"].as_array().filter(|e| !e.is_empty()) {
        let code = |e: &Value| e["extensions"]["code"].as_str().unwrap_or("").to_string();
        let msg = errs.iter().filter_map(|e| e["message"].as_str()).collect::<Vec<_>>().join("; ");
        if errs.iter().any(|e| code(e) == "RATELIMITED") {
            return Err(GqlError::RateLimited { retry_after_s: retry });
        }
        if status == 401 || status == 403 || errs.iter().any(|e| code(e).contains("AUTHENTICATION")) {
            return Err(GqlError::Unauthorized(msg));
        }
        return Err(GqlError::Failed(msg));
    }
    if !(200..300).contains(&status) {
        return Err(if status == 401 || status == 403 {
            GqlError::Unauthorized(format!("{status} from api.linear.app"))
        } else {
            GqlError::Failed(format!("{status} from api.linear.app"))
        });
    }
    Ok(v["data"].clone())
}

/// A workflow state of a team.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct State {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub position: f64,
    /// The column it maps to without a project mapping.
    pub default_column: TaskStatus,
}

/// The column of a state by its type (spec §3.9): a started state whose
/// name says "review" is Review.
pub fn default_column(kind: &str, name: &str) -> TaskStatus {
    match kind {
        "backlog" | "triage" => TaskStatus::Backlog,
        "unstarted" => TaskStatus::Ready,
        "started" if name.to_ascii_lowercase().contains("review") => TaskStatus::Review,
        "started" => TaskStatus::InProgress,
        "completed" | "canceled" => TaskStatus::Done,
        _ => TaskStatus::Backlog,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeamRef {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
struct TeamInfo {
    id: String,
    states: Vec<State>,
    projects: Vec<ProjectRef>,
}

const FIELDS: &str = "id identifier title description updatedAt state { id name type } labels { nodes { id name } } \
                      inverseRelations { nodes { id type issue { id identifier } } }";

/// `linear_teams`: the workspace's teams.
pub fn teams(gql: &dyn Graphql) -> Result<Vec<TeamRef>> {
    let d = gql
        .call("query OrchTeams { teams(first: 100) { nodes { id key name } } }", &json!({}))
        .map_err(|e| anyhow!("{e}"))?;
    Ok(d["teams"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| TeamRef { id: s(&t["id"]), key: s(&t["key"]), name: s(&t["name"]) })
        .collect())
}

/// `linear_states`: a team's workflow states (with their default column)
/// and its projects.
pub fn team_states(gql: &dyn Graphql, key: &str) -> Result<(Vec<State>, Vec<ProjectRef>)> {
    let t = team_info(gql, key)?;
    Ok((t.states, t.projects))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// An issue's key in the app: its identifier (`ENG-142`, what the user
/// sees; Linear takes it wherever an issue id goes), else its id.
fn key(v: &Value) -> String {
    v["identifier"].as_str().filter(|i| !i.is_empty()).map_or_else(|| s(&v["id"]), str::to_string)
}

const TEAM_QUERY: &str = "query OrchTeam($key: String!) { teams(filter: { key: { eq: $key } }) { nodes { id key name \
    states(first: 100) { nodes { id name type position } } projects(first: 100) { nodes { id name } } } } }";

/// Labels by name, over the workspace and every team (a team-scoped
/// listing returns 50 at most, and the `orch` label may be a workspace one).
const LABEL_QUERY: &str = "query OrchLabelFind($name: String!) { issueLabels(first: 50, filter: { name: { eq: $name } })     { nodes { id name team { id } } } }";

fn team_info(gql: &dyn Graphql, key: &str) -> Result<TeamInfo> {
    let d = gql.call(TEAM_QUERY, &json!({"key": key})).map_err(|e| anyhow!("{e}"))?;
    let t = d["teams"]["nodes"].as_array().and_then(|a| a.first()).ok_or_else(|| anyhow!("no Linear team {key}"))?;
    let mut states: Vec<State> = t["states"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|st| {
            let (kind, name) = (s(&st["type"]), s(&st["name"]));
            State {
                id: s(&st["id"]),
                default_column: default_column(&kind, &name),
                kind,
                name,
                position: st["position"].as_f64().unwrap_or(0.0),
            }
        })
        .collect();
    states.sort_by(|a, b| a.position.total_cmp(&b.position));
    let projects = t["projects"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| ProjectRef { id: s(&p["id"]), name: s(&p["name"]) })
        .collect();
    Ok(TeamInfo { id: s(&t["id"]), states, projects })
}

/// The managed issues of one Linear team (and project).
pub struct LinearIssues<T: Graphql> {
    gql: T,
    team_key: String,
    project: Option<String>,
    /// State id → column, overriding the default by type (per project).
    mapping: BTreeMap<String, TaskStatus>,
    /// The team's id, states and projects; fetched again once when a state
    /// is missing (added in Linear after the fetch).
    team: Mutex<Option<Arc<TeamInfo>>>,
    labels: Mutex<HashMap<String, String>>,
    /// (no call before this ms, the current back-off in s).
    backoff: Mutex<(u64, u64)>,
}

impl<T: Graphql> LinearIssues<T> {
    pub fn new(gql: T, team_key: &str, project: Option<String>, mapping: BTreeMap<String, TaskStatus>) -> Self {
        LinearIssues {
            gql,
            team_key: team_key.to_string(),
            project,
            mapping,
            team: Mutex::new(None),
            labels: Mutex::new(HashMap::new()),
            backoff: Mutex::new((0, 0)),
        }
    }

    /// One call, through the rate-limit back-off.
    fn call(&self, query: &str, vars: Value) -> Result<Value> {
        let now = now_ms();
        {
            let b = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
            if now < b.0 {
                bail!("Linear rate limit: retrying in {} s", (b.0 - now).div_ceil(1000));
            }
        }
        match self.gql.call(query, &vars) {
            Ok(d) => {
                *self.backoff.lock().unwrap_or_else(|e| e.into_inner()) = (0, 0);
                Ok(d)
            }
            Err(GqlError::RateLimited { retry_after_s }) => {
                let mut b = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
                let step = retry_after_s.unwrap_or_else(|| (b.1 * 2).clamp(60, 3600));
                *b = (now + step * 1000, step);
                bail!("Linear rate limit reached: retrying in {step} s")
            }
            Err(e) => bail!("{e}"),
        }
    }

    /// The team's state names (as last read) that the project's mapping does
    /// not name; empty when the project has no mapping (all by type).
    pub fn unmapped_states(&self) -> Vec<String> {
        if self.mapping.is_empty() {
            return Vec::new();
        }
        let t = self.team.lock().unwrap_or_else(|e| e.into_inner()).clone();
        t.map(|t| t.states.iter().filter(|s| !self.mapping.contains_key(&s.id)).map(|s| s.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Seconds until calls resume after a rate limit (0 = now).
    pub fn backoff_s(&self) -> u64 {
        let b = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
        b.0.saturating_sub(now_ms()).div_ceil(1000)
    }

    fn team(&self) -> Result<Arc<TeamInfo>> {
        if let Some(t) = self.team.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(t);
        }
        self.refresh_team()
    }

    fn refresh_team(&self) -> Result<Arc<TeamInfo>> {
        let d = self.call(TEAM_QUERY, json!({"key": self.team_key}))?;
        let info = Arc::new(team_info(&Fixed(d), &self.team_key)?);
        *self.team.lock().unwrap_or_else(|e| e.into_inner()) = Some(info.clone());
        Ok(info)
    }

    /// An issue's column: the project's mapping, else the state's type (an
    /// issue answer carries its state's name and type, so a state added in
    /// Linear after the mapping was saved still has its column).
    fn column(&self, state: &Value) -> TaskStatus {
        if let Some(c) = self.mapping.get(&s(&state["id"])) {
            return *c;
        }
        default_column(&s(&state["type"]), &s(&state["name"]))
    }

    fn state_for(&self, col: TaskStatus) -> Result<String> {
        let find = |t: &TeamInfo| {
            t.states
                .iter()
                .find(|s| self.mapping.get(&s.id).copied().unwrap_or(s.default_column) == col)
                .map(|s| s.id.clone())
        };
        if let Some(id) = find(self.team()?.as_ref()) {
            return Ok(id);
        }
        // Maybe added since the team was read: read it once more.
        find(self.refresh_team()?.as_ref())
            .ok_or_else(|| anyhow!("no Linear state of team {} maps to {}", self.team_key, col.as_str()))
    }

    /// A label id by name: cached, else found over the workspace and the
    /// team, else created in the team.
    fn label(&self, name: &str) -> Result<String> {
        if let Some(id) = self.labels.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
            return Ok(id.clone());
        }
        let team_id = self.team()?.id.clone();
        let d = self.call(LABEL_QUERY, json!({"name": name}))?;
        let found = d["issueLabels"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|l| l["name"] == name)
            .find(|l| l["team"].is_null() || l["team"]["id"] == team_id.as_str())
            .map(|l| s(&l["id"]));
        if let Some(id) = found.filter(|i| !i.is_empty()) {
            self.labels.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), id.clone());
            return Ok(id);
        }
        let d = self.call(
            "mutation OrchLabelCreate($input: IssueLabelCreateInput!) { issueLabelCreate(input: $input) { success issueLabel { id name } } }",
            json!({"input": {"name": name, "teamId": team_id}}),
        )?;
        let id = s(&d["issueLabelCreate"]["issueLabel"]["id"]);
        if id.is_empty() {
            bail!("Linear: label {name} not created");
        }
        self.labels.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), id.clone());
        Ok(id)
    }

    fn to_issue(&self, v: &Value) -> Result<Issue> {
        let labels: Vec<String> =
            v["labels"]["nodes"].as_array().into_iter().flatten().map(|l| s(&l["name"])).collect();
        let blocked_by = v["inverseRelations"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["type"] == "blocks")
            .map(|r| key(&r["issue"]))
            .collect();
        Ok(Issue {
            number: key(v),
            title: s(&v["title"]),
            body: s(&v["description"]),
            status: self.column(&v["state"]),
            node: labels.iter().find_map(|l| l.strip_prefix(NODE_PREFIX)).map(str::to_string),
            blocked_by,
            other_labels: labels.into_iter().filter(|l| l != LABEL && !l.starts_with("orch:")).collect(),
            updated_at: s(&v["updatedAt"]),
        })
    }

    fn raw(&self, id: &str) -> Result<Value> {
        let d = self
            .call(&format!("query OrchIssue($id: String!) {{ issue(id: $id) {{ {FIELDS} }} }}"), json!({"id": id}))?;
        let v = d["issue"].clone();
        if v.is_null() {
            bail!("no Linear issue {id}");
        }
        Ok(v)
    }

    /// The label ids to set: the managed ones plus the issue's own others.
    fn label_ids(&self, w: &IssueWrite, current: Option<&Value>) -> Result<Vec<String>> {
        let mut ids = vec![self.label(LABEL)?];
        if let Some(n) = w.node {
            ids.push(self.label(&format!("{NODE_PREFIX}{n}"))?);
        }
        if let Some(c) = current {
            for l in c["labels"]["nodes"].as_array().into_iter().flatten() {
                let name = s(&l["name"]);
                if name != LABEL && !name.starts_with("orch:") {
                    ids.push(s(&l["id"]));
                }
            }
        }
        Ok(ids)
    }

    /// Makes the issue's `blocks` relations match `w.blocked_by`.
    fn set_blockers(&self, issue: &Value, want: &[String]) -> Result<()> {
        let id = key(issue);
        let have: Vec<(String, String)> = issue["inverseRelations"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["type"] == "blocks")
            .map(|r| (s(&r["id"]), key(&r["issue"])))
            .collect();
        for b in want.iter().filter(|b| !have.iter().any(|(_, h)| h == *b)) {
            self.call(
                "mutation OrchRelationCreate($input: IssueRelationCreateInput!) { issueRelationCreate(input: $input) { success } }",
                json!({"input": {"issueId": b, "relatedIssueId": id, "type": "blocks"}}),
            )?;
        }
        for (rel, _) in have.iter().filter(|(_, h)| !want.contains(h)) {
            self.call(
                "mutation OrchRelationDelete($id: String!) { issueRelationDelete(id: $id) { success } }",
                json!({"id": rel}),
            )?;
        }
        Ok(())
    }

    fn input(&self, w: &IssueWrite, current: Option<&Value>) -> Result<Value> {
        let mut input = json!({
            "title": w.title,
            "description": w.body,
            "stateId": self.state_for(w.status)?,
            "labelIds": self.label_ids(w, current)?,
        });
        if current.is_none() {
            input["teamId"] = json!(self.team()?.id);
            if let Some(p) = &self.project {
                input["projectId"] = json!(p);
            }
        }
        Ok(input)
    }

    pub fn create(&self, w: &IssueWrite) -> Result<Issue> {
        let input = self.input(w, None)?;
        let d = self.call(
            &format!(
                "mutation OrchIssueCreate($input: IssueCreateInput!) {{ issueCreate(input: $input) {{ success issue {{ {FIELDS} }} }} }}"
            ),
            json!({"input": input}),
        )?;
        let v = d["issueCreate"]["issue"].clone();
        if v.is_null() {
            bail!("Linear: issue not created");
        }
        if w.blocked_by.is_empty() {
            return self.to_issue(&v);
        }
        self.set_blockers(&v, &w.blocked_by)?;
        self.get(&key(&v))
    }

    pub fn update(&self, id: &str, w: &IssueWrite) -> Result<Issue> {
        let cur = self.raw(id)?;
        let input = self.input(w, Some(&cur))?;
        let d = self.call(
            &format!(
                "mutation OrchIssueUpdate($id: String!, $input: IssueUpdateInput!) {{ issueUpdate(id: $id, input: $input) {{ success issue {{ {FIELDS} }} }} }}"
            ),
            json!({"id": id, "input": input}),
        )?;
        let v = d["issueUpdate"]["issue"].clone();
        if v.is_null() {
            bail!("Linear: issue {id} not updated");
        }
        let have: Vec<String> = self.to_issue(&v)?.blocked_by;
        if have == w.blocked_by {
            return self.to_issue(&v);
        }
        self.set_blockers(&v, &w.blocked_by)?;
        self.get(id)
    }

    pub fn get(&self, id: &str) -> Result<Issue> {
        self.to_issue(&self.raw(id)?)
    }

    pub fn comment(&self, id: &str, text: &str) -> Result<()> {
        self.call(
            "mutation OrchComment($input: CommentCreateInput!) { commentCreate(input: $input) { success } }",
            json!({"input": {"issueId": id, "body": text}}),
        )?;
        Ok(())
    }

    /// Every managed issue of the team (and project), all states.
    pub fn list(&self) -> Result<Vec<Issue>> {
        let team = self.team()?.id.clone();
        let mut filter = json!({"team": {"id": {"eq": team}}, "labels": {"name": {"eq": LABEL}}});
        if let Some(p) = &self.project {
            filter["project"] = json!({"id": {"eq": p}});
        }
        let mut out = vec![];
        let mut after = Value::Null;
        for _ in 0..50 {
            let d = self.call(
                &format!(
                    "query OrchIssues($filter: IssueFilter, $after: String) {{ issues(first: 25, after: $after, filter: $filter) {{ nodes {{ {FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }}"
                ),
                json!({"filter": filter, "after": after}),
            )?;
            for v in d["issues"]["nodes"].as_array().into_iter().flatten() {
                out.push(self.to_issue(v)?);
            }
            if d["issues"]["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = d["issues"]["pageInfo"]["endCursor"].clone();
        }
        Ok(out)
    }
}

/// No transport (a [`crate::TaskService::new`] without one): every call fails.
pub struct Unconfigured;

impl Graphql for Unconfigured {
    fn call(&self, _: &str, _: &Value) -> Result<Value, GqlError> {
        Err(GqlError::Unauthorized("no Linear transport".into()))
    }
}

/// A transport that answers one fixed `data` (parses a team answered once).
struct Fixed(Value);

impl Graphql for Fixed {
    fn call(&self, _: &str, _: &Value) -> Result<Value, GqlError> {
        Ok(self.0.clone())
    }
}

/// Linear in memory, for tests: one team `ENG` with the usual workflow, its
/// labels, issues, `blocks` relations and comments. It answers the
/// operations this module sends (by operation name).
pub mod fake {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::{GqlError, Graphql};

    #[derive(Clone, Debug)]
    pub struct FakeIssue {
        pub id: String,
        pub title: String,
        pub description: String,
        pub state: String,
        pub labels: Vec<String>,
        pub updated: u64,
    }

    #[derive(Default)]
    pub struct World {
        pub issues: Vec<FakeIssue>,
        /// (id, name).
        pub labels: Vec<(String, String)>,
        /// (id, blocker, blocked).
        pub relations: Vec<(String, String, String)>,
        pub comments: Vec<(String, String)>,
        pub clock: u64,
        /// The next calls answer `RATELIMITED`.
        pub rate_limited: u32,
        pub calls: Vec<String>,
        /// States added in Linear after the team was first read: (id, name, type, position).
        pub extra_states: Vec<(String, String, String, f64)>,
        /// Label creations (a second `orch` would be a bug).
        pub labels_created: Vec<String>,
    }

    #[derive(Clone, Default)]
    pub struct FakeLinear(pub Arc<Mutex<World>>);

    /// `(id, name, type, position)` of team ENG's states.
    pub const STATES: [(&str, &str, &str, f64); 6] = [
        ("st-backlog", "Backlog", "backlog", 0.0),
        ("st-todo", "Todo", "unstarted", 1.0),
        ("st-progress", "In Progress", "started", 2.0),
        ("st-review", "In Review", "started", 3.0),
        ("st-done", "Done", "completed", 4.0),
        ("st-canceled", "Canceled", "canceled", 5.0),
    ];

    impl FakeLinear {
        /// A change made in Linear: bumps `updatedAt`.
        pub fn edit(&self, id: &str, f: impl FnOnce(&mut FakeIssue)) {
            let mut w = self.0.lock().unwrap();
            w.clock += 1;
            let c = w.clock;
            let i = w.issues.iter_mut().find(|i| i.id == id).expect("issue");
            f(i);
            i.updated = c;
        }

        pub fn issue(&self, id: &str) -> FakeIssue {
            self.0.lock().unwrap().issues.iter().find(|i| i.id == id).cloned().expect("issue")
        }

        /// The ids of the issues blocking `id`.
        pub fn blockers(&self, id: &str) -> Vec<String> {
            let w = self.0.lock().unwrap();
            w.relations.iter().filter(|r| r.2 == id).map(|r| r.1.clone()).collect()
        }
    }

    fn op(query: &str) -> &str {
        let q = query.trim_start();
        let rest = q.strip_prefix("query ").or_else(|| q.strip_prefix("mutation ")).unwrap_or(q);
        rest.split(['(', ' ', '{']).next().unwrap_or("")
    }

    fn state_of(w: &World, id: &str) -> Value {
        STATES
            .iter()
            .map(|(i, n, t, _)| (i.to_string(), n.to_string(), t.to_string()))
            .chain(w.extra_states.iter().map(|(i, n, t, _)| (i.clone(), n.clone(), t.clone())))
            .find(|(i, _, _)| i == id)
            .map(|(i, n, t)| json!({"id": i, "name": n, "type": t}))
            .unwrap_or(json!({"id": id}))
    }

    fn render(w: &World, i: &FakeIssue) -> Value {
        let labels: Vec<Value> = i
            .labels
            .iter()
            .filter_map(|id| w.labels.iter().find(|(l, _)| l == id))
            .map(|(id, name)| json!({"id": id, "name": name}))
            .collect();
        let inv: Vec<Value> = w
            .relations
            .iter()
            .filter(|r| r.2 == i.id)
            .map(|r| json!({"id": r.0, "type": "blocks", "issue": {"id": format!("uuid-{}", r.1), "identifier": r.1}}))
            .collect();
        json!({"id": format!("uuid-{}", i.id), "identifier": i.id, "title": i.title, "description": i.description,
               "updatedAt": format!("2026-10-28T00:00:{:02}.000Z", i.updated), "state": state_of(w, &i.state),
               "labels": {"nodes": labels}, "inverseRelations": {"nodes": inv}})
    }

    impl Graphql for FakeLinear {
        fn call(&self, query: &str, v: &Value) -> Result<Value, GqlError> {
            let mut w = self.0.lock().unwrap();
            let name = op(query).to_string();
            w.calls.push(name.clone());
            if w.rate_limited > 0 {
                w.rate_limited -= 1;
                return Err(GqlError::RateLimited { retry_after_s: None });
            }
            w.clock += 1;
            let clock = w.clock;
            let s = |x: &Value| x.as_str().unwrap_or("").to_string();
            Ok(match name.as_str() {
                "OrchTeams" => json!({"teams": {"nodes": [{"id": "team-1", "key": "ENG", "name": "Engineering"}]}}),
                "OrchTeam" => {
                    if v["key"] != "ENG" {
                        return Ok(json!({"teams": {"nodes": []}}));
                    }
                    let states: Vec<Value> = super::fake::STATES
                        .iter()
                        .map(|(id, n, t, p)| json!({"id": id, "name": n, "type": t, "position": p}))
                        .chain(
                            w.extra_states
                                .iter()
                                .map(|(id, n, t, p)| json!({"id": id, "name": n, "type": t, "position": p})),
                        )
                        .collect();
                    json!({"teams": {"nodes": [{"id": "team-1", "key": "ENG", "name": "Engineering",
                        "states": {"nodes": states},
                        "projects": {"nodes": [{"id": "proj-1", "name": "Shop"}]}}]}})
                }
                "OrchLabelFind" => {
                    // Workspace labels have no team; the fake's are all workspace ones.
                    let nodes: Vec<Value> = w
                        .labels
                        .iter()
                        .filter(|(_, n)| *n == s(&v["name"]))
                        .map(|(id, n)| json!({"id": id, "name": n, "team": null}))
                        .collect();
                    json!({"issueLabels": {"nodes": nodes}})
                }
                "OrchLabelCreate" => {
                    let id = format!("lbl-{}", w.labels.len() + 1);
                    w.labels.push((id.clone(), s(&v["input"]["name"])));
                    let name = s(&v["input"]["name"]);
                    w.labels_created.push(name);
                    json!({"issueLabelCreate": {"success": true, "issueLabel": {"id": id, "name": s(&v["input"]["name"])}}})
                }
                "OrchIssueCreate" => {
                    let inp = &v["input"];
                    let i = FakeIssue {
                        id: format!("ENG-{}", w.issues.len() + 1),
                        title: s(&inp["title"]),
                        description: s(&inp["description"]),
                        state: s(&inp["stateId"]),
                        labels: inp["labelIds"].as_array().into_iter().flatten().map(s).collect(),
                        updated: clock,
                    };
                    w.issues.push(i.clone());
                    json!({"issueCreate": {"success": true, "issue": render(&w, &i)}})
                }
                "OrchIssueUpdate" => {
                    let id = s(&v["id"]);
                    let inp = v["input"].clone();
                    let Some(i) = w.issues.iter_mut().find(|i| i.id == id) else {
                        return Err(GqlError::Failed(format!("Entity not found: {id}")));
                    };
                    i.title = s(&inp["title"]);
                    i.description = s(&inp["description"]);
                    i.state = s(&inp["stateId"]);
                    i.labels = inp["labelIds"].as_array().into_iter().flatten().map(s).collect();
                    i.updated = clock;
                    let i = i.clone();
                    json!({"issueUpdate": {"success": true, "issue": render(&w, &i)}})
                }
                "OrchIssue" => {
                    let id = s(&v["id"]);
                    let i = w.issues.iter().find(|i| i.id == id).cloned();
                    json!({"issue": i.map(|i| render(&w, &i)).unwrap_or(Value::Null)})
                }
                "OrchIssues" => {
                    let orch = w.labels.iter().find(|(_, n)| n == "orch").map(|(id, _)| id.clone());
                    let nodes: Vec<Value> = w
                        .issues
                        .iter()
                        .filter(|i| orch.as_ref().is_some_and(|o| i.labels.contains(o)))
                        .map(|i| render(&w, i))
                        .collect();
                    json!({"issues": {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}})
                }
                "OrchRelationCreate" => {
                    let inp = &v["input"];
                    let id = format!("rel-{}", w.relations.len() + 1);
                    let (blocker, blocked) = (s(&inp["issueId"]), s(&inp["relatedIssueId"]));
                    w.relations.push((id, blocker, blocked.clone()));
                    if let Some(i) = w.issues.iter_mut().find(|i| i.id == blocked) {
                        i.updated = clock;
                    }
                    json!({"issueRelationCreate": {"success": true}})
                }
                "OrchRelationDelete" => {
                    let id = s(&v["id"]);
                    w.relations.retain(|r| r.0 != id);
                    json!({"issueRelationDelete": {"success": true}})
                }
                "OrchComment" => {
                    let inp = &v["input"];
                    w.comments.push((s(&inp["issueId"]), s(&inp["body"])));
                    json!({"commentCreate": {"success": true}})
                }
                other => return Err(GqlError::Failed(format!("unexpected operation {other}"))),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_columns() {
        assert_eq!(parse_answer(429, Some("30"), ""), Err(GqlError::RateLimited { retry_after_s: Some(30) }));
        let rl = r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#;
        assert!(matches!(parse_answer(400, None, rl), Err(GqlError::RateLimited { .. })));
        let auth = r#"{"errors":[{"message":"Authentication required","extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#;
        assert!(matches!(parse_answer(400, None, auth), Err(GqlError::Unauthorized(_))));
        assert!(matches!(parse_answer(401, None, "nope"), Err(GqlError::Unauthorized(_))));
        assert_eq!(parse_answer(200, None, r#"{"data":{"x":1}}"#).unwrap(), json!({"x": 1}));
        assert_eq!(default_column("started", "In Review"), TaskStatus::Review);
        assert_eq!(default_column("started", "Doing"), TaskStatus::InProgress);
        assert_eq!(default_column("unstarted", "Todo"), TaskStatus::Ready);
        assert_eq!(default_column("canceled", "Won't do"), TaskStatus::Done);
    }

    #[test]
    fn rate_limits_back_off() {
        let f = fake::FakeLinear::default();
        f.0.lock().unwrap().rate_limited = 1;
        let l = LinearIssues::new(f.clone(), "ENG", None, BTreeMap::new());
        let e = l.list().unwrap_err().to_string();
        assert!(e.contains("rate limit"), "{e}");
        assert!(l.backoff_s() >= 59, "60 s at first");
        let calls = f.0.lock().unwrap().calls.len();
        assert!(l.list().unwrap_err().to_string().contains("retrying in"));
        assert_eq!(f.0.lock().unwrap().calls.len(), calls, "no call while backing off");
        *l.backoff.lock().unwrap() = (0, 60);
        assert!(l.list().is_ok());
        assert_eq!(l.backoff_s(), 0);
    }
}
