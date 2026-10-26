#![doc = include_str!("../README.md")]

pub mod commands;
pub mod core;
pub mod editor;
pub mod fonts;
pub mod inbox;
pub mod kitty;
pub mod memory;
pub mod menu;
pub mod permissions;
pub mod planner;
pub mod services;
pub mod terms;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent};

pub fn run() {
    let core = core::Core::start(core::CoreConfig::from_env());
    let app = tauri::Builder::default()
        .manage(core.clone())
        .menu(menu::build)
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
            commands::term_spawn,
            commands::term_read,
            commands::term_write,
            commands::term_resize,
            commands::term_kill,
            commands::editor_open,
            commands::open_in_kitty,
            commands::inbox_act,
            commands::planner_view,
            commands::governor_act,
            commands::project_add,
            commands::project_remove,
            commands::settings_get,
            commands::settings_set,
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
