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
pub mod planner;
pub mod pr_builds;
pub mod project_files;
pub mod services;
pub mod setup;
pub mod snapshot;
pub mod teams;
pub mod terms;
pub mod update;
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
}

/// What this build is (api-contract §2). The sha and channel are baked in at compile time
/// by `scripts/app.sh`; the version comes from Tauri's package info (the bundle version,
/// `0.1.<n>` in CI), so callers pass it in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub sha: String,
    pub channel: String,
    /// The pull request number of a `pr` channel build (`ORCH_BUILD_PR`), else `None`.
    pub pr: Option<u64>,
}

pub fn build_info(version: &str) -> BuildInfo {
    build_info_from(
        version,
        option_env!("ORCH_BUILD_SHA"),
        option_env!("ORCH_BUILD_CHANNEL"),
        option_env!("ORCH_BUILD_PR"),
    )
}

fn build_info_from(version: &str, sha: Option<&str>, channel: Option<&str>, pr: Option<&str>) -> BuildInfo {
    let channel = channel.filter(|c| !c.is_empty()).unwrap_or("local").to_string();
    // Only a `pr` build carries a number; an unparsable one is dropped.
    let pr = if channel == "pr" { pr.and_then(|n| n.trim().parse().ok()) } else { None };
    BuildInfo { version: version.to_string(), sha: sha.unwrap_or("").to_string(), channel, pr }
}

impl std::fmt::Display for BuildInfo {
    /// The build as the start log line prints it: `0.1.57 (sha f9b1e20, channel pr, PR #12)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sha = if self.sha.is_empty() { "unknown" } else { &self.sha };
        write!(f, "{} (sha {sha}, channel {}", self.version, self.channel)?;
        if let Some(n) = self.pr {
            write!(f, ", PR #{n}")?;
        }
        write!(f, ")")
    }
}

pub fn run() {
    let cfg = core::CoreConfig::from_env();
    bundle_environment(&cfg);
    let updates = cfg.home.join("updates");
    let core = core::Core::start(cfg);
    let decided = core.clone();
    let context = tauri::generate_context!();
    let build = build_info(&context.package_info().version.to_string());
    snapshot::prune(&core.paths.db_file(), &build, jiff::Timestamp::now().as_second());
    let updater = update::Updater::new(build.clone(), updates);
    let app = tauri::Builder::default()
        .manage(core.clone())
        .manage(updater.clone())
        .plugin(tauri_plugin_notification::init())
        .menu(menu::build)
        .setup(move |app| {
            eprintln!("orchestrator {build} started (pid {})", std::process::id());
            devctl::start(app.handle().clone());
            // api-contract §3: the `update` event carries the whole state.
            let h = app.handle().clone();
            updater.start(Arc::new(move |v| {
                let _ = h.emit("update", v);
            }));
            notify::start(app.handle().clone(), decided.clone());
            // api-contract §3: a decided item's toast (and card drop).
            if let Ok(rt) = decided.runtime() {
                let h = app.handle().clone();
                rt.set_outcome_sink(Arc::new(move |v| {
                    let _ = h.emit("inbox_outcome", inbox::outcome_event(v));
                }));
                // P4.1: the one state push (api-contract §3.1).
                let h = app.handle().clone();
                rt.subscribe_now(Arc::new(move |v| {
                    let _ = h.emit("now", v);
                }));
            }
            Ok(())
        })
        .on_menu_event(|app, ev| {
            let _ = app.emit("menu", ev.id().0.clone());
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            commands::now_snapshot,
            commands::decision_patch,
            commands::watch_worktree,
            commands::unwatch_worktree,
            commands::digest_seen,
            commands::chat_days,
            commands::chat_day,
            commands::chat_search,
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
            commands::restart_fresh,
            commands::gate_detail,
            commands::teams_view,
            commands::team_save,
            commands::role_save,
            commands::team_import,
            commands::team_export,
            commands::team_launch,
            commands::feature_view,
            commands::feature_doc,
            commands::feature_asset,
            commands::board_view,
            commands::board_sync,
            commands::board_open_sync,
            commands::task_create,
            commands::task_update,
            commands::run_act,
            commands::feature_delete,
            commands::message_send,
            commands::project_settings,
            commands::project_settings_set,
            commands::project_files,
            commands::project_files_add,
            commands::project_file_update,
            commands::project_file_remove,
            commands::reveal_path,
            commands::pick_paths,
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
            commands::pr_refresh,
            commands::worktree_remove,
            commands::worktree_diff,
            commands::worktree_create_pr,
            commands::project_removal_plan,
            commands::project_remove,
            commands::set_editor,
            commands::import_kitty,
            commands::plan_form,
            commands::plan_set,
            commands::project_allocation,
            commands::project_allocation_set,
            commands::governor_enforce,
            commands::pacing_settings,
            commands::pacing_settings_set,
            commands::trust_command,
            commands::settings_get,
            commands::settings_set,
            commands::dev_enabled,
            commands::update_check,
            commands::update_restart,
            commands::pr_builds_list,
            commands::pr_build_install,
            devctl::dev_report,
        ])
        .build(context)
        .expect("building the Tauri app");
    app.run(|handle, ev| {
        match &ev {
            // D7: no click callback on desktop; the app coming forward soon
            // after a notification focuses that decision's card.
            RunEvent::WindowEvent { event: tauri::WindowEvent::Focused(true), .. } => notify::on_activated(handle),
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => notify::on_activated(handle),
            _ => {}
        }
        if let RunEvent::Exit = ev {
            // Agents are suspended (resumed by session id next time), shells closed.
            if let Some(core) = handle.try_state::<Arc<core::Core>>() {
                core.shutdown();
            }
            // Then the staged update, if any, replaces the bundle (spec §3.3).
            if let Some(u) = handle.try_state::<Arc<update::Updater>>() {
                u.on_exit();
            }
        }
    });
}

#[cfg(test)]
mod build_info_tests {
    use super::{BuildInfo, build_info_from};

    #[test]
    fn local_defaults_without_build_env() {
        let b = build_info_from("0.1.0", None, None, None);
        assert_eq!((b.version.as_str(), b.sha.as_str(), b.channel.as_str()), ("0.1.0", "", "local"));
        assert_eq!(b.pr, None);
        // scripts/app.sh exports an empty channel for a local build.
        assert_eq!(build_info_from("0.1.0", Some(""), Some(""), Some("")).channel, "local");
    }

    #[test]
    fn ci_build_env_is_reported() {
        let b = build_info_from("0.1.42", Some("abc123"), Some("main"), None);
        assert_eq!((b.version.as_str(), b.sha.as_str(), b.channel.as_str()), ("0.1.42", "abc123", "main"));
        assert_eq!(b.pr, None);
    }

    #[test]
    fn pr_build_carries_its_number() {
        let b = build_info_from("0.1.57", Some("f9b1e20"), Some("pr"), Some("12"));
        assert_eq!((b.channel.as_str(), b.pr), ("pr", Some(12)));
    }

    #[test]
    fn pr_number_only_on_the_pr_channel_and_only_if_a_number() {
        assert_eq!(build_info_from("0.1.1", None, Some("main"), Some("12")).pr, None);
        assert_eq!(build_info_from("0.1.1", None, None, Some("12")).pr, None);
        assert_eq!(build_info_from("0.1.1", None, Some("pr"), Some("")).pr, None);
        assert_eq!(build_info_from("0.1.1", None, Some("pr"), Some("abc")).pr, None);
        assert_eq!(build_info_from("0.1.1", None, Some("pr"), None).pr, None);
    }

    #[test]
    fn serialises_pr_as_null_or_a_number() {
        let main = serde_json::to_value(build_info_from("0.1.2", Some("a"), Some("main"), None)).unwrap();
        assert!(main["pr"].is_null());
        let pr = serde_json::to_value(build_info_from("0.1.2", Some("a"), Some("pr"), Some("12"))).unwrap();
        assert_eq!(pr["pr"], 12);
    }

    #[test]
    fn start_line_names_the_pr() {
        let line = |sha: &str, channel: &str, pr| {
            BuildInfo { version: "0.1.57".into(), sha: sha.into(), channel: channel.into(), pr }.to_string()
        };
        assert_eq!(line("f9b1e20", "pr", Some(12)), "0.1.57 (sha f9b1e20, channel pr, PR #12)");
        assert_eq!(line("", "local", None), "0.1.57 (sha unknown, channel local)");
    }
}
