//! implement-improve-intake B3: the Work commands over the tracker tree
//! cached by the refresher (`work_nodes`): `work_tree`, `work_search`,
//! `work_node`, `work_reveal`, `work_refresh`, `work_hide` / `work_unhide`,
//! `work_prefill` / `brief_prefill`, and the [`Ctx`] that builds a
//! `WorkNode` (contract §2): children counts, app state, roll-up, actions.
//! Everything reads the cache; the one network read is the lazy fetch of a
//! parent's sub-issues, with its own error in the answer.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use orch_store::{IntakeRow, IntakeState, Project, Task, Tracker, TrackerSettings};
use orch_tasks::intake as src;
use orch_team::RunState;
use serde::Deserialize;
use serde_json::{Value, json};

use super::tracker::upstream_json;
use crate::runtime::Runtime;

/// Roots are paged this many at a time, children half of it.
const ROOTS_PAGE: usize = 100;
const CHILDREN_PAGE: usize = 50;
/// A search answers at most this many nodes.
const SEARCH_MAX: usize = 100;
const SNIPPET: usize = 80;
/// Parents with more sub-issues than this are fetched in pages by cursor, never cut.
const REVEAL_SIBLINGS: usize = 50;

/// `WorkFilter` (contract §2, F3). Every field optional.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorkFilter {
    pub state: String,
    pub only_ready: bool,
    pub only_not_started: bool,
    pub labels: Vec<String>,
    /// `@me` allowed.
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub cycle: Option<String>,
    pub issue_types: Vec<String>,
    pub linear_projects: Vec<String>,
    pub include_team_tasks: bool,
    pub show_hidden: bool,
}

impl Default for WorkFilter {
    fn default() -> Self {
        WorkFilter {
            state: "open".into(),
            only_ready: false,
            only_not_started: false,
            labels: vec![],
            assignees: vec![],
            milestone: None,
            cycle: None,
            issue_types: vec![],
            linear_projects: vec![],
            include_team_tasks: false,
            show_hidden: false,
        }
    }
}

impl WorkFilter {
    pub fn parse(v: &Value) -> Result<WorkFilter> {
        if v.is_null() {
            return Ok(WorkFilter::default());
        }
        serde_json::from_value(v.clone()).map_err(|e| super::bad_request("filter", e.to_string()))
    }

    /// A constraint on the items themselves (containers then match only through their descendants).
    fn on_items(&self) -> bool {
        self.only_ready
            || self.only_not_started
            || !self.labels.is_empty()
            || !self.assignees.is_empty()
            || self.milestone.is_some()
            || self.cycle.is_some()
            || !self.issue_types.is_empty()
            || !self.linear_projects.is_empty()
    }
}

/// Projects whose tree could not be read, with why.
type Failed = Vec<(String, String)>;

pub(crate) fn is_item(kind: &str) -> bool {
    matches!(kind, "issue" | "local")
}

/// Open upstream: a Linear state type that is not completed / canceled, else GitHub's `state`.
fn is_open(meta: &Value) -> bool {
    match meta["state_type"].as_str() {
        Some(t) => !matches!(t, "completed" | "canceled"),
        None => meta["state"].as_str().is_none_or(|s| s.eq_ignore_ascii_case("open")),
    }
}

/// What a node stands for in the app, for the roll-up.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    None,
    Running,
    Pr,
    Merged,
    /// In a program, waiting for a dependency / held by the Governor, failed, skipped.
    Waiting,
    Queued,
    Failed,
    Skipped,
}

#[derive(Clone, Default)]
struct Roll {
    running: u32,
    pr: u32,
    waiting: u32,
    queued: u32,
    merged: u32,
    failed: u32,
    skipped: u32,
}

impl Roll {
    fn json(&self) -> Value {
        if self.running + self.pr + self.waiting + self.queued + self.merged + self.failed + self.skipped == 0 {
            return Value::Null;
        }
        json!({"running": self.running, "pr": self.pr, "waiting": self.waiting, "queued": self.queued,
               "merged": self.merged, "failed": self.failed, "skipped": self.skipped})
    }

    fn add(&mut self, o: &Roll) {
        self.running += o.running;
        self.pr += o.pr;
        self.waiting += o.waiting;
        self.queued += o.queued;
        self.merged += o.merged;
        self.failed += o.failed;
        self.skipped += o.skipped;
    }
}

/// A project's tree with what a `WorkNode` needs, built once per call.
pub(crate) struct Ctx {
    pub project: Project,
    pub settings: TrackerSettings,
    pub rows: Vec<IntakeRow>,
    by_id: HashMap<String, usize>,
    /// Children by parent index (`None`: the roots), in tree order.
    kids: HashMap<Option<usize>, Vec<usize>>,
    can_team: bool,
    apps: Vec<(Value, Class)>,
    /// The tasks of the runs started from a node.
    team: HashMap<usize, Vec<Task>>,
    rolls: Vec<Roll>,
    viewer: Option<String>,
    /// The write-back status of each node (contract §2 `writeback`).
    wb: HashMap<String, Value>,
}

fn phase(s: RunState) -> &'static str {
    match s {
        RunState::Draft | RunState::Spec | RunState::SpecGate => "spec",
        RunState::Planning => "planning",
        RunState::Integrating => "integrating",
        RunState::PrGate | RunState::PrOpen => "pr",
        _ => "executing",
    }
}

impl Ctx {
    pub(crate) fn new(rt: &Runtime, project: &Project) -> Result<Ctx> {
        let rows = rt.db.work_nodes_light(&project.id)?;
        let by_id: HashMap<String, usize> = rows.iter().enumerate().map(|(i, r)| (r.node_id.clone(), i)).collect();
        let mut kids: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            let p = r.parent_id.as_deref().and_then(|p| by_id.get(p)).copied().filter(|p| *p != i);
            kids.entry(p).or_default().push(i);
        }
        let settings = rt.db.tracker(&project.id)?;
        let facts = rt.project_facts(project);
        let mut ctx = Ctx {
            project: project.clone(),
            settings,
            by_id,
            kids,
            can_team: facts.can_team,
            apps: vec![],
            team: HashMap::new(),
            rolls: vec![],
            wb: rt.wb_statuses(&project.id),
            viewer: rt.db.setting(&viewer_key(&project.id)).ok().flatten().and_then(|v| v.as_str().map(str::to_string)),
            rows,
        };
        ctx.load_apps(rt);
        ctx.overlay_programs(rt);
        ctx.load_rolls();
        Ok(ctx)
    }

    /// The app state of each node: from the run or session it was started as.
    fn load_apps(&mut self, rt: &Runtime) {
        let mut runs: HashMap<String, Option<(orch_store::TeamRun, Option<String>)>> = HashMap::new();
        for (i, r) in self.rows.iter().enumerate() {
            let none = (json!({"state": "none"}), Class::None);
            let app = match (r.state, r.started_kind.as_deref(), r.started_ref.as_deref()) {
                (IntakeState::Started, Some("agent"), Some(a)) => {
                    (json!({"state": "solo", "agent": a, "name": a}), Class::Running)
                }
                (IntakeState::Started, Some("run"), Some(id)) => {
                    let got = runs.entry(id.to_string()).or_insert_with(|| {
                        let run = rt.db.run(id).ok().flatten()?;
                        let slug = rt.db.feature(&run.feature_id).ok().flatten().map(|f| f.slug);
                        Some((run, slug))
                    });
                    match got {
                        None => none,
                        Some((run, slug)) => {
                            let feature = slug.clone().unwrap_or_else(|| run.feature_id.clone());
                            match run.status {
                                RunState::Done => (
                                    json!({"state": "done", "feature": feature,
                                           "merged_at_ms": run.ended_at_ms.unwrap_or(run.started_at_ms)}),
                                    Class::Merged,
                                ),
                                RunState::PrOpen | RunState::PrGate if run.pr_url.is_some() => (
                                    json!({"state": "pr", "feature": feature,
                                           "pr": {"number": run.pr_number, "url": run.pr_url}}),
                                    Class::Pr,
                                ),
                                RunState::Failed => none,
                                s => {
                                    let s = if s.held() { run.resume_status.unwrap_or(s) } else { s };
                                    (
                                        json!({"state": "running", "feature": feature, "run": run.id,
                                               "phase": phase(s)}),
                                        Class::Running,
                                    )
                                }
                            }
                        }
                    }
                }
                _ => none,
            };
            if let (Some("run"), Some(id)) = (r.started_kind.as_deref(), r.started_ref.as_deref()) {
                let tasks = rt.db.tasks(Some(&self.project.id), None, Some(id)).unwrap_or_default();
                if !tasks.is_empty() {
                    self.team.entry(i).or_default().extend(tasks);
                }
            }
            self.apps.push(app);
        }
    }

    /// What the programs of the project say of their items: waiting / queued in a
    /// program, failed, skipped, and the roots that are a program's.
    fn overlay_programs(&mut self, rt: &Runtime) {
        for v in rt.db.programs_in_progress(Some(&self.project.id)).unwrap_or_default() {
            let Ok(p) = serde_json::from_value::<crate::program::Program>(v) else { continue };
            if !matches!(p.status.as_str(), "running" | "paused") {
                continue;
            }
            let by = json!({"id": p.id, "title": p.title});
            let slug = |k: &str| p.features.iter().find(|f| f.key == k).map_or(k.to_string(), |f| f.slug.clone());
            let merged = p.features.iter().filter(|f| f.state == "merged").count();
            let total = p.features.iter().filter(|f| !f.skip).count();
            for r in &p.root {
                if let Some(i) = self.index(r) {
                    self.apps[i] = (
                        json!({"state": "program_root", "program": by, "merged": merged, "total": total}),
                        Class::None,
                    );
                }
            }
            for f in &p.features {
                for id in &f.items {
                    let Some(i) = self.index(id) else { continue };
                    if self.apps[i].1 != Class::None || self.apps[i].0["state"] != "none" {
                        continue;
                    }
                    let waiting: Vec<String> =
                        if f.state == "waiting" { f.depends_on.iter().map(|k| slug(k)).collect() } else { vec![] };
                    self.apps[i] = match f.state.as_str() {
                        "waiting" | "planned" => (
                            json!({"state": "queued", "program": by, "waiting_on": waiting}),
                            if f.state == "waiting" { Class::Waiting } else { Class::Queued },
                        ),
                        "failed" => (self.apps[i].0.clone(), Class::Failed),
                        "skipped" => (self.apps[i].0.clone(), Class::Skipped),
                        _ => continue,
                    };
                }
            }
        }
    }

    /// Roll-ups over the descendants (not the node itself): running, PR open, merged.
    fn load_rolls(&mut self) {
        self.rolls = vec![Roll::default(); self.rows.len()];
        fn walk(c: &mut Ctx, i: usize, depth: usize) -> Roll {
            let mut total = Roll::default();
            let kids = c.kids.get(&Some(i)).cloned().unwrap_or_default();
            if depth < 60 {
                for k in kids {
                    let below = walk(c, k, depth + 1);
                    total.add(&below);
                    match c.apps[k].1 {
                        Class::Running => total.running += 1,
                        Class::Pr => total.pr += 1,
                        Class::Merged => total.merged += 1,
                        Class::Waiting => total.waiting += 1,
                        Class::Queued => total.queued += 1,
                        Class::Failed => total.failed += 1,
                        Class::Skipped => total.skipped += 1,
                        Class::None => {}
                    }
                }
            }
            c.rolls[i] = total.clone();
            total
        }
        for root in self.kids.get(&None).cloned().unwrap_or_default() {
            walk(self, root, 0);
        }
    }

    /// `x:run:<id>` / `x:agent:<id>`: what a program waits on while the node runs outside it.
    pub(crate) fn app_wait_key(&self, i: usize) -> Option<String> {
        let a = &self.apps[i].0;
        a["run"].as_str().map(|r| format!("x:run:{r}")).or_else(|| a["agent"].as_str().map(|r| format!("x:agent:{r}")))
    }

    /// The feature (or session) a node runs as, from its app state.
    pub(crate) fn app_feature(&self, i: usize) -> Option<String> {
        let a = &self.apps[i].0;
        a["feature"].as_str().or_else(|| a["name"].as_str()).map(str::to_string)
    }

    /// The children of node `i`, in tree order.
    pub(crate) fn kids_of(&self, i: usize) -> &[usize] {
        self.kids.get(&Some(i)).map_or(&[][..], Vec::as_slice)
    }

    pub(crate) fn index(&self, node: &str) -> Option<usize> {
        self.by_id.get(node).copied()
    }

    pub(crate) fn open(&self, i: usize) -> bool {
        is_open(&self.rows[i].meta)
    }

    fn ready(&self, i: usize) -> bool {
        let r = &self.rows[i];
        self.settings.is_ready(&r.kind, &r.meta)
    }

    pub(crate) fn app_state(&self, i: usize) -> &str {
        self.apps[i].0["state"].as_str().unwrap_or("none")
    }

    fn team_tasks(&self, i: usize) -> usize {
        self.team.get(&i).map_or(0, Vec::len)
    }

    /// `(total, open, done)` of a node's children (team tasks excluded). A parent
    /// whose children are not all cached reads the upstream summary.
    fn children(&self, i: usize) -> (usize, usize, usize) {
        let kids = self.kids.get(&Some(i)).map_or(&[][..], Vec::as_slice);
        let done = kids.iter().filter(|k| !self.open(**k)).count();
        let meta = &self.rows[i].meta;
        let up_total = meta["sub_total"].as_u64().unwrap_or(0) as usize;
        let up_done = meta["sub_done"].as_u64().unwrap_or(0) as usize;
        let total = kids.len().max(up_total.saturating_sub(self.team_tasks(i)));
        let done = done.max(up_done.min(total));
        (total, total - done, done)
    }

    /// Children the upstream says exist but the cache lacks.
    fn missing_children(&self, i: usize) -> bool {
        let r = &self.rows[i];
        if r.kind != "issue" {
            return false;
        }
        let cached = self.kids.get(&Some(i)).map_or(0, Vec::len);
        let upstream = r.meta["sub_total"].as_u64().unwrap_or(0) as usize;
        upstream > cached + self.team_tasks(i)
    }

    pub(crate) fn breadcrumb(&self, i: usize) -> Vec<Value> {
        let (mut chain, mut at) = (vec![], i);
        while let Some(p) = self.rows[at].parent_id.as_deref().and_then(|p| self.by_id.get(p)).copied() {
            if chain.len() > 50 || p == at {
                break;
            }
            let r = &self.rows[p];
            chain.push(json!({"id": r.node_id, "ref": r.reference, "title": r.title}));
            at = p;
        }
        chain.reverse();
        chain
    }

    fn tracker_name(&self) -> &'static str {
        match self.settings.tracker {
            Tracker::Github { .. } => "GitHub",
            Tracker::Linear { .. } => "Linear",
            Tracker::Local { .. } => "the backlog",
        }
    }

    /// Notes §5.1: the actions a node offers.
    pub(crate) fn actions(&self, i: usize) -> Value {
        let r = &self.rows[i];
        let item = is_item(&r.kind);
        let open = self.open(i);
        let state = self.app_state(i);
        let busy = matches!(state, "running" | "solo" | "pr" | "queued");
        let (kids_total, kids_open, _) = self.children(i);
        let start = |can: Value| -> Value {
            if !item || busy {
                json!("hidden")
            } else if !open {
                json!({"disabled": format!("Closed in {}. Reopen it there to start it.", self.tracker_name())})
            } else {
                can
            }
        };
        let team = if self.can_team { json!("ok") } else { json!({"disabled": "This project has no team yet"}) };
        let program = if let Some(id) = self.apps[i].0["program"]["id"].as_str().filter(|_| state == "program_root") {
            json!({"open_program": id})
        } else if kids_total == 0 || busy {
            json!("hidden")
        } else if kids_open == 0 {
            json!({"disabled": "No open sub-issues"})
        } else {
            json!("ok")
        };
        json!({
            "team": start(team),
            "solo": start(json!("ok")),
            "program": program,
            // Offered on what is ready, and on what is hidden (to unhide it).
            "hide": if self.ready(i) || r.state == IntakeState::Hidden { json!("ok") } else { json!("hidden") },
        })
    }

    /// The contract's `WorkNode` (§2).
    pub(crate) fn node(&self, i: usize) -> Value {
        let r = &self.rows[i];
        let (total, open, done) = self.children(i);
        let mut up = upstream_json(&r.meta, r.meta["updated_at_ms"].as_u64().unwrap_or(0));
        up["deleted"] = json!(r.deleted);
        json!({
            "id": r.node_id, "project": r.project_id, "kind": r.kind, "ref": r.reference, "title": r.title,
            "url": r.url,
            "parent": r.parent_id.as_deref().filter(|p| self.by_id.contains_key(*p)),
            "depth": r.depth,
            "children": {"total": total, "open": open, "done": done},
            "team_tasks": self.team_tasks(i),
            "upstream": up,
            "app": self.apps[i].0,
            "rollup": self.rolls[i].json(),
            "ready": self.ready(i),
            "hidden": r.state == IntakeState::Hidden,
            "writeback": self.wb.get(&r.node_id).cloned().unwrap_or(Value::Null),
            "actions": self.actions(i),
        })
    }

    /// Whether node `i` passes the filter.
    fn passes(&self, i: usize, f: &WorkFilter) -> bool {
        let r = &self.rows[i];
        let open = self.open(i);
        match f.state.as_str() {
            "closed" if open => return false,
            "open" if !open => return false,
            _ => {}
        }
        if !is_item(&r.kind) {
            return !f.on_items();
        }
        let meta = &r.meta;
        let has_any = |key: &str, want: &[String]| {
            let have: Vec<String> =
                meta[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_lowercase)).collect();
            want.iter().any(|w| have.contains(&w.to_lowercase()))
        };
        let me = self.viewer.clone();
        let assignee_ok = f.assignees.is_empty()
            || f.assignees.iter().any(|a| {
                let a = if a == "@me" { me.clone().unwrap_or_default() } else { a.clone() };
                !a.is_empty() && has_any("assignees", &[a])
            });
        let is = |key: &str, want: &str| meta[key].as_str().is_some_and(|v| v.eq_ignore_ascii_case(want));
        (!f.only_ready || (self.ready(i) && (f.show_hidden || r.state != IntakeState::Hidden)))
            && (!f.only_not_started || self.app_state(i) == "none")
            && (f.labels.is_empty() || has_any("labels", &f.labels))
            && assignee_ok
            && f.milestone.as_ref().is_none_or(|m| is("milestone", m))
            && f.cycle.as_ref().is_none_or(|c| meta["cycle_kind"].as_str() == Some(c))
            && (f.issue_types.is_empty() || f.issue_types.iter().any(|t| is("issue_type", t)))
            && (f.linear_projects.is_empty()
                || f.linear_projects.iter().any(|p| meta["project_id"].as_str() == Some(p)))
    }

    /// Per node: matches the filter, or keeps one that does below it (an ancestor, shown muted).
    fn keep(&self, f: &WorkFilter) -> (Vec<bool>, Vec<bool>) {
        let pass: Vec<bool> = (0..self.rows.len()).map(|i| self.passes(i, f)).collect();
        let mut keep = pass.clone();
        // A node's keep flag climbs to its ancestors; a climb stops at one already climbed
        // (everything above it is kept), which also ends a loop in the parents.
        let mut climbed = vec![false; self.rows.len()];
        for (i, _) in pass.iter().enumerate().filter(|(_, p)| **p) {
            let mut at = i;
            while let Some(p) = self.rows[at].parent_id.as_deref().and_then(|p| self.by_id.get(p)).copied() {
                if p == at || climbed[p] {
                    break;
                }
                keep[p] = true;
                climbed[p] = true;
                at = p;
            }
        }
        (pass, keep)
    }

    /// The team tasks of node `i` as fold rows (notes §2.7).
    fn team_rows(&self, i: usize, rt: &Runtime) -> Vec<Value> {
        let r = &self.rows[i];
        let tracker = self.settings.tracker.kind();
        self.team
            .get(&i)
            .into_iter()
            .flatten()
            .map(|t| {
                let feature = rt
                    .db
                    .run(t.team_run_id.as_deref().unwrap_or(""))
                    .ok()
                    .flatten()
                    .and_then(|run| rt.db.feature(&run.feature_id).ok().flatten().map(|f| f.slug));
                json!({
                    "id": format!("task:{}", t.id), "project": r.project_id, "kind": "issue", "team_task": true,
                    "ref": t.handle(), "title": t.title, "url": null, "parent": r.node_id, "depth": r.depth + 1,
                    "task": t.id, "run": t.team_run_id, "feature": feature, "tracker": tracker,
                    "children": {"total": 0, "open": 0, "done": 0}, "team_tasks": 0,
                    "upstream": {"state": t.status.as_str(), "board_status": t.status.as_str(),
                                 "assignees": t.node.iter().collect::<Vec<_>>(), "labels": [],
                                 "blocked_by": [], "blocking": [], "linked_prs": []},
                    "app": {"state": "none"}, "rollup": null, "ready": false, "hidden": false, "writeback": null,
                    "actions": {"team": "hidden", "solo": "hidden", "program": "hidden", "hide": "hidden"},
                })
            })
            .collect()
    }
}

/// The ancestors of a node (root first, the project row left out) from the trees' shape.
pub(crate) fn crumbs(
    by_node: &HashMap<(&str, &str), &orch_store::WorkIndex>,
    project: &str,
    parent: Option<&str>,
) -> Vec<Value> {
    let (mut chain, mut up) = (vec![], parent);
    while let Some(n) = up.and_then(|p| by_node.get(&(project, p))) {
        if chain.len() > 50 {
            break;
        }
        chain.push(json!({"id": n.node_id, "ref": n.reference, "title": n.title}));
        up = n.parent_id.as_deref();
    }
    chain.reverse();
    chain
}

/// `run.source` of a start from `r` (contract §2, F23): where the run or session came from.
pub(crate) fn source_json(r: &IntakeRow, breadcrumb: Vec<Value>, t: &TrackerSettings) -> Value {
    let write_back = t.write_back.enabled && t.tracker.kind() != "local";
    json!({
        "node": r.node_id, "ref": r.reference, "title": r.title, "url": r.url, "breadcrumb": breadcrumb,
        "tracker": t.tracker.kind(), "program": null, "deleted": r.deleted, "closed_upstream": !is_open(&r.meta),
        "write_back": if write_back { "on" } else { "off" }, "writeback": null,
    })
}

pub(crate) fn viewer_key(project: &str) -> String {
    format!("work.viewer:{project}")
}

fn page(list: &[usize], after: Option<&str>, n: usize) -> (Vec<usize>, Option<(usize, String)>) {
    let start = after.and_then(|a| a.parse::<usize>().ok()).unwrap_or(0).min(list.len());
    let end = (start + n).min(list.len());
    let more = (end < list.len()).then(|| (list.len() - end, end.to_string()));
    (list[start..end].to_vec(), more)
}

/// ≤ [`SNIPPET`] chars of `body` as plain text around the first hit of `needle`.
fn snippet(body: &str, needle: &str) -> Option<String> {
    let text: Vec<char> = body
        .replace("**", "")
        .replace(['`', '#', '>'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let lower: Vec<char> = text.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let n: Vec<char> = needle.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let at = (0..lower.len().saturating_sub(n.len().saturating_sub(1))).find(|i| lower[*i..*i + n.len()] == n[..])?;
    let from = at.saturating_sub((SNIPPET.saturating_sub(n.len())) / 2);
    let to = (from + SNIPPET).min(text.len());
    let cut: String = text[from..to].iter().collect();
    Some(format!("{}{}{}", if from > 0 { "…" } else { "" }, cut.trim(), if to < text.len() { "…" } else { "" }))
}

fn history_json(e: &orch_store::WorkEvent) -> Value {
    json!({"at_ms": e.at_ms, "what": e.what, "feature": e.feature, "program": e.program, "detail": e.detail})
}

impl Runtime {
    /// The projects a scope names (all of them when it names none).
    fn work_projects(&self, scope: &Value) -> Result<Vec<Project>> {
        let ids: Vec<String> =
            scope["projects"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        let all = self.db.projects(false)?;
        Ok(if ids.is_empty() { all } else { all.into_iter().filter(|p| ids.contains(&p.id)).collect() })
    }

    fn work_ctxs_with_errors(&self, scope: &Value) -> Result<(Vec<Ctx>, Failed)> {
        let (mut ok, mut failed) = (vec![], vec![]);
        for p in self.work_projects(scope)? {
            match Ctx::new(self, &p) {
                Ok(c) => ok.push(c),
                Err(e) => failed.push((p.id, format!("{e:#}"))),
            }
        }
        Ok((ok, failed))
    }

    /// The status of each project of a read (an unreadable one is `error`, never a failure of the call).
    fn work_statuses(
        &self,
        ctxs: &[Ctx],
        failed: Failed,
        inflight: &std::collections::BTreeSet<String>,
    ) -> serde_json::Map<String, Value> {
        let mut status: serde_json::Map<String, Value> =
            ctxs.iter().map(|c| (c.project.id.clone(), self.work_status(c, inflight))).collect();
        for (id, why) in failed {
            status.insert(
                id,
                json!({"status": "error", "error": {"code": "failed", "message": why}, "warning": null,
                       "offline": false, "refreshed_at_ms": null, "counts": {"items": 0, "roots": 0, "ready": 0}}),
            );
        }
        status
    }

    /// The project's source status (contract §2): refreshing / error / warning / offline, and what the cache holds.
    fn work_status(&self, ctx: &Ctx, inflight: &std::collections::BTreeSet<String>) -> Value {
        let st = self.db.intake_status(&ctx.project.id).unwrap_or_default();
        let roots = ctx.kids.get(&None).map_or(0, Vec::len);
        let ready = (0..ctx.rows.len()).filter(|i| ctx.ready(*i) && ctx.rows[*i].state == IntakeState::Ready).count();
        let state = if inflight.contains(&ctx.project.id) {
            "refreshing"
        } else if st.error.is_some() {
            "error"
        } else {
            "ok"
        };
        json!({"status": state, "error": st.error, "warning": st.warning, "offline": st.offline,
               "refreshed_at_ms": st.refreshed_at_ms,
               "counts": {"items": ctx.rows.len(), "roots": roots, "ready": ready}})
    }

    fn find_node(&self, ctxs: &[Ctx], id: &str) -> Option<(usize, usize)> {
        ctxs.iter().enumerate().find_map(|(c, ctx)| ctx.index(id).map(|i| (c, i)))
    }

    /// The tree of the one project that holds node `id`, and the node's index in it
    /// (not every project's tree: one read per call).
    fn work_ctx_of(&self, id: &str) -> Result<(Ctx, usize)> {
        let holders = self.db.work_node_rows(id)?;
        let project = self
            .work_projects(&json!({}))?
            .into_iter()
            .find(|p| holders.iter().any(|r| r.project_id == p.id))
            .ok_or_else(|| anyhow!(not_found(id)))?;
        let ctx = Ctx::new(self, &project)?;
        let i = ctx.index(id).ok_or_else(|| anyhow!(not_found(id)))?;
        Ok((ctx, i))
    }

    /// Fetches the sub-issues of node `i` when the cache lacks some (once per revision of the parent).
    /// `Some(error)` when the fetch failed; the cache is rebuilt after a fetch, so a node's
    /// index is asked again (`Ctx::index`) afterwards: the rows are in another order.
    fn work_fill_children(&self, ctx: &mut Ctx, i: usize) -> Option<Value> {
        if !ctx.missing_children(i) {
            return None;
        }
        let r = ctx.rows[i].clone();
        let marker = format!("work.children:{}:{}", r.project_id, r.node_id);
        if self.db.setting(&marker).ok().flatten().and_then(|v| v.as_str().map(str::to_string)) == Some(r.rev.clone()) {
            return None;
        }
        let p = ctx.project.clone();
        let now = orch_protocol::now_ms();
        let got: Result<Vec<src::Node>, src::SourceError> = match &ctx.settings.tracker {
            Tracker::Github { project_board, .. } => {
                let number = r.meta["number"].as_u64().unwrap_or(0);
                let repo =
                    r.node_id.strip_prefix("gh:").and_then(|s| s.rsplit_once('#')).map(|(r, _)| r.to_string())?;
                let o = src::GithubTree {
                    repo: &repo,
                    board: project_board.as_ref().map(|b| (b.owner.as_str(), b.number)),
                    group_by_milestone: ctx.settings.group_by_milestone,
                    since: None,
                    now_ms: now,
                };
                src::github_children(self.gh_in(&p).as_ref(), &o, number)
            }
            Tracker::Linear { .. } => {
                let gql = self.linear_transport();
                src::linear_children(gql.as_ref(), &self.intake.backoff(&p.id), now, &r.reference)
            }
            Tracker::Local { .. } => return None,
        };
        match got {
            Ok(nodes) => {
                let rows: Vec<_> = nodes.iter().map(src::Node::upsert).collect();
                let _ = self.db.intake_apply(&p.id, super::tracker_source(&ctx.settings), rows, false, now);
                let _ = self.db.set_setting(&marker, &json!(r.rev));
                if let Ok(fresh) = Ctx::new(self, &p) {
                    *ctx = fresh;
                }
                None
            }
            Err(e) => Some(e.json()),
        }
    }

    /// `work_tree`: the roots or the children of `parents`, from the cache.
    pub fn work_tree(&self, scope: &Value, parents: &Value, filter: &Value, after: Option<&str>) -> Result<Value> {
        let f = WorkFilter::parse(filter)?;
        let (mut ctxs, failed) = self.work_ctxs_with_errors(scope)?;
        let inflight = super::lock(&self.intake.sched).inflight.clone();
        let (mut nodes, mut more) = (vec![], serde_json::Map::new());
        let parent_ids: Option<Vec<String>> =
            parents.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
        match parent_ids {
            None => {
                // The roots of every project in scope, paged together.
                let keeps: Vec<(Vec<bool>, Vec<bool>)> = ctxs.iter().map(|c| c.keep(&f)).collect();
                let list: Vec<(usize, usize)> = ctxs
                    .iter()
                    .enumerate()
                    .flat_map(|(c, ctx)| {
                        ctx.kids
                            .get(&None)
                            .into_iter()
                            .flatten()
                            .filter(|i| keeps[c].1[**i])
                            .map(move |i| (c, *i))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let idx: Vec<usize> = (0..list.len()).collect();
                let (shown, left) = page(&idx, after, ROOTS_PAGE);
                for k in shown {
                    let (c, i) = list[k];
                    let mut n = ctxs[c].node(i);
                    if !keeps[c].0[i] {
                        n["ancestor_only"] = json!(true);
                    }
                    nodes.push(n);
                }
                if let Some((n, cursor)) = left {
                    more.insert("roots".into(), json!({"left": n, "cursor": cursor}));
                }
            }
            Some(ids) => {
                for id in &ids {
                    let Some((c, i)) = self.find_node(&ctxs, id) else { continue };
                    let err = self.work_fill_children(&mut ctxs[c], i);
                    let ctx = &ctxs[c];
                    let i = ctx.index(id).unwrap_or(i);
                    let (pass, keep) = ctx.keep(&f);
                    let kids: Vec<usize> =
                        ctx.kids.get(&Some(i)).into_iter().flatten().copied().filter(|k| keep[*k]).collect();
                    let cursor = (ids.len() == 1).then_some(after).flatten();
                    let (shown, left) = page(&kids, cursor, CHILDREN_PAGE);
                    for j in shown {
                        let mut n = ctx.node(j);
                        if !pass[j] {
                            n["ancestor_only"] = json!(true);
                        }
                        nodes.push(n);
                    }
                    if f.include_team_tasks && left.is_none() {
                        nodes.extend(ctx.team_rows(i, self));
                    }
                    let mut m = serde_json::Map::new();
                    if let Some((n, cursor)) = left {
                        m.insert("left".into(), json!(n));
                        m.insert("cursor".into(), json!(cursor));
                    }
                    if let Some(e) = err {
                        m.insert("error".into(), e);
                    }
                    if !m.is_empty() {
                        more.insert(id.clone(), Value::Object(m));
                    }
                }
            }
        }
        let status = self.work_statuses(&ctxs, failed, &inflight);
        Ok(json!({"nodes": nodes, "more": more, "status": status}))
    }

    /// `work_search`: ref exact first, then title, then body; ≤ 100.
    pub fn work_search(&self, scope: &Value, query: &str, filter: &Value) -> Result<Value> {
        let f = WorkFilter::parse(filter)?;
        let q = query.trim().to_lowercase();
        let (ctxs, failed) = self.work_ctxs_with_errors(scope)?;
        let inflight = super::lock(&self.intake.sched).inflight.clone();
        let status = self.work_statuses(&ctxs, failed, &inflight);
        if q.is_empty() {
            return Ok(json!({"nodes": [], "total": 0, "hidden_by_filters": 0, "status": status}));
        }
        let root = scope["root"].as_str();
        let bare = |s: &str| s.trim_start_matches('#').to_lowercase();
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u8, usize, usize, &'static str, Option<String>)> = vec![];
        let mut hidden = 0;
        for (c, ctx) in ctxs.iter().enumerate() {
            let under: Option<HashSet<usize>> = root.and_then(|r| ctx.index(r)).map(|r| {
                let (mut seen, mut stack) = (HashSet::new(), vec![r]);
                while let Some(n) = stack.pop() {
                    if seen.insert(n) {
                        stack.extend(ctx.kids.get(&Some(n)).into_iter().flatten().copied());
                    }
                }
                seen
            });
            if root.is_some() && under.is_none() {
                continue;
            }
            let bodies: HashMap<String, String> =
                self.db.work_nodes(Some(&ctx.project.id))?.into_iter().map(|r| (r.node_id, r.body)).collect();
            for i in 0..ctx.rows.len() {
                if under.as_ref().is_some_and(|u| !u.contains(&i)) {
                    continue;
                }
                let r = &ctx.rows[i];
                let title = r.title.to_lowercase();
                let hit: Option<(u8, &'static str, Option<String>)> = if bare(&r.reference) == bare(&q) {
                    Some((0, "ref", Some(r.reference.clone())))
                } else if r.reference.to_lowercase().starts_with(&q) {
                    Some((1, "ref", Some(r.reference.clone())))
                } else if tokens.iter().all(|t| title.contains(t)) {
                    Some((2, "title", Some(r.title.clone())))
                } else {
                    let body = bodies.get(&r.node_id).map_or("", String::as_str);
                    let low = body.to_lowercase();
                    tokens.iter().all(|t| low.contains(t)).then(|| (3, "body", snippet(body, tokens[0])))
                };
                let Some((rank, field, snip)) = hit else { continue };
                if !ctx.passes(i, &f) {
                    hidden += 1;
                    continue;
                }
                hits.push((rank, c, i, field, snip));
            }
        }
        hits.sort_by_key(|h| (h.0, h.1, h.2));
        let total = hits.len();
        let nodes: Vec<Value> = hits
            .into_iter()
            .take(SEARCH_MAX)
            .map(|(_, c, i, field, snip)| {
                let mut n = ctxs[c].node(i);
                n["breadcrumb"] = json!(ctxs[c].breadcrumb(i));
                n["match"] = json!({"field": field, "snippet": snip.unwrap_or_default()});
                n
            })
            .collect();
        Ok(json!({"nodes": nodes, "total": total, "hidden_by_filters": hidden, "status": status}))
    }

    /// `work_node`: the node with its breadcrumb, body, history and estimate; `children`: the first page.
    pub fn work_node(&self, id: &str, opts: &Value) -> Result<Value> {
        let (mut ctx, i) = self.work_ctx_of(id)?;
        let mut err = None;
        if opts["children"].as_bool() == Some(true) {
            err = self.work_fill_children(&mut ctx, i);
        }
        let ctx = &ctx;
        let i = ctx.index(id).unwrap_or(i);
        let full = self.db.work_node_rows(id)?.into_iter().find(|r| r.project_id == ctx.project.id);
        let mut n = ctx.node(i);
        n["breadcrumb"] = json!(ctx.breadcrumb(i));
        n["body"] = json!(full.as_ref().map_or("", |r| r.body.as_str()));
        n["body_clipped"] = json!(ctx.rows[i].meta["truncated"].as_bool() == Some(true));
        n["history"] = json!(self.db.work_history(&ctx.project.id, id)?.iter().map(history_json).collect::<Vec<_>>());
        if is_item(&ctx.rows[i].kind) {
            n["estimate"] = self.intake_estimate_of(&ctx.rows[i]);
        }
        if opts["children"].as_bool() == Some(true) {
            let kids: Vec<usize> = ctx.kids.get(&Some(i)).cloned().unwrap_or_default();
            n["child_nodes"] = json!(kids.iter().take(CHILDREN_PAGE).map(|k| ctx.node(*k)).collect::<Vec<_>>());
            n["children_more"] = json!(kids.len().saturating_sub(CHILDREN_PAGE));
            if let Some(e) = err {
                n["children_error"] = e;
            }
        }
        Ok(n)
    }

    /// `work_reveal`: the chain root → node and the ancestors' siblings, in one answer.
    pub fn work_reveal(&self, id: &str) -> Result<Value> {
        let (ctx, i) = self.work_ctx_of(id)?;
        let ctx = &ctx;
        let mut chain = vec![i];
        while let Some(p) =
            ctx.rows[*chain.last().unwrap()].parent_id.as_deref().and_then(|p| ctx.by_id.get(p)).copied()
        {
            if chain.len() > 50 || chain.contains(&p) {
                break;
            }
            chain.push(p);
        }
        chain.reverse();
        let mut shown: Vec<usize> = vec![];
        for at in &chain {
            let parent = ctx.rows[*at].parent_id.as_deref().and_then(|p| ctx.by_id.get(p)).copied();
            let sibs = ctx.kids.get(&parent).cloned().unwrap_or_default();
            // The page that holds the chain's node, so it is always in the answer.
            let pos = sibs.iter().position(|s| s == at).unwrap_or(0);
            let from = (pos / REVEAL_SIBLINGS) * REVEAL_SIBLINGS;
            for s in sibs.into_iter().skip(from).take(REVEAL_SIBLINGS) {
                if !shown.contains(&s) {
                    shown.push(s);
                }
            }
        }
        Ok(json!({"chain": chain.iter().map(|i| ctx.rows[*i].node_id.clone()).collect::<Vec<_>>(),
                  "nodes": shown.iter().map(|i| ctx.node(*i)).collect::<Vec<_>>()}))
    }

    /// `work_refresh`: asks for a full refresh and returns at once; a refresh in flight is joined, not repeated.
    pub fn work_refresh(&self, project: Option<&str>) -> Result<Value> {
        if let Some(p) = project {
            self.intake_project(p)?;
        }
        self.intake_request(project);
        Ok(json!({}))
    }

    fn work_row(&self, id: &str, project: Option<&str>) -> Result<IntakeRow> {
        self.db
            .work_node_rows(id)?
            .into_iter()
            .find(|r| project.is_none_or(|p| r.project_id == p))
            .ok_or_else(|| anyhow!(not_found(id)))
    }

    /// `work_hide`: the node leaves Ready until it changes upstream (it stays in the tree).
    pub fn work_hide(&self, id: &str) -> Result<Value> {
        let r = self.work_row(id, None)?;
        self.db.intake_hide(r.id, orch_protocol::now_ms())?;
        Ok(json!({}))
    }

    pub fn work_unhide(&self, id: &str) -> Result<Value> {
        let r = self.work_row(id, None)?;
        self.db.intake_unhide(r.id)?;
        Ok(json!({}))
    }

    /// `work_prefill(id, mode)`: the launch or session form of node `id` (as `intake_prefill`) and
    /// what the dialog adds (notes §5.2): origin, sub-issue scope, blockers, earlier run, write-back line.
    pub fn work_prefill(&self, id: &str, mode: Option<&str>) -> Result<Value> {
        let row = self.work_row(id, None)?;
        let row = self.startable_row(&row)?;
        let p = self.intake_project(&row.project_id)?;
        let ctx = Ctx::new(self, &p)?;
        let i = ctx.index(&row.node_id).ok_or_else(|| anyhow!(not_found(id)))?;
        let full = self.db.work_node_rows(id)?.into_iter().find(|r| r.project_id == p.id).unwrap_or(row.clone());
        let facts = self.project_facts(&p);
        let suggested = super::suggested_mode(&full.meta, &facts).to_string();
        let kids: Vec<usize> = ctx.kids.get(&Some(i)).cloned().unwrap_or_default();
        let open_kids = kids.iter().filter(|k| ctx.open(**k)).count();
        let deps = |key: &str| -> Vec<Value> {
            full.meta[key].as_array().into_iter().flatten().filter(|d| d["open"] == true).cloned().collect()
        };
        let blocked_open = deps("blocked_by");
        let text = work_brief(&ctx, i, &full, &kids, &p, self);
        let url = full.url.clone().unwrap_or_else(|| full.reference.clone());
        let mut out =
            self.prefill_forms(&p, &full.title, &text, &suggested, full.meta["team"].as_str(), None, Some(url), mode)?;
        let ran_before = match &ctx.apps[i] {
            (v, Class::Merged) => json!({"feature": v["feature"], "merged_at_ms": v["merged_at_ms"]}),
            _ => Value::Null,
        };
        out["origin"] = json!({"id": full.node_id, "ref": full.reference, "title": full.title, "url": full.url,
                               "breadcrumb": ctx.breadcrumb(i), "tracker": ctx.settings.tracker.kind()});
        out["open_sub_issues"] = json!(open_kids);
        out["blocked_by_open"] = json!(blocked_open);
        out["ran_before"] = ran_before;
        out["estimate"] = self.intake_estimate_of(&full);
        out["write_back_line"] = json!(write_back_line(&ctx.settings, &full.reference));
        Ok(out)
    }

    /// `brief_prefill(decision, mode)`: the form of a planned brief (no origin).
    pub fn brief_prefill(&self, decision: &str, mode: Option<&str>) -> Result<Value> {
        self.intake_prefill(None, Some(decision), mode)
    }

    /// Resolves who `@me` is for the project's tracker (the GitHub login, the Linear user name), once.
    pub(super) fn resolve_viewer(&self, p: &Project, t: &TrackerSettings) {
        if self.db.setting(&viewer_key(&p.id)).ok().flatten().is_some() {
            return;
        }
        let me = match &t.tracker {
            Tracker::Github { .. } => self
                .gh_in(p)
                .run(&["api".to_string(), "user".to_string()])
                .ok()
                .and_then(|o| serde_json::from_str::<Value>(&o).ok())
                .and_then(|v| v["login"].as_str().map(str::to_string)),
            Tracker::Linear { .. } => {
                use orch_tasks::linear::Graphql;
                self.linear_transport()
                    .call("query OrchViewer { viewer { name } }", &json!({}))
                    .ok()
                    .and_then(|v| v["viewer"]["name"].as_str().map(str::to_string))
            }
            Tracker::Local { .. } => None,
        };
        if let Some(me) = me {
            let _ = self.db.set_setting(&viewer_key(&p.id), &json!(me));
        }
    }

    /// History of a node from run events (spec approved, PR opened, merged).
    pub(crate) fn work_on_run_event(&self, run: &str, kind: &str, detail: &Value) {
        let what = match kind {
            "run.pr_opened" => "pr_opened",
            "run.state" if detail["to"] == "done" => "merged",
            "run.state" if detail["from"] == "spec_gate" && detail["to"] == "planning" => "spec_approved",
            _ => return,
        };
        if let Ok(Some(r)) = self.db.work_node_of_run(run) {
            self.work_log(&r, what, None, None);
        }
        if what == "merged" {
            // Every item of the run is closed upstream when GitHub / Linear did not (write-back), and a
            // merge may free a program's next feature.
            for r in self.db.work_nodes_of("run", run).unwrap_or_default() {
                self.wb_spawn(&r.project_id, &r.node_id, crate::writeback::WbOp::Merged);
            }
            self.programs_tick();
        }
    }

    /// An entry in the history of `r`: `feature` is the slug when known.
    pub(crate) fn work_log(&self, r: &IntakeRow, what: &str, feature: Option<&str>, detail: Option<&str>) {
        let e = orch_store::WorkEvent {
            at_ms: orch_protocol::now_ms(),
            what: what.to_string(),
            feature: feature.map(str::to_string),
            program: None,
            detail: detail.map(str::to_string),
        };
        if let Err(e) = self.db.work_note(&r.project_id, &r.node_id, &e) {
            eprintln!("warn: history of {} not written: {e:#}", r.node_id);
        }
    }
}

fn not_found(id: &str) -> String {
    json!({"code": "not_found", "message": format!("no node {id:?}")}).to_string()
}

/// Notes §10.4: what a start does in the tracker, in one sentence.
fn write_back_line(t: &TrackerSettings, reference: &str) -> Option<String> {
    if !t.write_back.enabled {
        return None;
    }
    let me = if t.write_back.assign_me { " It is assigned to you." } else { "" };
    match &t.tracker {
        Tracker::Github { project_board: Some(_), .. } => {
            let s = t.write_back.board_status.as_deref().unwrap_or("In progress");
            Some(format!("{reference} moves to {s} on the board and gets a comment in GitHub.{me}"))
        }
        Tracker::Github { .. } => {
            Some(format!("{reference} gets the label {} and a comment in GitHub.{me}", t.write_back.running_label))
        }
        Tracker::Linear { .. } => {
            Some(format!("{reference} moves to a started state and gets a comment in Linear.{me}"))
        }
        Tracker::Local { .. } => None,
    }
}

/// The brief of a start from a node (notes §5.2): where it is, its text, the open
/// sub-issues as the scope and what it depends on. A backlog file is the file's text.
pub(crate) fn work_brief(ctx: &Ctx, i: usize, r: &IntakeRow, kids: &[usize], p: &Project, rt: &Runtime) -> String {
    if r.kind == "local" {
        return super::start::item_brief(r);
    }
    let place = match &ctx.settings.tracker {
        Tracker::Github { repo, .. } => repo
            .clone()
            .or_else(|| rt.intake_repo_known(p, &ctx.settings.intake_source()))
            .unwrap_or_else(|| "GitHub".into()),
        Tracker::Linear { teams, .. } => format!("Linear {}", teams.first().cloned().unwrap_or_default()),
        Tracker::Local { .. } => String::new(),
    };
    let crumbs: Vec<String> = ctx
        .breadcrumb(i)
        .iter()
        .map(|c| {
            format!("{} {}", c["ref"].as_str().unwrap_or(""), c["title"].as_str().unwrap_or("")).trim().to_string()
        })
        .collect();
    let where_ = std::iter::once(place).chain(crumbs).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" › ");
    let url = r.url.as_deref().map(|u| format!(" {u}")).unwrap_or_default();
    let cut = if r.meta["truncated"].as_bool() == Some(true) { "\n\n(cut: read the source for the rest)" } else { "" };
    let mut out = format!("# {} ({})\n\nIn {where_}.{url}\n\n{}{cut}", r.title.trim(), r.reference, r.body.trim());
    if !kids.is_empty() {
        out.push_str("\n\n## Scope: sub-issues\n");
        for k in kids.iter().take(50) {
            let c = &ctx.rows[*k];
            let ty = c.meta["issue_type"].as_str().map(|t| format!(" ({t})")).unwrap_or_default();
            if ctx.open(*k) {
                out.push_str(&format!("- [ ] {} {}{ty}\n", c.reference, c.title));
            } else {
                out.push_str(&format!("- [x] {} {} (closed)\n", c.reference, c.title));
            }
        }
    }
    out.push_str("\n## Depends on\n");
    let deps: Vec<&Value> = r.meta["blocked_by"].as_array().into_iter().flatten().collect();
    if deps.is_empty() {
        out.push_str("- none\n");
    }
    for d in deps {
        let state = if d["open"] == true { "open" } else { "closed" };
        out.push_str(&format!(
            "- {} {} ({state})\n",
            d["ref"].as_str().unwrap_or(""),
            d["title"].as_str().unwrap_or("")
        ));
    }
    out.trim_end().to_string()
}
