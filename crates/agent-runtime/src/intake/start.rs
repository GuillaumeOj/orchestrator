//! Starting from an intake item or a planned brief (P4.5 spec §3.3, §3.5,
//! api-contract §3.2, D7): the dialogs are prefilled by the core
//! (`intake_prefill`); the user's submit carries `intake {item |
//! brief_decision}`, checked before the start (`stale`) and recorded after
//! it (the item is started, the brief card closes as launched). A run
//! deleted or a session killed before any outcome gives its item back.
//! Every other start is `intake.untracked_start`. Nothing here starts
//! anything: the caller does, on the user's submit.

use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use orch_store::{INTAKE_BRIEF_SOURCE, InboxItem, IntakeRow, IntakeState, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::item_id;
use crate::runtime::Runtime;

/// `intake` on `LaunchSpec` and `NewSession`: at most one set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief_decision: Option<String>,
    /// A node of the tracker tree (`gh:acme/shop#13`), started from Work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl IntakeLink {
    /// The link a start carries: its `intake`, or a bare `source` node id.
    pub fn of(intake: Option<&IntakeLink>, source: Option<&str>) -> Option<IntakeLink> {
        match (intake, source) {
            (Some(l), _) => Some(l.clone()),
            (None, Some(s)) => Some(IntakeLink { source: Some(s.to_string()), ..Default::default() }),
            (None, None) => None,
        }
    }
}

/// Where started runs and sessions came from, by run id and agent id.
#[derive(Default)]
pub(crate) struct Origins {
    pub runs: std::collections::BTreeMap<String, Value>,
    pub agents: std::collections::BTreeMap<String, Value>,
    /// `run.source` (contract §2, F23): the tree node, by run id and agent id.
    pub src_runs: std::collections::BTreeMap<String, Value>,
    pub src_agents: std::collections::BTreeMap<String, Value>,
}

/// What a start claimed, held until it succeeded or failed.
/// `None`: an untracked start.
pub struct Claim<'a> {
    rt: &'a Runtime,
    held: Option<(String, Claimed)>,
}

enum Claimed {
    Item(IntakeRow),
    Brief(InboxItem),
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if let Some((k, _)) = &self.held {
            self.rt.intake.claims.lock().unwrap_or_else(|e| e.into_inner()).remove(k);
        }
    }
}

/// Releases `key` when dropped before the claim holds it (a failed check).
struct KeyGuard<'a>(&'a Runtime, Option<String>);

impl Drop for KeyGuard<'_> {
    fn drop(&mut self) {
        if let Some(k) = self.1.take() {
            self.0.intake.claims.lock().unwrap_or_else(|e| e.into_inner()).remove(&k);
        }
    }
}

/// A command error whose text is the JSON `{code: "stale", message, …}`.
pub(super) fn stale(message: &str, started: Option<Value>) -> anyhow::Error {
    let mut v = json!({"code": "stale", "message": message});
    if let Some(s) = started {
        v["started"] = s;
    }
    anyhow!(v.to_string())
}

fn bad(field: &str, message: impl Into<String>) -> anyhow::Error {
    super::bad_request(field, message)
}

/// `brief-12` → 12.
pub(crate) fn brief_n(id: &str) -> Result<u64> {
    id.strip_prefix("brief-").and_then(|n| n.parse().ok()).ok_or_else(|| anyhow!("no brief decision {id:?}"))
}

/// Lower-case kebab of `s`, at most `max` chars (`""` when nothing is left).
pub fn kebab(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.chars().take(max).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// `base`, else `base-2`, `base-3`… (within `max`) while `taken`.
pub(crate) fn unique(base: &str, max: usize, taken: impl Fn(&str) -> bool) -> String {
    let base = if base.is_empty() { "work" } else { base };
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let head: String = base.chars().take(max.saturating_sub(suffix.len())).collect();
            format!("{}{suffix}", head.trim_end_matches('-'))
        })
        .find(|c| !taken(c))
        .expect("an unbounded range")
}

/// The item as the brief the user submits (spec §3.3): title, source
/// link, body. A GitHub `#n` stays in the text so the lead reads the issue.
pub fn item_brief(r: &IntakeRow) -> String {
    let link = r.url.as_deref().map(|u| format!(" · {u}")).unwrap_or_default();
    let from = match r.source.as_str() {
        "github" => format!("GitHub issue {}{link}", r.reference),
        "linear" => format!("Linear issue {}{link}", r.reference),
        _ => format!("{} (repo doc){link}", r.reference),
    };
    let body = r.body.trim();
    let cut = if r.meta["truncated"].as_bool() == Some(true) { "\n\n(cut: read the source for the rest)" } else { "" };
    format!("# {}\n\nFrom {from}\n\n{body}{cut}", r.title.trim()).trim_end().to_string()
}

impl Runtime {
    /// A feature slug for `title`, free in `project`.
    pub(crate) fn intake_slug(&self, project: &str, title: &str) -> String {
        // `solo` is not a feature slug (`launch` refuses it).
        unique(&kebab(title, 40), 40, |s| s == "solo" || self.db.feature_by_slug(project, s).ok().flatten().is_some())
    }

    /// A solo session name for `title`, free among the agents.
    pub(crate) fn intake_agent_name(&self, title: &str) -> String {
        let ids: BTreeSet<String> = self.store.read(|s| s.agents.keys().cloned().collect());
        unique(&kebab(title, 30), 30, |s| ids.contains(s) || crate::overwatch::refuse_reserved(s).is_err())
    }

    /// The team a run of `p` gets: `wanted` when it exists, else the
    /// project's default team, else its first team.
    pub(crate) fn intake_team(&self, p: &Project, wanted: Option<&str>) -> Option<String> {
        let exists = |t: &str| self.db.team_in(Some(&p.id), t).ok().flatten().is_some();
        wanted
            .filter(|t| exists(t))
            .map(str::to_string)
            .or_else(|| p.typed_settings().default_team.filter(|t| !t.is_empty() && exists(t)))
            .or_else(|| self.db.teams(Some(&p.id)).ok()?.into_iter().next().map(|t| t.id))
    }

    /// The brief decision `brief-<n>`, open or closed.
    pub(super) fn find_brief(&self, id: &str) -> Result<InboxItem> {
        let n = brief_n(id)?;
        let item = self.store.read(|s| s.inbox.iter().find(|i| i.id == n && i.kind == "brief").cloned());
        item.ok_or_else(|| anyhow!("no brief decision {id:?}"))
    }

    fn open_brief(&self, id: &str) -> Result<InboxItem> {
        let item = self.find_brief(id)?;
        if !item.open() {
            return Err(stale("brief closed", None));
        }
        Ok(item)
    }

    /// Why a row can't be started now: `stale` with what started it (F11, F12).
    fn ready_row(&self, item: &str) -> Result<IntakeRow> {
        let r = self.intake_row(item)?;
        match r.state {
            IntakeState::Ready => Ok(r),
            IntakeState::Hidden => Err(stale("not ready", None)),
            IntakeState::Started => {
                let kind = r.started_kind.clone().unwrap_or_default();
                let reference = r.started_ref.clone().unwrap_or_default();
                let name = match kind.as_str() {
                    "run" => self
                        .db
                        .run(&reference)
                        .ok()
                        .flatten()
                        .and_then(|run| self.db.feature(&run.feature_id).ok().flatten())
                        .map_or(reference.clone(), |f| f.slug),
                    _ => reference.clone(),
                };
                Err(stale("already started", Some(json!({"kind": kind, "name": name}))))
            }
        }
    }

    /// `intake_prefill(item | brief_decision, mode)` (api-contract §3.2): the
    /// launch or new-session form, built by the core.
    pub fn intake_prefill(&self, item: Option<&str>, brief: Option<&str>, mode: Option<&str>) -> Result<Value> {
        let (project, title, text, suggested, team, slug, url) = match (item, brief) {
            (Some(i), None) => {
                let r = self.ready_row(i)?;
                let p = self.intake_project(&r.project_id)?;
                let facts = self.project_facts(&p);
                let suggested = super::suggested_mode(&r.meta, &facts).to_string();
                let wanted = r.meta["team"].as_str().map(str::to_string);
                let url = r.url.clone().unwrap_or_else(|| r.reference.clone());
                (p, r.title.clone(), item_brief(&r), suggested, wanted, None, Some(url))
            }
            (None, Some(b)) => {
                let d = self.open_brief(b)?;
                let pl = &d.payload;
                let p = self.intake_project(pl["project"].as_str().unwrap_or(""))?;
                let s = |k: &str| pl[k].as_str().map(str::to_string);
                (
                    p,
                    s("title").unwrap_or_default(),
                    s("brief").unwrap_or_default(),
                    s("mode").unwrap_or_else(|| "team".into()),
                    s("team"),
                    s("slug"),
                    None,
                )
            }
            _ => bail!(bad("item", "give an item or a brief decision")),
        };
        self.prefill_forms(&project, &title, &text, &suggested, team.as_deref(), slug, url, mode)
    }

    /// The launch or new-session form of mode `mode` (else the `suggested` one).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prefill_forms(
        &self,
        project: &Project,
        title: &str,
        text: &str,
        suggested: &str,
        team: Option<&str>,
        slug: Option<String>,
        url: Option<String>,
        mode: Option<&str>,
    ) -> Result<Value> {
        let mode = mode.unwrap_or(suggested);
        Ok(match mode {
            "team" => {
                // A brief's slug stays unless an open feature took it since
                // (an archived one comes back on launch, as `propose_brief` allows).
                let slug = slug
                    .filter(|s| self.db.feature_by_slug(&project.id, s).ok().flatten().is_none_or(|f| f.archived))
                    .unwrap_or_else(|| self.intake_slug(&project.id, title));
                json!({"launch": {
                    "project_id": project.id,
                    "team_id": self.intake_team(project, team),
                    "slug": slug, "title": title, "brief": text, "base": null, "off_hours": false,
                }})
            }
            "solo" => {
                let done = url.map(|u| format!("\n\nWhen done, say so; the item is {u}")).unwrap_or_default();
                json!({"session": {
                    "project_id": project.id,
                    "name": self.intake_agent_name(title),
                    "message": format!("Brief from the user:\n\n{text}{done}"),
                }})
            }
            other => bail!(bad("mode", format!("mode is team or solo, not {other:?}"))),
        })
    }

    /// The node `node` of `project` (fresh from the store).
    pub(crate) fn node_row(&self, project: &str, node: &str) -> Result<IntakeRow> {
        self.db
            .work_node_rows(node)?
            .into_iter()
            .find(|r| r.project_id == project)
            .ok_or_else(|| anyhow!(json!({"code": "not_found", "message": format!("no node {node:?}")}).to_string()))
    }

    /// Whether the run a node was started from has merged (the node may run again).
    pub(crate) fn node_ran_before(&self, r: &IntakeRow) -> bool {
        r.state == IntakeState::Started
            && r.started_kind.as_deref() == Some("run")
            && r.started_ref
                .as_deref()
                .and_then(|id| self.db.run(id).ok().flatten())
                .is_some_and(|run| run.status == orch_team::RunState::Done)
    }

    /// Why a node can't be started now (`stale`), else the node: an issue or
    /// a backlog file that is not running (a hidden one, or one whose run
    /// merged, can start). Closed upstream is the UI's disabled action, not a refusal.
    pub(crate) fn startable_row(&self, r: &IntakeRow) -> Result<IntakeRow> {
        if !matches!(r.kind.as_str(), "issue" | "local") {
            bail!(bad("intake", format!("{} is not an item to start: run it as a program", r.reference)));
        }
        match r.state {
            IntakeState::Ready | IntakeState::Hidden => Ok(r.clone()),
            IntakeState::Started if self.node_ran_before(r) => Ok(r.clone()),
            IntakeState::Started => self.ready_row(&format!("it_{}", r.id)),
        }
    }

    /// Before a start: no other start holds the item (or brief), then it is
    /// ready (or open). Held until the returned claim is dropped; taken
    /// before the check, so a start that just finished is seen as started.
    pub(crate) fn intake_claim(&self, project: &str, link: Option<&IntakeLink>) -> Result<Claim<'_>> {
        enum Link<'l> {
            Item(&'l str),
            Brief(&'l str),
            Node(Box<IntakeRow>),
        }
        let link = match link.map(|l| (l.item.as_deref(), l.brief_decision.as_deref(), l.source.as_deref())) {
            None | Some((None, None, None)) => return Ok(Claim { rt: self, held: None }),
            Some((Some(i), None, None)) => Link::Item(i),
            Some((None, Some(b), None)) => Link::Brief(b),
            Some((None, None, Some(n))) => Link::Node(Box::new(self.node_row(project, n)?)),
            Some(_) => bail!(bad("intake", "an item, a brief decision or a source, not several")),
        };
        let key = match &link {
            Link::Item(i) => format!("it_{}", item_id(i)?),
            Link::Brief(b) => crate::decision::decision_id("brief", brief_n(b)?),
            Link::Node(r) => format!("it_{}", r.id),
        };
        if !self.intake.claims.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) {
            return Err(stale("already started", None));
        }
        // Released on any error below.
        let mut guard = KeyGuard(self, Some(key));
        let what = match link {
            Link::Item(i) => {
                let r = self.ready_row(i)?;
                if r.project_id != project {
                    bail!(bad("intake", format!("item {i} is not of project {project}")));
                }
                Claimed::Item(r)
            }
            Link::Node(r) => Claimed::Item(self.startable_row(&r)?),
            Link::Brief(b) => {
                let d = self.open_brief(b)?;
                if d.payload["project"].as_str() != Some(project) {
                    bail!(bad("intake", format!("{b} is not a brief of project {project}")));
                }
                Claimed::Brief(d)
            }
        };
        let key = guard.1.take().expect("held");
        Ok(Claim { rt: self, held: Some((key, what)) })
    }

    /// After a successful start of `kind` (`run` | `agent`) `reference`
    /// (`name`: the feature slug or the agent): the item is started, the
    /// brief closes as launched, or the start is untracked.
    pub(crate) fn intake_started(&self, claim: Claim, project: &str, kind: &str, reference: &str, name: &str) {
        let mode = if kind == "run" { "team" } else { "solo" };
        match claim.held.as_ref().map(|(_, w)| w) {
            None => {
                self.event("-", "intake.untracked_start", json!({"project": project, "kind": kind, "ref": name}));
            }
            Some(Claimed::Item(r)) => {
                // A node that ran before (its run merged) starts again: the new run is its mark.
                let started = match self.db.intake_mark_started(r.id, kind, reference) {
                    Ok(false) => self.db.intake_restart(r.id, kind, reference),
                    other => other,
                };
                match started {
                    Ok(true) => {
                        self.work_log(r, "started", (kind == "run").then_some(name), None);
                        // Written back to the tracker item (label or Status, comment, assignee).
                        self.wb_spawn(
                            project,
                            &r.node_id,
                            crate::writeback::WbOp::Started { feature: name.to_string() },
                        );
                    }
                    // Gone upstream while the start ran: no origin to keep.
                    Ok(false) => {
                        eprintln!("warn: intake item it_{} was no longer ready when {kind} {reference} started", r.id)
                    }
                    Err(e) => eprintln!("warn: intake item it_{} not marked started: {e:#}", r.id),
                }
                let est = self.intake_estimate_of(r);
                let est_pp = est[mode]["low_pp"]
                    .as_f64()
                    .zip(est[mode]["high_pp"].as_f64())
                    .map(|(l, h)| ((l + h) / 2.0 * 10.0).round() / 10.0);
                self.event(
                    "-",
                    "intake.started",
                    json!({"item": format!("it_{}", r.id), "project": project, "source": r.source, "mode": mode,
                           "ref": r.reference, "est_pp": est_pp, "started": reference}),
                );
            }
            Some(Claimed::Brief(d)) => {
                let id = crate::decision::decision_id("brief", d.id);
                let title = d.payload["title"].as_str().unwrap_or("");
                if let Err(e) =
                    self.db.intake_add_brief_start(project, &id, title, kind, reference, orch_protocol::now_ms())
                {
                    eprintln!("warn: brief {id} start not recorded: {e:#}");
                }
                self.brief_close(d.id, "launch", "launched", Some(name.to_string()), "user");
                self.event(
                    "-",
                    "intake.planned_started",
                    json!({"decision": id, "project": project, "mode": mode, "started": reference}),
                );
            }
        }
    }

    /// The run or session ended before any outcome: its item is ready again.
    fn intake_give_back(&self, kind: &str, reference: &str, why: &str) {
        match self.db.intake_return(kind, reference) {
            Ok(rows) => {
                for r in rows {
                    self.work_log(&r, "returned", None, Some(why));
                    self.event("-", "intake.returned", json!({"item": format!("it_{}", r.id), "why": why}));
                }
            }
            Err(e) => eprintln!("warn: intake items of {kind} {reference} not returned: {e:#}"),
        }
    }

    fn started_from_intake(&self, kind: &str, reference: &str) -> bool {
        self.db.intake_started_by(kind, reference).unwrap_or(false)
    }

    /// A run deleted by the user: its item comes back unless the run had an
    /// outcome (a task done, a merge, a PR, done).
    pub(crate) fn intake_on_run_deleted(&self, run: &str) {
        if !self.started_from_intake("run", run) {
            return;
        }
        let events = self.db.run_events(run, 5000).unwrap_or_default();
        let outcome = events.iter().any(|e| match e.kind.as_str() {
            "task.done" | "run.pr_opened" => true,
            "git.merge" => e.detail["result"]["outcome"] == "merged",
            "run.state" => e.detail["to"] == "done",
            _ => false,
        });
        if !outcome {
            self.intake_give_back("run", run, "run_deleted");
        }
    }

    /// A solo session killed (or deleted): its item comes back unless one of
    /// its turns finished.
    pub(crate) fn intake_on_session_killed(&self, agent: &str) {
        if !self.started_from_intake("agent", agent) {
            return;
        }
        let done = self.db.events(0, Some(agent), Some("turn.done"), 1).map(|e| !e.is_empty()).unwrap_or(false);
        if !done {
            self.intake_give_back("agent", agent, "session_killed");
        }
    }

    /// Where each started run and session came from (api-contract §2.3,
    /// F13, F14), by `(kind, ref)`.
    pub(crate) fn intake_origins(&self) -> Origins {
        let mut out = Origins::default();
        let started = self.db.intake_started().unwrap_or_default();
        // The breadcrumbs of the started nodes: one read of the trees' shape.
        let index = if started.iter().any(|r| r.source != INTAKE_BRIEF_SOURCE) {
            self.db.work_index().unwrap_or_default()
        } else {
            vec![]
        };
        let by_node: std::collections::HashMap<(&str, &str), &orch_store::WorkIndex> =
            index.iter().map(|n| ((n.project_id.as_str(), n.node_id.as_str()), n)).collect();
        let mut trackers: std::collections::HashMap<String, orch_store::TrackerSettings> = Default::default();
        let programs = self.program_of_starts();
        let mut statuses: std::collections::HashMap<String, std::collections::HashMap<String, Value>> =
            Default::default();
        for r in started {
            let (Some(kind), Some(reference)) = (r.started_kind.clone(), r.started_ref.clone()) else { continue };
            let v = if r.source == INTAKE_BRIEF_SOURCE {
                json!({"kind": "brief", "decision": r.reference, "title": r.title})
            } else {
                json!({"kind": "intake", "item": format!("it_{}", r.id), "source": r.source, "ref": r.reference,
                       "url": r.url})
            };
            let (tag, first) = programs.get(&reference).cloned().map_or((None, None), |(t, f)| (Some(t), Some(f)));
            let mut v = v;
            if let Some(t) = &tag {
                v["program"] = t.clone();
            }
            // (the source of a program's start is the feature's first item, not each item it runs)
            let source =
                (r.source != INTAKE_BRIEF_SOURCE && first.as_ref().is_none_or(|f| *f == r.node_id)).then(|| {
                    let chain = super::work::crumbs(&by_node, &r.project_id, r.parent_id.as_deref());
                    let t = trackers
                        .entry(r.project_id.clone())
                        .or_insert_with(|| self.db.tracker(&r.project_id).unwrap_or_default());
                    let mut s = super::work::source_json(&r, chain, t);
                    s["program"] = tag.clone().unwrap_or(Value::Null);
                    // The write-back state of the source node (contract §2).
                    s["writeback"] = statuses
                        .entry(r.project_id.clone())
                        .or_insert_with(|| self.wb_statuses(&r.project_id))
                        .get(&r.node_id)
                        .cloned()
                        .unwrap_or(Value::Null);
                    s
                });
            match kind.as_str() {
                "run" => {
                    out.src_runs.extend(source.map(|s| (reference.clone(), s)));
                    out.runs.insert(reference, v)
                }
                _ => {
                    out.src_agents.extend(source.map(|s| (reference.clone(), s)));
                    out.agents.insert(reference, v)
                }
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_and_unique_names() {
        assert_eq!(kebab("Refunds on partial orders!", 40), "refunds-on-partial-orders");
        assert_eq!(kebab("  Ünïcode — & co  ", 40), "n-code-co");
        assert_eq!(kebab("a very long title that goes past the limit", 12), "a-very-long");
        let taken = ["fix", "fix-2"];
        assert_eq!(unique("fix", 40, |s| taken.contains(&s)), "fix-3");
        assert_eq!(unique("", 40, |_| false), "work");
        assert_eq!(unique("abcdef", 6, |s| s == "abcdef"), "abcd-2");
    }
}
