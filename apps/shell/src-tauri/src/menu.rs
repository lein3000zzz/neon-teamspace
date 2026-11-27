//! The desktop menu: the platform's usual items, with a way back to the server picker and the
//! browser's back, forward and reload, which a window with no toolbar has nowhere else.

use tauri::menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

use crate::shell;

const SWITCH_SERVER: &str = "switch-server";
const BACK: &str = "back";
const FORWARD: &str = "forward";
const RELOAD: &str = "reload";

#[cfg(target_os = "macos")]
const BACK_KEY: &str = "Cmd+[";
#[cfg(target_os = "macos")]
const FORWARD_KEY: &str = "Cmd+]";
#[cfg(not(target_os = "macos"))]
const BACK_KEY: &str = "Alt+Left";
#[cfg(not(target_os = "macos"))]
const FORWARD_KEY: &str = "Alt+Right";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        ..AboutMetadata::default()
    };
    let switch_server =
        MenuItem::with_id(app, SWITCH_SERVER, "Switch Server…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &MenuItem::with_id(app, BACK, "Back", true, Some(BACK_KEY))?,
            &MenuItem::with_id(app, FORWARD, "Forward", true, Some(FORWARD_KEY))?,
            &MenuItem::with_id(app, RELOAD, "Reload", true, Some("CmdOrCtrl+R"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let first = Submenu::with_items(
        app,
        &package.name,
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &separator,
            &switch_server,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    #[cfg(not(target_os = "macos"))]
    let first = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &switch_server,
            &separator,
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    Menu::with_items(app, &[&first, &edit, &view, &window])
}

pub fn handle<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let result = match event.id().as_ref() {
        SWITCH_SERVER => shell::show_picker(app),
        BACK => shell::window(app).and_then(|window| window.eval("history.back()")),
        FORWARD => shell::window(app).and_then(|window| window.eval("history.forward()")),
        RELOAD => shell::window(app).and_then(|window| window.reload()),
        _ => Ok(()),
    };
    if let Err(error) = result {
        log::warn!("the menu item {:?} failed: {error}", event.id().as_ref());
    }
}
