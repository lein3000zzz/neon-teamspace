//! The Neon Teamspace apps: one window whose webview shows the bundled server picker, then the
//! chosen server's own web UI as a normal top-level page, locked to that server's origin.

pub mod address;
mod commands;
pub mod lock;
#[cfg(desktop)]
mod menu;
pub mod meta;
pub mod navigation;
pub mod servers;
mod shell;

use std::path::PathBuf;

use tauri::{App, AppHandle, Builder, Runtime, Url};

pub use shell::MAIN_WINDOW;

use crate::lock::Navigation;

/// Where the app keeps its server list.
#[derive(Debug, Clone)]
pub enum DataDir {
    /// The app's data directory, as the platform has it.
    App,
    /// A directory of the caller's, for tests.
    At(PathBuf),
}

/// The app's plugins and commands on `builder`. Without the logger, which a process can only
/// set once, and the desktop menu, which macOS only builds on the main thread: [`run`] adds
/// both.
pub fn shell<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
        .plugin(
            // Links are judged by the origin lock in Rust; the plugin's script, which would open
            // every link with a target in the browser from inside the server's page, stays out.
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::startup,
            commands::connect,
            commands::open_server,
            commands::rename_server,
            commands::remove_server,
        ])
}

/// Reads the server list, puts the app's state in place and opens its window on the picker:
/// what the app does once its event loop is ready.
pub fn setup<R: Runtime>(app: &mut App<R>, data_dir: &DataDir) -> tauri::Result<()> {
    shell::setup(app, data_dir)
}

/// What the app's window does with a navigation to `url` now: the origin lock's decision, which
/// the window's navigation handler acts on, opening outside or showing the picker as it says.
pub fn navigation<R: Runtime>(app: &AppHandle<R>, url: &Url) -> Navigation {
    shell::navigation(app, url)
}

/// The origin lock of the app's window, which its page loads move on.
pub fn lock<R: Runtime>(app: &AppHandle<R>) -> &lock::Lock {
    shell::lock(app)
}

/// Whether the app's window saves a download from `url`.
pub fn download_allowed<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    shell::download_allowed(app, url)
}

/// Runs the app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let log = tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets([
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: None }),
        ])
        .level(log::LevelFilter::Info)
        .max_file_size(1024 * 1024)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
        .build();
    let builder = tauri::Builder::default().plugin(log);
    #[cfg(desktop)]
    let builder = builder.menu(menu::build).on_menu_event(menu::handle);
    let app = shell(builder)
        .setup(|app| {
            setup(app, &DataDir::App)?;
            Ok(())
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(error) => {
            log::error!("the app could not start: {error}");
            std::process::exit(1);
        }
    }
}
