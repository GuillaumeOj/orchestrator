//! Intake (FR-1312, P4.5 spec §3.2–§3.3): each project's intake source is
//! refreshed into the store's cache by the [`Refresher`] (its own thread):
//! every `intake.refresh_secs` (10 min), at start (staggered 2 s per
//! project), at once on a source change or `intake_refresh`, and for repo
//! docs when the branch tip moves (checked on the 60 s fetch tick, no
//! network). One refresh in flight per project; a failure keeps the last
//! good items and records the source error. `now.intake` is built from the
//! cache, so the lane renders at start and a refresh never blocks `now`.
//!
//! Read-only towards the sources (D3); starting an item is the user's
//! action through the launch / new-session dialogs (never from here).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Result, anyhow};
use orch_store::{GithubIntakeFilter, IntakeRow, IntakeSource, IntakeState, Project};
use orch_tasks::intake::{self as src, Backoff, Fetched, LinearQuery, Listing, SourceError};
use serde_json::{Value, json};

use crate::runtime::Runtime;

pub mod brief;
pub mod estimate;
pub mod report;
pub mod start;

pub use start::IntakeLink;

pub const TOPIC: &str = orch_store::INTAKE_TOPIC;
/// Global setting: seconds between two refreshes of a project (F16).
pub const REFRESH_SECS_KEY: &str = "intake.refresh_secs";
pub const DEFAULT_REFRESH_SECS: u64 = 600;
/// Start-up refreshes are this far apart, per project.
pub const STAGGER_MS: u64 = 2000;
/// How often repo-docs branch tips are checked.
pub const TIP_EVERY_MS: u64 = 60_000;
/// How often the refresher thread looks at what is due.
const STEP: Duration = Duration::from_secs(1);
/// A `intake_source_set` waits this long for a refresh in flight.
const WAIT_INFLIGHT: Duration = Duration::from_secs(40);
/// Items in a preview.
const PREVIEW: usize = 5;
const EXCERPT: usize = 200;

#[derive(Default)]
struct Sched {
    /// The start-up stagger was laid out.
    started: bool,
    /// The last repo-docs tip check (ms).
    last_tip_ms: u64,
    /// When each project is next due (ms).
    due: BTreeMap<String, u64>,
    inflight: BTreeSet<String>,
}

/// The intake refresher's state, on the runtime.
#[derive(Default)]
pub struct Refresher {
    sched: Mutex<Sched>,
    /// Signalled on a request and when a refresh ends.
    wake: Condvar,
    /// The `now.intake` last sent.
    published: Mutex<Option<Value>>,
    /// Linear's 429 back-off, per project.
    backoff: Mutex<BTreeMap<String, Arc<Backoff>>>,
    /// The estimator's history and rooms (spec §3.4).
    pub(crate) est: Mutex<Option<Arc<estimate::EstState>>>,
    /// Items and briefs a start is claiming (one start each).
    pub(crate) claims: Mutex<BTreeSet<String>>,
    /// When the history was last read (ms): at most once a minute.
    pub(crate) est_at: std::sync::atomic::AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Refresher {
    pub(crate) fn published(&self) -> bool {
        lock(&self.published).is_some()
    }

    /// Records `v` as sent; true when it differs from the last one.
    pub(crate) fn publish(&self, v: &Value) -> bool {
        let mut p = lock(&self.published);
        let changed = p.as_ref() != Some(v);
        if changed {
            *p = Some(v.clone());
        }
        changed
    }

    fn backoff(&self, project: &str) -> Arc<Backoff> {
        lock(&self.backoff).entry(project.to_string()).or_default().clone()
    }
}

/// The command error of a rejected source (api-contract §1.1): its text is
/// the JSON `{code: "bad_request", field, message}`.
fn bad_request(field: &str, message: impl Into<String>) -> anyhow::Error {
    anyhow!(json!({"code": "bad_request", "field": field, "message": message.into()}).to_string())
}

/// Parses, normalizes and validates a source sent by the UI.
pub fn parse_source(v: &Value) -> Result<IntakeSource> {
    if v.is_null() {
        return Ok(IntakeSource::None);
    }
    let s: IntakeSource = serde_json::from_value(v.clone()).map_err(|e| bad_request("type", e.to_string()))?;
    let s = s.normalized();
    s.validate().map_err(|e| bad_request(e.field, e.message))?;
    if let IntakeSource::RepoDocs { glob, .. } = &s {
        src::compile_glob(glob).map_err(|e| bad_request("glob", format!("the glob does not parse: {e}")))?;
    }
    Ok(s)
}

/// `"it_41"` → 41.
fn item_id(id: &str) -> Result<u64> {
    id.strip_prefix("it_").and_then(|n| n.parse().ok()).ok_or_else(|| anyhow!("no intake item {id:?}"))
}

/// The project's base branch (settings, else the repository's default).
fn base_branch(p: &Project) -> String {
    p.typed_settings().base_branch.filter(|b| !b.is_empty()).unwrap_or_else(|| p.default_branch.clone())
}

/// First chars of a body as plain text (markdown marks dropped).
fn excerpt(body: &str) -> String {
    let text: String = body
        .lines()
        .map(|l| {
            let l = l.trim().trim_start_matches(['#', '>']).trim_start();
            // A list bullet, not a negative number or a rule.
            l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).unwrap_or(l).trim()
        })
        .filter(|l| !l.is_empty() && !l.starts_with("```") && *l != "---")
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
        .replace('`', "");
    match text.char_indices().nth(EXCERPT) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text,
    }
}

/// Linear's priority as its label (F5).
fn priority_label(p: &Value) -> Value {
    match p.as_u64() {
        Some(1) => json!("Urgent"),
        Some(2) => json!("High"),
        Some(3) => json!("Medium"),
        Some(4) => json!("Low"),
        _ => Value::Null,
    }
}

/// Facts of a project the items' JSON needs.
struct ProjectFacts {
    default_team: Option<String>,
    can_team: bool,
}

/// The mode an item suggests: its `mode` front matter, else team when the
/// project has a default team, else solo.
fn suggested_mode(meta: &Value, facts: &ProjectFacts) -> &'static str {
    match meta["mode"].as_str() {
        Some("team") => "team",
        Some("solo") => "solo",
        _ if facts.default_team.is_some() => "team",
        _ => "solo",
    }
}

/// An item as `now.intake` and the commands show it (api-contract §1.2).
fn item_json(f: &ItemFields, facts: &ProjectFacts, est: &estimate::EstState) -> Value {
    let meta = f.meta;
    let mode = suggested_mode(meta, facts);
    let team = meta["team"].as_str().map(str::to_string).or_else(|| facts.default_team.clone());
    json!({
        "id": f.id.map(|i| format!("it_{i}")),
        "project": f.project,
        "source": f.source,
        "ref": f.reference,
        "title": f.title,
        "url": f.url,
        "labels": meta["labels"].as_array().cloned().unwrap_or_default(),
        "updated_at_ms": meta["updated_at_ms"].as_u64().unwrap_or(0),
        "size": meta["size"],
        "mode": mode,
        "team": if mode == "team" { json!(team) } else { Value::Null },
        "can_team": facts.can_team,
        "estimate": estimate::estimate(est, f.project, meta["size"].as_str(), mode),
        "state": f.state.as_str(),
        "excerpt": excerpt(f.body),
    })
}

struct ItemFields<'a> {
    /// `None` in a preview (nothing stored).
    id: Option<u64>,
    project: &'a str,
    source: &'a str,
    reference: &'a str,
    title: &'a str,
    url: Option<&'a str>,
    body: &'a str,
    meta: &'a Value,
    state: IntakeState,
}

impl<'a> ItemFields<'a> {
    fn of_row(r: &'a IntakeRow) -> ItemFields<'a> {
        ItemFields {
            id: Some(r.id),
            project: &r.project_id,
            source: &r.source,
            reference: &r.reference,
            title: &r.title,
            url: r.url.as_deref(),
            body: &r.body,
            meta: &r.meta,
            state: r.state,
        }
    }

    fn of_upsert(u: &'a orch_store::IntakeUpsert, project: &'a str, source: &'a str) -> ItemFields<'a> {
        ItemFields {
            id: None,
            project,
            source,
            reference: &u.reference,
            title: &u.title,
            url: u.url.as_deref(),
            body: &u.body,
            meta: &u.meta,
            state: IntakeState::Ready,
        }
    }
}

/// The source's one-line label (`acme/shop · label ready`).
fn source_label(s: &IntakeSource, repo: Option<&str>, base: &str) -> String {
    match s {
        IntakeSource::None => "none".into(),
        IntakeSource::Github { filter, .. } => {
            let repo = repo.unwrap_or("GitHub");
            match filter {
                GithubIntakeFilter::Label { names } => format!("{repo} · label {}", names.join(" + ")),
                GithubIntakeFilter::Milestone { title } => format!("{repo} · milestone {title}"),
                GithubIntakeFilter::Project { owner, number, status } => match status {
                    Some(st) => format!("{repo} · project {owner}/{number} · {st}"),
                    None => format!("{repo} · project {owner}/{number}"),
                },
            }
        }
        IntakeSource::Linear { team, label, include_backlog, project, .. } => {
            let mut l = format!("Linear {team}");
            if project.is_some() {
                l.push_str(" · project");
            }
            if let Some(lb) = label {
                l.push_str(&format!(" · label {lb}"));
            }
            if *include_backlog {
                l.push_str(" · with backlog");
            }
            l
        }
        IntakeSource::RepoDocs { glob, branch } => format!("{glob} on {}", branch.as_deref().unwrap_or(base)),
    }
}

/// The source's list in its own UI (F10); `None` for repo docs.
fn source_url(s: &IntakeSource, repo: Option<&str>, listed: Option<&str>) -> Option<String> {
    match s {
        IntakeSource::Github { filter, .. } => {
            let q = |s: &str| src::enc(&s.replace('"', ""));
            Some(match filter {
                GithubIntakeFilter::Label { names } => {
                    let labels: String = names.iter().map(|n| format!("+label:%22{}%22", q(n))).collect();
                    format!("https://github.com/{}/issues?q=is:issue+is:open{labels}", repo?)
                }
                GithubIntakeFilter::Milestone { title } => {
                    format!("https://github.com/{}/issues?q=is:issue+is:open+milestone:%22{}%22", repo?, q(title))
                }
                // GitHub says whether the board is an organization's or a user's.
                GithubIntakeFilter::Project { .. } => listed?.to_string(),
            })
        }
        // The workspace slug, the team key and the project slug come with the issues.
        IntakeSource::Linear { .. } => listed.map(str::to_string),
        _ => None,
    }
}

impl Runtime {
    /// Starts the refresher thread (orchd and the app alike).
    pub fn spawn_intake(rt: Arc<Runtime>, stop: Arc<AtomicBool>) -> std::io::Result<()> {
        std::thread::Builder::new().name("orch-intake".into()).spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                rt.intake_step(orch_protocol::now_ms());
                let g = lock(&rt.intake.sched);
                let _ = rt.intake.wake.wait_timeout(g, STEP);
            }
        })?;
        Ok(())
    }

    fn refresh_ms(&self) -> u64 {
        let s = self.db.setting(REFRESH_SECS_KEY).ok().flatten().and_then(|v| v.as_u64());
        s.unwrap_or(DEFAULT_REFRESH_SECS).clamp(60, 3600) * 1000
    }

    /// Projects with a source, by id.
    fn intake_projects(&self) -> Vec<(Project, IntakeSource)> {
        self.db
            .projects(false)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| {
                let s = self.db.intake_source(&p.id).ok()?;
                (!s.is_none()).then_some((p, s))
            })
            .collect()
    }

    /// One pass: refreshes every project that is due. The first pass lays
    /// the start-up refreshes out 2 s apart.
    pub fn intake_step(&self, now: u64) {
        let projects = self.intake_projects();
        // Repo-docs branch tips, once a minute (local, no network). Not on the
        // first pass: the start-up stagger reads them all anyway.
        let tips = {
            let mut s = lock(&self.intake.sched);
            let due = s.started && now.saturating_sub(s.last_tip_ms) >= TIP_EVERY_MS;
            if due || !s.started {
                s.last_tip_ms = now;
            }
            due
        };
        if tips {
            self.tip_check_of(&projects);
            // The rooms move with the week; the history with the calibration.
            self.intake_estimates_update(false);
        }
        let due: Vec<String> = {
            let mut s = lock(&self.intake.sched);
            let ids: BTreeSet<&str> = projects.iter().map(|(p, _)| p.id.as_str()).collect();
            s.due.retain(|k, _| ids.contains(k.as_str()));
            let first = !s.started;
            s.started = true;
            for (i, (p, _)) in projects.iter().enumerate() {
                let at = if first { now + i as u64 * STAGGER_MS } else { now };
                s.due.entry(p.id.clone()).or_insert(at);
            }
            s.due.iter().filter(|(k, t)| **t <= now && !s.inflight.contains(*k)).map(|(k, _)| k.clone()).collect()
        };
        for id in due {
            self.intake_refresh_project(&id);
        }
    }

    /// Asks for a refresh of `project` (all when `None`) at the next pass.
    pub fn intake_request(&self, project: Option<&str>) {
        let mut s = lock(&self.intake.sched);
        match project {
            Some(p) => {
                s.due.insert(p.to_string(), 0);
            }
            None => s.due.values_mut().for_each(|t| *t = 0),
        }
        drop(s);
        self.intake.wake.notify_all();
    }

    /// A repo-docs project whose branch tip moved since its items were
    /// read is refreshed (no network).
    pub fn intake_tip_check(&self) {
        self.tip_check_of(&self.intake_projects());
    }

    fn tip_check_of(&self, projects: &[(Project, IntakeSource)]) {
        for (p, s) in projects {
            let IntakeSource::RepoDocs { branch, .. } = &s else { continue };
            let branch = branch.clone().unwrap_or_else(|| base_branch(p));
            let tip = src::branch_tip(&p.path, &branch);
            let read = self.db.intake_status(&p.id).map(|st| st.tip).unwrap_or_default();
            if tip.is_some() && tip != read {
                self.intake_request(Some(&p.id));
            }
        }
    }

    /// The project's GitHub repository: the source's, else the known one,
    /// else `gh repo view` (kept).
    fn intake_repo(&self, p: &Project, configured: Option<&str>) -> Result<String, SourceError> {
        if let Some(r) = configured.map(str::to_string).or_else(|| crate::github_issue::known_repo(p)) {
            return Ok(r);
        }
        if let Some(r) = self.issue_repos.lock().unwrap_or_else(|e| e.into_inner()).get(&p.id) {
            return Ok(r.clone());
        }
        let r = src::gh_repo(self.gh_in(p).as_ref())?;
        self.issue_repos.lock().unwrap_or_else(|e| e.into_inner()).insert(p.id.clone(), r.clone());
        Ok(r)
    }

    /// Lists `source` for `p` (no store write): the listing and, for repo
    /// docs, the tip it was read at.
    fn intake_fetch(&self, p: &Project, source: &IntakeSource) -> Result<(Listing, Option<String>), SourceError> {
        match source {
            IntakeSource::None => Ok((Listing::default(), None)),
            IntakeSource::Github { repo, filter } => {
                let repo = self.intake_repo(p, repo.as_deref())?;
                Ok((src::github(self.gh_in(p).as_ref(), &repo, filter)?, None))
            }
            IntakeSource::Linear { team, project, label, include_backlog } => {
                let q = LinearQuery {
                    team,
                    project: project.as_deref(),
                    label: label.as_deref(),
                    include_backlog: *include_backlog,
                };
                let gql = self.linear_transport();
                Ok((src::linear(gql.as_ref(), &self.intake.backoff(&p.id), orch_protocol::now_ms(), &q)?, None))
            }
            IntakeSource::RepoDocs { glob, branch } => {
                let branch = branch.clone().unwrap_or_else(|| base_branch(p));
                let (l, tip) = src::repo_docs(&p.path, glob, &branch)?;
                Ok((l, Some(tip)))
            }
        }
    }

    /// Refreshes one project now, unless one is in flight (false then). The
    /// items are written only when the source did not change meanwhile.
    pub fn intake_refresh_project(&self, id: &str) -> bool {
        {
            let mut s = lock(&self.intake.sched);
            if !s.inflight.insert(id.to_string()) {
                return false;
            }
            // Not due while in flight: a request made meanwhile sets 0 again
            // (the request that started this one is answered by it).
            s.due.insert(id.to_string(), u64::MAX);
        }
        // The lane shows "refreshing".
        self.db.note([TOPIC]);
        self.intake_refresh_inner(id);
        let mut s = lock(&self.intake.sched);
        s.inflight.remove(id);
        let next = orch_protocol::now_ms() + self.refresh_ms();
        // A request made during the refresh stays due.
        if s.due.get(id) != Some(&0) {
            s.due.insert(id.to_string(), next);
        }
        drop(s);
        self.intake.wake.notify_all();
        self.db.note([TOPIC]);
        true
    }

    fn intake_refresh_inner(&self, id: &str) {
        let Ok(Some(p)) = self.db.project(id) else { return };
        let Ok(source) = self.db.intake_source(id) else { return };
        if source.is_none() {
            return;
        }
        let t0 = std::time::Instant::now();
        let mut status = self.db.intake_status(id).unwrap_or_default();
        // Repo docs at the tip already read: nothing can have changed (the
        // glob is the stored source's; a source change clears the status).
        if let IntakeSource::RepoDocs { branch, .. } = &source {
            let branch = branch.clone().unwrap_or_else(|| base_branch(&p));
            if status.error.is_none() && status.tip.is_some() && status.tip == src::branch_tip(&p.path, &branch) {
                status.refreshed_at_ms = Some(orch_protocol::now_ms());
                let _ = self.db.set_intake_status(id, &status);
                return;
            }
        }
        let got = self.intake_fetch(&p, &source);
        // Changed while we listed: this answer is the old source's.
        if self.db.intake_source(id).ok().as_ref() != Some(&source) {
            self.intake_request(Some(id));
            return;
        }
        let now = orch_protocol::now_ms();
        match got {
            Ok((listing, tip)) => {
                let rows = listing.items.iter().map(Fetched::upsert).collect();
                if let Err(e) = self.db.intake_apply(id, source.kind(), rows, now) {
                    status.error = Some(SourceError::new("failed", format!("{e:#}")).json());
                } else {
                    status.error = None;
                    status.refreshed_at_ms = Some(now);
                    status.more = listing.more();
                    status.partial = listing.partial;
                    status.tip = tip;
                    // An empty answer has no link to give: the last one stays.
                    status.url = listing.url.or(status.url.take());
                    self.intake_estimates_update(true);
                    self.event(
                        "-",
                        "intake.refreshed",
                        json!({"project": id, "source": source.kind(), "count": listing.items.len(),
                               "ms": t0.elapsed().as_millis() as u64}),
                    );
                }
            }
            Err(e) => {
                let same = status.error.as_ref().is_some_and(|o| o["code"] == e.code);
                if !same {
                    self.event("-", "intake.error", json!({"project": id, "code": e.code}));
                }
                status.error = Some(e.json());
            }
        }
        let _ = self.db.set_intake_status(id, &status);
    }

    /// Refreshes now, waiting for one in flight to end first (a source
    /// change must not be answered by the old source's refresh).
    fn intake_refresh_now(&self, id: &str) {
        let deadline = std::time::Instant::now() + WAIT_INFLIGHT;
        // Retried: the refresher thread can take the slot between the wait
        // and our own start.
        while !self.intake_refresh_project(id) && std::time::Instant::now() < deadline {
            let s = lock(&self.intake.sched);
            if s.inflight.contains(id) {
                let _ = self.intake.wake.wait_timeout(s, Duration::from_millis(100));
            }
        }
    }

    /// The project's source as one line (`acme/shop · label ready`), `None` without one.
    pub(crate) fn intake_label(&self, p: &Project) -> Option<String> {
        let source = self.db.intake_source(&p.id).ok().filter(|s| !s.is_none())?;
        let repo = self.intake_repo_known(p, &source);
        Some(source_label(&source, repo.as_deref(), &base_branch(p)))
    }

    /// A GitHub source's repository without asking `gh` (configured, known, or seen).
    fn intake_repo_known(&self, p: &Project, source: &IntakeSource) -> Option<String> {
        match source {
            IntakeSource::Github { repo, .. } => repo
                .clone()
                .or_else(|| crate::github_issue::known_repo(p))
                .or_else(|| self.issue_repos.lock().unwrap_or_else(|e| e.into_inner()).get(&p.id).cloned()),
            _ => None,
        }
    }

    fn project_facts(&self, p: &Project) -> ProjectFacts {
        let default_team = p.typed_settings().default_team.filter(|t| !t.is_empty());
        let can_team = default_team.is_some() || self.db.teams(Some(&p.id)).map(|t| !t.is_empty()).unwrap_or(false);
        ProjectFacts { default_team, can_team }
    }

    /// `now.intake` (api-contract §2.1, F9, F10, F17): the ready and hidden
    /// items of every project (capped per project) and one source row per
    /// registered project.
    pub fn intake_view(&self) -> Value {
        let projects = self.db.projects(false).unwrap_or_default();
        let rows = self.db.intake_listed(None).unwrap_or_default();
        let inflight = lock(&self.intake.sched).inflight.clone();
        let est = self.intake_est();
        let mut items = vec![];
        let mut sources = vec![];
        for p in &projects {
            let source = self.db.intake_source(&p.id).unwrap_or_default();
            if source.is_none() {
                sources.push(json!({"project": p.id, "type": "none", "label": "none", "status": "none",
                    "error": null, "refreshed_at_ms": null, "url": null, "count": 0, "more": 0, "partial": false, "hidden": 0}));
                continue;
            }
            let facts = self.project_facts(p);
            let mine: Vec<&IntakeRow> =
                rows.iter().filter(|r| r.project_id == p.id && r.source == source.kind()).collect();
            let ready: Vec<&&IntakeRow> = mine.iter().filter(|r| r.state == IntakeState::Ready).collect();
            let hidden: Vec<&&IntakeRow> = mine.iter().filter(|r| r.state == IntakeState::Hidden).collect();
            let status = self.db.intake_status(&p.id).unwrap_or_default();
            let shown = ready.len().min(src::CAP);
            for r in ready.iter().take(src::CAP).chain(hidden.iter()) {
                items.push(item_json(&ItemFields::of_row(r), &facts, &est));
            }
            let repo = self.intake_repo_known(p, &source);
            let state = if inflight.contains(&p.id) {
                "refreshing"
            } else if status.error.is_some() {
                "error"
            } else {
                "ok"
            };
            sources.push(json!({
                "project": p.id,
                "type": source.kind(),
                "label": source_label(&source, repo.as_deref(), &base_branch(p)),
                "status": state,
                "error": status.error,
                "refreshed_at_ms": status.refreshed_at_ms,
                "url": source_url(&source, repo.as_deref(), status.url.as_deref()),
                "count": shown,
                "more": ready.len().saturating_sub(shown) + status.more,
                "partial": status.partial,
                "hidden": hidden.len(),
            }));
        }
        json!({"items": items, "sources": sources})
    }

    // ------------------------------------------------------------ commands

    fn intake_project(&self, project: &str) -> Result<Project> {
        self.db.project(project)?.ok_or_else(|| anyhow!("no project {project}"))
    }

    /// `intake_source`: the stored source and the defaults the form shows.
    pub fn intake_source(&self, project: &str) -> Result<Value> {
        let p = self.intake_project(project)?;
        let s = self.db.intake_source(project)?;
        Ok(json!({
            "source": (!s.is_none()).then_some(s),
            "default_glob": orch_store::DEFAULT_INTAKE_GLOB,
            "default_branch": base_branch(&p),
            "known_repo": crate::github_issue::known_repo(&p),
        }))
    }

    /// `intake_source_set`: validates, stores, refreshes at once and
    /// returns the preview of what the cache now holds.
    pub fn intake_source_set(&self, project: &str, source: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let s = parse_source(source)?;
        self.db.set_intake_source(&p.id, &s)?;
        if !s.is_none() {
            self.intake_refresh_now(&p.id);
        }
        let status = self.db.intake_status(&p.id)?;
        let facts = self.project_facts(&p);
        let rows: Vec<IntakeRow> = self.db.intake_listed(Some(&p.id))?;
        let ready: Vec<&IntakeRow> = rows.iter().filter(|r| r.state == IntakeState::Ready).collect();
        let more = ready.len().saturating_sub(src::CAP) + status.more;
        let est = self.intake_est();
        let items: Vec<Value> =
            ready.iter().take(PREVIEW).map(|r| item_json(&ItemFields::of_row(r), &facts, &est)).collect();
        Ok(
            json!({"preview": {"count": ready.len().min(src::CAP), "more": more, "partial": status.partial, "items": items, "error": status.error}}),
        )
    }

    /// `intake_preview`: a dry run of `source` (nothing stored).
    pub fn intake_preview(&self, project: &str, source: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let s = parse_source(source)?;
        let facts = self.project_facts(&p);
        let est = self.intake_est();
        Ok(match self.intake_fetch(&p, &s) {
            Ok((l, _)) => {
                let items: Vec<Value> = l
                    .items
                    .iter()
                    .take(PREVIEW)
                    .map(|f| item_json(&ItemFields::of_upsert(&f.upsert(), &p.id, s.kind()), &facts, &est))
                    .collect();
                json!({"count": l.items.len(), "more": l.more(), "partial": l.partial, "items": items, "error": null})
            }
            Err(e) => json!({"count": 0, "more": 0, "partial": false, "items": [], "error": e.json()}),
        })
    }

    /// `intake_options`: what the settings form offers for a (partial,
    /// F23) source: GitHub `repo`, Linear `team`, repo docs `glob`/`branch`.
    pub fn intake_options(&self, project: &str, source: &Value) -> Result<Value> {
        let p = self.intake_project(project)?;
        let str_of = |k: &str| source[k].as_str().map(str::trim).filter(|s| !s.is_empty());
        let err = |e: SourceError| json!({"error": e.json()});
        Ok(match source["type"].as_str().unwrap_or("") {
            "github" => {
                let repo = str_of("repo");
                if let Some(r) = repo
                    && !orch_store::valid_repo(r)
                {
                    return Err(bad_request("repo", "the repository must be owner/name"));
                }
                match self.intake_repo(&p, repo) {
                    Ok(repo) => src::github_options(self.gh_in(&p).as_ref(), &repo).unwrap_or_else(err),
                    Err(e) => err(e),
                }
            }
            "linear" => {
                let team = str_of("team").ok_or_else(|| bad_request("team", "choose a Linear team"))?;
                let gql = self.linear_transport();
                src::linear_options(gql.as_ref(), &self.intake.backoff(&p.id), orch_protocol::now_ms(), team)
                    .unwrap_or_else(err)
            }
            "repo_docs" => {
                let glob = str_of("glob").unwrap_or(orch_store::DEFAULT_INTAKE_GLOB);
                parse_source(&json!({"type": "repo_docs", "glob": glob}))?;
                let branch = str_of("branch").map(str::to_string).unwrap_or_else(|| base_branch(&p));
                match src::repo_docs_matches(&p.path, glob, &branch) {
                    Ok(n) => json!({"matches": n}),
                    Err(e) => json!({"matches": 0, "error": e.json()}),
                }
            }
            other => return Err(bad_request("type", format!("no options for source type {other:?}"))),
        })
    }

    /// `intake_refresh`: one project, or every project with a source.
    pub fn intake_refresh(&self, project: Option<&str>) -> Result<Value> {
        if let Some(p) = project {
            self.intake_project(p)?;
        }
        self.intake_request(project);
        Ok(json!({}))
    }

    fn intake_row(&self, item: &str) -> Result<IntakeRow> {
        self.db.intake_item(item_id(item)?)?.ok_or_else(|| anyhow!("no intake item {item:?}"))
    }

    /// `intake_item`: the item with its full body (F5 extras).
    pub fn intake_item(&self, item: &str) -> Result<Value> {
        let r = self.intake_row(item)?;
        let p = self.intake_project(&r.project_id)?;
        let m = &r.meta;
        Ok(json!({
            "item": item_json(&ItemFields::of_row(&r), &self.project_facts(&p), &self.intake_est()),
            "body": r.body,
            "truncated": m["truncated"].as_bool().unwrap_or(false),
            "milestone": m["milestone"],
            "priority": priority_label(&m["priority"]),
            "assignees": m["assignees"].as_array().cloned().unwrap_or_default(),
            "points": m["estimate"],
        }))
    }

    /// The estimate of a cached item, as `now.intake` shows it.
    pub(crate) fn intake_estimate_of(&self, r: &IntakeRow) -> Value {
        let Ok(p) = self.intake_project(&r.project_id) else { return Value::Null };
        let mode = suggested_mode(&r.meta, &self.project_facts(&p));
        estimate::estimate(&self.intake_est(), &r.project_id, r.meta["size"].as_str(), mode)
    }

    pub fn intake_hide(&self, item: &str) -> Result<Value> {
        let r = self.intake_row(item)?;
        self.db.intake_hide(r.id, orch_protocol::now_ms())?;
        Ok(json!({}))
    }

    pub fn intake_unhide(&self, item: &str) -> Result<Value> {
        let r = self.intake_row(item)?;
        self.db.intake_unhide(r.id)?;
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test]
fn excerpts_keep_identifiers_and_numbers() {
    assert_eq!(
        excerpt("# Title\n- Rename user_id\n-5% latency\n**bold** `code`"),
        "Title Rename user_id -5% latency bold code"
    );
}
