//! implement-improve-intake B4: programs (contract §4, notes §6–§7). A program
//! runs the open work under a tracker node (or several siblings) as several
//! features, each started when the features it depends on have merged,
//! within `max_parallel`, under the Governor. This file: the document
//! ([`Program`]), the planner (`program_plan`), the start check
//! (`program_start`, which validates the user's draft and never re-plans over
//! it), the edits (`program_update`) and the reads; [`engine`] runs it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Result, anyhow};
use orch_store::IntakeRow;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::intake::start::{kebab, unique};
use crate::intake::work::{Ctx, is_item, work_brief};
use crate::runtime::Runtime;

pub mod engine;

/// The most features a program runs at once.
pub const MAX_PARALLEL: u32 = 5;
pub const DEFAULT_PARALLEL: u32 = 2;

/// One feature of a program (contract §4 `ProgramFeature`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub key: String,
    /// WorkNode ids; the first is the feature's source node.
    pub items: Vec<String>,
    pub slug: String,
    pub title: String,
    /// `team` | `solo`.
    pub mode: String,
    pub team: Option<String>,
    pub brief: String,
    pub base: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub skip: bool,
    #[serde(default)]
    pub estimate: Value,
    /// planned | waiting | queued | running | pr | merged | skipped | failed.
    pub state: String,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub pr: Option<Value>,
    #[serde(default)]
    pub merged_at_ms: Option<u64>,
    #[serde(default)]
    pub actual_pp: Option<f64>,
    /// `closed_upstream` | `new_dependency` | `deleted_upstream`.
    #[serde(default)]
    pub flags: Vec<String>,
    /// Why it failed to start or run.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// `tracker` | `sequential` | `user`.
    pub source: String,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LeftOut {
    #[serde(default)]
    pub closed: Vec<String>,
    #[serde(default)]
    pub running: Vec<Value>,
}

/// The document of a program (contract §4 `Program`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub id: String,
    pub project: String,
    pub title: String,
    pub root: Vec<String>,
    pub root_ref: String,
    /// draft | running | paused | done | cancelled.
    pub status: String,
    /// child | grandchild | single.
    pub grouping: String,
    pub max_parallel: u32,
    pub sequential: bool,
    pub features: Vec<Feature>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Edges the user removed, `[from, to]`; kept on a re-plan (F10).
    #[serde(default)]
    pub cut: Vec<[String; 2]>,
    /// Children added upstream since the plan (F7), not in any feature.
    #[serde(default)]
    pub new_items: Vec<String>,
    /// Upstream items the user chose to ignore.
    #[serde(default)]
    pub ignored: Vec<String>,
    /// Closed items the user chose to "Include anyway": plan and start keep them in their feature.
    #[serde(default)]
    pub include: Vec<String>,
    /// When each of `new_items` was seen upstream, by node id (ms).
    #[serde(default)]
    pub new_items_added: BTreeMap<String, u64>,
    #[serde(default)]
    pub estimate: Value,
    #[serde(default)]
    pub left_out: LeftOut,
    /// Decisions already posted (`failed:pf-2`, `done`…), so each is posted once.
    #[serde(default)]
    pub notified: Vec<String>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub done_at_ms: Option<u64>,
}

/// What stops or colours a plan (contract §4 `PlanIssue`, F8).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlanIssue {
    /// cycle | already_running | closed | no_team | over_room
    pub kind: &'static str,
    pub blocking: bool,
    pub features: Vec<String>,
    pub items: Vec<String>,
    pub feature: Option<String>,
    pub edges: Vec<[String; 2]>,
    pub message: String,
}

impl PlanIssue {
    fn new(kind: &'static str, blocking: bool, message: String) -> PlanIssue {
        PlanIssue { kind, blocking, features: vec![], items: vec![], feature: None, edges: vec![], message }
    }
}

fn bad(field: &str, message: impl Into<String>) -> anyhow::Error {
    crate::intake::bad_request(field, message)
}

fn stale(message: &str, extra: Value) -> anyhow::Error {
    let mut v = json!({"code": "stale", "message": message});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    anyhow!(v.to_string())
}

fn new_id(now_ms: u64) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("prg-{now_ms:x}{:x}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 4096)
}

/// The open items below node `i` (itself first) that nothing runs yet; closed and running ones go to `out`.
fn open_scope(ctx: &Ctx, i: usize, out: &mut Scope, seen: &mut HashSet<usize>) -> Vec<usize> {
    let mut found = vec![];
    let mut stack = vec![i];
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        let row = &ctx.rows[n];
        if !ctx.open(n) && !out.include.contains(&row.node_id) {
            out.closed.push(row.node_id.clone());
            continue;
        }
        let state = ctx.app_state(n);
        if matches!(state, "running" | "solo" | "pr" | "queued") {
            out.running.push(json!({"id": row.node_id, "feature": ctx.app_feature(n), "wait": ctx.app_wait_key(n)}));
            continue;
        }
        if state == "done" {
            out.closed.push(row.node_id.clone());
            continue;
        }
        if is_item(&row.kind) {
            found.push(n);
        }
        // Children in tree order (the stack pops last first).
        stack.extend(ctx.kids_of(n).iter().rev());
    }
    found
}

#[derive(Default)]
struct Scope {
    closed: Vec<String>,
    running: Vec<Value>,
    /// Closed items kept in anyway (`Program.include`).
    include: HashSet<String>,
}

/// The items of each feature under `roots` for `grouping` (first item of a group leads).
fn groups(ctx: &Ctx, roots: &[usize], grouping: &str, scope: &mut Scope) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![];
    let mut seen = HashSet::new();
    // Level 1: the children of an item root; for a container, its issues (through nested containers).
    let level1 = |root: usize| -> Vec<usize> {
        let mut v = vec![];
        let mut stack: Vec<usize> = ctx.kids_of(root).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if is_item(&ctx.rows[n].kind) {
                v.push(n);
            } else {
                stack.extend(ctx.kids_of(n).iter().rev());
            }
        }
        v
    };
    let mut single: Vec<usize> = vec![];
    for &root in roots {
        let l1 = level1(root);
        if l1.is_empty() {
            // A leaf, or a container without issues: the node itself is the work.
            let mut s = HashSet::new();
            let items = open_scope(ctx, root, scope, &mut s);
            if grouping == "single" {
                single.extend(items);
            } else if !items.is_empty() {
                out.push(items);
            }
            continue;
        }
        for n in l1 {
            let targets: Vec<usize> = match grouping {
                "grandchild" => {
                    let kids: Vec<usize> =
                        ctx.kids_of(n).iter().copied().filter(|k| is_item(&ctx.rows[*k].kind)).collect();
                    if kids.is_empty() { vec![n] } else { kids }
                }
                _ => vec![n],
            };
            for t in targets {
                let items = open_scope(ctx, t, scope, &mut seen);
                match grouping {
                    "single" => single.extend(items),
                    _ if !items.is_empty() => out.push(items),
                    _ => {}
                }
            }
        }
    }
    if grouping == "single" && !single.is_empty() {
        out = vec![single];
    }
    out
}

/// One cycle among the non-skipped features' edges, as the edges that close it.
fn find_cycle(features: &[Feature], edges: &[Edge]) -> Option<Vec<[String; 2]>> {
    let live: HashSet<&str> = features.iter().filter(|f| !f.skip).map(|f| f.key.as_str()).collect();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in edges.iter().filter(|e| live.contains(e.from.as_str()) && live.contains(e.to.as_str())) {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    // 0 unseen, 1 on the path, 2 done.
    let mut mark: HashMap<&str, u8> = HashMap::new();
    fn dfs<'a>(
        n: &'a str,
        adj: &BTreeMap<&'a str, Vec<&'a str>>,
        mark: &mut HashMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<[String; 2]>> {
        mark.insert(n, 1);
        path.push(n);
        for &m in adj.get(n).into_iter().flatten() {
            match mark.get(m).copied().unwrap_or(0) {
                1 => {
                    let at = path.iter().position(|p| *p == m).unwrap_or(0);
                    let cyc: Vec<&str> = path[at..].to_vec();
                    return Some(
                        cyc.iter()
                            .enumerate()
                            .map(|(i, a)| [a.to_string(), cyc[(i + 1) % cyc.len()].to_string()])
                            .collect(),
                    );
                }
                0 => {
                    if let Some(c) = dfs(m, adj, mark, path) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }
        path.pop();
        mark.insert(n, 2);
        None
    }
    for f in features.iter().filter(|f| !f.skip) {
        if mark.get(f.key.as_str()).copied().unwrap_or(0) == 0
            && let Some(c) = dfs(f.key.as_str(), &adj, &mut mark, &mut vec![])
        {
            return Some(c);
        }
    }
    None
}

/// `to` waits for `from`: from the trackers' dependencies between the features' items.
fn tracker_edges(ctx: &Ctx, features: &[Feature], running: &[Value]) -> Vec<Edge> {
    // Items that run outside the program (as another feature or a session): `x:run:<id>` / `x:agent:<id>`.
    let ext: HashMap<&str, &str> =
        running.iter().filter_map(|r| Some((r["id"].as_str()?, r["wait"].as_str()?))).collect();
    let owner: HashMap<&str, &str> =
        features.iter().flat_map(|f| f.items.iter().map(move |i| (i.as_str(), f.key.as_str()))).collect();
    let mut by_pair: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for f in features {
        for item in &f.items {
            let Some(i) = ctx.index(item) else { continue };
            let meta = &ctx.rows[i].meta;
            // `item` is blocked by `d`: d's feature goes first.
            for d in meta["blocked_by"].as_array().into_iter().flatten() {
                // Blocked by an item that is already running: wait for its merge (an external dependency).
                if let Some(id) = d["id"].as_str()
                    && let Some(x) = ext.get(id)
                    && d["open"] == true
                {
                    let e = by_pair.entry((x.to_string(), f.key.clone())).or_default();
                    e.extend([id.to_string(), item.clone()]);
                }
                if let Some(id) = d["id"].as_str()
                    && let Some(from) = owner.get(id)
                    && *from != f.key
                {
                    let e = by_pair.entry((from.to_string(), f.key.clone())).or_default();
                    e.extend([id.to_string(), item.clone()]);
                }
            }
            // `item` blocks `b`: item's feature goes first.
            for b in meta["blocking"].as_array().into_iter().flatten() {
                if let Some(id) = b["id"].as_str()
                    && let Some(to) = owner.get(id)
                    && *to != f.key
                {
                    let e = by_pair.entry((f.key.clone(), to.to_string())).or_default();
                    e.extend([item.clone(), id.to_string()]);
                }
            }
        }
    }
    by_pair
        .into_iter()
        .map(|((from, to), mut items)| {
            let mut seen = HashSet::new();
            items.retain(|i| seen.insert(i.clone()));
            let source = if from.starts_with("x:") { "external" } else { "tracker" };
            Edge { from, to, source: source.into(), items }
        })
        .collect()
}

/// Sets `depends_on` from the edges (skipped features neither wait nor are waited for).
fn set_depends(p: &mut Program) {
    let skipped: HashSet<String> = p.features.iter().filter(|f| f.skip).map(|f| f.key.clone()).collect();
    let cut: HashSet<(String, String)> = p.cut.iter().map(|c| (c[0].clone(), c[1].clone())).collect();
    for f in &mut p.features {
        f.depends_on = p
            .edges
            .iter()
            .filter(|e| e.to == f.key && !skipped.contains(&e.from) && !cut.contains(&(e.from.clone(), e.to.clone())))
            .map(|e| e.from.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
}

impl Runtime {
    /// The estimate of one feature (P4.5 estimator over its items) and the totals.
    fn feature_estimate(&self, rows: &[&IntakeRow], mode: &str) -> Value {
        let est = self.intake_est();
        let (mut low, mut high, mut basis) = (0.0, 0.0, String::new());
        for r in rows {
            let e = crate::intake::estimate::estimate(&est, &r.project_id, r.meta["size"].as_str(), mode);
            low += e[mode]["low_pp"].as_f64().unwrap_or(0.0);
            high += e[mode]["high_pp"].as_f64().unwrap_or(0.0);
            if basis.is_empty() {
                basis = e[mode]["basis"].as_str().unwrap_or("").to_string();
            }
        }
        let r1 = |x: f64| (x * 10.0).round() / 10.0;
        json!({"low_pp": r1(low), "high_pp": r1(high), "basis": basis, "rough": basis == "default"})
    }

    /// Totals over the non-skipped features, against the room of the week (F9).
    fn program_totals(&self, p: &Program) -> Value {
        let (low, high) = p.features.iter().filter(|f| !f.skip).fold((0.0, 0.0), |(l, h), f| {
            (l + f.estimate["low_pp"].as_f64().unwrap_or(0.0), h + f.estimate["high_pp"].as_f64().unwrap_or(0.0))
        });
        let est = self.intake_est();
        let room = est.rooms.get(&p.project);
        let over = match room {
            Some(r) if low > r.pp => "low",
            Some(r) if high > r.pp => "high",
            _ => "none",
        };
        json!({"low_pp": low, "high_pp": high, "room_pp": room.map(|r| r.pp), "room_scope": room.map(|r| r.scope),
               "over_room": over})
    }

    /// The issues of a program against the cache and the project's teams (F8).
    fn program_issues(&self, p: &Program) -> Vec<PlanIssue> {
        let mut issues = vec![];
        if let Some(cyc) = find_cycle(
            &p.features,
            &p.edges
                .iter()
                .filter(|e| !p.cut.iter().any(|c| c[0] == e.from && c[1] == e.to))
                .cloned()
                .collect::<Vec<_>>(),
        ) {
            let keys: Vec<String> = cyc.iter().map(|e| e[0].clone()).collect();
            let slug = |k: &str| p.features.iter().find(|f| f.key == k).map_or(k.to_string(), |f| f.slug.clone());
            let names: Vec<String> = keys.iter().map(|k| slug(k)).collect();
            let mut i =
                PlanIssue::new("cycle", true, format!("Order loop: {} wait on each other.", names.join(" and ")));
            i.items = cyc
                .iter()
                .flat_map(|e| p.edges.iter().filter(|x| x.from == e[0] && x.to == e[1]).flat_map(|x| x.items.clone()))
                .collect();
            i.features = keys;
            i.edges = cyc;
            issues.push(i);
        }
        let teams_ok = |t: &Option<String>| {
            t.as_deref().is_some_and(|t| self.db.team_in(Some(&p.project), t).ok().flatten().is_some())
        };
        for f in p.features.iter().filter(|f| !f.skip && f.mode == "team" && !teams_ok(&f.team)) {
            let mut i = PlanIssue::new(
                "no_team",
                true,
                format!(
                    "{} needs a team: {} has no team.",
                    f.slug,
                    self.db.project(&p.project).ok().flatten().map_or(p.project.clone(), |x| x.name)
                ),
            );
            (i.features, i.feature) = (vec![f.key.clone()], Some(f.slug.clone()));
            issues.push(i);
        }
        for r in &p.left_out.running {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let mut i = PlanIssue::new(
                "already_running",
                false,
                format!(
                    "{} is already running as {}. It is left out; features that depend on it wait for its merge.",
                    id.rsplit('#').next().map(|n| format!("#{n}")).unwrap_or(id.clone()),
                    r["feature"].as_str().unwrap_or("a feature")
                ),
            );
            (i.items, i.feature) = (vec![id], r["feature"].as_str().map(str::to_string));
            issues.push(i);
        }
        if !p.left_out.closed.is_empty() {
            let mut i =
                PlanIssue::new("closed", false, format!("{} closed item(s) are left out.", p.left_out.closed.len()));
            i.items = p.left_out.closed.clone();
            issues.push(i);
        }
        let t = self.program_totals(p);
        if let Some(room) = t["room_pp"].as_f64()
            && t["over_room"] != "none"
        {
            let (what, n) =
                if t["over_room"] == "low" { ("low", t["low_pp"].as_f64()) } else { ("high", t["high_pp"].as_f64()) };
            issues.push(PlanIssue::new(
                "over_room",
                false,
                format!("About {:.0} pp is over this week's room ({room:.0} pp left) at the {what} end. Features wait for room under the Governor.", n.unwrap_or(0.0)),
            ));
        }
        issues
    }

    fn program_ctx(&self, project: &str) -> Result<Ctx> {
        let p = self.db.project(project)?.ok_or_else(|| anyhow!("no project {project}"))?;
        Ctx::new(self, &p)
    }

    /// `program_plan(roots, grouping, opts)` → `{program (draft), issues}`: nothing is stored.
    pub fn program_plan(&self, roots: &Value, grouping: &str, opts: &Value) -> Result<Value> {
        let ids: Vec<String> =
            roots.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        if ids.is_empty() {
            return Err(bad("roots", "give a node or two or more siblings"));
        }
        if !matches!(grouping, "child" | "grandchild" | "single") {
            return Err(bad("grouping", "grouping is child, grandchild or single"));
        }
        let project = ids
            .iter()
            .find_map(|id| self.db.work_node_rows(id).ok()?.into_iter().next())
            .map(|r| r.project_id)
            .ok_or_else(|| bad("roots", "no such node"))?;
        let ctx = self.program_ctx(&project)?;
        let idx: Vec<usize> = ids
            .iter()
            .map(|id| ctx.index(id).ok_or_else(|| bad("roots", format!("no node {id:?} in {project}"))))
            .collect::<Result<_>>()?;
        if idx.len() > 1 {
            let parent = &ctx.rows[idx[0]].parent_id;
            if idx.iter().any(|i| &ctx.rows[*i].parent_id != parent) {
                return Err(bad("roots", "select items under the same parent to run them as one program"));
            }
        }
        // Already the root of a program in progress: stale (notes §6.5).
        if let Some(running) = self
            .db
            .programs_in_progress(Some(&project))?
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Program>(v).ok())
            .find(|q| matches!(q.status.as_str(), "running" | "paused") && q.root.iter().any(|r| ids.contains(r)))
        {
            return Err(stale(&format!("already in program {}", running.title), json!({"program": running.id})));
        }
        let sequential = opts["sequential"].as_bool().unwrap_or(false);
        let max = if sequential {
            1
        } else {
            (opts["max_parallel"].as_u64().unwrap_or(DEFAULT_PARALLEL as u64) as u32).clamp(1, MAX_PARALLEL)
        };
        let cut: Vec<[String; 2]> = opts["cut"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some([c[0].as_str()?.to_string(), c[1].as_str()?.to_string()]))
            .collect();
        let include: Vec<String> =
            opts["include"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        let mut p = self.plan_for(&ctx, &idx, grouping, sequential, max, cut, include)?;
        p.created_at_ms = orch_protocol::now_ms();
        let issues = self.program_issues(&p);
        Ok(json!({"program": p, "issues": issues}))
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_for(
        &self,
        ctx: &Ctx,
        roots: &[usize],
        grouping: &str,
        sequential: bool,
        max: u32,
        cut: Vec<[String; 2]>,
        include: Vec<String>,
    ) -> Result<Program> {
        let project = ctx.project.clone();
        let mut scope = Scope { include: include.iter().cloned().collect(), ..Default::default() };
        let groups = groups(ctx, roots, grouping, &mut scope);
        let team = self.intake_team(&project, None);
        let base = project
            .typed_settings()
            .base_branch
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| project.default_branch.clone());
        let mut slugs: HashSet<String> = HashSet::new();
        let mut features = vec![];
        for (n, items) in groups.iter().enumerate() {
            let first = &ctx.rows[items[0]];
            let full = self
                .db
                .work_node_rows(&first.node_id)?
                .into_iter()
                .find(|r| r.project_id == project.id)
                .unwrap_or_else(|| first.clone());
            let rest: Vec<usize> = items[1..].to_vec();
            let brief = work_brief(ctx, items[0], &full, &rest, &project, self);
            let slug = unique(&kebab(&first.title, 40), 40, |s| {
                s == "solo" || slugs.contains(s) || self.db.feature_by_slug(&project.id, s).ok().flatten().is_some()
            });
            slugs.insert(slug.clone());
            let mode = if team.is_some() { "team" } else { "solo" };
            let rows: Vec<&IntakeRow> = items.iter().map(|i| &ctx.rows[*i]).collect();
            features.push(Feature {
                key: format!("pf-{}", n + 1),
                items: items.iter().map(|i| ctx.rows[*i].node_id.clone()).collect(),
                slug,
                title: first.title.clone(),
                mode: mode.into(),
                team: team.clone(),
                brief,
                base: base.clone(),
                estimate: self.feature_estimate(&rows, mode),
                state: "planned".into(),
                ..Default::default()
            });
        }
        let root_ids: Vec<String> = roots.iter().map(|r| ctx.rows[*r].node_id.clone()).collect();
        let (title, root_ref) = if roots.len() == 1 {
            (ctx.rows[roots[0]].title.clone(), ctx.rows[roots[0]].reference.clone())
        } else {
            let parent = ctx.rows[roots[0]].parent_id.as_deref().and_then(|p| ctx.index(p));
            match parent {
                Some(pi) => {
                    (format!("{} · {} selected", ctx.rows[pi].title, roots.len()), ctx.rows[pi].reference.clone())
                }
                None => (format!("{} · {} selected", project.name, roots.len()), project.name.clone()),
            }
        };
        let mut edges = if sequential {
            features
                .windows(2)
                .map(|w| Edge {
                    from: w[0].key.clone(),
                    to: w[1].key.clone(),
                    source: "sequential".into(),
                    items: vec![],
                })
                .collect()
        } else {
            tracker_edges(ctx, &features, &scope.running)
        };
        edges.retain(|e| !cut.iter().any(|c| c[0] == e.from && c[1] == e.to));
        let mut p = Program {
            id: new_id(orch_protocol::now_ms()),
            project: project.id.clone(),
            title,
            root: root_ids,
            root_ref,
            status: "draft".into(),
            grouping: grouping.into(),
            max_parallel: max,
            sequential,
            features,
            edges,
            cut,
            left_out: LeftOut { closed: scope.closed, running: scope.running },
            include,
            ..Default::default()
        };
        set_depends(&mut p);
        p.estimate = self.program_totals(&p);
        Ok(p)
    }

    /// `program_start(program)`: checks the user's edited draft against the cache (never re-plans over it)
    /// and starts it; `{issues}` while a blocking one remains, `{program}` once it runs.
    pub fn program_start(&self, draft: &Value) -> Result<Value> {
        let mut p: Program = serde_json::from_value(draft.clone()).map_err(|e| bad("program", e.to_string()))?;
        if p.features.is_empty() {
            return Err(bad("program", "nothing to run: the program has no feature"));
        }
        let ctx = self.program_ctx(&p.project)?;
        // The roots must still exist and not be a program in progress.
        let in_progress: Vec<Program> = self
            .db
            .programs_in_progress(Some(&p.project))?
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Program>(v).ok())
            .filter(|q| q.id != p.id && matches!(q.status.as_str(), "running" | "paused"))
            .collect();
        for r in &p.root {
            if ctx.index(r).is_none() {
                return Err(stale("the root is gone upstream", json!({})));
            }
            if let Some(q) = in_progress.iter().find(|q| q.root.contains(r)) {
                return Err(stale(&format!("already in program {}", q.title), json!({"program": q.id})));
            }
        }
        // Slugs: valid and unique, the user's own; not a feature that has a team running.
        let mut seen = HashSet::new();
        for f in &p.features {
            if !orch_team::valid_slug(&f.slug) || f.slug == "solo" || !seen.insert(f.slug.clone()) {
                return Err(bad("slug", format!("slug {:?}: a-z, 0-9, - and _, at most 40, one per feature", f.slug)));
            }
            if let Some(feat) = self.db.feature_by_slug(&p.project, &f.slug)?
                && !self.db.runs(Some(&feat.id), true)?.is_empty()
            {
                return Err(bad("slug", format!("feature {} already has a running team", f.slug)));
            }
            if !matches!(f.mode.as_str(), "team" | "solo") {
                return Err(bad("mode", "mode is team or solo"));
            }
        }
        // Items that closed or started meanwhile are left out; a feature left with none is skipped.
        for f in &mut p.features {
            let before = f.items.len();
            f.items.retain(|id| {
                let Some(i) = ctx.index(id) else { return false };
                if !ctx.open(i) && !p.include.contains(id) {
                    p.left_out.closed.push(id.clone());
                    return false;
                }
                if matches!(ctx.app_state(i), "running" | "solo" | "pr" | "queued") {
                    p.left_out
                        .running
                        .push(json!({"id": id, "feature": ctx.app_feature(i), "wait": ctx.app_wait_key(i)}));
                    return false;
                }
                true
            });
            if f.items.is_empty() && !f.skip {
                f.skip = true;
                f.flags.push("closed_upstream".into());
            } else if f.items.len() < before {
                f.flags.push("closed_upstream".into());
            }
        }
        let known: HashSet<String> = p.features.iter().map(|f| f.key.clone()).collect();
        p.edges.retain(|e| (known.contains(&e.from) || e.from.starts_with("x:")) && known.contains(&e.to));
        p.max_parallel = if p.sequential { 1 } else { p.max_parallel.clamp(1, MAX_PARALLEL) };
        set_depends(&mut p);
        p.estimate = self.program_totals(&p);
        let issues = self.program_issues(&p);
        if issues.iter().any(|i| i.blocking) {
            return Ok(json!({"issues": issues}));
        }
        let now = orch_protocol::now_ms();
        p.id = new_id(now);
        p.status = "running".into();
        p.created_at_ms = now;
        p.started_at_ms = Some(now);
        for f in &mut p.features {
            f.state = if f.skip { "skipped".into() } else { "planned".into() };
        }
        self.program_save(&p)?;
        let by = json!({"id": p.id, "title": p.title});
        for f in &p.features {
            for id in &f.items {
                if let Ok(r) = self.node_row(&p.project, id) {
                    let e = orch_store::WorkEvent {
                        at_ms: now,
                        what: "program_queued".into(),
                        feature: Some(f.slug.clone()),
                        program: Some(by.clone()),
                        detail: None,
                    };
                    let _ = self.db.work_note(&r.project_id, &r.node_id, &e);
                }
            }
        }
        self.event("-", "program.started", json!({"id": p.id, "project": p.project}));
        self.programs_tick();
        Ok(json!({"program": self.program_view(&p.id)?}))
    }

    /// Stores a program and posts it to `now`.
    pub(crate) fn program_save(&self, p: &Program) -> Result<()> {
        self.db.upsert_program(&p.id, &p.project, &p.status, &serde_json::to_value(p)?, orch_protocol::now_ms())
    }

    pub(crate) fn program_get(&self, id: &str) -> Result<Program> {
        let v = self.db.program(id)?.ok_or_else(|| {
            anyhow!(json!({"code": "not_found", "message": format!("no program {id:?}")}).to_string())
        })?;
        Ok(serde_json::from_value(v)?)
    }

    /// `program(id)`: the program with its live states.
    pub fn program(&self, id: &str) -> Result<Value> {
        self.programs_tick();
        self.program_view(id)
    }

    pub(crate) fn program_view(&self, id: &str) -> Result<Value> {
        Ok(serde_json::to_value(self.program_get(id)?)?)
    }

    /// `program_update(id, patch)`: pause · resume · skip · add_item · ignore_item · max_parallel · cancel.
    pub fn program_update(&self, id: &str, patch: &Value) -> Result<Value> {
        // Serialised with the engine's pass and the refresh: each loads, edits and stores the whole
        // document, so an edit in between would be lost (a pause undone, a started feature started twice).
        let guard = self.intake.prog_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = self.program_get(id)?;
        let live = matches!(p.status.as_str(), "running" | "paused");
        let obj = patch.as_object().ok_or_else(|| bad("patch", "an object with one edit"))?;
        // (the edit is the known key; `add_item` carries `mode` / `team` / `after` beside it)
        const OPS: [&str; 7] = ["pause", "resume", "max_parallel", "skip", "ignore_item", "add_item", "cancel"];
        let Some((op, arg)) = obj.iter().find(|(k, _)| OPS.contains(&k.as_str())).or_else(|| obj.iter().next()) else {
            return Err(bad("patch", "one edit"));
        };
        if !live && p.status != "draft" {
            return Err(stale("the program is over", json!({"status": p.status})));
        }
        let now = orch_protocol::now_ms();
        match op.as_str() {
            "pause" => p.status = "paused".into(),
            "resume" => p.status = "running".into(),
            "max_parallel" => {
                let n = arg.as_u64().ok_or_else(|| bad("max_parallel", "1 to 5"))? as u32;
                p.max_parallel = if p.sequential { 1 } else { n.clamp(1, MAX_PARALLEL) };
            }
            "skip" => {
                let key = arg.as_str().ok_or_else(|| bad("skip", "a feature key"))?;
                let f = p.features.iter_mut().find(|f| f.key == key).ok_or_else(|| bad("skip", "no such feature"))?;
                // (a "queued" feature held by the Governor already has its run: skipping it would orphan it)
                if !matches!(f.state.as_str(), "planned" | "waiting" | "queued" | "failed")
                    || (f.state == "queued" && (f.run.is_some() || f.agent.is_some()))
                {
                    return Err(bad("skip", "only a feature that has not started can be skipped"));
                }
                f.skip = true;
                f.state = "skipped".into();
                // Dependents stop waiting for it.
                set_depends(&mut p);
                self.program_close_decisions(&p, "skip", |k| k.ends_with(&format!(":{key}")));
            }
            "ignore_item" => {
                let item = arg.as_str().ok_or_else(|| bad("ignore_item", "a node id"))?.to_string();
                p.new_items.retain(|i| *i != item);
                p.new_items_added.remove(&item);
                if !p.ignored.contains(&item) {
                    p.ignored.push(item.clone());
                }
                self.program_close_decisions(&p, "ignore", |k| k == format!("new:{item}"));
            }
            "add_item" => self.program_add_item(&mut p, arg, obj)?,
            "cancel" => {
                // Running features go on as normal features; the ones waiting are dropped.
                for f in &mut p.features {
                    if matches!(f.state.as_str(), "planned" | "waiting" | "queued")
                        && f.run.is_none()
                        && f.agent.is_none()
                    {
                        f.state = "skipped".into();
                        f.skip = true;
                    }
                }
                p.status = "cancelled".into();
                p.done_at_ms = Some(now);
                self.event("-", "program.cancelled", json!({"id": p.id, "project": p.project, "title": p.title}));
                self.program_close_decisions(&p, "dismiss", |_| true);
            }
            other => return Err(bad("patch", format!("{other} is not a program edit"))),
        }
        self.program_save(&p)?;
        drop(guard);
        self.programs_tick();
        self.program_view(id)
    }

    /// `add_item: <node id>` with `mode`, `team`, `after: [keys]` (patch fields beside it): a new feature.
    fn program_add_item(&self, p: &mut Program, arg: &Value, patch: &serde_json::Map<String, Value>) -> Result<()> {
        let node = arg.as_str().ok_or_else(|| bad("add_item", "a node id"))?.to_string();
        let ctx = self.program_ctx(&p.project)?;
        let i = ctx.index(&node).ok_or_else(|| bad("add_item", "no such node"))?;
        if p.features.iter().any(|f| f.items.contains(&node)) {
            return Err(bad("add_item", "already in the program"));
        }
        let mode = match patch.get("mode").and_then(Value::as_str) {
            Some(m @ ("team" | "solo")) => m,
            None => {
                if self.intake_team(&ctx.project, None).is_some() {
                    "team"
                } else {
                    "solo"
                }
            }
            Some(_) => return Err(bad("mode", "mode is team or solo")),
        };
        let team = (mode == "team")
            .then(|| {
                patch
                    .get("team")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| self.intake_team(&ctx.project, None))
            })
            .flatten();
        let full = self
            .db
            .work_node_rows(&node)?
            .into_iter()
            .find(|r| r.project_id == p.project)
            .ok_or_else(|| bad("add_item", "no such node"))?;
        let kids: Vec<usize> = ctx.kids_of(i).to_vec();
        let taken = |s: &str| {
            s == "solo"
                || p.features.iter().any(|f| f.slug == s)
                || self.db.feature_by_slug(&p.project, s).ok().flatten().is_some()
        };
        // The user's slug, when given (valid and free), else one made from the title.
        let slug = match patch.get("slug").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) if !orch_team::valid_slug(s) || s == "solo" || taken(s) => {
                return Err(bad("slug", format!("slug {s:?}: a-z, 0-9, - and _, at most 40, and not taken")));
            }
            Some(s) => s.to_string(),
            None => unique(&kebab(&full.title, 40), 40, taken),
        };
        let key = format!(
            "pf-{}",
            p.features.iter().filter_map(|f| f.key.strip_prefix("pf-")?.parse::<u32>().ok()).max().unwrap_or(0) + 1
        );
        let row = &ctx.rows[i];
        let base = p.features.first().map_or_else(|| ctx.project.default_branch.clone(), |f| f.base.clone());
        p.features.push(Feature {
            key: key.clone(),
            items: vec![node.clone()],
            slug,
            title: full.title.clone(),
            mode: mode.into(),
            team,
            brief: work_brief(&ctx, i, &full, &kids, &ctx.project, self),
            base,
            estimate: self.feature_estimate(&[row], mode),
            state: "planned".into(),
            ..Default::default()
        });
        for a in patch.get("after").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if p.features.iter().any(|f| f.key == a) {
                p.edges.push(Edge { from: a.to_string(), to: key.clone(), source: "user".into(), items: vec![] });
            }
        }
        p.new_items.retain(|n| *n != node);
        p.new_items_added.remove(&node);
        set_depends(p);
        p.estimate = self.program_totals(p);
        self.program_close_decisions(p, "add", |k| k == format!("new:{node}"));
        Ok(())
    }
}

impl Runtime {
    /// An action on decision `prog-<n>` (`inbox_act`): add · ignore · skip · keep · open.
    /// `add` and `open` only say where to go (the row editor, the program, the feature); the others edit the program.
    pub fn program_decision_act(&self, n: u64, action: &str) -> Result<Value> {
        let item = self
            .store
            .read(|s| s.inbox.iter().find(|i| i.id == n && i.kind == "program").cloned())
            .ok_or_else(|| anyhow!("no program decision {n}"))?;
        let pl = &item.payload;
        let program = pl["program"].as_str().unwrap_or("").to_string();
        let id = crate::decision::decision_id("program", n);
        let key = pl["key"].as_str().unwrap_or("");
        match (pl["kind"].as_str().unwrap_or(""), action) {
            ("program_new_item", "add") => {
                Ok(json!({"ok": true, "open": "program_add", "program": program, "item": key.strip_prefix("new:")}))
            }
            // The edit closes the card with the user's word, once.
            ("program_new_item", "ignore") => {
                self.program_update(&program, &json!({"ignore_item": key.strip_prefix("new:").unwrap_or("")}))?;
                Ok(json!({"ok": true, "effect": "ignored", "decision": id}))
            }
            ("program_item_closed", "skip") | ("program_feature_failed", "skip") => {
                self.program_update(&program, &json!({"skip": pl["feature"]}))?;
                Ok(json!({"ok": true, "effect": "skipped", "decision": id}))
            }
            ("program_item_closed", "keep") => {
                self.program_close_one(n, "keep");
                Ok(json!({"ok": true, "effect": "kept", "decision": id}))
            }
            ("program_feature_failed", "open") => {
                Ok(json!({"ok": true, "open": "feature", "program": program, "feature": pl["slug"]}))
            }
            ("writeback_failing", "settings") => {
                Ok(json!({"ok": true, "open": "settings", "section": "tracker", "project": pl["project"]}))
            }
            ("writeback_failing", "turn_off") => {
                self.wb_turn_off(pl["project"].as_str().unwrap_or(""))?;
                self.program_close_one(n, "turn_off");
                Ok(json!({"ok": true, "effect": "turned off", "decision": id}))
            }
            (_, "open") => {
                self.program_close_one(n, "open");
                Ok(json!({"ok": true, "open": "program", "program": program}))
            }
            (k, a) => Err(anyhow!("{a} does not apply to {k}")),
        }
    }
}

impl Runtime {
    /// The program each started run / session belongs to, by run id and agent id
    /// (`{id, title, index, total}`, index among the features that are not skipped) and the
    /// feature's first item (the source node of the start).
    pub(crate) fn program_of_starts(&self) -> HashMap<String, (Value, String)> {
        let mut out = HashMap::new();
        for v in self.db.programs(None).unwrap_or_default() {
            let Ok(p) = serde_json::from_value::<Program>(v) else { continue };
            let live: Vec<&Feature> = p.features.iter().filter(|f| !f.skip).collect();
            for (n, f) in live.iter().enumerate() {
                let tag = json!({"id": p.id, "title": p.title, "index": n + 1, "total": live.len()});
                for id in f.run.iter().chain(f.agent.iter()) {
                    out.insert(id.clone(), (tag.clone(), f.items.first().cloned().unwrap_or_default()));
                }
            }
        }
        out
    }

    /// A program as `now` shows it (contract §4 `ProgramRow`).
    fn program_row(&self, p: &Program) -> Value {
        let slug = |k: &str| {
            p.features.iter().find(|f| f.key == k).map(|f| f.slug.clone()).unwrap_or_else(|| {
                // An item running outside the program: the feature it runs as.
                let r = p.left_out.running.iter().find(|r| r["wait"] == k);
                r.and_then(|r| r["feature"].as_str()).unwrap_or(k).to_string()
            })
        };
        let n = |s: &[&str]| p.features.iter().filter(|f| s.contains(&f.state.as_str())).count();
        let features: Vec<Value> = p
            .features
            .iter()
            .map(|f| {
                let mut v = json!({"key": f.key, "slug": f.slug, "title": f.title, "state": f.state, "mode": f.mode,
                                   "team": f.team, "flags": f.flags});
                if let Some(ph) = &f.phase {
                    v["phase"] = json!(ph);
                }
                if let Some(pr) = &f.pr {
                    v["pr"] = pr.clone();
                }
                if f.state == "waiting" {
                    v["waiting_on"] = json!(f.depends_on.iter().map(|k| slug(k)).collect::<Vec<_>>());
                }
                v
            })
            .collect();
        json!({
            "id": p.id, "project": p.project, "title": p.title, "root_ref": p.root_ref, "status": p.status,
            "counts": {"merged": n(&["merged"]), "running": n(&["running"]), "pr": n(&["pr"]),
                       "waiting": n(&["waiting"]), "queued": n(&["queued", "planned"]), "failed": n(&["failed"]),
                       "skipped": n(&["skipped"])},
            "estimate": {"low_pp": p.estimate["low_pp"], "high_pp": p.estimate["high_pp"]},
            "features": features,
        })
    }

    /// The programs in progress, as rows by id.
    fn programs_rows(&self) -> BTreeMap<String, Value> {
        self.db
            .programs_in_progress(None)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Program>(v).ok())
            .filter(|p| matches!(p.status.as_str(), "running" | "paused"))
            .map(|p| (p.id.clone(), self.program_row(&p)))
            .collect()
    }

    /// `now.programs` in the snapshot: the whole list (and what later deltas are relative to).
    pub(crate) fn programs_snapshot(&self) -> Value {
        let rows = self.programs_rows();
        let list: Vec<Value> = rows.values().cloned().collect();
        *crate::intake::lock(&self.intake.prog_published) = Some(rows);
        json!(list)
    }

    /// `now.programs` in a delta: `{upsert, remove}` since the last one sent; `None` when nothing moved.
    pub(crate) fn programs_delta(&self) -> Option<Value> {
        let rows = self.programs_rows();
        let mut sent = crate::intake::lock(&self.intake.prog_published);
        let old = sent.take().unwrap_or_default();
        let upsert: Vec<Value> = rows.iter().filter(|(k, v)| old.get(*k) != Some(*v)).map(|(_, v)| v.clone()).collect();
        let remove: Vec<&String> = old.keys().filter(|k| !rows.contains_key(*k)).collect();
        let out = (!upsert.is_empty() || !remove.is_empty()).then(|| json!({"upsert": upsert, "remove": remove}));
        *sent = Some(rows);
        out
    }

    /// After a refresh: what the tracker changed under the programs in progress (notes §7.3).
    pub(crate) fn programs_on_refresh(&self, project: &str) {
        let Ok(_guard) = self.intake.prog_lock.lock() else { return };
        let progs: Vec<Program> = self
            .db
            .programs_in_progress(Some(project))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Program>(v).ok())
            .filter(|p| matches!(p.status.as_str(), "running" | "paused"))
            .collect();
        if progs.is_empty() {
            return;
        }
        let Ok(ctx) = self.program_ctx(project) else { return };
        for mut p in progs {
            let before = p.clone();
            self.program_upstream(&ctx, &mut p);
            if p != before {
                let _ = self.program_save(&p);
            }
        }
    }

    fn program_upstream(&self, ctx: &Ctx, p: &mut Program) {
        // A child added under a root, not in any feature (F7).
        let roots: Vec<usize> = p.root.iter().filter_map(|r| ctx.index(r)).collect();
        let mut scope = Scope { include: p.include.iter().cloned().collect(), ..Default::default() };
        // (the program's own grouping: a "grandchild" plan leaves the child items that have sub-issues
        // out of every feature, and they are not new)
        let all: Vec<usize> = groups(ctx, &roots, &p.grouping, &mut scope).into_iter().flatten().collect();
        let known: HashSet<&str> = p
            .features
            .iter()
            .flat_map(|f| f.items.iter().map(String::as_str))
            .chain(p.new_items.iter().map(String::as_str))
            .chain(p.ignored.iter().map(String::as_str))
            .collect();
        let fresh: Vec<String> =
            all.iter().map(|i| ctx.rows[*i].node_id.clone()).filter(|id| !known.contains(id.as_str())).collect();
        for id in fresh {
            let (r, parent) = {
                let i = ctx.index(&id).unwrap_or(0);
                (
                    ctx.rows[i].reference.clone(),
                    ctx.rows[i].parent_id.as_deref().and_then(|q| ctx.index(q)).map(|q| ctx.rows[q].reference.clone()),
                )
            };
            p.new_items_added.insert(id.clone(), orch_protocol::now_ms());
            p.new_items.push(id.clone());
            let text = format!(
                "{r} was added{} in {} upstream.",
                parent.map(|q| format!(" under {q}")).unwrap_or_default(),
                p.title
            );
            self.program_decision(
                p,
                "program_new_item",
                &format!("New sub-issue in {}", p.title),
                &text,
                json!({}),
                &format!("new:{id}"),
            );
        }
        // A feature's item closed or deleted upstream (not stopped when it runs).
        let mut closed: Vec<(String, String)> = vec![];
        for f in p.features.iter_mut().filter(|f| !f.skip) {
            for id in &f.items {
                let gone = ctx.index(id).is_none();
                let shut = !p.include.contains(id) && ctx.index(id).is_some_and(|i| !ctx.open(i));
                let flag = if gone {
                    "deleted_upstream"
                } else if shut {
                    "closed_upstream"
                } else {
                    continue;
                };
                if !f.flags.iter().any(|x| x == flag) {
                    f.flags.push(flag.into());
                    if matches!(f.state.as_str(), "planned" | "waiting" | "queued") {
                        closed.push((f.key.clone(), f.slug.clone()));
                    }
                }
            }
        }
        for (key, slug) in closed {
            self.program_decision(
                p,
                "program_item_closed",
                &format!("An item of {slug} was closed upstream"),
                &format!("{slug} is in {} and is waiting. It is not stopped.", p.title),
                json!({"feature": key, "slug": slug}),
                &format!("closed:{key}"),
            );
        }
        // A dependency added upstream between features that have not started.
        let have: HashSet<(String, String)> = p.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
        let cut: HashSet<(String, String)> = p.cut.iter().map(|c| (c[0].clone(), c[1].clone())).collect();
        if !p.sequential {
            let mut added = false;
            let slug_of =
                |p: &Program, k: &str| p.features.iter().find(|f| f.key == k).map_or(k.to_string(), |f| f.slug.clone());
            for e in tracker_edges(
                ctx,
                &p.features.iter().filter(|f| !f.skip).cloned().collect::<Vec<_>>(),
                &p.left_out.running,
            ) {
                let k = (e.from.clone(), e.to.clone());
                let started = p.features.iter().any(|f| f.key == e.to && (f.run.is_some() || f.agent.is_some()));
                if have.contains(&k) || cut.contains(&k) || started {
                    continue;
                }
                let (from, to) = (slug_of(p, &e.from), slug_of(p, &e.to));
                if let Some(f) = p.features.iter_mut().find(|f| f.key == e.to) {
                    f.flags.push("new_dependency".into());
                }
                let key = format!("dep:{}>{}", e.from, e.to);
                p.edges.push(e);
                added = true;
                self.program_decision(
                    p,
                    "program_dependency_added",
                    &format!("New order in {}", p.title),
                    &format!("{to} now waits for {from}: a dependency was added upstream."),
                    json!({}),
                    &key,
                );
            }
            if added {
                set_depends(p);
            }
        }
    }
}
