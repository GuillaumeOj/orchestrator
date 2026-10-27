//! Tauri commands: thin wrappers over [`Core`](crate::core::Core). PTY output
//! uses the pull transport: `*_read` waits for output on a blocking thread and
//! returns everything buffered as one raw IPC response (ADR-001 §4).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::State;
use tauri::ipc::Response;

use crate::LockOk;
use crate::core::{Core, EditorOpen, NewSession, TermInfo};
use crate::kitty::KittyImport;
use crate::planner::PlannerView;
use crate::services::Project;
use crate::setup::PlanForm;
use crate::terms::TermKind;
use orch_git::project::RemovalChoices;

type Res<T> = Result<T, String>;
type CoreState<'a> = State<'a, Arc<Core>>;

/// A pull returns at least this often, so a closed pane's loop ends promptly.
const PULL_WAIT: Duration = Duration::from_secs(2);

fn e(err: anyhow::Error) -> String {
    format!("{err:#}")
}

/// Runs `f` on the blocking pool: async commands keep the main thread free
/// (sync commands run on it) and may wait on PTYs, processes or git.
async fn blocking<T: Send + 'static>(
    core: CoreState<'_>,
    f: impl FnOnce(&Core) -> anyhow::Result<T> + Send + 'static,
) -> Res<T> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&core).map_err(e)).await.map_err(|err| err.to_string())?
}

/// Off the main thread (sync commands run there): it runs git and scans processes.
#[tauri::command]
pub async fn app_state(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| Ok(c.state())).await
}

#[tauri::command]
pub fn kitty_config(core: CoreState) -> KittyImport {
    core.kitty.lock_ok().clone()
}

#[tauri::command]
pub fn font_files(family: String) -> Vec<(String, String)> {
    crate::fonts::font_files(&family)
}

#[tauri::command]
pub fn read_font(path: String) -> Res<Response> {
    crate::fonts::read_font(&path).map(Response::new).map_err(e)
}

// ---------------------------------------------------------------- agents

#[tauri::command]
pub fn agent_start(core: CoreState, req: NewSession) -> Res<String> {
    core.agent_start(&req).map_err(e)
}

#[tauri::command]
pub fn agent_attach(core: CoreState, id: String, rows: u16, cols: u16) -> Res<u64> {
    core.agent_attach(&id, rows, cols).map_err(e)
}

#[tauri::command]
pub fn agent_detach(core: CoreState, id: String, reader: u64) {
    core.agent_detach(&id, reader)
}

#[tauri::command]
pub async fn agent_read(core: CoreState<'_>, id: String, reader: u64) -> Res<Response> {
    blocking(core, move |c| c.agent_read(&id, reader, PULL_WAIT).map(Response::new)).await
}

#[tauri::command]
pub fn agent_write(core: CoreState, id: String, data: String) -> Res<()> {
    core.agent_write(&id, data.as_bytes()).map_err(e)
}

#[tauri::command]
pub fn agent_resize(core: CoreState, id: String, rows: u16, cols: u16) -> Res<()> {
    core.runtime().and_then(|rt| rt.resize(&id, rows, cols)).map_err(e)
}

#[tauri::command]
pub async fn agent_action(core: CoreState<'_>, id: String, action: String, message: Option<String>) -> Res<Value> {
    // Suspend and kill wait (bounded) for the process to exit.
    blocking(core, move |c| c.agent_action(&id, &action, message.as_deref())).await
}

// ---------------------------------------------------------------- terminals

#[derive(serde::Deserialize)]
pub struct TermSpawn {
    pub kind: TermKind,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub run: Option<String>,
    pub rows: u16,
    pub cols: u16,
}

#[tauri::command]
pub fn term_spawn(core: CoreState, req: TermSpawn) -> Res<TermInfo> {
    let cwd = req.cwd.unwrap_or_else(|| core.pane_cwd(req.project_id.as_deref(), req.agent_id.as_deref()));
    core.term_spawn(req.kind, &cwd, req.run.as_deref(), req.rows, req.cols).map_err(e)
}

#[tauri::command]
pub async fn term_read(core: CoreState<'_>, id: u32) -> Res<Response> {
    blocking(core, move |c| {
        c.terms.get(id)?.output.read_wait(PULL_WAIT).map(Response::new).ok_or_else(|| anyhow::anyhow!("eof"))
    })
    .await
}

#[tauri::command]
pub fn term_write(core: CoreState, id: u32, data: String) -> Res<()> {
    core.terms.get(id).and_then(|t| t.write(data.as_bytes())).map_err(e)
}

#[tauri::command]
pub fn term_resize(core: CoreState, id: u32, rows: u16, cols: u16) -> Res<()> {
    core.terms.get(id).and_then(|t| t.resize(rows, cols)).map_err(e)
}

#[tauri::command]
pub fn term_kill(core: CoreState, id: u32) {
    core.terms.kill(id)
}

#[tauri::command]
pub async fn editor_open(
    core: CoreState<'_>,
    project_id: Option<String>,
    agent_id: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    rows: u16,
    cols: u16,
) -> Res<EditorOpen> {
    blocking(core, move |c| {
        c.editor_open(project_id.as_deref(), agent_id.as_deref(), file.as_deref(), line, rows, cols)
    })
    .await
}

#[tauri::command]
pub async fn open_in_kitty(core: CoreState<'_>, cwd: PathBuf) -> Res<()> {
    blocking(core, move |c| c.open_in_kitty(&cwd)).await
}

// ---------------------------------------------------------------- inbox, governor

#[tauri::command]
pub async fn inbox_act(core: CoreState<'_>, item: String, action: String, message: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.inbox_act(&item, &action, message.as_deref())).await
}

// ---- P2 teams

#[tauri::command]
pub async fn teams_view(core: CoreState<'_>, project: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.teams_view(project.as_deref())).await
}

#[tauri::command]
pub async fn team_save(core: CoreState<'_>, team: Value) -> Res<Value> {
    blocking(core, move |c| c.team_save(team)).await
}

#[tauri::command]
pub async fn role_save(core: CoreState<'_>, role: Value) -> Res<Value> {
    blocking(core, move |c| c.role_save(role)).await
}

#[tauri::command]
pub async fn team_import(core: CoreState<'_>, json: String) -> Res<Value> {
    blocking(core, move |c| c.team_import(&json)).await
}

#[tauri::command]
pub async fn team_export(core: CoreState<'_>, team: String, project: Option<String>) -> Res<String> {
    blocking(core, move |c| c.team_export(&team, project.as_deref())).await
}

#[tauri::command]
pub async fn team_launch(core: CoreState<'_>, form: agent_runtime::team::LaunchSpec) -> Res<Value> {
    blocking(core, move |c| c.team_launch(form)).await
}

#[tauri::command]
pub async fn feature_view(core: CoreState<'_>, feature: String) -> Res<Value> {
    blocking(core, move |c| c.feature_view(&feature)).await
}

#[tauri::command]
pub async fn feature_doc(core: CoreState<'_>, feature: String, name: String) -> Res<String> {
    blocking(core, move |c| c.feature_doc(&feature, &name)).await
}

// ---- P3 design artifacts and presets (api-contract §3–4)

#[tauri::command]
pub async fn artifact_preview(core: CoreState<'_>, feature: String, artifact: u64) -> Res<Value> {
    blocking(core, move |c| c.artifact_preview(&feature, artifact)).await
}

#[tauri::command]
pub async fn artifact_open(core: CoreState<'_>, feature: String, artifact: u64) -> Res<()> {
    blocking(core, move |c| c.artifact_open(&feature, artifact)).await
}

#[tauri::command]
pub async fn team_duplicate(
    core: CoreState<'_>,
    team: String,
    from: orch_team::Scope,
    to: orch_team::Scope,
    new_id: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.team_duplicate(&team, &from, &to, new_id.as_deref())).await
}

#[tauri::command]
pub async fn role_duplicate(
    core: CoreState<'_>,
    role: String,
    from: orch_team::Scope,
    to: orch_team::Scope,
    new_id: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.role_duplicate(&role, &from, &to, new_id.as_deref())).await
}

#[tauri::command]
pub async fn team_set_preset(core: CoreState<'_>, team: String, scope: orch_team::Scope, preset: bool) -> Res<Value> {
    blocking(core, move |c| c.team_set_preset(&team, &scope, preset)).await
}

#[tauri::command]
pub async fn preset_update(core: CoreState<'_>, team: String, scope: orch_team::Scope) -> Res<Value> {
    blocking(core, move |c| c.preset_update(&team, &scope)).await
}

#[tauri::command]
pub async fn board_view(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.board_view(&project)).await
}

#[tauri::command]
pub async fn board_sync(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.board_sync(&project)).await
}

#[tauri::command]
pub async fn task_create(core: CoreState<'_>, form: Value) -> Res<Value> {
    blocking(core, move |c| c.task_create(form)).await
}

#[tauri::command]
pub async fn task_update(
    core: CoreState<'_>,
    id: u64,
    status: Option<String>,
    node: Option<String>,
    note: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.task_update(id, status.as_deref(), node.as_deref(), note.as_deref())).await
}

#[tauri::command]
pub async fn run_act(core: CoreState<'_>, run: String, action: String) -> Res<Value> {
    blocking(core, move |c| c.run_act(&run, &action)).await
}

#[tauri::command]
pub async fn message_send(core: CoreState<'_>, run: String, to: String, body: String) -> Res<Value> {
    blocking(core, move |c| c.message_send(&run, &to, &body)).await
}

#[tauri::command]
pub async fn project_settings(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.project_settings(&project)).await
}

#[tauri::command]
pub async fn project_settings_set(
    core: CoreState<'_>,
    project: String,
    form: crate::teams::ProjectSettingsForm,
) -> Res<Value> {
    blocking(core, move |c| c.project_settings_set(&project, form)).await
}

// ---- P3 integrations and the MCP registry (api-contract §1–2)

#[tauri::command]
pub async fn integrations_view(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| c.integrations_view()).await
}

/// The secret is wrapped at once and never logged or returned (NFR-3).
#[tauri::command]
pub async fn integration_secret_set(core: CoreState<'_>, id: String, secret: String) -> Res<Value> {
    let secret = orch_secrets::Secret::new(secret);
    blocking(core, move |c| c.integration_secret_set(&id, secret)).await
}

#[tauri::command]
pub async fn integration_secret_remove(core: CoreState<'_>, id: String) -> Res<Value> {
    blocking(core, move |c| c.integration_secret_remove(&id)).await
}

#[tauri::command]
pub async fn integration_test(core: CoreState<'_>, id: String) -> Res<Value> {
    blocking(core, move |c| c.integration_test(&id)).await
}

#[tauri::command]
pub async fn mcp_registry_view(core: CoreState<'_>, project: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.mcp_registry_view(project.as_deref())).await
}

#[tauri::command]
pub async fn mcp_server_save(
    core: CoreState<'_>,
    server: orch_team::mcp::McpServer,
    scope: orch_team::Scope,
) -> Res<Value> {
    blocking(core, move |c| c.mcp_server_save(server, scope)).await
}

#[tauri::command]
pub async fn mcp_server_delete(core: CoreState<'_>, id: String, scope: orch_team::Scope) -> Res<Value> {
    blocking(core, move |c| c.mcp_server_delete(&id, scope)).await
}

#[tauri::command]
pub async fn mcp_server_test(core: CoreState<'_>, id: String, project: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.mcp_server_test(&id, project.as_deref())).await
}

#[tauri::command]
pub async fn gate_detail(core: CoreState<'_>, item: String) -> Res<Value> {
    blocking(core, move |c| c.gate_detail(&item)).await
}

#[tauri::command]
pub async fn planner_view(core: CoreState<'_>) -> Res<PlannerView> {
    blocking(core, |c| c.planner()).await
}

#[tauri::command]
pub fn governor_act(core: CoreState, action: String, arg: Option<f64>) -> Res<Value> {
    core.governor_act(&action, arg).map_err(e)
}

// ---------------------------------------------------------------- projects, settings

#[tauri::command]
pub async fn project_add(core: CoreState<'_>, path: PathBuf) -> Res<Project> {
    blocking(core, move |c| c.project_add(&path)).await
}

#[tauri::command]
pub async fn project_clone(
    core: CoreState<'_>,
    url: String,
    parent: Option<PathBuf>,
    name: Option<String>,
) -> Res<Project> {
    blocking(core, move |c| c.project_clone(&url, parent.as_deref(), name.as_deref())).await
}

#[tauri::command]
pub async fn project_detail(core: CoreState<'_>, id: String, with_prs: bool) -> Res<Value> {
    blocking(core, move |c| c.project_detail(&id, with_prs)).await
}

#[tauri::command]
pub async fn project_fetch(core: CoreState<'_>, id: String) -> Res<Value> {
    blocking(core, move |c| c.project_fetch(&id)).await
}

#[tauri::command]
pub async fn worktree_remove(
    core: CoreState<'_>,
    project: String,
    path: PathBuf,
    delete_branch: bool,
    discard: bool,
) -> Res<Value> {
    blocking(core, move |c| c.worktree_remove(&project, &path, delete_branch, discard)).await
}

#[tauri::command]
pub async fn worktree_diff(core: CoreState<'_>, project: String, path: PathBuf) -> Res<Value> {
    blocking(core, move |c| c.worktree_diff(&project, &path)).await
}

#[tauri::command]
pub async fn worktree_create_pr(
    core: CoreState<'_>,
    project: String,
    path: PathBuf,
    title: String,
    body: String,
    draft: bool,
) -> Res<String> {
    blocking(core, move |c| c.worktree_create_pr(&project, &path, &title, &body, draft)).await
}

#[tauri::command]
pub async fn project_removal_plan(core: CoreState<'_>, id: String) -> Res<Value> {
    blocking(core, move |c| c.removal_plan(&id)).await
}

#[tauri::command]
pub async fn project_remove(core: CoreState<'_>, id: String, choices: RemovalChoices) -> Res<Value> {
    blocking(core, move |c| c.project_remove(&id, &choices)).await
}

// ---------------------------------------------------------------- first run (J0)

#[tauri::command]
pub fn set_editor(core: CoreState, template: String) -> Res<()> {
    core.set_editor(&template).map_err(e)
}

#[tauri::command]
pub fn import_kitty(core: CoreState, path: PathBuf) -> Res<KittyImport> {
    core.import_kitty(&path).map_err(e)
}

#[tauri::command]
pub fn plan_form(core: CoreState) -> Res<PlanForm> {
    core.plan_form().map_err(e)
}

#[tauri::command]
pub fn plan_set(core: CoreState, form: PlanForm) -> Res<()> {
    core.set_plan(&form).map_err(e)
}

#[tauri::command]
pub fn governor_enforce(core: CoreState, enforce: bool) -> Res<()> {
    core.set_enforce(enforce).map_err(e)
}

/// The command a trust pane runs: plain `claude` in the project root, for the
/// user to answer the workspace trust dialog (J0 step 2). No orchestrator
/// hooks, auto-update off; the pane is closed by the user.
#[tauri::command]
pub fn trust_command(core: CoreState) -> String {
    let claude = core.claude_check().map(|c| c.path).unwrap_or_else(|| PathBuf::from("claude"));
    format!("DISABLE_AUTOUPDATER=1 {}", agent_runtime::shell::sh_quote(&claude.display().to_string()))
}

#[tauri::command]
pub fn settings_get(core: CoreState, key: String) -> Option<Value> {
    core.settings.get(&key)
}

#[tauri::command]
pub fn settings_set(core: CoreState, key: String, value: Value) -> Res<()> {
    core.settings.set(&key, value).map_err(e)
}

#[tauri::command]
pub fn dev_enabled() -> bool {
    crate::devctl::enabled()
}
