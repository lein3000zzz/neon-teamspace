//! The desktop menu: the platform's usual items, with a way back to the server picker, the
//! browser's back, forward and reload, which a window with no toolbar has nowhere else, and the
//! app's check for updates.

use tauri::menu::{
    AboutMetadata, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, Runtime};

use crate::{shell, updates};

/// The id of the item that shows the server picker again.
pub const SWITCH_SERVER: &str = "switch-server";
/// The id of the item that checks for updates.
pub const CHECK_FOR_UPDATES: &str = "check-for-updates";
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

/// The label of the item that checks for updates, which says how far an update is while one
/// downloads and installs.
pub const CHECK_FOR_UPDATES_LABEL: &str = "Check for Updates…";

/// What a menu item of the app's own does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    SwitchServer,
    CheckForUpdates,
    Back,
    Forward,
    Reload,
}

impl Action {
    /// The action of the item `id`; none for the platform's own items.
    fn of(id: &str) -> Option<Self> {
        match id {
            SWITCH_SERVER => Some(Self::SwitchServer),
            CHECK_FOR_UPDATES => Some(Self::CheckForUpdates),
            BACK => Some(Self::Back),
            FORWARD => Some(Self::Forward),
            RELOAD => Some(Self::Reload),
            _ => None,
        }
    }
}

/// The menu, for the app's window.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        ..AboutMetadata::default()
    };
    let switch_server =
        MenuItem::with_id(app, SWITCH_SERVER, "Switch Server…", true, None::<&str>)?;
    let check_for_updates = MenuItem::with_id(
        app,
        CHECK_FOR_UPDATES,
        CHECK_FOR_UPDATES_LABEL,
        true,
        None::<&str>,
    )?;
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
            &check_for_updates,
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
            &check_for_updates,
            &separator,
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    Menu::with_items(app, &[&first, &edit, &view, &window])
}

/// Does what the item chosen in `event` is for.
pub fn handle<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let result = match Action::of(event.id().as_ref()) {
        Some(Action::SwitchServer) => shell::show_picker(app),
        Some(Action::CheckForUpdates) => {
            updates::check_from_menu(app);
            Ok(())
        }
        Some(Action::Back) => shell::window(app).and_then(|window| window.eval("history.back()")),
        Some(Action::Forward) => {
            shell::window(app).and_then(|window| window.eval("history.forward()"))
        }
        Some(Action::Reload) => shell::window(app).and_then(|window| window.reload()),
        None => Ok(()),
    };
    if let Err(error) = result {
        log::warn!("the menu item {:?} failed: {error}", event.id().as_ref());
    }
}

/// The menu's **Check for Updates…**, which shows an update's progress.
pub fn check_for_updates<R: Runtime>(app: &AppHandle<R>) -> Option<MenuItem<R>> {
    item(&app.menu()?, CHECK_FOR_UPDATES)
}

/// The item `id` of `menu`, in whichever of its submenus it is.
pub fn item<R: Runtime>(menu: &Menu<R>, id: &str) -> Option<MenuItem<R>> {
    find(&menu.items().ok()?, id)
}

/// The item `id` among `items` and their submenus'.
fn find<R: Runtime>(items: &[MenuItemKind<R>], id: &str) -> Option<MenuItem<R>> {
    items.iter().find_map(|item| match item {
        MenuItemKind::MenuItem(item) if item.id() == id => Some(item.clone()),
        MenuItemKind::Submenu(submenu) => find(&submenu.items().ok()?, id),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_item_of_the_apps_runs_its_action() {
        for (id, action) in [
            (SWITCH_SERVER, Action::SwitchServer),
            (CHECK_FOR_UPDATES, Action::CheckForUpdates),
            (BACK, Action::Back),
            (FORWARD, Action::Forward),
            (RELOAD, Action::Reload),
        ] {
            assert_eq!(Action::of(id), Some(action), "{id}");
        }
        assert_eq!(Action::of("about"), None);
    }
}
