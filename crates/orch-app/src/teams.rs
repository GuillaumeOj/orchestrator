//! Teams in the app (P2): the library for the Team canvas, launching on a
//! feature, the Feature pane, the Board and the project's task settings.
//! Thin over the runtime (`agent_runtime::team`, `team_admin`); the views are
//! polled by their panes like the Week planner.

use anyhow::{Result, anyhow, bail};
use orch_store::{ProjectSettings, TaskBackendConfig};
use orch_team::Scope;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::Core;

/// The project settings form (FR-109).
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectSettingsForm {
    pub settings: ProjectSettings,
    pub backend: TaskBackendConfig,
    pub track_orchestrator_dir: bool,
    /// D8, switching to Linear with tasks: `push_open` or `leave`.
    #[serde(default)]
    pub migrate: Option<String>,
}

/// Every feature with its latest run, for the sidebar (Projects → Features
/// → agents), the palette and ⌘P. A deleted (archived) feature is listed,
/// flagged `archived`, while one of its agents is not deleted: its team is
/// never hidden. An error is not "no features": the UI would hide every
/// feature and close their panes for that poll.
pub fn features(rt: &agent_runtime::runtime::Runtime) -> Result<Vec<Value>> {
    // Polled with the app state: three queries in all, not two per feature,
    // plus one per deleted feature whose team has agents left.
    let mut latest: std::collections::BTreeMap<String, orch_store::TeamRun> = std::collections::BTreeMap::new();
    for r in rt.db.runs(None, false)? {
        latest.entry(r.feature_id.clone()).or_insert(r); // newest first
    }
    let counts = rt.db.task_counts_by_run()?;
    let mut list = rt.db.features(None)?;
    let mut seated: std::collections::BTreeSet<String> = rt.store.read(|s| {
        s.agents
            .values()
            .filter(|a| a.tracker.state != agent_runtime::state::AgentState::Archived)
            .filter_map(|a| a.team.as_ref().map(|t| t.feature_id.clone()))
            .collect()
    });
    for f in &list {
        seated.remove(&f.id);
    }
    for id in seated {
        list.extend(rt.db.feature(&id)?);
    }
    Ok(list
        .into_iter()
        .map(|f| {
            let run = latest.remove(&f.id);
            let (done, total) = run.as_ref().and_then(|r| counts.get(&r.id).copied()).unwrap_or_default();
            json!({
                "id": f.id, "project_id": f.project_id, "slug": f.slug, "title": f.title,
                "branch": f.integration_branch(), "archived": f.archived,
                "run": run.as_ref().map(|r| json!({"id": r.id, "status": r.status, "team_id": r.team_id, "pr_url": r.pr_url})),
                "tasks": {"done": done, "total": total},
            })
        })
        .collect())
}

impl Core {
    pub fn teams_view(&self, project: Option<&str>) -> Result<Value> {
        let rt = self.runtime()?;
        // The starter library is there from the first look (FR-306).
        rt.ensure_presets()?;
        let mut v = rt.teams_json(project)?;
        v["presets"] = json!(orch_team::presets::all().iter().map(|(_, t)| t.id.clone()).collect::<Vec<_>>());
        Ok(v)
    }

    pub fn team_save(&self, team: Value) -> Result<Value> {
        self.runtime()?.save_team(serde_json::from_value(team)?)
    }

    pub fn role_save(&self, role: Value) -> Result<Value> {
        Ok(json!(self.runtime()?.save_role(serde_json::from_value(role)?)?))
    }

    pub fn team_import(&self, text: &str) -> Result<Value> {
        Ok(json!(self.runtime()?.import_team(text)?))
    }

    pub fn team_export(&self, team_id: &str, project: Option<&str>) -> Result<String> {
        self.runtime()?.export_team(team_id, project)
    }

    pub fn team_launch(&self, spec: agent_runtime::team::LaunchSpec) -> Result<Value> {
        Ok(json!(self.runtime()?.launch(&spec)?))
    }

    /// The Feature pane: a feature's latest run with its agents, tasks, docs,
    /// artifacts and timeline (no run yet: the feature alone).
    pub fn feature_view(&self, feature_id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let f = rt.db.feature(feature_id)?.ok_or_else(|| anyhow!("no feature {feature_id}"))?;
        match rt.db.runs(Some(&f.id), false)?.into_iter().next() {
            Some(run) => rt.run_json(&run.id),
            None => Ok(
                json!({"feature": f, "run": null, "agents": [], "tasks": [], "docs": [], "artifacts": [], "timeline": []}),
            ),
        }
    }

    /// FR-603: an exported image's bytes (base64) for the Design section.
    pub fn artifact_preview(&self, feature_id: &str, artifact: u64) -> Result<Value> {
        self.runtime()?.artifact_preview(feature_id, artifact)
    }

    /// A link in the browser, a file in Quick Look.
    pub fn artifact_open(&self, feature_id: &str, artifact: u64) -> Result<()> {
        self.runtime()?.artifact_open(feature_id, artifact)
    }

    pub fn team_duplicate(&self, team: &str, from: &Scope, to: &Scope, new_id: Option<&str>) -> Result<Value> {
        self.runtime()?.team_duplicate(team, from, to, new_id)
    }

    /// The shipped starter, copied into `to` (not the installed, maybe edited one).
    pub fn starter_copy(&self, starter: &str, to: &Scope, new_id: Option<&str>) -> Result<Value> {
        self.runtime()?.starter_copy(starter, to, new_id)
    }

    pub fn role_duplicate(&self, role: &str, from: &Scope, to: &Scope, new_id: Option<&str>) -> Result<Value> {
        Ok(json!(self.runtime()?.role_duplicate(role, from, to, new_id)?))
    }

    pub fn team_set_preset(&self, team: &str, scope: &Scope, preset: bool) -> Result<Value> {
        Ok(json!(self.runtime()?.team_set_preset(team, scope, preset)?))
    }

    pub fn preset_update(&self, team: &str, scope: &Scope) -> Result<Value> {
        Ok(json!(self.runtime()?.preset_update(team, scope)?))
    }

    pub fn feature_doc(&self, feature_id: &str, name: &str) -> Result<String> {
        self.runtime()?.feature_doc(feature_id, name)
    }

    /// The Board (FR-502): a project's tasks in one model whatever the
    /// backend, with the nodes each active run can assign to.
    pub fn board_view(&self, project_id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        let features = rt.db.features(Some(project_id))?;
        // One query for the active runs (newest first), then each feature's snapshot.
        let mut active: std::collections::HashMap<String, orch_store::TeamRun> = std::collections::HashMap::new();
        for r in rt.db.runs(None, true)? {
            active.entry(r.feature_id.clone()).or_insert(r);
        }
        let runs: Vec<Value> = features
            .iter()
            .filter_map(|f| {
                let run = active.remove(&f.id)?;
                let (team, _) = rt.db.team_version(&run.team_scope, &run.team_id, run.team_version).ok()??;
                let nodes = team
                    .nodes
                    .into_iter()
                    .map(|n| json!({"id": n.id, "role": n.role_id, "lead": n.is_lead}))
                    .collect::<Vec<_>>();
                Some(json!({"id": run.id, "feature_id": f.id, "slug": f.slug, "status": run.status, "nodes": nodes}))
            })
            .collect();
        let backend = p.backend();
        let last = rt.last_sync(project_id);
        let won: Vec<u64> = last["conflicts"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
        let marks = rt.off_hours_marks();
        let tasks: Vec<Value> = rt
            .db
            .tasks(Some(project_id), None, None)?
            .into_iter()
            .map(|t| {
                let off = marks.tasks.contains(&t.id) || t.team_run_id.as_ref().is_some_and(|r| marks.runs.contains(r));
                let mut v = json!(t);
                v["remote_won"] = json!(won.contains(&t.id));
                v["off_hours"] = json!(off);
                v
            })
            .collect();
        Ok(json!({
            "project": {"id": p.id, "name": p.name},
            "backend": backend.kind(),
            "repo": match &backend { TaskBackendConfig::Github { repo } => json!(repo), _ => Value::Null },
            "linear_team": match &backend { TaskBackendConfig::Linear { team, .. } => json!(team), _ => Value::Null },
            "last_sync_at_ms": last["at_ms"],
            "last_sync": last.get("report").cloned().unwrap_or(Value::Null),
            "last_sync_error": self
                .sync_errors
                .lock()
                .ok()
                .and_then(|m| m.get(project_id).cloned())
                .map(Value::from)
                .unwrap_or_else(|| last["error"].clone()),
            "tasks": tasks,
            "features": features.iter().map(|f| json!({"id": f.id, "slug": f.slug})).collect::<Vec<_>>(),
            "runs": runs,
        }))
    }

    /// The Board's `r`: sync the project's remote backend now (FR-503).
    pub fn board_sync(&self, project_id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        let r = rt.sync_tasks(&p);
        if let Ok(mut m) = self.sync_errors.lock() {
            match &r {
                Ok(_) => m.remove(project_id),
                Err(e) => m.insert(project_id.to_string(), format!("{e:#}")),
            };
        }
        Ok(r?.summary())
    }

    pub fn task_create(&self, f: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct F {
            project_id: String,
            title: String,
            #[serde(default)]
            body: String,
            #[serde(default)]
            node: Option<String>,
            #[serde(default)]
            run_id: Option<String>,
            #[serde(default)]
            depends_on: Vec<u64>,
        }
        let f: F = serde_json::from_value(f)?;
        let t = self.runtime()?.user_create_task(
            &f.project_id,
            &f.title,
            &f.body,
            f.node.as_deref(),
            f.run_id.as_deref(),
            f.depends_on,
        )?;
        Ok(json!(t))
    }

    pub fn task_update(&self, id: u64, status: Option<&str>, node: Option<&str>, note: Option<&str>) -> Result<Value> {
        Ok(json!(self.runtime()?.user_update_task(id, status, node, note)?))
    }

    pub fn run_act(&self, run_id: &str, action: &str) -> Result<Value> {
        self.runtime()?.run_act(run_id, action)
    }

    pub fn message_send(&self, run_id: &str, to: &str, body: &str) -> Result<Value> {
        Ok(json!({"ids": self.runtime()?.user_message(run_id, to, body)?}))
    }

    pub fn project_settings(&self, project_id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        Ok(
            json!({"settings": p.typed_settings(), "backend": p.backend(), "track_orchestrator_dir": p.track_orchestrator_dir,
                  "default_branch": p.default_branch}),
        )
    }

    /// D8: switching **to** Linear a project that has tasks needs `migrate`:
    /// without it nothing is saved and the answer is `{confirm: {kind:
    /// "linear_migrate", open, done}}`. `push_open` gives the open tasks to
    /// Linear (created at the sync that follows); `leave` keeps every task
    /// local and unsynced. Done tasks are never pushed.
    pub fn project_settings_set(&self, project_id: &str, f: ProjectSettingsForm) -> Result<Value> {
        let rt = self.runtime()?;
        let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        let old = p.backend();
        // A change to Linear, or to another Linear team or project (D8).
        let to_linear = match (&old, &f.backend) {
            (
                TaskBackendConfig::Linear { team: a, project: pa, .. },
                TaskBackendConfig::Linear { team: b, project: pb, .. },
            ) => a != b || pa != pb,
            (_, TaskBackendConfig::Linear { .. }) => true,
            _ => false,
        };
        let relink = to_linear && old.kind() == "linear";
        let mut push = vec![];
        let mut detach = vec![];
        if to_linear {
            let tasks = rt.db.tasks(Some(project_id), None, None)?;
            let (open, done): (Vec<_>, Vec<_>) =
                tasks.into_iter().partition(|t| t.status != orch_team::TaskStatus::Done);
            if !open.is_empty() || !done.is_empty() {
                match f.migrate.as_deref() {
                    None => {
                        return Ok(
                            json!({"confirm": {"kind": "linear_migrate", "open": open.len(), "done": done.len()}}),
                        );
                    }
                    Some("push_open") => {
                        push = open;
                        detach = done;
                    }
                    Some("leave") => detach = open.into_iter().chain(done).collect(),
                    Some(other) => bail!("migrate {other:?}: use push_open or leave"),
                }
            }
        }
        rt.db.set_project_settings(project_id, &f.settings, &f.backend, f.track_orchestrator_dir)?;
        // From one Linear team or project to another: the old issues are not
        // the new team's, so the tasks not pushed stay local (never written
        // to the old issues again).
        if relink {
            for mut t in detach.into_iter().filter(|t| t.backend == "linear") {
                t.backend = "local".into();
                rt.db.upsert_task(&t)?;
            }
        }
        if !push.is_empty() {
            for mut t in push {
                (t.backend, t.external_id, t.remote_updated_at) = ("linear".into(), None, None);
                rt.db.upsert_task(&t)?;
            }
            let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
            if let Err(e) = rt.sync_tasks(&p) {
                // Kept for the next sync; the Board shows the error.
                if let Ok(mut m) = self.sync_errors.lock() {
                    m.insert(project_id.to_string(), format!("{e:#}"));
                }
            }
        }
        self.project_settings(project_id)
    }

    pub fn linear_teams(&self) -> Result<Value> {
        self.runtime()?.linear_teams()
    }

    pub fn linear_states(&self, team: &str) -> Result<Value> {
        self.runtime()?.linear_states(team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_parse_like_the_ui_sends_them() {
        let f: agent_runtime::team::LaunchSpec =
            serde_json::from_value(json!({"project_id": "p", "team_id": "full-stack", "slug": "x"})).unwrap();
        assert_eq!((f.title.as_str(), f.base), ("", None));
        let s: ProjectSettingsForm = serde_json::from_value(json!({
            "settings": {"default_team": "full-stack", "copy_env": false},
            "backend": {"type": "github", "repo": "me/shop"},
            "track_orchestrator_dir": true
        }))
        .unwrap();
        assert_eq!(s.backend, TaskBackendConfig::Github { repo: Some("me/shop".into()) });
    }
}
