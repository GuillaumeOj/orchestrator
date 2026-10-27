//! Teams in the app (P2): the library for the Team canvas, launching on a
//! feature, the Feature pane, the Board and the project's task settings.
//! Thin over the runtime (`agent_runtime::team`, `team_admin`); the views are
//! polled by their panes like the Week planner.

use anyhow::{Result, anyhow};
use orch_store::{ProjectSettings, TaskBackendConfig};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::Core;

/// The project settings form (FR-109).
#[derive(Clone, Debug, Deserialize)]
pub struct ProjectSettingsForm {
    pub settings: ProjectSettings,
    pub backend: TaskBackendConfig,
    pub track_orchestrator_dir: bool,
}

/// Every feature with its latest run, for the sidebar (Projects → Features
/// → agents), the palette and ⌘P.
pub fn features(rt: &agent_runtime::runtime::Runtime) -> Vec<Value> {
    // Polled with the app state: three queries in all, not two per feature.
    let mut latest: std::collections::BTreeMap<String, orch_store::TeamRun> = std::collections::BTreeMap::new();
    for r in rt.db.runs(None, false).unwrap_or_default() {
        latest.entry(r.feature_id.clone()).or_insert(r); // newest first
    }
    let counts = rt.db.task_counts_by_run().unwrap_or_default();
    rt.db
        .features(None)
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            let run = latest.remove(&f.id);
            let (done, total) = run.as_ref().and_then(|r| counts.get(&r.id).copied()).unwrap_or_default();
            json!({
                "id": f.id, "project_id": f.project_id, "slug": f.slug, "title": f.title,
                "branch": f.integration_branch(),
                "run": run.as_ref().map(|r| json!({"id": r.id, "status": r.status, "team_id": r.team_id, "pr_url": r.pr_url})),
                "tasks": {"done": done, "total": total},
            })
        })
        .collect()
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
        Ok(json!({
            "project": {"id": p.id, "name": p.name},
            "backend": backend.kind(),
            "repo": match &backend { TaskBackendConfig::Github { repo } => json!(repo), TaskBackendConfig::Local => Value::Null },
            "last_sync_error": self.sync_errors.lock().ok().and_then(|m| m.get(project_id).cloned()),
            "tasks": rt.db.tasks(Some(project_id), None, None)?,
            "features": features.iter().map(|f| json!({"id": f.id, "slug": f.slug})).collect::<Vec<_>>(),
            "runs": runs,
        }))
    }

    /// The Board's `r`: sync the project's remote backend now (FR-503).
    pub fn board_sync(&self, project_id: &str) -> Result<Value> {
        let rt = self.runtime()?;
        let p = rt.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))?;
        let r = rt.tasks_for(&p).sync(orch_store::now_ms());
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

    pub fn project_settings_set(&self, project_id: &str, f: ProjectSettingsForm) -> Result<Value> {
        let rt = self.runtime()?;
        rt.db.set_project_settings(project_id, &f.settings, &f.backend, f.track_orchestrator_dir)?;
        self.project_settings(project_id)
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
