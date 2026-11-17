//! allow-multiple-prs: the issues of a feature (spec §3.1) and the run's PRs
//! as the UI reads them (`prs_view`, api-contract §2). Pure: no store, no
//! network.

use anyhow::{Result, anyhow};
use orch_store::{Feature, IntakeRow, IssueLink, PrMode, PrStatus, TeamRun, issue_key};
use serde::Serialize;
use serde_json::json;

/// Most issues a feature lists.
pub const MAX_ISSUES: usize = 10;

/// A command error whose text is the JSON `{code, message, ref?}`.
pub fn coded(code: &str, message: impl Into<String>, reference: Option<&str>) -> anyhow::Error {
    let mut v = json!({"code": code, "message": message.into()});
    if let Some(r) = reference {
        v["ref"] = json!(r);
    }
    anyhow!(v.to_string())
}

/// `ABC-123` (upper-cased) when `t` is a Linear issue id.
fn linear_ref(t: &str) -> Option<String> {
    let up = t.to_ascii_uppercase();
    let (team, n) = up.split_once('-')?;
    let mut cs = team.chars();
    let team_ok = cs.next().is_some_and(|c| c.is_ascii_uppercase()) && cs.all(|c| c.is_ascii_alphanumeric());
    let n_ok = !n.is_empty() && n.chars().all(|c| c.is_ascii_digit());
    (team_ok && n_ok).then_some(up)
}

/// The canonical ref of an issue as typed: `#12` for an issue of `repo` (the
/// project's GitHub repository: `12`, `#12`, `owner/repo#12` or its issue
/// URL), `ABC-123` for Linear. Anything else, or another repository, is
/// `bad_ref`.
pub fn parse_ref(text: &str, repo: Option<&str>) -> Result<String> {
    let t = text.trim();
    let bad = |why: &str| coded("bad_ref", format!("{t:?}: {why}"), Some(t));
    if let Some(r) = linear_ref(t) {
        return Ok(r);
    }
    // A pull request URL names no issue to close.
    let r = crate::github_issue::parse_issue_ref(t)
        .ok()
        .filter(|_| !(t.contains("github.com/") && t.contains("/pull/")))
        .ok_or_else(|| bad("not an issue (#12, owner/repo#12, an issue URL or a Linear id like ABC-123)"))?;
    match (&r.repo, repo) {
        (None, _) => Ok(format!("#{}", r.number)),
        (Some(other), Some(mine)) if other.eq_ignore_ascii_case(mine) => Ok(format!("#{}", r.number)),
        (Some(other), Some(mine)) => Err(bad(&format!("{other} is not this project's repository ({mine})"))),
        (Some(other), None) => Err(bad(&format!("this project has no GitHub repository to match {other}"))),
    }
}

/// The canonical refs of a list as typed: parsed, duplicates dropped (the
/// first stays), at most [`MAX_ISSUES`].
pub fn normalize(list: &[String], repo: Option<&str>) -> Result<Vec<String>> {
    let mut out: Vec<String> = vec![];
    for t in list.iter().filter(|t| !t.trim().is_empty()) {
        let r = parse_ref(t, repo)?;
        if !out.contains(&r) {
            out.push(r);
        }
    }
    if out.len() > MAX_ISSUES {
        return Err(coded("too_many", format!("at most {MAX_ISSUES} issues, got {}", out.len()), None));
    }
    Ok(out)
}

/// Checks the mode against the issues: `per_issue` needs at least two.
pub fn check_mode(mode: PrMode, issues: usize) -> Result<()> {
    if mode == PrMode::PerIssue && issues < 2 {
        return Err(coded("per_issue_needs_two", "one PR per issue needs at least 2 issues", None));
    }
    Ok(())
}

/// The links of `refs`, with title and URL from the project's intake rows
/// when it lists them (no network). `repo`: the project's GitHub repository.
pub fn links(refs: &[String], rows: &[IntakeRow], repo: Option<&str>) -> Vec<IssueLink> {
    refs.iter()
        .map(|r| {
            let mut l = IssueLink::new(r);
            if let Some(row) = rows.iter().find(|row| row_ref(row, repo).as_deref() == Some(r.as_str())) {
                l.title = Some(row.title.clone()).filter(|t| !t.is_empty());
                l.url = row.url.clone();
            }
            l
        })
        .collect()
}

/// The tracker issues among a project's work nodes (milestones, projects and
/// briefs are not issues a PR closes).
pub fn issue_rows(rows: Vec<IntakeRow>) -> Vec<IntakeRow> {
    rows.into_iter().filter(|r| r.kind == "issue" && !r.deleted).collect()
}

/// The canonical ref of an intake row (GitHub or Linear; a repo doc has none).
/// A GitHub item of another repository than `repo` (the project's) has none:
/// an intake source or a project board may list other repositories' issues,
/// and `#12` names the project's own.
pub fn row_ref(row: &IntakeRow, repo: Option<&str>) -> Option<String> {
    match row.source.as_str() {
        "github" => {
            let r = crate::github_issue::parse_issue_ref(&row.reference).ok()?;
            let of_url = || row.url.as_deref().and_then(|u| crate::github_issue::parse_issue_ref(u).ok())?.repo;
            match (r.repo.clone().or_else(of_url), repo) {
                (Some(other), Some(mine)) if !other.eq_ignore_ascii_case(mine) => None,
                // `owner/repo#12` with no project repository to match.
                (Some(_), None) if r.repo.is_some() => None,
                _ => Some(format!("#{}", r.number)),
            }
        }
        "linear" => linear_ref(row.reference.trim()),
        _ => None,
    }
}

/// `issue_suggest`: the intake rows whose ref or title matches `text`
/// (case-insensitive), at most 8, as links. `repo`: the project's GitHub
/// repository.
pub fn suggest(text: &str, rows: &[IntakeRow], repo: Option<&str>) -> Vec<IssueLink> {
    let q = text.trim().to_lowercase();
    let q_ref = q.trim_start_matches('#');
    let mut out: Vec<IssueLink> = vec![];
    for row in rows {
        let Some(r) = row_ref(row, repo) else { continue };
        let hit = q.is_empty()
            || issue_key(&r).starts_with(q_ref)
            || r.to_lowercase() == q
            || row.title.to_lowercase().contains(&q);
        if hit && !out.iter().any(|l| l.reference == r) {
            out.push(IssueLink {
                title: Some(row.title.clone()).filter(|t| !t.is_empty()),
                url: row.url.clone(),
                ..IssueLink::new(&r)
            });
        }
        if out.len() == 8 {
            break;
        }
    }
    out
}

const KEYWORDS: &[&str] = &["close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved"];

/// The body closes `reference`: `(?i)\b(close[sd]?|fix(e[sd])?|resolve[sd]?)\b:?\s+<ref>\b`
/// (spec §3.1): a whole keyword (`enclosed #12` is none), a whole ref
/// (`#123` does not close `#12`).
pub fn body_closes(body: &str, reference: &str) -> bool {
    let (b, r) = (body.to_lowercase(), reference.to_lowercase());
    let word = |c: char| c.is_alphanumeric() || c == '_';
    b.match_indices(&r).any(|(at, _)| {
        // `<ref>\b`: the ref ends on a word char, so the next one is not.
        if b[at + r.len()..].chars().next().is_some_and(word) {
            return false;
        }
        // `\s+`, then `:?`, then the keyword on a word boundary.
        let before = &b[..at];
        let spaced = before.trim_end_matches(char::is_whitespace);
        if spaced.len() == before.len() {
            return false;
        }
        let kw = spaced.strip_suffix(':').unwrap_or(spaced);
        KEYWORDS.iter().any(|k| kw.strip_suffix(k).is_some_and(|rest| !rest.chars().next_back().is_some_and(word)))
    })
}

/// `body` with one `Closes <ref>` line per issue it does not already close.
pub fn with_closes(body: &str, refs: &[String]) -> String {
    let missing: Vec<String> = refs.iter().filter(|r| !body_closes(body, r)).map(|r| format!("Closes {r}")).collect();
    if missing.is_empty() {
        return body.to_string();
    }
    let body = body.trim_end();
    if body.is_empty() { missing.join("\n") } else { format!("{body}\n\n{}", missing.join("\n")) }
}

/// A commit message with a git trailer added after a blank line (or to the
/// trailer block the message already ends with), so `git interpret-trailers
/// --parse` reads it.
pub fn with_trailer(message: &str, key: &str, value: &str) -> String {
    let m = message.trim_end();
    let last = m.rsplit("\n\n").next().unwrap_or("");
    let has_body = m.contains("\n\n");
    let is_trailers = has_body
        && !last.trim().is_empty()
        && last.lines().all(|l| l.split_once(": ").is_some_and(|(k, _)| !k.is_empty() && !k.contains(' ')));
    if is_trailers { format!("{m}\n{key}: {value}") } else { format!("{m}\n\n{key}: {value}") }
}

/// A pending PR gate of a run, as `prs_view` needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct GateRef {
    pub id: u64,
    /// `push_pr` or `push_pr_update`.
    pub action: String,
    /// `None` in single mode.
    pub issue: Option<String>,
}

/// One PR line of a run (api-contract §2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrView {
    pub issue: Option<String>,
    pub branch: String,
    /// `none` | `pending` | `open` | `merged` | `closed`.
    pub state: String,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub pr: Option<PrStatus>,
    pub gate: Option<u64>,
}

fn pr_state(pr: Option<&PrStatus>) -> String {
    match pr.map(|p| p.state.as_str()) {
        Some(s @ ("merged" | "closed")) => s.to_string(),
        _ => "open".into(),
    }
}

fn is_closed(pr: Option<&PrStatus>) -> bool {
    pr.is_some_and(|p| p.state == "closed")
}

impl PrView {
    /// A line with no PR yet: `none`, or `pending` on its gate.
    fn waiting(issue: Option<&str>, branch: String, gate: Option<u64>) -> PrView {
        PrView {
            issue: issue.map(str::to_string),
            branch,
            state: if gate.is_some() { "pending" } else { "none" }.into(),
            number: None,
            url: None,
            pr: None,
            gate,
        }
    }
}

/// The run's PRs for the UI, both modes: single → `[]` or the feature's one
/// PR (`issue: null`); per_issue → one line per feature issue, in order.
/// `gates` are the run's pending `push_pr` / `push_pr_update` gates.
pub fn prs_view(run: &TeamRun, feature: &Feature, gates: &[GateRef]) -> Vec<PrView> {
    let gate = |action: &str, issue: Option<&str>| {
        gates.iter().find(|g| g.action == action && g.issue.as_deref() == issue).map(|g| g.id)
    };
    match feature.pr_mode {
        PrMode::Single => {
            let branch = feature.integration_branch();
            // A new PR asked for replaces a closed one.
            let replaced = run.pr_number.is_none() || is_closed(run.pr.as_ref());
            if let Some(id) = gate("push_pr", None).filter(|_| replaced) {
                return vec![PrView::waiting(None, branch, Some(id))];
            }
            if run.pr_number.is_none() && run.pr_url.is_none() {
                return vec![];
            }
            vec![PrView {
                issue: None,
                branch,
                state: pr_state(run.pr.as_ref()),
                number: run.pr_number,
                url: run.pr_url.clone(),
                pr: run.pr.clone(),
                gate: gate("push_pr_update", None),
            }]
        }
        PrMode::PerIssue => feature
            .issues
            .iter()
            .map(|i| {
                let r = i.reference.as_str();
                let pr = run.prs.iter().rev().find(|p| p.issue == r);
                let replaced = pr.is_none_or(|p| is_closed(p.pr.as_ref()));
                match (pr, gate("push_pr", Some(r)).filter(|_| replaced)) {
                    (Some(p), None) => PrView {
                        issue: Some(r.into()),
                        branch: p.branch.clone(),
                        state: pr_state(p.pr.as_ref()),
                        number: Some(p.number),
                        url: Some(p.url.clone()),
                        pr: p.pr.clone(),
                        gate: gate("push_pr_update", Some(r)),
                    },
                    (_, pending) => PrView::waiting(Some(r), feature.issue_branch(r), pending),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use orch_store::RunPr;
    use orch_team::RunState;

    const REPO: Option<&str> = Some("o/r");

    /// The `code` of a coded error (its text when it is not one).
    pub(crate) fn code(e: anyhow::Error) -> String {
        serde_json::from_str::<serde_json::Value>(&e.to_string())
            .map(|v| v["code"].as_str().unwrap_or("").to_string())
            .unwrap_or_else(|_| e.to_string())
    }

    /// `git interpret-trailers --parse` of a commit message.
    pub(crate) fn trailers(msg: &str) -> String {
        use std::io::Write;
        let mut c = std::process::Command::new("git")
            .args(["interpret-trailers", "--parse"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(msg.as_bytes()).unwrap();
        String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()
    }

    #[test]
    fn issue_rows_keep_the_issues_only() {
        let of = |kind: &str, deleted: bool| IntakeRow {
            kind: kind.into(),
            deleted,
            node_id: format!("n-{kind}-{deleted}"),
            ..row("github", "#1", "")
        };
        let rows = vec![
            of("issue", false),
            of("milestone", false),
            of("linear_project", false),
            of("brief", false),
            of("issue", true),
        ];
        let kept: Vec<String> = issue_rows(rows).into_iter().map(|r| r.node_id).collect();
        assert_eq!(kept, ["n-issue-false"]);
    }

    #[test]
    fn refs_parse_to_their_canonical_form() {
        for t in ["12", "#12", " o/r#12 ", "O/R#12", "https://github.com/o/r/issues/12"] {
            assert_eq!(parse_ref(t, REPO).unwrap(), "#12", "{t}");
        }
        assert_eq!(parse_ref("eng-7", REPO).unwrap(), "ENG-7");
        assert_eq!(parse_ref("A1-23", None).unwrap(), "A1-23");
        assert_eq!(parse_ref("#3", None).unwrap(), "#3");
        for t in [
            "x/y#12",
            "https://github.com/x/y/issues/12",
            "https://github.com/o/r/pull/12",
            "nope",
            "1-2",
            "ENG-",
            "#0",
        ] {
            assert_eq!(code(parse_ref(t, REPO).unwrap_err()), "bad_ref", "{t}");
        }
        let e = parse_ref("x/y#1", REPO).unwrap_err().to_string();
        assert!(e.contains("\"ref\":\"x/y#1\""), "{e}");
        assert_eq!(code(parse_ref("o/r#1", None).unwrap_err()), "bad_ref");
    }

    #[test]
    fn keys_name_the_branches() {
        assert_eq!(issue_key("#12"), "12");
        assert_eq!(issue_key("ABC-123"), "abc-123");
    }

    #[test]
    fn a_list_is_deduplicated_and_capped() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(normalize(&l(&["12", "#12", "eng-1", "ENG-1", " "]), REPO).unwrap(), ["#12", "ENG-1"]);
        let many: Vec<String> = (1..=11).map(|n| n.to_string()).collect();
        assert_eq!(code(normalize(&many, REPO).unwrap_err()), "too_many");
        assert_eq!(normalize(&many[..10], REPO).unwrap().len(), 10);
        assert_eq!(code(check_mode(PrMode::PerIssue, 1).unwrap_err()), "per_issue_needs_two");
        assert!(check_mode(PrMode::PerIssue, 2).is_ok() && check_mode(PrMode::Single, 0).is_ok());
    }

    fn row(source: &str, reference: &str, title: &str) -> IntakeRow {
        IntakeRow {
            id: 1,
            project_id: "p".into(),
            source: source.into(),
            reference: reference.into(),
            title: title.into(),
            url: Some(format!("https://x/{reference}")),
            body: String::new(),
            meta: json!({}),
            rev: String::new(),
            fetched_at_ms: 0,
            position: 0,
            state: Default::default(),
            started_kind: None,
            started_ref: None,
            hidden_at_ms: None,
            hidden_rev: None,
            node_id: format!("n:{reference}"),
            kind: "issue".into(),
            parent_id: None,
            depth: 1,
            deleted: false,
        }
    }

    #[test]
    fn titles_come_from_the_intake_cache_and_suggestions_match_ref_or_title() {
        let rows = vec![
            row("github", "#12", "Crash on empty brief"),
            row("linear", "ENG-7", "Faster boot"),
            row("repo_docs", "docs/x.md", "Crash doc"),
        ];
        let ls = links(&["#12".into(), "ENG-8".into()], &rows, REPO);
        assert_eq!(ls[0].title.as_deref(), Some("Crash on empty brief"));
        assert_eq!(ls[0].url.as_deref(), Some("https://x/#12"));
        assert_eq!((ls[1].key.as_str(), ls[1].title.clone()), ("eng-8", None));
        let refs = |q: &str| suggest(q, &rows, REPO).into_iter().map(|l| l.reference).collect::<Vec<_>>();
        assert_eq!(refs("crash"), ["#12"], "a repo doc has no ref");
        assert_eq!(refs("#1"), ["#12"]);
        assert_eq!(refs("eng"), ["ENG-7"]);
        assert_eq!(refs("boot"), ["ENG-7"]);
        assert!(refs("zzz").is_empty());
        let many: Vec<IntakeRow> = (1..20).map(|n| row("github", &format!("#{n}"), "t")).collect();
        assert_eq!(suggest("t", &many, REPO).len(), 8);
        // Another repository's issue is not the project's `#12`.
        let foreign = vec![
            row("github", "x/y#12", "Other repo"),
            IntakeRow { url: Some("https://github.com/x/y/issues/13".into()), ..row("github", "#13", "Other source") },
            IntakeRow { url: Some("https://github.com/o/r/issues/14".into()), ..row("github", "#14", "Ours") },
        ];
        let refs = |q: &str| suggest(q, &foreign, REPO).into_iter().map(|l| l.reference).collect::<Vec<_>>();
        assert_eq!(refs(""), ["#14"]);
        assert_eq!(row_ref(&foreign[1], None).as_deref(), Some("#13"), "no project repository to tell");
        assert_eq!(row_ref(&foreign[0], None), None);
    }

    #[test]
    fn closes_lines_are_added_only_when_missing() {
        assert!(body_closes("This closes #12.", "#12"));
        assert!(body_closes("FIXES: ENG-7", "ENG-7"));
        assert!(body_closes("Resolved #3\n", "#3"));
        assert!(!body_closes("Closes #123", "#12"), "another issue");
        assert!(!body_closes("See #12", "#12"));
        assert!(!body_closes("prefix #12", "#12"), "fix inside a word is no keyword");
        assert!(!body_closes("Closes XENG-7", "ENG-7"));
        assert!(!body_closes("enclosed #12", "#12"), "a whole keyword");
        assert!(!body_closes("Closes #123", "#12"), "a whole ref");
        assert!(!body_closes("Closes:#12", "#12"), "a space after the keyword");
        assert!(!body_closes("closes: : #12", "#12"));
        assert!(body_closes("x (closes  #12)", "#12"));
        let refs = vec!["#1".to_string(), "#2".to_string()];
        assert_eq!(with_closes("Body.\n\nFixes #1\n", &refs), "Body.\n\nFixes #1\n\nCloses #2");
        assert_eq!(with_closes("", &refs), "Closes #1\nCloses #2");
        assert_eq!(with_closes("closes #1, closes #2", &refs), "closes #1, closes #2");
        assert_eq!(with_closes("x", &[]), "x");
    }

    #[test]
    fn trailers_are_read_by_git_interpret_trailers() {
        let cases = [
            ("Merge orch/f/back-1", "Merge orch/f/back-1\n\nOrch-Task: 7"),
            ("feat: x\n\nWhy it changed.\n", "feat: x\n\nWhy it changed.\n\nOrch-Task: 7"),
            ("feat: x\n\nCo-Authored-By: A <a@b>", "feat: x\n\nCo-Authored-By: A <a@b>\nOrch-Task: 7"),
        ];
        for (m, want) in cases {
            let got = with_trailer(m, "Orch-Task", "7");
            assert_eq!(got, want);
            let parsed = trailers(&got);
            assert!(parsed.lines().any(|l| l == "Orch-Task: 7"), "{m:?} → {parsed:?}");
        }
    }

    fn feature(mode: PrMode, refs: &[&str]) -> Feature {
        Feature {
            id: "p-f".into(),
            project_id: "p".into(),
            slug: "f".into(),
            title: "f".into(),
            brief: String::new(),
            base: "main".into(),
            integration_worktree: None,
            allocation: None,
            created_at_ms: 0,
            archived: false,
            issues: refs.iter().map(|r| IssueLink::new(r)).collect(),
            pr_mode: mode,
        }
    }

    fn run() -> TeamRun {
        TeamRun {
            id: "r".into(),
            team_id: "t".into(),
            team_scope: Default::default(),
            team_version: 1,
            feature_id: "p-f".into(),
            status: RunState::Integrating,
            resume_status: None,
            pr_url: None,
            started_at_ms: 0,
            ended_at_ms: None,
            pr_number: None,
            pr: None,
            prs: vec![],
        }
    }

    fn status(state: &str) -> PrStatus {
        PrStatus { state: state.into(), ..Default::default() }
    }

    fn g(id: u64, action: &str, issue: Option<&str>) -> GateRef {
        GateRef { id, action: action.into(), issue: issue.map(str::to_string) }
    }

    #[test]
    fn single_mode_has_no_line_then_the_gate_then_the_pr() {
        let f = feature(PrMode::Single, &["#1", "#2"]);
        let mut r = run();
        assert!(prs_view(&r, &f, &[]).is_empty());
        let v = prs_view(&r, &f, &[g(5, "push_pr", None)]);
        assert_eq!((v[0].state.as_str(), v[0].gate, v[0].issue.clone()), ("pending", Some(5), None));
        assert_eq!(v[0].branch, "orch/f/integration");
        (r.pr_url, r.pr_number) = (Some("https://github.com/o/r/pull/9".into()), Some(9));
        let v = prs_view(&r, &f, &[]);
        assert_eq!((v[0].state.as_str(), v[0].number, v[0].pr.clone()), ("open", Some(9), None));
        r.pr = Some(status("open"));
        let v = prs_view(&r, &f, &[g(6, "push_pr_update", None)]);
        assert_eq!((v.len(), v[0].gate, v[0].pr.is_some()), (1, Some(6), true));
        r.pr = Some(status("merged"));
        assert_eq!(prs_view(&r, &f, &[]).first().map(|v| v.state.clone()).as_deref(), Some("merged"));
        r.pr = Some(status("closed"));
        assert_eq!(prs_view(&r, &f, &[]).first().map(|v| v.state.clone()).as_deref(), Some("closed"));
        let v = prs_view(&r, &f, &[g(7, "push_pr", None)]);
        assert_eq!((v[0].state.as_str(), v[0].number), ("pending", None), "a new PR replaces the closed one");
        let json = serde_json::to_value(&v[0]).unwrap();
        let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["branch", "gate", "issue", "number", "pr", "state", "url"]);
    }

    #[test]
    fn per_issue_mode_has_one_line_per_issue_in_order() {
        let f = feature(PrMode::PerIssue, &["#1", "ENG-2", "#3"]);
        let mut r = run();
        r.prs = vec![
            RunPr {
                issue: "ENG-2".into(),
                branch: "orch/f/issue/eng-2".into(),
                number: 4,
                url: "u4".into(),
                pr: Some(status("open")),
                ..Default::default()
            },
            RunPr {
                issue: "#3".into(),
                branch: "orch/f/issue/3".into(),
                number: 5,
                url: "u5".into(),
                pr: Some(status("closed")),
                ..Default::default()
            },
        ];
        let v = prs_view(&r, &f, &[g(8, "push_pr", Some("#1")), g(9, "push_pr_update", Some("ENG-2"))]);
        let s: Vec<(&str, &str, Option<u64>, Option<u64>)> =
            v.iter().map(|p| (p.issue.as_deref().unwrap(), p.state.as_str(), p.number, p.gate)).collect();
        assert_eq!(
            s,
            [("#1", "pending", None, Some(8)), ("ENG-2", "open", Some(4), Some(9)), ("#3", "closed", Some(5), None)]
        );
        assert_eq!(v[0].branch, "orch/f/issue/1");
        let v = prs_view(&r, &f, &[]);
        assert_eq!(v[0].state, "none");
        let v = prs_view(&r, &f, &[g(10, "push_pr", Some("#3"))]);
        assert_eq!((v[2].state.as_str(), v[2].gate), ("pending", Some(10)), "a new PR replaces the closed one");
        let v = prs_view(&r, &f, &[g(11, "push_pr", Some("ENG-2"))]);
        assert_eq!(v[1].state, "open", "an open PR is not replaced");
    }
}
