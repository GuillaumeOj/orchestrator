//! The app's own macOS menu (§9.7): system items must not take the chords the
//! keyboard contract uses. Close Window moves to ⇧⌘W so ⌘W closes a pane; the
//! app's commands are listed with their chords. The webview handles the chords
//! first (it calls preventDefault); a menu click is emitted to the UI as a
//! `menu` event with the item id.

use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

/// `(id, label, accelerator)`.
pub type Item = (&'static str, &'static str, &'static str);

/// The app's own items, by submenu ("Orchestrator" items go in the app menu).
pub const ITEMS: &[(&str, &[Item])] = &[
    ("Orchestrator", &[("settings", "Settings…", "CmdOrCtrl+,")]),
    (
        "File",
        &[
            ("project.add", "Add Project…", "CmdOrCtrl+N"),
            ("session.new", "New Session…", "CmdOrCtrl+T"),
            ("pane.close", "Close Tab", "CmdOrCtrl+W"),
            ("pane.reopen", "Reopen Closed Tab", "CmdOrCtrl+Shift+T"),
            ("window.close", "Close Window", "CmdOrCtrl+Shift+W"),
        ],
    ),
    (
        "View",
        &[
            ("palette", "Command Palette", "CmdOrCtrl+K"),
            ("jump", "Jump…", "CmdOrCtrl+P"),
            ("sidebar", "Toggle Sidebar", "CmdOrCtrl+B"),
            ("inspector", "Toggle Inspector", "CmdOrCtrl+I"),
            ("dock.toggle", "Toggle Shell Dock", "CmdOrCtrl+J"),
            ("decisions", "Show Decisions", "CmdOrCtrl+Shift+I"),
            ("overwatch.ask", "Ask the Overwatch", "CmdOrCtrl+L"),
            ("pane.focus.1", "Tab 1", "CmdOrCtrl+1"),
            ("pane.focus.2", "Tab 2", "CmdOrCtrl+2"),
            ("pane.focus.3", "Tab 3", "CmdOrCtrl+3"),
            ("pane.focus.4", "Tab 4", "CmdOrCtrl+4"),
            ("pane.focus.5", "Tab 5", "CmdOrCtrl+5"),
            ("pane.focus.6", "Tab 6", "CmdOrCtrl+6"),
            ("pane.focus.7", "Tab 7", "CmdOrCtrl+7"),
            ("pane.focus.8", "Tab 8", "CmdOrCtrl+8"),
            ("pane.focus.9", "Tab 9", "CmdOrCtrl+9"),
        ],
    ),
    ("Help", &[("shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")]),
];

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let mut app_menu = SubmenuBuilder::new(app, "Orchestrator")
        .item(&PredefinedMenuItem::about(app, Some("About Orchestrator"), None)?)
        .separator();
    for (_, items) in ITEMS.iter().filter(|(t, _)| *t == "Orchestrator") {
        for (id, label, accel) in *items {
            app_menu = app_menu.item(&MenuItemBuilder::with_id(*id, *label).accelerator(*accel).build(app)?);
        }
    }
    let app_menu = app_menu
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;
    // Standard editing items: copy/paste in dialogs, and paste into panes.
    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;
    let menu = Menu::new(app)?;
    menu.append(&app_menu)?;
    for (title, items) in ITEMS.iter().filter(|(t, _)| *t != "Orchestrator") {
        let mut sub = SubmenuBuilder::new(app, *title);
        for (id, label, accel) in *items {
            sub = sub.item(&MenuItemBuilder::with_id(*id, *label).accelerator(*accel).build(app)?);
        }
        let sub = sub.build()?;
        if *title == "Help" {
            continue; // appended last, after Window
        }
        menu.append(&sub)?;
        if *title == "File" {
            menu.append(&edit)?;
        }
    }
    let window = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()?;
    menu.append(&window)?;
    let mut help = SubmenuBuilder::new(app, "Help");
    for (_, items) in ITEMS.iter().filter(|(t, _)| *t == "Help") {
        for (id, label, accel) in *items {
            help = help.item(&MenuItemBuilder::with_id(*id, *label).accelerator(*accel).build(app)?);
        }
    }
    let help = help.build()?;
    menu.append(&help)?;
    #[cfg(target_os = "macos")]
    help.set_as_help_menu_for_nsapp()?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_chords_belong_to_the_app() {
        let all: Vec<(&str, &str)> = ITEMS.iter().flat_map(|(_, i)| i.iter().map(|(id, _, a)| (*id, *a))).collect();
        let by_accel = |a: &str| all.iter().find(|(_, x)| *x == a).map(|(id, _)| *id);
        assert_eq!(by_accel("CmdOrCtrl+W"), Some("pane.close"), "⌘W closes a pane, not the window");
        for (chord, id) in [
            ("CmdOrCtrl+K", "palette"),
            ("CmdOrCtrl+P", "jump"),
            ("CmdOrCtrl+Shift+I", "decisions"),
            ("CmdOrCtrl+L", "overwatch.ask"),
            ("CmdOrCtrl+,", "settings"),
        ] {
            assert_eq!(by_accel(chord), Some(id));
        }
        let label = |id: &str| ITEMS.iter().flat_map(|(_, i)| i.iter()).find(|(i, _, _)| *i == id).map(|(_, l, _)| *l);
        for n in 1..=9 {
            assert_eq!(label(&format!("pane.focus.{n}")), Some(format!("Tab {n}").as_str()));
        }
        assert_eq!(label("pane.close"), Some("Close Tab"));
        assert_eq!(label("pane.reopen"), Some("Reopen Closed Tab"));
        assert_eq!(by_accel("CmdOrCtrl+/"), Some("shortcuts"));
        assert_eq!(label("shortcuts"), Some("Keyboard Shortcuts"));
        for gone in ["pane.split.right", "pane.split.below", "pane.zoom", "focus.left", "focus.right"] {
            assert_eq!(label(gone), None);
        }
        let mut accels: Vec<&str> = all.iter().map(|(_, a)| *a).collect();
        accels.sort();
        let n = accels.len();
        accels.dedup();
        assert_eq!(accels.len(), n, "no chord bound twice");
    }
}
