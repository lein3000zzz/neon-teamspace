//! The desktop menu on Tauri's mock runtime: where **Check for Updates…** is, and that the app
//! finds it to show an update's progress. macOS builds menus only on the main thread, so this
//! test runs there, without the test harness: each check is a function `main` calls, and a
//! failed one panics.

#[cfg(desktop)]
use neon_shell::menu::{self, CHECK_FOR_UPDATES, CHECK_FOR_UPDATES_LABEL, SWITCH_SERVER};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItemKind};
#[cfg(desktop)]
use tauri::test::{MockRuntime, mock_app};

#[cfg(desktop)]
fn main() {
    check_for_updates_follows_about_on_macos_and_switch_server_elsewhere();
    the_item_is_found_to_show_an_updates_progress();
}

/// Only the desktop has a menu.
#[cfg(not(desktop))]
fn main() {}

/// The first menu's items, the app menu on macOS and File elsewhere: the app's by their ids,
/// the platform's by their text.
#[cfg(desktop)]
fn first_menu(menu: &Menu<MockRuntime>) -> Vec<String> {
    let items = menu.items().expect("the menu's items");
    let first = items
        .first()
        .and_then(MenuItemKind::as_submenu)
        .expect("a submenu first");
    first
        .items()
        .expect("the first menu's items")
        .iter()
        .map(|item| match item {
            MenuItemKind::Predefined(item) => item.text().expect("a text"),
            other => other.id().0.clone(),
        })
        .collect()
}

#[cfg(desktop)]
fn check_for_updates_follows_about_on_macos_and_switch_server_elsewhere() {
    let app = mock_app();
    let menu = menu::build(app.handle()).expect("the menu builds");
    let items = first_menu(&menu);
    let at = items
        .iter()
        .position(|item| item == CHECK_FOR_UPDATES)
        .unwrap_or_else(|| panic!("no {CHECK_FOR_UPDATES} in {items:?}"));
    let before = at
        .checked_sub(1)
        .and_then(|before| items.get(before))
        .expect("an item before it");
    if cfg!(target_os = "macos") {
        assert!(before.starts_with("About"), "{items:?}");
    } else {
        assert_eq!(before, SWITCH_SERVER, "{items:?}");
    }
    assert_eq!(
        items
            .iter()
            .filter(|item| *item == CHECK_FOR_UPDATES)
            .count(),
        1,
        "{items:?}"
    );
}

#[cfg(desktop)]
fn the_item_is_found_to_show_an_updates_progress() {
    let app = mock_app();
    let menu = menu::build(app.handle()).expect("the menu builds");
    let item = menu::item(&menu, CHECK_FOR_UPDATES).expect("the item");
    assert_eq!(item.text().expect("its text"), CHECK_FOR_UPDATES_LABEL);
    assert!(item.is_enabled().expect("whether it is enabled"));
    assert!(menu::item(&menu, SWITCH_SERVER).is_some());
    assert!(menu::item(&menu, "no-such-item").is_none());
}
