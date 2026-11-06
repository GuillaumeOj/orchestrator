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
use crate::setup::{AllocationEdit, PlanForm, ProjectAllocationView};
use crate::terms::TermKind;
use crate::update::{RestartTarget, Updater};
use orch_git::project::RemovalChoices;

type Res<T> = Result<T, String>;
type CoreState<'a> = State<'a, Arc<Core>>;
type UpdaterState<'a> = State<'a, Arc<Updater>>;

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
pub async fn app_state(core: CoreState<'_>, updater: UpdaterState<'_>) -> Res<Value> {
    let mut v = blocking(core, |c| Ok(c.state())).await?;
    v["update"] = updater.json();
    Ok(v)
}

/// P4.1: the attention world, at the `now` event's last rev (api-contract §3.2).
#[tauri::command]
pub async fn now_snapshot(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| Ok(c.runtime()?.now_snapshot())).await
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

#[tauri::command]
pub async fn agent_delete(core: CoreState<'_>, id: String, remove_worktree: bool, confirm_discard: bool) -> Res<Value> {
    // Kills (and checks) first: waits for the process like kill does.
    blocking(core, move |c| c.agent_delete(&id, remove_worktree, confirm_discard)).await
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
pub async fn restart_fresh(core: CoreState<'_>, agent: String) -> Res<Value> {
    blocking(core, move |c| c.restart_fresh(&agent)).await
}

/// A Diff pane opened on `path`: its `worktree:<path>` topic follows file
/// changes (ref-counted; a registered project's worktree only).
#[tauri::command]
pub async fn watch_worktree(core: CoreState<'_>, path: String) -> Res<()> {
    blocking(core, move |c| c.runtime()?.watch_worktree(std::path::Path::new(&path))).await
}

/// The Diff pane on `path` closed.
#[tauri::command]
pub async fn unwatch_worktree(core: CoreState<'_>, path: String) -> Res<()> {
    blocking(core, move |c| {
        c.runtime()?.unwatch_worktree(std::path::Path::new(&path));
        Ok(())
    })
    .await
}

/// The PR gate's patch, whole or one file (api-contract §3.2).
#[tauri::command]
pub async fn decision_patch(core: CoreState<'_>, id: String, path: Option<String>) -> Res<String> {
    blocking(core, move |c| {
        let n: u64 = id.strip_prefix("gate-").unwrap_or(&id).parse()?;
        c.runtime()?.decision_patch(n, path.as_deref())
    })
    .await
}

/// Today's outcomes up to `up_to` leave the digest (`done_today` keeps them).
#[tauri::command]
pub async fn digest_seen(core: CoreState<'_>, up_to: String) -> Res<()> {
    blocking(core, move |c| c.runtime()?.digest_seen(&up_to)).await
}

/// P4.3: days of the chat log before `before` (today excluded), newest first (api-contract §3).
#[tauri::command]
pub async fn chat_days(core: CoreState<'_>, limit: Option<usize>, before: Option<String>) -> Res<Value> {
    blocking(core, move |c| Ok(serde_json::json!(c.runtime()?.chat_days(limit, before.as_deref())?))).await
}

/// P4.3: one day of the chat log, with that day's outcomes.
#[tauri::command]
pub async fn chat_day(core: CoreState<'_>, day: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.chat_day(&day)).await
}

/// P4.4: the user's message to the Overwatch (api-contract §3.3): `{entry}`.
#[tauri::command]
pub async fn chat_send(
    core: CoreState<'_>,
    text: String,
    thread: Option<String>,
    context: Option<Value>,
) -> Res<Value> {
    blocking(core, move |c| {
        Ok(serde_json::json!({"entry": c.runtime()?.chat_send_with(&text, thread.as_deref(), context.as_ref())?}))
    })
    .await
}

/// P4.4: removes a message the Overwatch has not taken (it was down or off).
#[tauri::command]
pub async fn chat_discard(core: CoreState<'_>, entry: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.chat_discard(&entry)).await
}

/// P4.4: undoes an oversight line (`text` corrects an answer).
#[tauri::command]
pub async fn chat_undo(core: CoreState<'_>, entry: String, text: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.chat_undo(&entry, text.as_deref())).await
}

/// P4.4: the Overwatch's settings (api-contract §3.3).
#[tauri::command]
pub async fn overwatch_settings(core: CoreState<'_>) -> Res<Value> {
    blocking(core, move |c| Ok(serde_json::json!(c.runtime()?.overwatch_settings()))).await
}

/// P4.4: a missing field keeps its value; `permission_secs` must stay under the hold.
#[tauri::command]
pub async fn set_overwatch_settings(core: CoreState<'_>, settings: Value) -> Res<Value> {
    blocking(core, move |c| {
        let rt = c.runtime()?;
        let s = rt.merge_overwatch_settings(&settings)?;
        Ok(serde_json::json!(rt.set_overwatch_settings(s)?))
    })
    .await
}

/// P4.4: a fresh Overwatch session with a new briefing (Restart when it is down).
#[tauri::command]
pub async fn overwatch_restart(core: CoreState<'_>) -> Res<Value> {
    blocking(core, move |c| {
        c.runtime()?.overwatch_restart()?;
        Ok(serde_json::json!({"ok": true}))
    })
    .await
}

/// P4.4: the policy files in effect, also missing ones (api-contract §3.3, §5 F15).
#[tauri::command]
pub async fn policy_files(core: CoreState<'_>) -> Res<Value> {
    blocking(core, move |c| Ok(serde_json::json!(c.runtime()?.policy_files()))).await
}

/// P4.4: the default effects when no rule matches (team roles, solo sessions).
#[tauri::command]
pub async fn policy_defaults(core: CoreState<'_>) -> Res<Value> {
    blocking(core, move |c| Ok(serde_json::json!(c.runtime()?.policy_defaults()))).await
}

#[tauri::command]
pub async fn set_policy_defaults(core: CoreState<'_>, team: String, solo: String) -> Res<Value> {
    blocking(core, move |c| Ok(serde_json::json!(c.runtime()?.set_policy_defaults(&team, &solo)?))).await
}

/// P4.4: opens a policy file in the editor (FR-1101), created first when missing.
#[tauri::command]
pub async fn policy_open(
    core: CoreState<'_>,
    scope: String,
    project: Option<String>,
    rows: u16,
    cols: u16,
) -> Res<EditorOpen> {
    blocking(core, move |c| {
        let path = c.runtime()?.policy_open(&scope, project.as_deref())?;
        c.editor_open(project.as_deref(), None, Some(&path.display().to_string()), None, rows, cols)
    })
    .await
}

/// P4.3: chat entries matching `query` (case-insensitive), newest first.
#[tauri::command]
pub async fn chat_search(core: CoreState<'_>, query: String, limit: Option<usize>) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.chat_search(&query, limit)).await
}

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

#[tauri::command]
pub async fn feature_asset(core: CoreState<'_>, feature: String, path: String) -> Res<Value> {
    blocking(core, move |c| c.feature_asset(&feature, &path)).await
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
pub async fn starter_copy(
    core: CoreState<'_>,
    starter: String,
    to: orch_team::Scope,
    new_id: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.starter_copy(&starter, &to, new_id.as_deref())).await
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
pub async fn board_open_sync(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.board_open_sync(&project)).await
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
    title: Option<String>,
    body: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| {
        c.task_update(
            id,
            crate::teams::TaskEdit {
                status: status.as_deref(),
                node: node.as_deref(),
                note: note.as_deref(),
                title: title.as_deref(),
                body: body.as_deref(),
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn run_act(core: CoreState<'_>, run: String, action: String) -> Res<Value> {
    blocking(core, move |c| c.run_act(&run, &action)).await
}

#[tauri::command]
pub async fn feature_delete(core: CoreState<'_>, feature: String, remove_worktrees: bool, discard: bool) -> Res<Value> {
    blocking(core, move |c| c.feature_delete(&feature, remove_worktrees, discard)).await
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

// ---- FR-112 Project settings › Files (add-external-files-to-project api-contract)

#[tauri::command]
pub async fn project_files(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.project_files(&project)).await
}

#[tauri::command]
pub async fn project_files_add(
    core: CoreState<'_>,
    project: String,
    paths: Vec<String>,
    access: Option<orch_store::FileAccess>,
) -> Res<Value> {
    blocking(core, move |c| c.project_files_add(&project, &paths, access)).await
}

/// `note`: absent keeps it; `""` (or only spaces) clears it.
#[tauri::command]
pub async fn project_file_update(
    core: CoreState<'_>,
    project: String,
    path: String,
    access: Option<orch_store::FileAccess>,
    note: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.project_file_update(&project, &path, access, note.as_deref())).await
}

#[tauri::command]
pub async fn project_file_remove(core: CoreState<'_>, project: String, path: String) -> Res<Value> {
    blocking(core, move |c| c.project_file_remove(&project, &path)).await
}

/// Reveal in Finder, for a path listed in some project's files.
#[tauri::command]
pub async fn reveal_path(core: CoreState<'_>, path: String) -> Res<()> {
    blocking(core, move |c| c.reveal_path(&path)).await
}

/// The native open panel: files and folders together, several at once;
/// `[]` when cancelled.
#[tauri::command]
pub async fn pick_paths(window: tauri::Window) -> Res<Vec<String>> {
    let picked = rfd::AsyncFileDialog::new().set_parent(&window).pick_files_or_folders().await;
    Ok(picked.unwrap_or_default().iter().map(|h| h.path().display().to_string()).collect())
}

// ---- P3 weekly reports (api-contract §5)

#[tauri::command]
pub async fn reports_list(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| c.reports_list()).await
}

#[tauri::command]
pub async fn report_view(core: CoreState<'_>, id: String) -> Res<Value> {
    blocking(core, move |c| c.report_view(&id)).await
}

/// An http(s) or mailto link in the default app (any other scheme is refused).
#[tauri::command]
pub async fn open_url(core: CoreState<'_>, url: String) -> Res<()> {
    blocking(core, move |c| c.open_url(&url)).await
}

#[tauri::command]
pub async fn notion_publish(core: CoreState<'_>, feature: String) -> Res<Value> {
    blocking(core, move |c| c.notion_publish(&feature)).await
}

#[tauri::command]
pub async fn notion_status(core: CoreState<'_>, feature: String) -> Res<Value> {
    blocking(core, move |c| c.notion_status(&feature)).await
}

#[tauri::command]
pub async fn linear_teams(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| c.linear_teams()).await
}

#[tauri::command]
pub async fn linear_states(core: CoreState<'_>, team: String) -> Res<Value> {
    blocking(core, move |c| c.linear_states(&team)).await
}

// ---- P4.5 intake (api-contract §3.1)

#[tauri::command]
pub async fn intake_source(core: CoreState<'_>, project: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_source(&project)).await
}

/// Validates, stores and refreshes at once: `{ preview }`.
#[tauri::command]
pub async fn intake_source_set(core: CoreState<'_>, project: String, source: Value) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_source_set(&project, &source)).await
}

#[tauri::command]
pub async fn intake_preview(core: CoreState<'_>, project: String, source: Value) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_preview(&project, &source)).await
}

#[tauri::command]
pub async fn intake_options(core: CoreState<'_>, project: String, source: Value) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_options(&project, &source)).await
}

#[tauri::command]
pub async fn intake_refresh(core: CoreState<'_>, project: Option<String>) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_refresh(project.as_deref())).await
}

#[tauri::command]
pub async fn intake_item(core: CoreState<'_>, item: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_item(&item)).await
}

#[tauri::command]
pub async fn intake_hide(core: CoreState<'_>, item: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_hide(&item)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn intake_prefill(
    core: CoreState<'_>,
    item: Option<String>,
    brief_decision: Option<String>,
    mode: Option<String>,
) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_prefill(item.as_deref(), brief_decision.as_deref(), mode.as_deref()))
        .await
}

#[tauri::command]
pub async fn intake_unhide(core: CoreState<'_>, item: String) -> Res<Value> {
    blocking(core, move |c| c.runtime()?.intake_unhide(&item)).await
}

/// `queue_off_hours`: `entry` and `task` are string ids per the contract
/// (`"12"`, a task's `"T-12"` too); numbers are accepted as well.
#[tauri::command]
pub async fn queue_off_hours(
    core: CoreState<'_>,
    entry: Option<crate::ecosystem::IdArg>,
    task: Option<crate::ecosystem::IdArg>,
    off: bool,
) -> Res<Value> {
    blocking(core, move |c| {
        let id = |a: Option<crate::ecosystem::IdArg>| a.map(|a| a.parse()).transpose();
        c.queue_off_hours(id(entry)?, id(task)?, off)
    })
    .await
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

/// The value is wrapped at once and never logged or returned (NFR-3).
#[tauri::command]
pub async fn mcp_secret_set(core: CoreState<'_>, account: String, value: String) -> Res<()> {
    let value = orch_secrets::Secret::new(value);
    blocking(core, move |c| c.mcp_secret_set(&account, value)).await
}

#[tauri::command]
pub async fn mcp_secret_delete(core: CoreState<'_>, account: String) -> Res<()> {
    blocking(core, move |c| c.mcp_secret_delete(&account)).await
}

#[tauri::command]
pub async fn mcp_secrets_status(core: CoreState<'_>, accounts: Vec<String>) -> Res<Value> {
    blocking(core, move |c| c.mcp_secrets_status(&accounts)).await
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

/// Checks a feature's PR now (it can wait on `gh` and a fetch: off the UI thread).
#[tauri::command]
pub async fn pr_refresh(core: CoreState<'_>, feature: String) -> Res<Value> {
    blocking(core, move |c| c.pr_refresh(&feature)).await
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

/// Project settings › Allocation (api-contract.md).
#[tauri::command]
pub fn project_allocation(core: CoreState, project: String) -> Res<ProjectAllocationView> {
    core.project_allocation(&project).map_err(e)
}

/// `form` null (or weight 0) takes the project out of the plan.
#[tauri::command]
pub fn project_allocation_set(
    core: CoreState,
    project: String,
    form: Option<AllocationEdit>,
) -> Res<ProjectAllocationView> {
    core.project_allocation_set(&project, form.as_ref()).map_err(e)
}

/// Settings › Pacing policy: the live caps and what they give (api-contract).
/// Off the main thread: it computes the Governor's snapshot.
#[tauri::command]
pub async fn pacing_settings(core: CoreState<'_>) -> Res<agent_runtime::live_cap::PacingSettings> {
    blocking(core, |c| c.pacing_settings()).await
}

/// Sets `live_cap` and/or `off_hours_live_cap` (absent = unchanged), applied
/// now; returns the new view. Off the main thread: it rebuilds the Governor.
/// The arguments keep their snake_case names, as the api-contract gives them
/// (`{ live_cap, off_hours_live_cap }`): camelCase keys would read as absent.
#[tauri::command(rename_all = "snake_case")]
pub async fn pacing_settings_set(
    core: CoreState<'_>,
    live_cap: Option<u32>,
    off_hours_live_cap: Option<u32>,
) -> Res<agent_runtime::live_cap::PacingSettings> {
    blocking(core, move |c| c.set_pacing_settings(live_cap, off_hours_live_cap)).await
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

// ---------------------------------------------------------------- Claude Code updates

/// The latest Claude Code version now; returns `setup.claude` once the check settled.
#[tauri::command]
pub async fn claude_update_check(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| c.claude_update_check()).await
}

/// Starts `claude update` (waits for a running check first); progress via the Setup topic.
#[tauri::command]
pub async fn claude_update_run(core: CoreState<'_>) -> Res<Value> {
    blocking(core, |c| c.claude_update_run().map(|()| serde_json::json!({"started": true}))).await
}

// ---------------------------------------------------------------- updates

/// Checks for an update now (api-contract §4); returns the state once the check started.
#[tauri::command]
pub fn update_check(updater: UpdaterState) -> Res<Value> {
    updater.check_now().map_err(e)
}

/// Quits as ⌘Q does (agents suspended), installs the staged update and relaunches it.
/// `target` is `"main"` (the default) or `"pr"` (the PR build installed with `pr_build_install`;
/// the database snapshot runs first and can refuse). The pre-flight runs before the quit: if it
/// fails, the app stays up and the state is `error`.
/// Async: the pre-flight may copy the bundle (across volumes), off the main thread.
#[tauri::command]
pub async fn update_restart(
    app: tauri::AppHandle,
    core: CoreState<'_>,
    updater: UpdaterState<'_>,
    target: Option<String>,
) -> Res<()> {
    let target = match target.as_deref() {
        None | Some("main") => RestartTarget::Main,
        Some("pr") => RestartTarget::Pr,
        Some(other) => return Err(format!("unknown restart target {other:?}")),
    };
    let updater = updater.inner().clone();
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if target == RestartTarget::Pr {
            let pr = updater.pr_ready()?;
            crate::snapshot::before_pr_restart(&core, &updater.current(), pr)?;
        }
        updater.prepare_restart_to(target)
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(e)?;
    app.exit(0);
    Ok(())
}

/// Open PRs and their CI builds (api-contract §4). Runs gh, off the main thread; gh and network
/// trouble are in the result's `error`, only a permanently off updater is an `Err`.
#[tauri::command]
pub async fn pr_builds_list(updater: UpdaterState<'_>) -> Res<Value> {
    let updater = updater.inner().clone();
    tauri::async_runtime::spawn_blocking(move || updater.pr_builds_list())
        .await
        .map_err(|err| err.to_string())?
        .map_err(e)
}

/// Starts installing PR `number`'s build (progress in the `update` event's `pr_install`).
#[tauri::command]
pub async fn pr_build_install(updater: UpdaterState<'_>, number: u64) -> Res<Value> {
    let updater = updater.inner().clone();
    tauri::async_runtime::spawn_blocking(move || updater.install_pr(number))
        .await
        .map_err(|err| err.to_string())?
        .map_err(e)
}
