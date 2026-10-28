#![doc = include_str!("../README.md")]

pub mod commands;
pub mod core;
pub mod devctl;
pub mod ecosystem;
pub mod editor;
pub mod fonts;
pub mod inbox;
pub mod kitty;
pub mod memory;
pub mod menu;
pub mod notify;
pub mod permissions;
pub mod planner;
pub mod services;
pub mod setup;
pub mod teams;
pub mod terms;
pub mod watch;

use std::sync::{Arc, Mutex, MutexGuard};

use tauri::{Emitter, Manager, RunEvent};

/// Locks a mutex, recovering from a poisoned one: a panic in one command
/// must not take the app's shared state down with it.
pub(crate) trait LockOk<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}

impl<T> LockOk<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Started from the Finder (a bundle), the app has launchd's environment:
/// stderr goes nowhere and PATH lacks Homebrew (`gh`, a newer git). Logs go
/// to `<home>/logs/orchestrator.log` (the previous run's kept as `.1`) and
/// PATH comes from the login shell, as for agents.
fn bundle_environment(cfg: &core::CoreConfig) {
    let in_bundle = std::env::current_exe().is_ok_and(|e| e.to_string_lossy().contains(".app/Contents/MacOS/"));
    // SAFETY: isatty on fd 2.
    let tty = unsafe { libc::isatty(2) } == 1;
    if !in_bundle || tty {
        return;
    }
    let dir = cfg.home.join("logs");
    let log = dir.join("orchestrator.log");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::rename(&log, dir.join("orchestrator.log.1"));
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            use std::os::fd::AsRawFd;
            // SAFETY: fd 2 now points at the log; `f` may close its own fd afterwards.
            unsafe { libc::dup2(f.as_raw_fd(), 2) };
        }
    }
    let home = agent_runtime::paths::home_dir();
    let shell = std::env::var_os("SHELL").map(std::path::PathBuf::from).unwrap_or_else(|| "/bin/zsh".into());
    let path = agent_runtime::env::login_path(&shell, &home);
    if !path.is_empty() {
        // SAFETY: called first in `run`, before any other thread starts.
        unsafe { std::env::set_var("PATH", path) };
    }
    eprintln!("orchestrator {} started (pid {})", env!("CARGO_PKG_VERSION"), std::process::id());
}

pub fn run() {
    let cfg = core::CoreConfig::from_env();
    bundle_environment(&cfg);
    let core = core::Core::start(cfg);
    let app = tauri::Builder::default()
        .manage(core.clone())
        .plugin(tauri_plugin_notification::init())
        .menu(menu::build)
        .setup(|app| {
            devctl::start(app.handle().clone());
            notify::start(app.handle().clone());
            Ok(())
        })
        .on_menu_event(|app, ev| {
            let _ = app.emit("menu", ev.id().0.clone());
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            commands::kitty_config,
            commands::font_files,
            commands::read_font,
            commands::agent_start,
            commands::agent_attach,
            commands::agent_detach,
            commands::agent_read,
            commands::agent_write,
            commands::agent_resize,
            commands::agent_action,
            commands::agent_delete,
            commands::term_spawn,
            commands::term_read,
            commands::term_write,
            commands::term_resize,
            commands::term_kill,
            commands::editor_open,
            commands::open_in_kitty,
            commands::inbox_act,
            commands::gate_detail,
            commands::teams_view,
            commands::team_save,
            commands::role_save,
            commands::team_import,
            commands::team_export,
            commands::team_launch,
            commands::feature_view,
            commands::feature_doc,
            commands::board_view,
            commands::board_sync,
            commands::task_create,
            commands::task_update,
            commands::run_act,
            commands::feature_delete,
            commands::message_send,
            commands::project_settings,
            commands::project_settings_set,
            commands::planner_view,
            commands::reports_list,
            commands::report_view,
            commands::queue_off_hours,
            commands::open_url,
            commands::linear_teams,
            commands::notion_publish,
            commands::notion_status,
            commands::linear_states,
            commands::artifact_preview,
            commands::artifact_open,
            commands::team_duplicate,
            commands::role_duplicate,
            commands::team_set_preset,
            commands::preset_update,
            commands::integrations_view,
            commands::integration_secret_set,
            commands::integration_secret_remove,
            commands::integration_test,
            commands::mcp_registry_view,
            commands::mcp_server_save,
            commands::mcp_server_delete,
            commands::mcp_server_test,
            commands::mcp_secret_set,
            commands::mcp_secret_delete,
            commands::mcp_secrets_status,
            commands::starter_copy,
            commands::governor_act,
            commands::project_add,
            commands::project_clone,
            commands::project_detail,
            commands::project_fetch,
            commands::worktree_remove,
            commands::worktree_diff,
            commands::worktree_create_pr,
            commands::project_removal_plan,
            commands::project_remove,
            commands::set_editor,
            commands::import_kitty,
            commands::plan_form,
            commands::plan_set,
            commands::governor_enforce,
            commands::trust_command,
            commands::settings_get,
            commands::settings_set,
            commands::dev_enabled,
            devctl::dev_report,
        ])
        .build(tauri::generate_context!())
        .expect("building the Tauri app");
    app.run(|handle, ev| {
        if let RunEvent::Exit = ev {
            // Agents are suspended (resumed by session id next time), shells closed.
            if let Some(core) = handle.try_state::<Arc<core::Core>>() {
                core.shutdown();
            }
        }
    });
}
