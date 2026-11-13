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
            "timeline": self.timeline_json(run_id)?,
        }))
    }

    /// The run's events, newest first; a message event gains `delivery`
    /// (api-contract §3): how far its message(s) got.
    fn timeline_json(&self, run_id: &str) -> Result<Vec<Value>> {
        use orch_store::DeliveryState as D;
        let now = now_ms();
        let events = self.db.run_events(run_id, 200)?;
        let ids_of = |e: &orch_store::Event| -> Vec<u64> {
            e.detail["ids"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()
        };
        // One lookup for every message of the timeline.
        let all: Vec<u64> = events.iter().flat_map(ids_of).collect();
        let by_id: std::collections::HashMap<u64, orch_store::Message> =
            self.db.messages_by_id(&all).unwrap_or_default().into_iter().map(|m| (m.id, m)).collect();
        let mut out = vec![];
        // One blocker per agent for the whole timeline.
        let mut blockers: std::collections::HashMap<String, Option<String>> = Default::default();
        for e in events {
            let ids = ids_of(&e);
            let mut v = serde_json::to_value(&e)?;
            if !ids.is_empty() {
                let msgs: Vec<orch_store::Message> = ids.iter().filter_map(|i| by_id.get(i).cloned()).collect();
                // The least advanced message tells how far the event got.
                let rank = |m: &orch_store::Message| match m.delivery_state() {
                    D::Failed => 0,
                    D::Queued => 1,
                    D::Handed => 2,
                    D::Seen => 3,
                    D::Read => 4,
                };
                if let Some(m) = msgs.iter().min_by_key(|m| rank(m)) {
                    let waiting = matches!(m.delivery_state(), D::Queued | D::Handed);
                    let blocker = m
                        .to_agent
                        .as_deref()
                        .filter(|_| {
                            waiting && now.saturating_sub(m.created_at_ms) > crate::attention::UNDELIVERED_AFTER_MS
                        })
                        .and_then(|a| {
                            blockers
                                .entry(a.to_string())
                                .or_insert_with(|| {
                                    let rec = self.store.read(|s| s.agents.get(a).cloned())?;
                                    Some(self.blocker_for(&rec, now))
                                })
                                .clone()
                        });
                    v["delivery"] = json!({
                        "state": m.delivery_state(),
                        "attempts": msgs.iter().map(|m| m.attempts).max().unwrap_or(0),
                        "via": m.via,
                        "blocker": blocker,
                        "seen_at_ms": m.delivered_at_ms,
                        "read_at_ms": m.read_at_ms,
                    });
                }
            }
            out.push(v);
        }
        Ok(out)
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

    /// The user moves, reassigns, retitles, rewrites or annotates a task
    /// (board, Feature pane). A title or body equal to the task's is no
    /// change; status, title and body are saved and pushed together.
    pub fn user_update_task(
        &self,
        task_id: u64,
        status: Option<&str>,
        node: Option<&str>,
        note: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Task> {
        let title = title.map(str::trim);
        if title == Some("") {
            bail!("the task needs a title");
        }
        let body = body.map(str::trim_end);
        let mut t = self.db.task(task_id)?.ok_or_else(|| anyhow!("no task {task_id}"))?;
        if let Some(n) = node {
            t = self.reassign_task(task_id, n)?;
        }
        let project = self.db.project(&t.project_id)?.ok_or_else(|| anyhow!("no project {}", t.project_id))?;
        let svc = self.tasks_for(&project);
        let mut changed = false;
        let mut moved = None;
        if let Some(s) = status {
            let s: TaskStatus = s.parse().map_err(|e: String| anyhow!(e))?;
            if s != t.status {
                self.apply_status(&mut t, s);
                changed = true;
                moved = Some(s);
            }
        }
        if let Some(title) = title.filter(|x| *x != t.title) {
            t.title = title.to_string();
            changed = true;
        }
        if let Some(body) = body.filter(|x| *x != t.body) {
            t.body = body.to_string();
            changed = true;
        }
        if changed {
            t = svc.update(t, now_ms())?.value;
            if let (Some(s), Some(r)) = (moved, &t.team_run_id) {
                self.run_event(r, None, "task.updated", json!({"task": task_id, "status": s, "by": "user"}));
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
    use std::sync::{Arc, Mutex};

    use crate::runtime::Runtime;
    use crate::runtime::tests::{test_cfg, test_runtime};
    use crate::team::{GhFactory, OPEN_SYNC_AFTER_MS};
    use orch_store::{Project, Task};
    use orch_team::{PermissionProfile, RoleDef, Scope};
    use serde_json::json;

    /// A `gh` over an empty repo that logs the PATCHes and list calls it gets.
    #[derive(Clone, Default)]
    struct CountingGh(Arc<Mutex<Vec<String>>>);

    impl orch_tasks::github::Gh for CountingGh {
        fn run(&self, args: &[String]) -> anyhow::Result<String> {
            self.0.lock().unwrap().push(args.join(" "));
            match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["api", "--paginate", ..] => Ok("[]".into()),
                ["api", "-X", "PATCH", ..] => Ok(json!({"number": 7, "title": "T", "body": "", "state": "open",
                    "labels": [], "updated_at": "2026-11-02T10:00:00Z"})
                .to_string()),
                _ => Ok("{}".into()),
            }
        }
    }

    fn project(rt: &Runtime, id: &str, backend: serde_json::Value) -> Project {
        let p = Project {
            id: id.into(),
            name: id.into(),
            path: std::env::temp_dir().join(id),
            default_branch: "main".into(),
            remotes: vec![],
            worktree_root: std::env::temp_dir().join(id).join("w"),
            task_backend: backend,
            track_orchestrator_dir: true,
            settings: json!({}),
            created_at_ms: 1,
            last_active_at_ms: 1,
            last_fetch_at_ms: None,
            last_fetch_error: None,
            archived: false,
        };
        rt.db.upsert_project(&p).unwrap();
        p
    }

    fn github_runtime() -> (tempfile::TempDir, Arc<Runtime>, CountingGh) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let gh = CountingGh::default();
        let mut cfg = test_cfg(d.path(), false);
        let g = gh.clone();
        cfg.gh = Some(GhFactory(Arc::new(move || Box::new(g.clone()))));
        (d, Runtime::new(cfg).unwrap(), gh)
    }

    fn lists(gh: &CountingGh) -> usize {
        gh.0.lock().unwrap().iter().filter(|c| c.contains("--paginate")).count()
    }

    #[test]
    fn the_task_form_edits_title_and_body() {
        let (_d, rt, gh) = github_runtime();
        project(&rt, "p", json!({"type": "github", "repo": "me/shop"}));
        let mut t = Task::new("p", "github", "Old title", 1);
        t.external_id = Some("7".into());
        t.body = "old body".into();
        t.remote_updated_at = Some("2026-11-02T09:00:00Z".into());
        let id = rt.db.upsert_task(&t).unwrap();
        let patches = || gh.0.lock().unwrap().iter().filter(|c| c.contains("PATCH")).count();
        // Trimmed; one push for title and body.
        let t = rt.user_update_task(id, None, None, None, Some("  New title \n"), Some("new body \n\n")).unwrap();
        assert_eq!((t.title.as_str(), t.body.as_str()), ("New title", "new body"));
        assert_eq!(patches(), 1);
        let call = gh.0.lock().unwrap().iter().find(|c| c.contains("PATCH")).cloned().unwrap();
        assert!(call.contains("title=New title") && call.contains("body=new body"), "{call}");
        assert!(!call.contains("labels") && !call.contains("state="), "{call}");
        // Nothing changed: no write. An empty title is refused, nothing saved.
        rt.user_update_task(id, None, None, None, Some("New title"), Some("new body")).unwrap();
        rt.user_update_task(id, None, None, None, None, None).unwrap();
        assert_eq!(patches(), 1);
        let e = rt.user_update_task(id, None, None, None, Some("  "), Some("x")).unwrap_err();
        assert_eq!(e.to_string(), "the task needs a title");
        assert_eq!(rt.db.task(id).unwrap().unwrap().body, "new body");
        // An empty body clears it.
        let t = rt.user_update_task(id, None, None, None, None, Some("")).unwrap();
        assert_eq!(t.body, "");
    }

    #[test]
    fn opening_a_board_syncs_at_most_once_a_minute_and_watches_the_project() {
        let (_d, rt, gh) = github_runtime();
        let p = project(&rt, "p", json!({"type": "github", "repo": "me/shop"}));
        let local = project(&rt, "l", json!({"type": "local"}));
        assert!(rt.open_sync(&local).is_none(), "a local backend has nothing to sync");
        assert!(rt.open_sync(&p).unwrap().is_ok());
        assert_eq!(lists(&gh), 2, "open issues + closed orch issues");
        // Quick re-open: nothing, but still watched.
        assert!(rt.open_sync(&p).is_none());
        assert_eq!(lists(&gh), 2);
        // Older than the throttle: syncs again.
        let mut last = rt.last_sync("p");
        last["at_ms"] = json!(orch_protocol::now_ms() - OPEN_SYNC_AFTER_MS - 1);
        rt.db.set_setting(&format!("{}p", crate::team::LAST_SYNC_KEY), &last).unwrap();
        assert!(rt.open_sync(&p).unwrap().is_ok());
        assert_eq!(lists(&gh), 4, "synced again");
    }

    /// B1: the task sync reads the tracker, whatever way it was set.
    #[test]
    fn the_task_sync_follows_the_tracker() {
        let (_d, rt, gh) = github_runtime();
        let p = project(&rt, "p", json!({"type": "local"}));
        assert!(rt.open_sync(&p).is_none(), "Local: nothing to sync");
        let mut t = rt.db.tracker("p").unwrap();
        t.tracker = orch_store::Tracker::Github { repo: Some("me/shop".into()), project_board: None };
        t.ready = orch_store::ReadyFilter::defaults("github");
        rt.db.set_tracker("p", &t).unwrap();
        let p = rt.db.project("p").unwrap().unwrap();
        assert!(rt.open_sync(&p).unwrap().is_ok());
        let repos: std::collections::BTreeSet<_> =
            gh.0.lock().unwrap().iter().filter(|c| c.contains("--paginate")).map(|c| c.contains("me/shop")).collect();
        assert_eq!(repos, [true].into(), "the tracker's repository is listed");
        // Back to Local: no more sync.
        t.tracker = orch_store::Tracker::Local { backlog_glob: None };
        t.ready = orch_store::ReadyFilter::Local;
        rt.db.set_tracker("p", &t).unwrap();
        let p = rt.db.project("p").unwrap().unwrap();
        let before = lists(&gh);
        assert!(rt.open_sync(&p).is_none());
        assert_eq!(lists(&gh), before);
    }

    /// Two syncs at once of a project with a never-pushed task make one issue.
    #[test]
    fn concurrent_syncs_of_a_project_create_one_issue() {
        #[derive(Clone, Default)]
        struct SlowGh(Arc<Mutex<Vec<String>>>);
        impl orch_tasks::github::Gh for SlowGh {
            fn run(&self, args: &[String]) -> anyhow::Result<String> {
                let line = args.join(" ");
                self.0.lock().unwrap().push(line.clone());
                if line.contains("--paginate") {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    return Ok("[]".into());
                }
                if line.contains("-X POST") && line.ends_with("/issues")
                    || line.contains("-X POST repos/me/shop/issues")
                {
                    return Ok(json!({"number": 9, "title": "T", "body": "", "state": "open", "labels": [],
                        "updated_at": "2026-11-02T10:00:00Z"})
                    .to_string());
                }
                Ok("{}".into())
            }
        }
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let gh = SlowGh::default();
        let mut cfg = test_cfg(d.path(), false);
        let g = gh.clone();
        cfg.gh = Some(GhFactory(Arc::new(move || Box::new(g.clone()))));
        let rt = Runtime::new(cfg).unwrap();
        let p = project(&rt, "p", json!({"type": "github", "repo": "me/shop"}));
        rt.db.upsert_task(&Task::new("p", "github", "Never pushed", 1)).unwrap();
        let reports: Vec<_> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..2).map(|_| s.spawn(|| rt.sync_tasks(&p).unwrap())).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let creates = gh.0.lock().unwrap().iter().filter(|c| c.contains("-X POST repos/me/shop/issues ")).count();
        assert_eq!(creates, 1, "{:?}", gh.0.lock().unwrap());
        assert_eq!(reports[0], reports[1], "the second returns the first's report");
        assert_eq!(rt.db.tasks(Some("p"), None, None).unwrap().len(), 1);
    }

    #[test]
    fn the_background_sync_covers_watched_projects() {
        let (_d, rt, gh) = github_runtime();
        let p = project(&rt, "p", json!({"type": "github", "repo": "me/shop"}));
        project(&rt, "q", json!({"type": "github", "repo": "me/other"}));
        rt.sync_task_backends();
        assert_eq!(lists(&gh), 0, "no run, not watched");
        rt.open_sync(&p);
        let before = lists(&gh);
        rt.sync_task_backends();
        assert!(lists(&gh) > before, "the watched project is synced in the background");
        let repos: std::collections::BTreeSet<_> =
            gh.0.lock().unwrap().iter().filter(|c| c.contains("--paginate")).map(|c| c.contains("me/shop")).collect();
        assert_eq!(repos, [true].into(), "only the watched project");
    }

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
