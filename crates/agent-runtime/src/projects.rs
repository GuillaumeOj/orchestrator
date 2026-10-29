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
use orch_git::worktree::WorktreeLedger;

/// Patch size kept in memory for a diff view.
pub const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;

impl Runtime {
    /// FR-101: registers a folder (its main checkout). The first background
    /// fetch happens on the next fetch pass.
    pub fn add_project(&self, path: &Path) -> Result<Project> {
        let p = project::register(&self.db, path)?;
        self.db.audit("user", None, "project_added", p.path.to_str(), "ok", json!({"id": p.id}))?;
        self.mark_full_index();
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
        let rows = self.store.rows_of(&p.path);
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
                .get_row(&wt)
                .map(|w| w.base)
                .or_else(|| {
                    let main = real(&orch_git::worktree::main_checkout(&wt).ok()?);
                    self.db.project_by_path(&main).ok().flatten().map(|p| p.default_branch)
                })
                .unwrap_or_else(|| "main".into()),
        };
        orch_git::diff::diff(&wt, &base, MAX_PATCH_BYTES)
    }

    /// FR-104 cleanup of one worktree of the project's sibling folder (the
    /// user's own worktrees elsewhere are never removed from here). Refuses
    /// while an agent that is not archived or killed uses it; unmerged work
    /// needs `confirm_discard`.
    pub fn remove_worktree(
        &self,
        project_id: &str,
        path: &Path,
        delete_branch: bool,
        confirm_discard: bool,
    ) -> Result<RemoveOutcome> {
        let p = self.project(project_id)?;
        // A worktree whose folder is gone (prunable) is resolved through its parent.
        let wt = match (path.exists(), path.parent(), path.file_name()) {
            (false, Some(dir), Some(name)) => real(dir).join(name),
            _ => real(path),
        };
        if !wt.starts_with(real(&p.worktree_root)) {
            bail!("{} is not in {}: the app only removes its own worktrees", path.display(), p.worktree_root.display());
        }
        if let Some(a) = self.worktree_users(&wt, "").first() {
            bail!("agent {a} uses this worktree: archive or kill it first");
        }
        let base = self.store.get_row(&wt).map(|w| w.base).unwrap_or(p.default_branch.clone());
        let confirm = if confirm_discard { Confirm::DiscardUnmerged } else { Confirm::Safe };
        let out = cleanup::remove_worktree(&p.path, &wt, &base, delete_branch, confirm)?;
        self.store.write(|s| {
            if let Some(w) = s.worktrees.get_mut(&wt) {
                w.state = WorktreeRowState::Removed;
            }
        })?;
        self.db.audit(
            "user",
            None,
            "worktree_removed",
            wt.to_str(),
            if confirm_discard { "discard" } else { "safe" },
            json!(out),
        )?;
        Ok(out)
    }

    /// What the delete dialog shows: the agent's worktree (merged, dirty,
    /// safe to remove) and the other agents still using it.
    pub fn delete_plan(&self, id: &str) -> Result<Value> {
        let rec = self.store.read(|s| s.agents.get(id).cloned()).ok_or_else(|| anyhow!("no agent {id}"))?;
        let wt = real(&rec.worktree);
        let worktree = match self.db.project_by_path(&rec.repo)? {
            Some(p) if wt.starts_with(real(&p.worktree_root)) => {
                cleanup::worktree(&p.path, &wt, &self.store.rows_of(&p.path), &p.default_branch)?
            }
            _ => None,
        };
        Ok(json!({"worktree": worktree, "shared_with": self.worktree_users(&wt, id)}))
    }

    /// Agents other than `except` that use `wt` (canonical) and are neither
    /// archived nor killed: while one does, the worktree is not removed.
    fn worktree_users(&self, wt: &Path, except: &str) -> Vec<String> {
        // Canonicalized outside the lock (a syscall per agent).
        let candidates: Vec<(String, PathBuf)> = self.store.read(|s| {
            s.agents
                .values()
                .filter(|a| a.id != except && !matches!(a.tracker.state, AgentState::Archived | AgentState::Killed))
                .map(|a| (a.id.clone(), a.worktree.clone()))
                .collect()
        });
        candidates.into_iter().filter(|(_, w)| real(w) == wt).map(|(id, _)| id).collect()
    }

    /// Hides an agent for good (FR-202): archived, its queued turns dropped,
    /// its open Inbox items closed (`why`: nobody could act on them, and they
    /// would count as waiting), and the Governor told to forget it, so no
    /// release ever comes for it. Its process must already be stopped.
    pub(crate) fn archive_agent(&self, id: &str, why: &str) -> Result<()> {
        self.store.write(|s| {
            if let Some(a) = s.agents.get_mut(id) {
                a.tracker.state = AgentState::Archived;
                a.tracker.stop_intent = crate::state::StopIntent::None;
                // A late exit callback matches on pid: with none it changes nothing.
                a.pid = None;
                a.pgid = None;
            }
        })?;
        self.drop_queued_turns(id);
        self.close_items_of(id, why);
        self.governor_forget(id);
        Ok(())
    }

    /// FR-202 archive, from the UI's Delete: kills the agent (FR-207, checked:
    /// nothing is archived while one of its processes survives), drops its
    /// queued turns and hides it. With `remove_worktree`, its worktree and
    /// branch go too when no other agent uses them; unmerged work needs
    /// `confirm_discard`. A worktree that can't be removed is reported, the
    /// agent stays deleted.
    pub fn delete_agent(&self, id: &str, remove_worktree: bool, confirm_discard: bool) -> Result<Value> {
        let rec = self.store.read(|s| s.agents.get(id).cloned()).ok_or_else(|| anyhow!("no agent {id}"))?;
        // Always through kill, even for a suspended, crashed or killed agent:
        // its scan finds leftovers of an earlier run of the app.
        let log = self.kill(id)?;
        self.archive_agent(id, "agent deleted")?;
        let worktree = if !remove_worktree {
            Value::Null
        } else {
            match self.db.project_by_path(&rec.repo) {
                Ok(Some(p)) => self.remove_worktree_report(&p.id, &real(&rec.worktree), confirm_discard),
                Ok(None) => json!({"error": "no project for this agent"}),
                Err(e) => json!({"error": format!("{e:#}")}),
            }
        };
        let out = json!({"deleted": id, "log": log, "worktree": worktree});
        self.db.audit("user", Some(id), "agent_deleted", rec.worktree.to_str(), "ok", out.clone())?;
        self.event(id, "deleted", out.clone());
        Ok(out)
    }

    /// [`Self::remove_worktree`] (branch included) as a delete reports it:
    /// the outcome, or why the worktree stays (another agent uses it, or the
    /// removal was refused).
    pub(crate) fn remove_worktree_report(&self, project_id: &str, wt: &Path, confirm_discard: bool) -> Value {
        let users = self.worktree_users(wt, "");
        if !users.is_empty() {
            return json!({"kept": format!("used by {}", users.join(", "))});
        }
        match self.remove_worktree(project_id, wt, true, confirm_discard) {
            Ok(o) => json!(o),
            Err(e) => json!({"error": format!("{e:#}")}),
        }
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
            if self.live(&a.id).is_some() {
                let _ = self.suspend(&a.id);
            }
            self.archive_agent(&a.id, "agent archived")?;
        }
        let report = project::remove(&self.db, &self.store, id, choices)?;
        if choices.history == project::History::Delete {
            // The project's rows went with it (kept worktrees stay on disk, unmanaged).
            self.store.write(|s| s.worktrees.retain(|_, w| w.repo != p.path))?;
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
        let top =
            orch_git::worktree::toplevel(&new).with_context(|| format!("{} is not a git repository", new.display()))?;
        if real(&top) != new || !new.join(".git").is_dir() {
            bail!("{} is not a main checkout", new.display());
        }
        if let Some(other) = self.db.project_by_path(&new)?.filter(|o| o.id != p.id) {
            bail!("{} is already registered as project {}", new.display(), other.id);
        }
        let old = p.path.clone();
        let rows = self.store.rows_of(&old);
        let map = repair::plan_relocation(&old, &new, &rows);
        let moved =
            |x: &Path| map.iter().find(|(o, _)| o == x).map(|(_, n)| n.clone()).unwrap_or_else(|| x.to_path_buf());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use orch_git::fixtures::{commit_file, scratch_repo};
    use orch_protocol::WorktreeKindSpec;

    #[test]
    fn projects_through_the_runtime() {
        let (_d, rt) = test_runtime(true);
        let (r, repo) = scratch_repo();
        let p = rt.add_project(&repo).unwrap();
        let row = rt.create_worktree(&repo, &WorktreeKindSpec::Solo { slug: "a1".into() }, None).unwrap();
        rt.store
            .write(|s| {
                let a = s.agents.get_mut("a1").unwrap();
                a.repo = repo.clone();
                a.worktree = row.path.clone();
            })
            .unwrap();
        commit_file(&row.path, "x.txt", "x", "work");
        let ov = rt.projects_overview().unwrap();
        assert_eq!(ov[0]["project"]["id"], p.id.as_str());
        assert_eq!(ov[0]["agents"], 1);
        let det = rt.project_detail(&p.id, false).unwrap();
        assert!(
            det["worktrees"].as_array().unwrap().iter().any(|w| w["verdict"] == "ok" && w["unmerged_commits"] == 1)
        );
        let d = rt.diff(&row.path, None).unwrap();
        assert_eq!(d.against, "main");
        assert_eq!(d.files[0].path, "x.txt");
        // In use by an agent: refused, even with confirmation.
        assert!(
            rt.remove_worktree(&p.id, &row.path, true, true).unwrap_err().to_string().contains("uses this worktree")
        );
        rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = AgentState::Killed).unwrap();
        // Unmerged: needs confirmation.
        assert!(rt.remove_worktree(&p.id, &row.path, true, false).is_err());
        let out = rt.remove_worktree(&p.id, &row.path, true, true).unwrap();
        assert!(out.worktree_removed && out.branch_deleted);
        assert_eq!(rt.store.read(|s| s.worktrees[&row.path].state), WorktreeRowState::Removed);
        assert!(
            rt.db
                .audit_log(None, 10)
                .unwrap()
                .iter()
                .any(|e| e.action == "worktree_removed" && e.decision == "discard")
        );

        // Relocate: the repo moved alone; rows, agents and the project follow.
        let row2 = rt.create_worktree(&repo, &WorktreeKindSpec::Solo { slug: "b".into() }, None).unwrap();
        let new_repo = r.path().join("elsewhere/demo");
        std::fs::create_dir_all(new_repo.parent().unwrap()).unwrap();
        std::fs::rename(&repo, &new_repo).unwrap();
        let rep = rt.relocate_project(&p.id, &new_repo).unwrap();
        assert_eq!(rep.repaired, vec![row2.path.clone()], "{rep:?}");
        let new_repo = real(&new_repo);
        assert_eq!(rt.db.project(&p.id).unwrap().unwrap().path, new_repo);
        assert_eq!(rt.store.read(|s| s.worktrees[&row2.path].repo.clone()), new_repo);
        assert_eq!(rt.store.read(|s| s.agents["a1"].repo.clone()), new_repo);
        assert!(orch_git::repair::check_links(&row2.path).ok);
        assert!(rt.relocate_project(&p.id, &row2.path).is_err(), "a worktree is not a main checkout");
        let (_o, other) = scratch_repo();
        rt.add_project(&other).unwrap();
        let e = rt.relocate_project(&p.id, &other).unwrap_err();
        assert!(e.to_string().contains("already registered"), "{e}");
        assert_eq!(rt.store.read(|s| s.agents["a1"].repo.clone()), new_repo, "nothing changed");

        // J11 delete: agents archived, rows gone from memory and database.
        let report = rt
            .remove_project(
                &p.id,
                &RemovalChoices { remove_merged: true, discard: vec![], history: project::History::Delete },
            )
            .unwrap();
        assert!(report.removed.contains(&row2.path));
        assert!(rt.db.project(&p.id).unwrap().is_none());
        assert_eq!(rt.store.read(|s| s.agents["a1"].tracker.state), AgentState::Archived);
        assert!(rt.store.read(|s| s.worktrees.values().all(|w| w.repo != new_repo)));
        assert!(rt.db.worktrees_of(&new_repo).unwrap().is_empty());
    }

    #[test]
    fn delete_kills_leftovers_archives_and_removes_the_worktree_on_request() {
        let (_d, rt) = test_runtime(true);
        let (_r, repo) = scratch_repo();
        rt.add_project(&repo).unwrap();
        let row = rt.create_worktree(&repo, &WorktreeKindSpec::Solo { slug: "a1".into() }, None).unwrap();
        commit_file(&row.path, "x.txt", "x", "unmerged work");
        rt.store
            .write(|s| {
                let a = s.agents.get_mut("a1").unwrap();
                a.repo = real(&repo);
                a.worktree = row.path.clone();
                a.tracker.state = AgentState::Suspended;
                s.turns.push(crate::store::TurnRow::queued(
                    "a1",
                    "later",
                    governor::TurnKind::Message,
                    crate::store::Admission::Pending,
                ));
            })
            .unwrap();
        // A leftover of an earlier run: detached, tty-less, cwd in the worktree.
        let mut child = crate::procs::tests::spawn_detached(&row.path, "exec sleep 300");
        let pid = child.id() as i32;
        std::thread::sleep(Duration::from_millis(200));
        let reaper = std::thread::spawn(move || child.wait());

        let plan = rt.delete_plan("a1").unwrap();
        assert_eq!(plan["worktree"]["safe_to_remove"], false, "{plan}");
        assert_eq!(plan["worktree"]["unmerged_commits"], 1);

        // Unmerged and not confirmed: the agent goes, the worktree stays.
        let out = rt.delete_agent("a1", true, false).unwrap();
        let _ = reaper.join();
        assert!(!crate::procs::alive(pid), "the leftover is dead");
        assert!(out["worktree"]["error"].as_str().unwrap().contains("not merged"), "{out}");
        assert!(row.path.is_dir());
        let a = rt.store.read(|s| s.agents["a1"].clone());
        assert_eq!(a.tracker.state, AgentState::Archived);
        assert!(rt.store.read(|s| s.turns.iter().all(|t| t.agent_id != "a1")));
        assert!(rt.spawn("a1", None).unwrap_err().to_string().contains("deleted"));

        // Confirmed: the worktree and its branch go.
        let out = rt.delete_agent("a1", true, true).unwrap();
        assert_eq!(out["worktree"]["worktree_removed"], true, "{out}");
        assert!(!row.path.exists());
        assert!(rt.db.audit_log(Some("a1"), 10).unwrap().iter().any(|e| e.action == "agent_deleted"));
        assert!(rt.delete_agent("nope", false, false).is_err());
    }
}
