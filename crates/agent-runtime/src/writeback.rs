//! Write-back (implement-improve-intake B5, contract §5, notes §10.4): what
//! the core writes to the tracker item a run or session came from: a label or
//! board Status and a comment when it starts, the team's tasks as sub-issues,
//! the item closed when its work merges, the root of a program when it is
//! done. Only when `write_back.enabled` and the tracker is GitHub or Linear,
//! only items of the project's tracker, every write audited (FR-709). A
//! failed write never stops a run: it shows on the node (`writeback`), in its
//! history and as an event, can be retried, and three in a row in a project
//! raise the `writeback_failing` decision.

use anyhow::{Result, anyhow};
use orch_store::{IntakeRow, Project, Tracker, TrackerSettings, WorkEvent};
use orch_tasks::intake::SourceError;
use orch_tasks::writeback as w;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// Failures in a row in one project that raise the decision.
const FAILING_AFTER: u64 = 3;

/// One write-back, as it is retried.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WbOp {
    /// A feature or session started from the node (`feature`: its slug or agent).
    Started { feature: String },
    /// Team task `task` (of a run started from the node) becomes a sub-issue of the node.
    TeamTask { task: u64 },
    /// The work of the node merged: close it, drop the running label.
    Merged,
    /// Every child of the program root is done: close the root.
    Parent,
}

impl WbOp {
    /// The word history and the status use.
    pub fn what(&self) -> &'static str {
        match self {
            WbOp::Started { .. } => "started",
            WbOp::TeamTask { .. } => "team_task",
            WbOp::Merged => "merged",
            WbOp::Parent => "parent_closed",
        }
    }
}

pub(crate) fn status_key(project: &str, node: &str) -> String {
    format!("work.writeback:{project}:{node}")
}

fn fails_key(project: &str) -> String {
    format!("work.writeback.fails:{project}")
}

/// Whether Linear team key `key` is one of the project's `teams` (keys, or team ids).
pub(crate) fn linear_team_ok(teams: &[String], key: &str) -> bool {
    teams.iter().any(|x| x.eq_ignore_ascii_case(key) || x.len() == 36)
}

/// Whether `hay` carries `id` as a whole reference (`eng-4` is not in `eng-42`).
fn carries(hay: &str, id: &str) -> bool {
    hay.match_indices(id).any(|(i, _)| {
        let before = hay[..i].chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric());
        let after = hay[i + id.len()..].chars().next().is_none_or(|c| !c.is_ascii_digit());
        before && after
    })
}

/// A node's id says which tracker item it is: `gh:owner/name#12`.
fn github_item(node: &str) -> Option<(String, u64)> {
    let rest = node.strip_prefix("gh:")?;
    let (repo, n) = rest.rsplit_once('#')?;
    Some((repo.to_string(), n.parse().ok()?))
}

impl Runtime {
    /// Runs `op` on `node` off the caller's path (a short thread; inline in tests).
    pub(crate) fn wb_spawn(&self, project: &str, node: &str, op: WbOp) {
        #[cfg(test)]
        {
            self.wb_run(project, node, &op);
        }
        #[cfg(not(test))]
        if let Some(me) = self.me.upgrade() {
            let (project, node) = (project.to_string(), node.to_string());
            std::thread::spawn(move || me.wb_run(&project, &node, &op));
        }
    }

    /// Runs `op` when write-back applies; records the outcome.
    pub(crate) fn wb_run(&self, project: &str, node: &str, op: &WbOp) {
        let Ok(Some(p)) = self.db.project(project) else { return };
        let Ok(t) = self.db.tracker(project) else { return };
        if !t.write_back.enabled || t.tracker.kind() == "local" {
            return;
        }
        let Some(row) = self.db.work_node_rows(node).ok().and_then(|r| r.into_iter().find(|r| r.project_id == project))
        else {
            return;
        };
        // Only items of the project's tracker.
        if !self.wb_owns(&p, &t, &row) {
            return;
        }
        let r = self.wb_apply(&p, &t, &row, op);
        self.wb_record(&p, &row, op, r);
    }

    /// Whether `row` is an item of the tracker this project is set to.
    fn wb_owns(&self, p: &Project, t: &TrackerSettings, row: &IntakeRow) -> bool {
        match &t.tracker {
            Tracker::Github { repo, .. } => {
                let Some((r, _)) = github_item(&row.node_id) else { return false };
                let mine = repo.clone().or_else(|| self.intake_repo_known(p, &t.intake_source()));
                mine.is_none_or(|m| m.eq_ignore_ascii_case(&r))
            }
            Tracker::Linear { teams, .. } => {
                let Some(rest) = row.node_id.strip_prefix("lin:") else { return false };
                let team = rest.rsplit_once('-').map_or("", |(t, _)| t);
                linear_team_ok(teams, team)
            }
            Tracker::Local { .. } => false,
        }
    }

    fn wb_apply(&self, p: &Project, t: &TrackerSettings, row: &IntakeRow, op: &WbOp) -> Result<(), SourceError> {
        let wbk = &t.write_back;
        match &t.tracker {
            Tracker::Github { project_board, .. } => {
                let (repo, n) =
                    github_item(&row.node_id).ok_or_else(|| SourceError::new("failed", "not a GitHub item"))?;
                let gh = self.gh_in(p);
                let gh = gh.as_ref();
                match op {
                    WbOp::Started { feature } => {
                        match project_board {
                            Some(b) => {
                                w::gh_board_status(gh, &repo, n, &b.owner, b.number, wbk.board_status.as_deref())?;
                            }
                            None => w::gh_add_label(gh, &repo, n, &wbk.running_label)?,
                        }
                        w::gh_comment(gh, &repo, n, &format!("Started in Orchestrator: feature {feature}"))?;
                        if wbk.assign_me
                            && let Some(me) = self
                                .db
                                .setting(&crate::intake::work::viewer_key(&p.id))
                                .ok()
                                .flatten()
                                .and_then(|v| v.as_str().map(str::to_string))
                        {
                            w::gh_assign(gh, &repo, n, &me)?;
                        }
                    }
                    WbOp::Merged | WbOp::Parent => {
                        if w::gh_is_open(gh, &repo, n)? {
                            w::gh_close(gh, &repo, n)?;
                        }
                        if project_board.is_none() {
                            w::gh_remove_label(gh, &repo, n, &wbk.running_label)?;
                        }
                    }
                    WbOp::TeamTask { task } => {
                        let child = self
                            .db
                            .task(*task)
                            .ok()
                            .flatten()
                            .and_then(|t| t.external_id)
                            .and_then(|e| e.parse::<u64>().ok())
                            .ok_or_else(|| SourceError::new("failed", "the task has no issue yet"))?;
                        // The sub-issue link; where GitHub refuses it, the task keeps its top-level issue and names its parent.
                        if w::gh_add_sub_issue(gh, &repo, n, child).is_err() {
                            w::gh_append_body_line(gh, &repo, child, &format!("Sub-task of #{n}"))?;
                        }
                    }
                }
            }
            Tracker::Linear { .. } => {
                let gql = self.linear_transport();
                let gql = gql.as_ref();
                let issue = w::lin_issue(gql, &row.reference)?;
                match op {
                    WbOp::Started { feature } => {
                        // An issue already in a started state (In Review…) is not moved back.
                        let state = (issue.state_type != "started")
                            .then(|| wbk.linear_state.clone().or_else(|| issue.first_of("started").map(str::to_string)))
                            .flatten();
                        let mut input = json!({});
                        if let Some(s) = state {
                            input["stateId"] = json!(s);
                        }
                        if wbk.assign_me {
                            input["assigneeId"] = json!(w::lin_viewer_id(gql)?);
                        }
                        if input.as_object().is_some_and(|o| !o.is_empty()) {
                            w::lin_update(gql, &issue.id, input)?;
                        }
                        w::lin_comment(gql, &issue.id, &format!("Started in Orchestrator: feature {feature}"))?;
                    }
                    WbOp::Merged | WbOp::Parent => {
                        // A canceled issue stays canceled.
                        if !matches!(issue.state_type.as_str(), "completed" | "canceled")
                            && let Some(s) = issue.first_of("completed")
                        {
                            w::lin_update(gql, &issue.id, json!({"stateId": s}))?;
                        }
                    }
                    WbOp::TeamTask { task } => {
                        let child = self
                            .db
                            .task(*task)
                            .ok()
                            .flatten()
                            .and_then(|t| t.external_id)
                            .ok_or_else(|| SourceError::new("failed", "the task has no issue yet"))?;
                        let child = w::lin_issue(gql, &child)?;
                        w::lin_update(gql, &child.id, json!({"parentId": issue.id}))?;
                    }
                }
            }
            Tracker::Local { .. } => {}
        }
        Ok(())
    }

    /// The outcome of a write: audit, history, event, the node's status and the project's failure count.
    fn wb_record(&self, p: &Project, row: &IntakeRow, op: &WbOp, r: Result<(), SourceError>) {
        let now = orch_protocol::now_ms();
        let what = op.what();
        let (ok, code, message) = match &r {
            Ok(()) => (true, None, None),
            Err(e) => (false, Some(e.code), Some(e.message.clone())),
        };
        let _ = self.db.audit(
            "core",
            None,
            "writeback",
            Some(&row.node_id),
            if ok { "ok" } else { "failed" },
            json!({"what": what, "code": code, "message": message}),
        );
        let e = WorkEvent {
            at_ms: now,
            what: if ok { "writeback_done" } else { "writeback_failed" }.into(),
            feature: None,
            program: None,
            detail: Some(match &message {
                Some(m) => format!("{what}: {m}"),
                None => what.to_string(),
            }),
        };
        let _ = self.db.work_note(&p.id, &row.node_id, &e);
        let status = json!({"status": if ok { "ok" } else { "failed" }, "what": what, "at_ms": now, "message": message,
                            "code": code, "op": op});
        // A success of another write does not hide a failure that can still be retried.
        let key = status_key(&p.id, &row.node_id);
        let keeps_failure = ok
            && self
                .db
                .setting(&key)
                .ok()
                .flatten()
                .is_some_and(|prev| prev["status"] == "failed" && prev["what"] != what);
        if !keeps_failure {
            let _ = self.db.set_setting(&key, &status);
        }
        self.event(
            "-",
            if ok { "writeback.done" } else { "writeback.failed" },
            json!({"node": row.node_id, "what": what, "code": code}),
        );
        self.db.note([crate::intake::TOPIC]);
        if ok {
            let _ = self.db.set_setting(&fails_key(&p.id), &json!(0));
            return;
        }
        let n = self.db.setting(&fails_key(&p.id)).ok().flatten().and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        let _ = self.db.set_setting(&fails_key(&p.id), &json!(n));
        let open = self.store.read(|s| {
            s.inbox.iter().any(|i| {
                i.open()
                    && i.kind == "program"
                    && i.payload["kind"] == "writeback_failing"
                    && i.payload["project"] == p.id.as_str()
            })
        });
        if n >= FAILING_AFTER && !open {
            let payload = json!({"kind": "writeback_failing", "project": p.id, "key": "writeback",
                                 "title": format!("Write-back is failing for {}", p.name),
                                 "text": format!("{}. Fix it in the tracker settings or turn write-back off.", message.unwrap_or_default())});
            if let Err(e) =
                self.add_item_in(None, "program", format!("Write-back is failing for {}", p.name), payload, 0, None)
            {
                eprintln!("warn: writeback_failing not posted: {e:#}");
            }
        }
    }

    /// `writeback_retry(node)`: repeats the write that failed on the node.
    pub fn writeback_retry(&self, node: &str) -> Result<Value> {
        let rows = self.db.work_node_rows(node)?;
        let row = rows
            .first()
            .ok_or_else(|| anyhow!(json!({"code": "not_found", "message": format!("no node {node:?}")}).to_string()))?;
        let st = self.db.setting(&status_key(&row.project_id, node))?.unwrap_or_default();
        if st["status"] != "failed" {
            return Err(anyhow!(json!({"code": "stale", "message": "nothing to retry on this node"}).to_string()));
        }
        let op: WbOp =
            serde_json::from_value(st["op"].clone()).map_err(|e| anyhow!("the failed write is not readable: {e}"))?;
        self.wb_run(&row.project_id, node, &op);
        Ok(self.db.setting(&status_key(&row.project_id, node))?.unwrap_or_default())
    }

    /// The write-back status of every node of a project, by node id (for `WorkNode.writeback`).
    pub(crate) fn wb_statuses(&self, project: &str) -> std::collections::HashMap<String, Value> {
        let prefix = status_key(project, "");
        self.db
            .settings_prefixed(&prefix)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| {
                let node = k.strip_prefix(&prefix)?.to_string();
                Some((
                    node,
                    json!({"status": v["status"], "what": v["what"], "at_ms": v["at_ms"], "message": v["message"]}),
                ))
            })
            .collect()
    }

    /// The PR's title and body with what write-back adds: `Closes #n` (`Closes owner/name#n` when the
    /// tracker's repository is not the PR's) for every GitHub item the agent's run or session came
    /// from; on Linear the identifier goes in the title only when the branch does not carry it.
    pub(crate) fn pr_write_back(&self, agent: Option<&str>, branch: &str, title: &str, body: &str) -> (String, String) {
        let rows = agent
            .and_then(|a| {
                let run = self.store.read(|s| s.agents.get(a).and_then(|r| r.team.as_ref().map(|t| t.run_id.clone())));
                match run {
                    Some(run) => self.db.work_nodes_of("run", &run).ok(),
                    None => self.db.work_nodes_of("agent", a).ok(),
                }
            })
            .unwrap_or_default();
        let (mut title, mut body) = (title.to_string(), body.to_string());
        let mut closes = vec![];
        for r in rows {
            let Ok(Some(p)) = self.db.project(&r.project_id) else { continue };
            let Ok(t) = self.db.tracker(&r.project_id) else { continue };
            if !t.write_back.enabled || !self.wb_owns(&p, &t, &r) {
                continue;
            }
            match &t.tracker {
                Tracker::Github { .. } => {
                    let Some((repo, n)) = github_item(&r.node_id) else { continue };
                    let here = crate::github_issue::known_repo(&p);
                    let line = if here.as_deref().is_none_or(|h| h.eq_ignore_ascii_case(&repo)) {
                        format!("Closes #{n}")
                    } else {
                        format!("Closes {repo}#{n}")
                    };
                    if !body.lines().any(|l| l.trim() == line) && !closes.contains(&line) {
                        closes.push(line);
                    }
                }
                Tracker::Linear { .. } => {
                    let id = r.reference.to_lowercase();
                    if !carries(&branch.to_lowercase(), &id) && !carries(&title.to_lowercase(), &id) {
                        title = format!("[{}] {title}", r.reference);
                    }
                }
                Tracker::Local { .. } => {}
            }
        }
        if !closes.is_empty() {
            body = format!("{}\n\n{}", body.trim_end(), closes.join("\n")).trim_start().to_string();
        }
        (title, body)
    }

    /// Turns write-back off for the project and clears its failure count (the decision's "Turn off write-back").
    pub(crate) fn wb_turn_off(&self, project: &str) -> Result<()> {
        let mut t = self.db.tracker(project)?;
        t.write_back.enabled = false;
        self.db.set_tracker(project, &t)?;
        self.db.set_setting(&fails_key(project), &json!(0))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::carries;

    #[test]
    fn a_reference_is_carried_as_a_whole() {
        assert!(carries("eng-42-refunds", "eng-42"));
        assert!(carries("orch/eng-42", "eng-42"));
        assert!(!carries("eng-421-x", "eng-42"));
        assert!(!carries("reng-42", "eng-42"));
    }
}
