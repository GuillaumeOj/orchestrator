//! Projects and git through the runtime (FR-101–111, J1, J11, J10 repair):
//! the git service ([`orch_git`]) with the runtime's worktree rows and agents,
//! so a worktree in use is never removed and the in-memory state stays right.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use orch_git::cleanup::{self, Confirm, RemoveOutcome};
use orch_git::diff::Diff;
use orch_git::fetch::{Conditions, FetchOutcome, FetchPolicy};
use orch_git::project::{self, RemovalChoices, RemovalReport};
use orch_git::repair::{self, RepairReport};
use orch_protocol::now_ms;
use orch_store::Project;
use serde_json::{Value, json};

use crate::paths::real;
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::WorktreeRowState;

/// Patch size kept in memory for a diff view.
pub const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;

impl Runtime {
    /// FR-101: registers a folder (its main checkout). The first background
    /// fetch happens on the next fetch pass.
    pub fn add_project(&self, path: &Path) -> Result<Project> {
        let p = project::register(&self.db, path)?;
        self.db.audit("user", None, "project_added", p.path.to_str(), "ok", json!({"id": p.id}))?;
        self.request_full_index();
        Ok(p)
    }

    pub fn clone_project(&self, url: &str, parent: &Path, name: Option<&str>) -> Result<Project> {
        let p = project::clone(&self.db, url, parent, name, Duration::from_secs(600))?;
        self.db.audit("user", None, "project_cloned", Some(url), "ok", json!({"id": p.id, "path": p.path}))?;
        Ok(p)
    }

    fn project(&self, id: &str) -> Result<Project> {
        self.db.project(id)?.ok_or_else(|| anyhow!("no project {id}"))
    }

    fn agents_of(&self, repo: &Path) -> Vec<crate::store::AgentRecord> {
        self.store.read(|s| s.agents.values().filter(|a| a.repo == repo).cloned().collect())
    }

    /// J1 row per project: status (FR-102), live agents, last fetch.
    pub fn projects_overview(&self) -> Result<Vec<Value>> {
        let live = self.live_ids();
        Ok(self
            .db
            .projects(false)?
            .into_iter()
            .map(|p| {
                let status = orch_git::status::status(&p.path).map_err(|e| e.to_string());
                let agents = self.agents_of(&p.path);
                json!({
                    "project": p,
                    "status": status.as_ref().ok(),
                    "status_error": status.as_ref().err(),
                    "dirty": status.as_ref().map(|s| s.dirty()).unwrap_or(0),
                    "agents": agents.len(),
                    "live_agents": agents.iter().filter(|a| live.contains(&a.id)).count(),
                })
            })
            .collect())
    }

    /// Status, worktrees (orphans flagged, FR-104), branches, and with
    /// `with_prs` open PRs with CI (FR-106).
    pub fn project_detail(&self, id: &str, with_prs: bool) -> Result<Value> {
        let p = self.project(id)?;
        let rows = self.store.read(|s| s.worktrees.values().filter(|w| w.repo == p.path).cloned().collect::<Vec<_>>());
        let worktrees = cleanup::worktrees(&p.path, &rows, &p.default_branch)?;
        let agents: Vec<Value> = self
            .agents_of(&p.path)
            .iter()
            .map(|a| json!({"id": a.id, "state": a.tracker.state, "worktree": a.worktree}))
            .collect();
        let mut v = json!({
            "project": p,
            "status": orch_git::status::status(&p.path)?,
            "worktrees": worktrees,
            "branches": cleanup::branches(&p.path, &p.default_branch)?,
            "agents": agents,
        });
        if with_prs {
            v["prs"] = match orch_git::gh::prs(&p.path) {
                Ok(prs) => json!(prs),
                Err(e) => json!({"error": format!("{e:#}")}),
            };
        }
        Ok(v)
    }

    /// FR-103 on demand (skips the throttle, not the offline check).
    pub fn fetch_project(&self, id: &str, policy: &FetchPolicy, c: &dyn Conditions) -> Result<FetchOutcome> {
        let p = self.project(id)?;
        Ok(orch_git::fetch::fetch_now(&self.db, &p, policy, c))
    }

    /// FR-103 background pass (call every minute or so).
    pub fn fetch_tick(&self, policy: &FetchPolicy, c: &dyn Conditions) {
        match orch_git::fetch::tick(&self.db, policy, c, now_ms()) {
            Ok(v) if !v.is_empty() => self.event("-", "fetch", json!(v)),
            Ok(_) => {}
            Err(e) => self.event("-", "fetch_error", json!(e.to_string())),
        }
    }

    /// FR-107: a worktree's diff against its base (or `against`).
    pub fn diff(&self, worktree: &Path, against: Option<&str>) -> Result<Diff> {
        let wt = real(worktree);
        let base = match against {
            Some(b) => b.to_string(),
            None => self
                .store
                .read(|s| s.worktrees.get(&wt).map(|w| w.base.clone()))
                .or_else(|| {
                    let main = orch_git::worktree::common_dir(&wt).ok()?.parent()?.to_path_buf();
                    self.db.project_by_path(&main).ok().flatten().map(|p| p.default_branch)
                })
                .unwrap_or_else(|| "main".into()),
        };
        orch_git::diff::diff(&wt, &base, MAX_PATCH_BYTES)
    }

    /// FR-104 cleanup of one worktree. Refuses while an agent that is not
    /// archived or killed uses it; unmerged work needs `confirm_discard`.
    pub fn remove_worktree(&self, project_id: &str, path: &Path, delete_branch: bool, confirm_discard: bool) -> Result<RemoveOutcome> {
        let p = self.project(project_id)?;
        let wt = real(path);
        if let Some(a) = self.store.read(|s| {
            s.agents
                .values()
                .find(|a| real(&a.worktree) == wt && !matches!(a.tracker.state, AgentState::Archived | AgentState::Killed))
                .map(|a| a.id.clone())
        }) {
            bail!("agent {a} uses this worktree: archive or kill it first");
        }
        let base = self.store.read(|s| s.worktrees.get(&wt).map(|w| w.base.clone())).unwrap_or(p.default_branch.clone());
        let confirm = if confirm_discard { Confirm::DiscardUnmerged } else { Confirm::Safe };
        let out = cleanup::remove_worktree(&p.path, &wt, &base, delete_branch, confirm)?;
        self.store.write(|s| {
            if let Some(w) = s.worktrees.get_mut(&wt) {
                w.state = WorktreeRowState::Removed;
            }
        })?;
        self.db.audit("user", None, "worktree_removed", wt.to_str(), if confirm_discard { "discard" } else { "safe" }, json!(out))?;
        Ok(out)
    }

    /// J11 step 1: worktrees, branches and agents of the project.
    pub fn removal_plan(&self, id: &str) -> Result<Value> {
        let plan = project::removal_plan(&self.db, &self.store, id)?;
        let agents: Vec<Value> = self
            .agents_of(&plan.project.path)
            .iter()
            .map(|a| json!({"id": a.id, "state": a.tracker.state, "live": self.live_ids().contains(&a.id)}))
            .collect();
        Ok(json!({"plan": plan, "agents": agents}))
    }

    /// J11: stops the project's agents (suspend keeps their sessions), archives
    /// them, removes merged worktrees and the confirmed ones, keeps the rest.
    pub fn remove_project(&self, id: &str, choices: &RemovalChoices) -> Result<RemovalReport> {
        let p = self.project(id)?;
        for a in self.agents_of(&p.path) {
            if self.live_ids().contains(&a.id) {
                let _ = self.suspend(&a.id);
            }
        }
        self.store.write(|s| {
            for a in s.agents.values_mut().filter(|a| a.repo == p.path) {
                a.tracker.state = AgentState::Archived;
            }
            s.turns.retain(|t| !(s.agents.get(&t.agent_id).is_some_and(|a| a.repo == p.path) && t.status == crate::store::TurnStatus::Queued));
        })?;
        let report = project::remove(&self.db, &self.store, id, choices)?;
        if choices.history == project::History::Delete {
            self.store.write(|s| s.worktrees.retain(|_, w| !(w.repo == p.path && w.state == WorktreeRowState::Removed)))?;
        }
        self.db.audit("user", None, "project_removed", p.path.to_str(), "ok", json!(report))?;
        Ok(report)
    }

    /// The repo moved (J10, ADR-003): point the project, its worktree rows and
    /// its agents at the new place, run a targeted `git worktree repair` on
    /// the known worktrees, and re-lock them.
    pub fn relocate_project(&self, id: &str, new_path: &Path) -> Result<RepairReport> {
        let p = self.project(id)?;
        let new = real(new_path);
        let top = orch_git::worktree::toplevel(&new).with_context(|| format!("{} is not a git repository", new.display()))?;
        if real(&top) != new || !new.join(".git").is_dir() {
            bail!("{} is not a main checkout", new.display());
        }
        let old = p.path.clone();
        let rows = self.store.read(|s| s.worktrees.values().filter(|w| w.repo == old).cloned().collect::<Vec<_>>());
        let map = repair::plan_relocation(&old, &new, &rows);
        let moved = |x: &Path| map.iter().find(|(o, _)| o == x).map(|(_, n)| n.clone()).unwrap_or_else(|| x.to_path_buf());
        let targets: Vec<(PathBuf, Option<String>)> = rows
            .iter()
            .filter(|r| r.state != WorktreeRowState::Removed)
            .map(|r| (moved(&r.path), r.agent_id.clone()))
            .collect();
        let report = repair::repair(&new, &targets)?;
        self.store.write(|s| {
            for r in &rows {
                s.worktrees.remove(&r.path);
                let mut n = r.clone();
                n.path = moved(&r.path);
                n.repo = new.clone();
                s.worktrees.insert(n.path.clone(), n);
            }
            for a in s.agents.values_mut().filter(|a| a.repo == old) {
                a.repo = new.clone();
                a.worktree = moved(&a.worktree);
                if a.flag.as_deref().is_some_and(|f| f.contains("moved")) {
                    a.flag = None;
                }
            }
        })?;
        let mut np = p.clone();
        np.path = new.clone();
        np.worktree_root = orch_git::worktree::worktree_root(&new);
        self.db.upsert_project(&np)?;
        self.db.audit("user", None, "project_relocated", new.to_str(), "ok", json!({"from": old, "report": report}))?;
        Ok(report)
    }

    fn request_full_index(&self) {
        self.mark_full_index();
    }
}
