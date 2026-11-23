//! allow-launch-team-on-update-pr §5.1–§5.2: the project's PR list with what a
//! launch needs to know per row, and the launch prefill of one PR (brief with
//! the failing checks' log tails).

use std::path::Path;

use anyhow::Result;
use orch_git::gh::{self, PullRequest};
use orch_store::Project;
use serde_json::{Value, json};

use crate::intake::start::{kebab, unique};
use crate::pr_link::{feature_ref, gh_error_code, refused};
use crate::runtime::Runtime;

/// The longest slug of a PR launch.
const SLUG_MAX: usize = 40;
/// Failing jobs whose log is read, lines kept per job, bytes of logs in all, bytes of PR body.
const MAX_JOBS: usize = 4;
const LOG_LINES: usize = 60;
const LOG_BYTES: usize = 12 * 1024;
const BODY_BYTES: usize = 4 * 1024;

const VIEW_FIELDS: &str = "number,url,state,title,body,headRefName,headRefOid,baseRefName,isCrossRepository,headRepositoryOwner,author,statusCheckRollup";

/// The fork reason (api-contract §9).
fn fork_reason(owner: Option<&str>) -> String {
    match owner {
        Some(o) => format!("Fork PR (from {o}): the app pushes to origin only."),
        None => "Fork PR: the app pushes to origin only.".into(),
    }
}

/// `pr-<n>-<kebab title>` cut at `SLUG_MAX` on a word boundary.
pub fn pr_slug(number: u64, title: &str) -> String {
    let full = format!("pr-{number}-{}", kebab(title, 200));
    let full = full.trim_end_matches('-');
    if full.len() <= SLUG_MAX {
        return full.to_string();
    }
    let cut = &full[..SLUG_MAX];
    // A cut inside a word drops that word; the number alone is kept when it comes to that.
    let end = if full.as_bytes()[SLUG_MAX] == b'-' { cut } else { cut.rfind('-').map_or(cut, |i| &cut[..i]) };
    let end = end.trim_end_matches('-');
    if end.len() <= format!("pr-{number}").len() { cut.trim_end_matches('-').to_string() } else { end.to_string() }
}

/// `(run id, job id)` of a GitHub Actions job URL (`…/actions/runs/<run>/job/<job>`).
fn actions_ids(url: &str) -> Option<(String, Option<String>)> {
    let rest = url.split_once("/actions/runs/")?.1;
    let mut it = rest.split(['/', '?', '#']);
    let run = it.next().filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))?;
    let job = (it.next() == Some("job")).then(|| it.next()).flatten().filter(|j| j.bytes().all(|b| b.is_ascii_digit()));
    Some((run.to_string(), job.map(str::to_string)))
}

/// The text of a `gh run view --log-failed` line: `job\tstep\t<timestamp> text` → `text`.
fn log_text(line: &str) -> &str {
    let t = line.splitn(3, '\t').nth(2).unwrap_or(line);
    match t.split_once(' ') {
        Some((ts, rest)) if ts.len() >= 20 && ts.as_bytes()[4] == b'-' && ts.ends_with('Z') => rest,
        _ => t,
    }
}

/// The last `n` lines of `text`; whether any was left out.
fn tail(text: &str, n: usize) -> (String, bool) {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let lines = &lines[..lines.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1)];
    let from = lines.len().saturating_sub(n);
    (lines[from..].join("\n"), from > 0)
}

/// `text` cut to its last `max` bytes on a line start.
fn tail_bytes(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut from = text.len() - max;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    let cut = &text[from..];
    (cut.split_once('\n').map_or(cut, |(_, rest)| rest).to_string(), true)
}

/// `text` as a Markdown fence that survives any `~~~` inside it.
fn fenced(text: &str) -> String {
    let longest =
        text.lines().map(|l| l.trim_start().chars().take_while(|c| *c == '~').count()).max().unwrap_or(0).max(2);
    let f = "~".repeat(longest + 1);
    let indented: String = text.lines().map(|l| format!("  {l}\n")).collect();
    format!("  {f}\n{indented}  {f}")
}

/// What the brief says of one failing check, and how its log came.
struct Failing {
    name: String,
    url: Option<String>,
    log: Option<String>,
}

impl Runtime {
    /// `gh <args>` in `dir`; tests replace it through `cfg.gh`.
    fn gh_text(&self, dir: &Path, args: &[String]) -> Result<String> {
        match self.gh_override() {
            Some(g) => g.run(args),
            None => gh::gh(dir, &args.iter().map(String::as_str).collect::<Vec<_>>()),
        }
    }

    /// The open PRs of `project` with CI, and per row the linked feature and
    /// whether a team can launch on it (api-contract §1).
    pub(crate) fn pr_list(&self, project: &Project) -> Result<Vec<Value>> {
        let prs = match self.gh_override() {
            Some(g) => gh::parse_pr_list(&g.run(&gh::pr_list_args())?)?,
            None => gh::prs(&project.path)?,
        };
        prs.iter().map(|pr| self.pr_row(project, pr)).collect()
    }

    fn pr_row(&self, project: &Project, pr: &PullRequest) -> Result<Value> {
        let linked = self.feature_on_pr(&project.id, pr.number)?;
        let feature = linked.as_ref().map(|(f, run)| feature_ref(f, run.as_ref()));
        let active =
            linked.as_ref().filter(|(_, run)| run.as_ref().is_some_and(|r| !crate::team::effective(r).finished()));
        let launch = if pr.cross_repo {
            json!({"ok": false, "code": "fork", "reason": fork_reason(pr.fork_owner.as_deref())})
        } else if let Some((f, run)) = active {
            json!({"ok": false, "code": "active_feature",
                "reason": format!("PR #{} already has a team: feature {}", pr.number, f.slug),
                "feature": feature_ref(f, run.as_ref())})
        } else {
            json!({"ok": true})
        };
        let mut v = json!(pr);
        v["feature"] = json!(feature);
        v["launch"] = launch;
        Ok(v)
    }

    /// The launch prefill of PR `number` (api-contract §2, F2, F3).
    pub fn pr_prefill(&self, project: &str, number: u64) -> Result<Value> {
        let project = self.project(project)?;
        let n = number;
        if let Some((f, run)) = self.feature_on_pr(&project.id, n)?
            && run.as_ref().is_some_and(|r| !crate::team::effective(r).finished())
        {
            return Err(refused(
                "active_feature",
                format!("PR #{n} already has a team: feature {}", f.slug),
                json!({"feature": feature_ref(&f, run.as_ref())}),
            ));
        }
        if !self.cfg.conditions.online() {
            return Err(refused("offline", format!("offline: PR #{n} can't be read"), json!({})));
        }
        let args: Vec<String> = ["pr", "view", &n.to_string(), "--json", VIEW_FIELDS].map(str::to_string).to_vec();
        let read = |e: anyhow::Error| {
            refused(gh_error_code(&e, "fetch_failed"), format!("could not read PR #{n}: {e:#}"), json!({}))
        };
        let text = self.gh_text(&project.path, &args).map_err(read)?;
        let view: Value = serde_json::from_str(&text).map_err(|e| read(e.into()))?;
        let h = gh::parse_pr_head(&text).map_err(read)?;
        if h.cross_repo {
            let owner = view["headRepositoryOwner"]["login"].as_str().filter(|o| !o.is_empty());
            return Err(refused("fork", fork_reason(owner), json!({})));
        }
        match h.state.as_str() {
            "open" => {}
            "merged" => return Err(refused("pr_merged", format!("PR #{n} is merged"), json!({}))),
            _ => return Err(refused("pr_closed", format!("PR #{n} is closed"), json!({}))),
        }
        let title = view["title"].as_str().unwrap_or("").to_string();
        let rollup = &view["statusCheckRollup"];
        let counts = gh::summarize_rollup(rollup);
        let (failing, logs) = self.failing_checks(&project, rollup);
        let brief = brief(&h, &title, view["body"].as_str().unwrap_or(""), author_name(&view), &failing, &counts);
        let team = self.intake_team(&project, None);
        let slug = unique(&pr_slug(n, &title), SLUG_MAX, |s| {
            s == "solo" || self.db.feature_by_slug(&project.id, s).ok().flatten().is_some()
        });
        let author = h.author.clone();
        Ok(json!({
            "launch": {
                "project_id": project.id, "team_id": team, "slug": slug,
                "title": format!("PR #{n} · {title}"), "brief": brief, "base": h.base_ref,
                "off_hours": false, "issues": [], "pr_mode": null,
                "pr": {"number": n, "url": h.url, "head_ref": h.head_ref, "base_ref": h.base_ref,
                       "head_sha": h.head_sha, "author": author},
            },
            "origin": {"kind": "pr", "label": format!("PR #{n}"), "url": h.url},
            "notes": {"failing": counts.failing, "pending": counts.pending, "logs": logs},
            "warnings": self.pr_warnings(&project, &h.head_ref, n),
        }))
    }

    /// The failing checks, with the tail of their GitHub Actions log (or their
    /// annotations), within the limits; and how the logs came: `full`, `cut`,
    /// `annotations` or `none` (F2).
    fn failing_checks(&self, project: &Project, rollup: &Value) -> (Vec<Failing>, &'static str) {
        let jobs: Vec<_> = gh::summarize_jobs(rollup).jobs.into_iter().filter(|j| gh::job_failed(&j.state)).collect();
        let mut cut = jobs.len() > MAX_JOBS;
        let (mut any_log, mut any_annotation) = (false, false);
        let mut budget = LOG_BYTES;
        let mut out = Vec::new();
        for (i, j) in jobs.iter().enumerate() {
            let name = match &j.workflow {
                Some(w) if !w.is_empty() => format!("{w} / {}", j.name),
                _ => j.name.clone(),
            };
            let mut log = None;
            if i < MAX_JOBS
                && budget > 0
                && let Some((run, job)) = j.url.as_deref().and_then(actions_ids)
            {
                let mut args: Vec<String> = ["run", "view", &run, "--log-failed"].map(str::to_string).to_vec();
                if let Some(job) = &job {
                    args.extend(["--job".to_string(), job.clone()]);
                }
                let text = self.gh_text(&project.path, &args).unwrap_or_default();
                let text = text.lines().map(log_text).collect::<Vec<_>>().join("\n");
                let (mut t, mut was_cut) = tail(&text, LOG_LINES);
                if t.trim().is_empty()
                    && let Some(job) = &job
                {
                    let args: Vec<String> =
                        ["api".to_string(), format!("repos/{{owner}}/{{repo}}/check-runs/{job}/annotations")].to_vec();
                    t = annotations(&self.gh_text(&project.path, &args).unwrap_or_default());
                    any_annotation |= !t.is_empty();
                    was_cut = false;
                } else if !t.trim().is_empty() {
                    any_log = true;
                }
                if !t.trim().is_empty() {
                    let (b, c) = tail_bytes(&t, budget);
                    budget -= b.len();
                    cut |= was_cut || c;
                    log = Some(b);
                } else {
                    cut |= was_cut;
                }
            }
            out.push(Failing { name, url: j.url.clone(), log });
        }
        let logs = if out.is_empty() || (!any_log && !any_annotation) {
            "none"
        } else if cut {
            "cut"
        } else if any_annotation && !any_log {
            "annotations"
        } else {
            "full"
        };
        (out, logs)
    }

    /// F3: what the launch would refuse, found without a fetch (it stays enabled).
    fn pr_warnings(&self, project: &Project, head: &str, n: u64) -> Vec<Value> {
        let mut w = Vec::new();
        if let Err(e) = self.branch_checked_out(project, head)
            && let Ok(v) = serde_json::from_str::<Value>(&e.to_string())
        {
            w.push(json!({"code": "branch_checked_out", "message": v["message"], "details": v["details"]}));
        }
        let (local, remote) = (format!("refs/heads/{head}"), format!("refs/remotes/origin/{head}"));
        if orch_git::worktree::branch_exists(&project.path, head)
            && gh::rev_sha(&project.path, &remote).is_some()
            && let Some((ahead, _)) = orch_git::cleanup::try_ahead_behind(&project.path, &local, &remote)
            && ahead > 0
        {
            w.push(json!({
                "code": "local_ahead",
                "message": format!("Local {head} has {ahead} commit(s) PR #{n} does not have. Rename or delete that local branch first."),
                "details": {"commits": ahead},
            }));
        }
        w
    }
}

fn author_name(view: &Value) -> Option<String> {
    gh::parse_author(view).map(|a| a.login)
}

/// The annotations of a check run as `path:line: message` lines.
fn annotations(json: &str) -> String {
    let v: Value = serde_json::from_str(json).unwrap_or_default();
    let lines: Vec<String> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["annotation_level"].as_str() != Some("notice"))
        .map(|a| {
            let at = a["path"]
                .as_str()
                .filter(|p| !p.is_empty() && *p != ".github")
                .map_or(String::new(), |p| format!("{p}:{}: ", a["start_line"].as_u64().unwrap_or(0)));
            format!("{at}{}", a["message"].as_str().unwrap_or("").trim())
        })
        .collect();
    tail(&lines.join("\n"), LOG_LINES).0
}

/// The brief of api-contract §2.
fn brief(
    h: &gh::PrHead,
    title: &str,
    body: &str,
    author: Option<String>,
    failing: &[Failing],
    counts: &gh::ChecksSummary,
) -> String {
    let n = h.number;
    let by = author.unwrap_or_else(|| "an unknown author".into());
    let mut s = format!(
        "Make PR #{n} pass CI: {title}\n{} · {} → {} · opened by {by}\n\n\
         Work on the PR's branch and push the fix to it; do not open another pull request.\n\n## Failing checks\n",
        h.url, h.head_ref, h.base_ref
    );
    if failing.is_empty() {
        s += "No check is failing right now";
        if counts.pending > 0 {
            s += &format!(" ({} pending)", counts.pending);
        }
        s += ".\n";
    }
    for f in failing {
        s += &format!("- {}{}\n", f.name, f.url.as_ref().map(|u| format!(" · {u}")).unwrap_or_default());
        if let Some(log) = &f.log {
            s += &fenced(log);
            s.push('\n');
        }
    }
    let (body, cut) = {
        let body = body.trim();
        if body.len() <= BODY_BYTES {
            (body.to_string(), false)
        } else {
            let mut end = BODY_BYTES;
            while !body.is_char_boundary(end) {
                end -= 1;
            }
            (body[..end].to_string(), true)
        }
    };
    s += "\n## PR description\n";
    s += if body.is_empty() { "No description." } else { &body };
    if cut {
        s += "\n…";
    }
    s.push('\n');
    s
}

#[cfg(test)]
#[path = "pr_prefill_tests.rs"]
mod tests;
