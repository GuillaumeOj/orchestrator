//! Tauri commands: thin wrappers over [`Core`](crate::core::Core). PTY output
//! uses the pull transport: `*_read` waits for output on a blocking thread and
//! returns everything buffered as one raw IPC response (ADR-001 §4).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::State;
use tauri::ipc::Response;

use crate::core::{Core, EditorOpen, NewSession, TermInfo};
use crate::kitty::KittyImport;
use crate::planner::PlannerView;
use crate::terms::TermKind;

type Res<T> = Result<T, String>;
type CoreState<'a> = State<'a, Arc<Core>>;

/// A pull returns at least this often, so a closed pane's loop ends promptly.
const PULL_WAIT: Duration = Duration::from_secs(2);

fn e(err: anyhow::Error) -> String {
    format!("{err:#}")
}

#[tauri::command]
pub fn app_state(core: CoreState) -> Value {
    core.state()
}

#[tauri::command]
pub fn kitty_config(core: CoreState) -> KittyImport {
    core.kitty.clone()
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
pub fn agent_attach(core: CoreState, id: String) -> Res<()> {
    core.agent_attach(&id).map_err(e)
}

#[tauri::command]
pub fn agent_detach(core: CoreState, id: String) {
    core.agent_detach(&id)
}

#[tauri::command]
pub async fn agent_read(core: CoreState<'_>, id: String) -> Res<Response> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.agent_read(&id, PULL_WAIT).map(Response::new).map_err(e))
        .await
        .map_err(|err| err.to_string())?
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
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.agent_action(&id, &action, message.as_deref()).map_err(e))
        .await
        .map_err(|err| err.to_string())?
}

// ---------------------------------------------------------------- terminals

#[tauri::command]
pub fn term_spawn(
    core: CoreState,
    kind: TermKind,
    project_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<PathBuf>,
    run: Option<String>,
    rows: u16,
    cols: u16,
) -> Res<TermInfo> {
    let cwd = cwd.unwrap_or_else(|| core.pane_cwd(project_id.as_deref(), agent_id.as_deref()));
    core.term_spawn(kind, &cwd, run.as_deref(), rows, cols).map_err(e)
}

#[tauri::command]
pub async fn term_read(core: CoreState<'_>, id: u32) -> Res<Response> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = core.terms.get(id).map_err(e)?;
        t.output.read_wait(PULL_WAIT).map(Response::new).ok_or_else(|| "eof".to_string())
    })
    .await
    .map_err(|err| err.to_string())?
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
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        core.editor_open(project_id.as_deref(), agent_id.as_deref(), file.as_deref(), line, rows, cols).map_err(e)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub fn open_in_kitty(core: CoreState, cwd: PathBuf) -> Res<()> {
    core.open_in_kitty(&cwd).map_err(e)
}

// ---------------------------------------------------------------- inbox, governor

#[tauri::command]
pub async fn inbox_act(core: CoreState<'_>, item: String, action: String, message: Option<String>) -> Res<Value> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.inbox_act(&item, &action, message.as_deref()).map_err(e))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
pub fn planner_view(core: CoreState) -> Res<PlannerView> {
    core.planner().map_err(e)
}

#[tauri::command]
pub fn governor_act(core: CoreState, action: String, arg: Option<f64>) -> Res<Value> {
    core.governor_act(&action, arg).map_err(e)
}

// ---------------------------------------------------------------- projects, settings

#[tauri::command]
pub fn project_add(core: CoreState, path: PathBuf) -> Res<Value> {
    core.projects.add(&path).map(|p| serde_json::json!(p)).map_err(e)
}

#[tauri::command]
pub fn project_remove(core: CoreState, id: String) -> Res<()> {
    core.projects.remove(&id).map_err(e)
}

#[tauri::command]
pub fn settings_get(core: CoreState, key: String) -> Option<Value> {
    core.settings.get(&key)
}

#[tauri::command]
pub fn settings_set(core: CoreState, key: String, value: Value) -> Res<()> {
    core.settings.set(&key, value).map_err(e)
}
