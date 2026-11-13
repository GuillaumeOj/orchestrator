//! Write-back (implement-improve-intake B5, contract §5): the writes the core
//! makes to a tracker item when work starts, ends or is split into team
//! tasks. Every function is one write (or the few calls it takes) against
//! GitHub through `Gh` or Linear through `Graphql`; errors are classified
//! `gh_write` / `linear_write` (a refused write) or keep the source codes
//! (`offline`, `gh_auth`, `rate_limited`…). Which items may be written, when,
//! and what is audited belongs to the caller (`agent-runtime/writeback.rs`).

use serde_json::{Value, json};

use crate::github::Gh;
use crate::intake::{SourceError, enc, gh_error, linear_error};
use crate::linear::Graphql;

type R<T> = Result<T, SourceError>;

/// The error of a refused or failed GitHub write: a permission problem is `gh_write`.
fn gh_write_error(e: &anyhow::Error) -> SourceError {
    let m = format!("{e:#}");
    let l = m.to_ascii_lowercase();
    let refused = ["403", "resource not accessible", "must have", "not permitted", "permission", "forbidden", "422"]
        .iter()
        .any(|p| l.contains(p));
    let base = gh_error(e);
    if refused || base.code == "failed" {
        let first = m.lines().next().unwrap_or(&m).trim().to_string();
        SourceError::new("gh_write", format!("GitHub refused the change: {first}"))
    } else {
        base
    }
}

fn gh(g: &dyn Gh, args: &[&str]) -> R<String> {
    g.run(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).map_err(|e| gh_write_error(&e))
}

fn gh_json(g: &dyn Gh, args: &[&str]) -> R<Value> {
    let out = gh(g, args)?;
    serde_json::from_str(&out).map_err(|_| SourceError::new("failed", "gh: not JSON"))
}

// ------------------------------------------------------------------ GitHub

/// Makes sure `label` exists in `repo` (an existing one is fine).
fn ensure_label(g: &dyn Gh, repo: &str, label: &str) -> R<()> {
    match gh(
        g,
        &["api", "-X", "POST", &format!("repos/{repo}/labels"), "-f", &format!("name={label}"), "-f", "color=6E7681"],
    ) {
        Err(e) if !e.message.contains("already_exists") && !e.message.contains("422") => Err(e),
        _ => Ok(()),
    }
}

pub fn gh_add_label(g: &dyn Gh, repo: &str, n: u64, label: &str) -> R<()> {
    ensure_label(g, repo, label)?;
    gh(g, &["api", "-X", "POST", &format!("repos/{repo}/issues/{n}/labels"), "-f", &format!("labels[]={label}")])?;
    Ok(())
}

/// Removes a label (not having it is fine).
pub fn gh_remove_label(g: &dyn Gh, repo: &str, n: u64, label: &str) -> R<()> {
    match gh(g, &["api", "-X", "DELETE", &format!("repos/{repo}/issues/{n}/labels/{}", enc(label))]) {
        Err(e) if !e.message.contains("404") && !e.message.contains("Not Found") => Err(e),
        _ => Ok(()),
    }
}

pub fn gh_comment(g: &dyn Gh, repo: &str, n: u64, text: &str) -> R<()> {
    gh(g, &["api", "-X", "POST", &format!("repos/{repo}/issues/{n}/comments"), "-f", &format!("body={text}")])?;
    Ok(())
}

pub fn gh_assign(g: &dyn Gh, repo: &str, n: u64, login: &str) -> R<()> {
    gh(
        g,
        &["api", "-X", "POST", &format!("repos/{repo}/issues/{n}/assignees"), "-f", &format!("assignees[]={login}")],
    )?;
    Ok(())
}

/// Whether the issue is open now.
pub fn gh_is_open(g: &dyn Gh, repo: &str, n: u64) -> R<bool> {
    let v = gh_json(g, &["api", &format!("repos/{repo}/issues/{n}")])?;
    Ok(v["state"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("open")))
}

pub fn gh_close(g: &dyn Gh, repo: &str, n: u64) -> R<()> {
    gh(
        g,
        &[
            "api",
            "-X",
            "PATCH",
            &format!("repos/{repo}/issues/{n}"),
            "-f",
            "state=closed",
            "-f",
            "state_reason=completed",
        ],
    )?;
    Ok(())
}

fn gh_node_id(g: &dyn Gh, repo: &str, n: u64) -> R<String> {
    let v = gh_json(g, &["api", &format!("repos/{repo}/issues/{n}")])?;
    v["node_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| SourceError::new("failed", format!("no node id for {repo}#{n}")))
}

fn gh_graphql(g: &dyn Gh, query: &str, vars: &[(&str, &str)]) -> R<Value> {
    let mut args: Vec<String> = vec!["api".into(), "graphql".into(), "-f".into(), format!("query={query}")];
    for (k, v) in vars {
        args.extend(["-f".into(), format!("{k}={v}")]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let v = gh_json(g, &refs)?;
    if let Some(errs) = v["errors"].as_array().filter(|e| !e.is_empty()) {
        let msg = errs.iter().filter_map(|e| e["message"].as_str()).collect::<Vec<_>>().join("; ");
        return Err(gh_write_error(&anyhow::anyhow!(msg)));
    }
    Ok(v)
}

const BOARD_QUERY: &str = "query OrchBoard($owner: String!, $number: Int!) { repositoryOwner(login: $owner) { \
    ... on ProjectV2Owner { projectV2(number: $number) { id field(name: \"Status\") { \
    ... on ProjectV2SingleSelectField { id options { id name } } } } } } }";

/// The Status option to move to: the one named `want` (any case), else the first named like "in progress".
fn pick_status<'a>(options: &'a [Value], want: Option<&str>) -> Option<&'a Value> {
    let named = |f: &dyn Fn(&str) -> bool| options.iter().find(|o| o["name"].as_str().is_some_and(f));
    match want {
        Some(w) => named(&|n| n.eq_ignore_ascii_case(w)),
        None => named(&|n| n.to_ascii_lowercase().contains("in progress")),
    }
}

/// Moves issue `n` of `repo` to a Status of the Projects v2 board `owner`/`number` (adding it to the
/// board first); returns the status name set. `want`: the configured Status, else the first "in progress".
pub fn gh_board_status(g: &dyn Gh, repo: &str, n: u64, owner: &str, number: u64, want: Option<&str>) -> R<String> {
    let issue = gh_node_id(g, repo, n)?;
    let v = gh_graphql(g, BOARD_QUERY, &[("owner", owner), ("number", &number.to_string())])?;
    let project = &v["data"]["repositoryOwner"]["projectV2"];
    let (pid, field) = (project["id"].as_str(), &project["field"]);
    let (Some(pid), Some(fid)) = (pid, field["id"].as_str()) else {
        return Err(SourceError::new("gh_write", format!("the board {owner}/{number} has no Status field")));
    };
    let options = field["options"].as_array().cloned().unwrap_or_default();
    let opt = pick_status(&options, want).ok_or_else(|| {
        SourceError::new("gh_write", format!("the board has no Status {:?}", want.unwrap_or("In progress")))
    })?;
    let (oid, name) = (opt["id"].as_str().unwrap_or(""), opt["name"].as_str().unwrap_or("").to_string());
    let add = gh_graphql(
        g,
        "mutation OrchBoardAdd($project: ID!, $content: ID!) { addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } } }",
        &[("project", pid), ("content", &issue)],
    )?;
    let item = add["data"]["addProjectV2ItemById"]["item"]["id"]
        .as_str()
        .ok_or_else(|| SourceError::new("gh_write", "GitHub did not add the issue to the board"))?;
    gh_graphql(
        g,
        "mutation OrchBoardSet($project: ID!, $item: ID!, $field: ID!, $option: String!) { updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field, value: {singleSelectOptionId: $option}}) { projectV2Item { id } } }",
        &[("project", pid), ("item", item), ("field", fid), ("option", oid)],
    )?;
    Ok(name)
}

/// Makes issue `child` a sub-issue of `parent` (both of `repo`).
pub fn gh_add_sub_issue(g: &dyn Gh, repo: &str, parent: u64, child: u64) -> R<()> {
    let (p, c) = (gh_node_id(g, repo, parent)?, gh_node_id(g, repo, child)?);
    gh_graphql(
        g,
        "mutation OrchSubIssue($issue: ID!, $sub: ID!) { addSubIssue(input: {issueId: $issue, subIssueId: $sub}) { issue { id } } }",
        &[("issue", &p), ("sub", &c)],
    )?;
    Ok(())
}

/// Appends a line to the body of issue `n` (the `Sub-task of #13` fallback).
pub fn gh_append_body_line(g: &dyn Gh, repo: &str, n: u64, line: &str) -> R<()> {
    let v = gh_json(g, &["api", &format!("repos/{repo}/issues/{n}")])?;
    let body = v["body"].as_str().unwrap_or("");
    if body.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    let body = if body.trim().is_empty() { line.to_string() } else { format!("{}\n\n{line}", body.trim_end()) };
    gh(g, &["api", "-X", "PATCH", &format!("repos/{repo}/issues/{n}"), "-f", &format!("body={body}")])?;
    Ok(())
}

/// The state of a pull request (`OPEN` | `MERGED` | `CLOSED`) by its URL.
pub fn gh_pr_state(g: &dyn Gh, url: &str) -> R<String> {
    let v = gh_json(g, &["pr", "view", url, "--json", "state"])?;
    Ok(v["state"].as_str().unwrap_or("").to_string())
}

// ------------------------------------------------------------------ Linear

fn lin(g: &dyn Graphql, query: &str, vars: Value) -> R<Value> {
    g.call(query, &vars).map_err(|e| {
        let base = linear_error(&e);
        if base.code == "failed" {
            SourceError::new("linear_write", format!("Linear refused the change: {}", base.message))
        } else {
            base
        }
    })
}

/// A Linear issue as the writes need it: its id, its team's states, its current state type.
pub struct LinIssue {
    pub id: String,
    pub state_type: String,
    /// `(id, type)` by position.
    pub states: Vec<(String, String)>,
}

const LIN_ISSUE: &str = "query OrchWbIssue($id: String!) { issue(id: $id) { id state { type } team { states(first: 100) { \
    nodes { id type position } } } } }";

pub fn lin_issue(g: &dyn Graphql, identifier: &str) -> R<LinIssue> {
    let d = lin(g, LIN_ISSUE, json!({"id": identifier}))?;
    let i = &d["issue"];
    let id = i["id"].as_str().ok_or_else(|| SourceError::new("not_found", format!("no Linear issue {identifier}")))?;
    let mut states: Vec<(f64, String, String)> = i["team"]["states"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            (
                s["position"].as_f64().unwrap_or(0.0),
                s["id"].as_str().unwrap_or("").to_string(),
                s["type"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    states.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(LinIssue {
        id: id.to_string(),
        state_type: i["state"]["type"].as_str().unwrap_or("").to_string(),
        states: states.into_iter().map(|(_, id, t)| (id, t)).collect(),
    })
}

impl LinIssue {
    /// The team's first state of `kind` (`started` | `completed`).
    pub fn first_of(&self, kind: &str) -> Option<&str> {
        self.states.iter().find(|(_, t)| t == kind).map(|(id, _)| id.as_str())
    }
}

pub fn lin_update(g: &dyn Graphql, issue: &str, input: Value) -> R<()> {
    let d = lin(
        g,
        "mutation OrchWbUpdate($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success } }",
        json!({"id": issue, "input": input}),
    )?;
    if d["issueUpdate"]["success"] == false {
        return Err(SourceError::new("linear_write", "Linear did not apply the change"));
    }
    Ok(())
}

pub fn lin_comment(g: &dyn Graphql, issue: &str, body: &str) -> R<()> {
    lin(
        g,
        "mutation OrchWbComment($input: CommentCreateInput!) { commentCreate(input: $input) { success } }",
        json!({"input": {"issueId": issue, "body": body}}),
    )?;
    Ok(())
}

/// The id of the API key's user.
pub fn lin_viewer_id(g: &dyn Graphql) -> R<String> {
    let d = lin(g, "query OrchWbViewer { viewer { id } }", json!({}))?;
    d["viewer"]["id"].as_str().map(str::to_string).ok_or_else(|| SourceError::new("linear_write", "no Linear user"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Rec(Mutex<Vec<Vec<String>>>, Box<dyn Fn(&[String]) -> Result<String, String> + Send + Sync>);

    impl Gh for Rec {
        fn run(&self, args: &[String]) -> anyhow::Result<String> {
            self.0.lock().unwrap().push(args.to_vec());
            (self.1)(args).map_err(|e| anyhow::anyhow!("gh {}: {e}", args.join(" ")))
        }
    }

    fn rec(f: impl Fn(&[String]) -> Result<String, String> + Send + Sync + 'static) -> Rec {
        Rec(Mutex::default(), Box::new(f))
    }

    fn has(a: &[String], s: &str) -> bool {
        a.iter().any(|x| x.contains(s))
    }

    #[test]
    fn labels_comments_assignees_and_close_are_single_rest_writes() {
        let g = rec(|a| {
            if has(a, "repos/a/b/issues/12") && !has(a, "-X") {
                return Ok(json!({"state": "open", "node_id": "I_12", "body": ""}).to_string());
            }
            Ok("{}".into())
        });
        gh_add_label(&g, "a/b", 12, "orch:running").unwrap();
        gh_comment(&g, "a/b", 12, "Started in Orchestrator: feature x").unwrap();
        gh_assign(&g, "a/b", 12, "ann").unwrap();
        gh_remove_label(&g, "a/b", 12, "orch:running").unwrap();
        assert!(gh_is_open(&g, "a/b", 12).unwrap());
        gh_close(&g, "a/b", 12).unwrap();
        let calls = g.0.lock().unwrap();
        assert!(
            calls.iter().any(|c| has(c, "repos/a/b/labels") && has(c, "name=orch:running")),
            "the label is created first"
        );
        assert!(calls.iter().any(|c| has(c, "issues/12/labels") && has(c, "labels[]=orch:running")));
        assert!(
            calls.iter().any(|c| has(c, "issues/12/comments") && has(c, "body=Started in Orchestrator: feature x"))
        );
        assert!(calls.iter().any(|c| has(c, "issues/12/assignees") && has(c, "assignees[]=ann")));
        assert!(calls.iter().any(|c| has(c, "DELETE") && has(c, "issues/12/labels/orch%3Arunning")));
        assert!(calls.iter().any(|c| has(c, "PATCH") && has(c, "state=closed") && has(c, "state_reason=completed")));
    }

    #[test]
    fn a_refused_write_is_gh_write_and_a_missing_label_to_remove_is_fine() {
        let g = rec(|a| {
            if has(a, "DELETE") {
                return Err("HTTP 404: Not Found".into());
            }
            Err("HTTP 403: Resource not accessible by integration".into())
        });
        let e = gh_comment(&g, "a/b", 1, "x").unwrap_err();
        assert_eq!(e.code, "gh_write");
        assert!(e.message.starts_with("GitHub refused the change"));
        gh_remove_label(&g, "a/b", 1, "nope").unwrap();
        // Offline keeps its own code (nothing to fix).
        let off = rec(|_| Err("dial tcp: network is unreachable".into()));
        assert_eq!(gh_comment(&off, "a/b", 1, "x").unwrap_err().code, "offline");
    }

    #[test]
    fn the_board_status_adds_the_issue_and_picks_the_option() {
        let g = rec(|a| {
            if has(a, "issues/13") {
                return Ok(json!({"node_id": "I_13"}).to_string());
            }
            if has(a, "OrchBoard(") {
                return Ok(json!({"data": {"repositoryOwner": {"projectV2": {"id": "P1", "field": {"id": "F1",
                    "options": [{"id": "o1", "name": "Todo"}, {"id": "o2", "name": "In progress"}, {"id": "o3", "name": "Done"}]}}}}}).to_string());
            }
            if has(a, "addProjectV2ItemById") {
                return Ok(json!({"data": {"addProjectV2ItemById": {"item": {"id": "IT1"}}}}).to_string());
            }
            Ok(json!({"data": {}}).to_string())
        });
        assert_eq!(gh_board_status(&g, "a/b", 13, "acme", 4, None).unwrap(), "In progress");
        assert_eq!(gh_board_status(&g, "a/b", 13, "acme", 4, Some("done")).unwrap(), "Done");
        let e = gh_board_status(&g, "a/b", 13, "acme", 4, Some("Blocked")).unwrap_err();
        assert_eq!(e.code, "gh_write");
        let calls = g.0.lock().unwrap();
        let set = calls.iter().find(|c| has(c, "updateProjectV2ItemFieldValue")).unwrap();
        assert!(
            has(set, "item=IT1") && has(set, "field=F1") && has(set, "option=o2") && has(set, "project=P1"),
            "{set:?}"
        );
    }

    #[test]
    fn sub_issues_and_the_fallback_body_line() {
        let g = rec(|a| {
            if has(a, "issues/13") {
                return Ok(json!({"node_id": "I_13", "body": "Do it."}).to_string());
            }
            if has(a, "issues/70") {
                return Ok(json!({"node_id": "I_70", "body": "Task body"}).to_string());
            }
            Ok(json!({"data": {}}).to_string())
        });
        gh_add_sub_issue(&g, "a/b", 13, 70).unwrap();
        gh_append_body_line(&g, "a/b", 70, "Sub-task of #13").unwrap();
        {
            let calls = g.0.lock().unwrap();
            let add = calls.iter().find(|c| has(c, "addSubIssue")).unwrap();
            assert!(has(add, "issue=I_13") && has(add, "sub=I_70"), "{add:?}");
            let patch = calls.iter().find(|c| has(c, "PATCH")).unwrap();
            assert!(has(patch, "body=Task body\n\nSub-task of #13"), "{patch:?}");
        }
        // GitHub without the API: the mutation's error is gh_write.
        let no = rec(|a| {
            if has(a, "addSubIssue") {
                return Ok(json!({"errors": [{"message": "Field 'addSubIssue' doesn't exist on type 'Mutation'"}]})
                    .to_string());
            }
            Ok(json!({"node_id": "I"}).to_string())
        });
        assert_eq!(gh_add_sub_issue(&no, "a/b", 1, 2).unwrap_err().code, "gh_write");
    }

    struct Lin(Mutex<Vec<(String, Value)>>);

    impl Graphql for Lin {
        fn call(&self, q: &str, v: &Value) -> Result<Value, crate::linear::GqlError> {
            self.0.lock().unwrap().push((q.to_string(), v.clone()));
            Ok(if q.contains("OrchWbIssue") {
                json!({"issue": {"id": "uuid-42", "state": {"type": "unstarted"}, "team": {"states": {"nodes": [
                    {"id": "s-done", "type": "completed", "position": 5.0}, {"id": "s-todo", "type": "unstarted", "position": 1.0},
                    {"id": "s-prog", "type": "started", "position": 2.0}, {"id": "s-rev", "type": "started", "position": 3.0}]}}}})
            } else if q.contains("OrchWbViewer") {
                json!({"viewer": {"id": "u1"}})
            } else {
                json!({"issueUpdate": {"success": true}, "commentCreate": {"success": true}})
            })
        }
    }

    #[test]
    fn linear_writes_use_the_issue_id_and_the_teams_first_state_of_a_kind() {
        let g = Lin(Mutex::default());
        let i = lin_issue(&g, "ENG-42").unwrap();
        assert_eq!((i.id.as_str(), i.state_type.as_str()), ("uuid-42", "unstarted"));
        assert_eq!(
            (i.first_of("started"), i.first_of("completed"), i.first_of("canceled")),
            (Some("s-prog"), Some("s-done"), None)
        );
        lin_update(&g, &i.id, json!({"stateId": "s-prog", "assigneeId": "u1"})).unwrap();
        lin_comment(&g, &i.id, "Started in Orchestrator: feature x").unwrap();
        assert_eq!(lin_viewer_id(&g).unwrap(), "u1");
        let calls = g.0.lock().unwrap();
        let up = calls.iter().find(|c| c.0.contains("issueUpdate")).unwrap();
        assert_eq!(up.1["input"], json!({"stateId": "s-prog", "assigneeId": "u1"}));
        assert_eq!(calls.iter().find(|c| c.0.contains("commentCreate")).unwrap().1["input"]["issueId"], "uuid-42");
    }

    #[test]
    fn a_linear_refusal_is_linear_write() {
        struct No;
        impl Graphql for No {
            fn call(&self, _: &str, _: &Value) -> Result<Value, crate::linear::GqlError> {
                Err(crate::linear::GqlError::Failed("not allowed".into()))
            }
        }
        assert_eq!(lin_comment(&No, "x", "y").unwrap_err().code, "linear_write");
    }
}
