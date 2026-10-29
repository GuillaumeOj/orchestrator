//! The user's side of teams: the library of roles and teams (FR-306/307),
//! run details for the Feature pane, and the user's own task and message
//! actions (FR-406, FR-409, FR-502).

use anyhow::{Result, anyhow, bail};
use orch_protocol::now_ms;
use orch_store::Task;
use orch_team::{RoleDef, TaskStatus, Team, mcp, rules};
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::team::{feature_docs, read_feature_asset, read_feature_doc};

/// A team's validation issues for the canvas: the node each is about, and
/// its text (FR-305).
fn issues_json(team: &Team, roles: &[RoleDef]) -> Vec<Value> {
    orch_team::validate(team, Some(roles))
        .into_iter()
        .map(|i| json!({"node": i.node(), "text": i.to_string(), "issue": i}))
        .collect()
}

impl Runtime {
    /// [`Runtime::install_presets`] once per run of the app (the views call
    /// it on every load).
    pub fn ensure_presets(&self) -> Result<()> {
        if !self.presets_checked.load(std::sync::atomic::Ordering::Relaxed) {
            self.install_presets()?;
            self.presets_checked.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    /// Teams and roles visible from `project` (global + its own), each team
    /// with its validation issues and Rules-panel lines (FR-305).
    pub fn teams_json(&self, project: Option<&str>) -> Result<Value> {
        let roles = self.db.roles(project)?;
        let reg = self.db.mcp_registry(project)?;
        let teams: Vec<Value> = self
            .db
            .teams(project)?
            .into_iter()
            .map(|t| {
                let (preset, starter, update_available) = crate::presets::preset_fields(&t);
                let mut issues = issues_json(&t, &roles);
                // A node whose role's MCP servers can't work (FR-601) is an
                // issue; one the compile works around is a warning (no block).
                let mut warnings = vec![];
                for n in &t.nodes {
                    if let Some(r) = roles.iter().find(|r| r.id == n.role_id) {
                        for i in mcp::role_issues(r, &reg) {
                            let v = json!({"node": n.id, "text": format!("{}: {}", n.id, i.text), "issue": {"kind": "mcp"}});
                            if i.warning { warnings.push(v) } else { issues.push(v) }
                        }
                    }
                }
                json!({"team": t, "issues": issues, "warnings": warnings, "rules": rules::rules_text(&t),
                       "preset": preset, "starter": starter, "update_available": update_available})
            })
            .collect();
        let roles: Vec<Value> = roles
            .iter()
            .map(|r| {
                let mut v = json!(r);
                v["issues"] = json!(mcp::role_issues(r, &reg).into_iter().map(|i| i.text).collect::<Vec<_>>());
                v
            })
            .collect();
        Ok(json!({"teams": teams, "roles": roles}))
    }

    /// Saves a team from the canvas. An invalid team is saved too (a draft);
    /// it can't be launched until its issues are fixed. The version goes up
    /// when the graph changed.
    pub fn save_team(&self, mut team: Team) -> Result<Value> {
        if !orch_team::valid_slug(&team.id) {
            bail!("invalid team id {:?}", team.id);
        }
        if team.name.trim().is_empty() {
            bail!("the team needs a name");
        }
        let roles = self.db.roles(team.scope.project())?;
        if let Some(old) = self.db.team(&team.scope, &team.id)? {
            let same = old.nodes == team.nodes && old.edges == team.edges && old.name == team.name;
            team.version = if same { old.version } else { old.version + 1 };
            // Where a starter came from stays with it (the canvas may not send it back).
            if team.origin.is_none() {
                team.origin = old.origin;
            }
        }
        self.db.save_team(&team, &roles, now_ms())?;
        Ok(json!({"issues": issues_json(&team, &roles), "team": team}))
    }

    /// Saves a role (the version goes up when it changed).
    pub fn save_role(&self, mut role: RoleDef) -> Result<RoleDef> {
        if !orch_team::valid_slug(&role.id) {
            bail!("invalid role id {:?}", role.id);
        }
        if let Some(old) = self.db.role(&role.scope, &role.id)? {
            if role.origin.is_none() {
                role.origin = old.origin.clone();
            }
            let mut cmp = old.clone();
            cmp.version = role.version;
            role.version = if cmp == role { old.version } else { old.version + 1 };
        }
        self.db.upsert_role(&role, now_ms())?;
        Ok(role)
    }

    /// FR-307 import: the roles it carries are added when missing (an
    /// existing role of the same id is kept).
    pub fn import_team(&self, json_text: &str) -> Result<Team> {
        let file = orch_team::export::import(json_text).map_err(|e| anyhow!("{e}"))?;
        for r in &file.roles {
            if self.db.role(&r.scope, &r.id)?.is_none() {
                self.db.upsert_role(r, now_ms())?;
            }
        }
        let mut team = file.team;
        if let Some(old) = self.db.team(&team.scope, &team.id)? {
            team.version = team.version.max(old.version + 1);
        }
        let roles = self.db.roles(team.scope.project())?;
        self.db.save_team(&team, &roles, now_ms())?;
        Ok(team)
    }

    /// Exports the team `team_id` as seen from `project` (its own, else the
    /// global one).
    pub fn export_team(&self, team_id: &str, project: Option<&str>) -> Result<String> {
        let team = self.db.team_in(project, team_id)?.ok_or_else(|| anyhow!("no team {team_id}"))?;
        Ok(orch_team::export::export(&team, &self.db.roles(team.scope.project())?))
    }

    /// Everything the Feature pane shows about a run.
    pub fn run_json(&self, run_id: &str) -> Result<Value> {
        let cx = self.run_ctx(run_id)?;
        let agents: Vec<Value> = self
            .run_agents(run_id)
            .into_iter()
            .map(|a| {
                let seat = a.team.clone();
                json!({"id": a.id, "node": seat.as_ref().map(|s| s.node.clone()), "task": seat.and_then(|s| s.task),
                       "state": a.tracker.state, "tool": a.tracker.current_tool, "worktree": a.worktree, "flag": a.flag})
            })
            .collect();
        let tasks = self.db.tasks(None, None, Some(run_id))?;
        let wt = cx.feature.integration_worktree.clone();
        let docs = wt.as_deref().map(|w| feature_docs(w, &cx.feature.slug)).unwrap_or_default();
        Ok(json!({
            "run": cx.run, "feature": cx.feature, "team": cx.team, "project": {"id": cx.project.id, "name": cx.project.name, "backend": cx.project.backend().kind()},
            "rules": rules::rules_text(&cx.team), "agents": agents, "tasks": tasks, "docs": docs,
            "artifacts": self.artifacts_json(&cx.feature)?,
            "timeline": self.db.run_events(run_id, 200)?,
        }))
    }

    /// One feature doc from the integration worktree (rendered by the UI).
    pub fn feature_doc(&self, feature_id: &str, name: &str) -> Result<String> {
        let (wt, slug) = self.integration_worktree(feature_id)?;
        read_feature_doc(&wt, &slug, name)
    }

    /// An image a feature doc points at, from the integration worktree
    /// (`{mime, data_b64, bytes}`).
    pub fn feature_asset(&self, feature_id: &str, path: &str) -> Result<Value> {
        let (wt, slug) = self.integration_worktree(feature_id)?;
        read_feature_asset(&wt, &slug, path)
    }

    /// A feature's integration worktree and slug.
    fn integration_worktree(&self, feature_id: &str) -> Result<(std::path::PathBuf, String)> {
        let f = self.db.feature(feature_id)?.ok_or_else(|| anyhow!("no feature {feature_id}"))?;
        let wt = f.integration_worktree.ok_or_else(|| anyhow!("feature {} has no integration worktree", f.slug))?;
        Ok((wt, f.slug))
    }

    pub fn run_act(&self, run_id: &str, action: &str) -> Result<Value> {
        let run = match action {
            "pause" => self.run_pause(run_id)?,
            "resume" => self.run_resume(run_id)?,
            "stop" => self.run_stop(run_id)?,
            other => bail!("unknown run action {other:?} (pause | resume | stop)"),
        };
        Ok(json!(run))
    }

    /// The user creates a task on the board (in a run when `run_id` is set).
    pub fn user_create_task(
        &self,
        project_id: &str,
        title: &str,
        body: &str,
        node: Option<&str>,
        run_id: Option<&str>,
        depends_on: Vec<u64>,
    ) -> Result<Task> {
        let project = self.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        if title.trim().is_empty() {
            bail!("the task needs a title");
        }
        let mut t = Task::new(project_id, "", title.trim(), now_ms());
        t.body = body.trim().to_string();
        t.depends_on = depends_on;
        if let Some(r) = run_id {
            let cx = self.run_ctx(r)?;
            if let Some(n) = node
                && cx.team.node(n).is_none()
            {
                bail!("no node {n} in the team");
            }
            t.team_run_id = Some(r.into());
            t.feature_id = Some(cx.feature.id.clone());
        }
        t.node = node.map(str::to_string);
        t.notes.push(orch_store::TaskNote { ts_ms: now_ms(), by: "user".into(), text: "created by the user".into() });
        let out = self.tasks_for(&project).create(t)?;
        if let Some(r) = run_id {
            self.run_event(
                r,
                None,
                "task.created",
                json!({"task": out.value.id, "title": out.value.title, "by": "user"}),
            );
        }
        Ok(out.value)
    }

    /// The user moves, reassigns or annotates a task (board, Feature pane).
    pub fn user_update_task(
        &self,
        task_id: u64,
        status: Option<&str>,
        node: Option<&str>,
        note: Option<&str>,
    ) -> Result<Task> {
        let mut t = self.db.task(task_id)?.ok_or_else(|| anyhow!("no task {task_id}"))?;
        if let Some(n) = node {
            t = self.reassign_task(task_id, n)?;
        }
        let project = self.db.project(&t.project_id)?.ok_or_else(|| anyhow!("no project {}", t.project_id))?;
        let svc = self.tasks_for(&project);
        if let Some(s) = status {
            let s: TaskStatus = s.parse().map_err(|e: String| anyhow!(e))?;
            if s != t.status {
                self.apply_status(&mut t, s);
                t = svc.update(t, now_ms())?.value;
                if let Some(r) = &t.team_run_id {
                    self.run_event(r, None, "task.updated", json!({"task": task_id, "status": s, "by": "user"}));
                }
            }
        }
        if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
            t = svc.note(task_id, "user", n, now_ms())?.value;
        }
        Ok(t)
    }

    /// FR-409: the user messages the lead or any agent of a run.
    pub fn user_message(&self, run_id: &str, to: &str, body: &str) -> Result<Vec<u64>> {
        let cx = self.run_ctx(run_id)?;
        self.post_message(&cx, crate::team::Sender(rules::USER, None), to, "message", body, Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::tests::test_runtime;
    use orch_team::{PermissionProfile, RoleDef, Scope};

    #[test]
    fn gated_roles_move_to_auto_mode_once() {
        let (_d, rt) = test_runtime(true);
        let mut r = RoleDef::new("mine", "Mine");
        r.permission_profile = PermissionProfile::Gated;
        rt.db.upsert_role(&r, 1).unwrap();
        rt.install_presets().unwrap();
        let get = || rt.db.role(&Scope::Global, "mine").unwrap().unwrap().permission_profile;
        assert_eq!(get(), PermissionProfile::Autonomous);
        // Set back to gated by the user: it stays.
        rt.db.upsert_role(&r, 2).unwrap();
        rt.install_presets().unwrap();
        assert_eq!(get(), PermissionProfile::Gated);
    }
}
