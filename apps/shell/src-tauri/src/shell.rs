//! The app's state and its one window, whose webview the origin lock guards.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

use tauri::utils::config::{WebviewUrl, WindowConfig};
use tauri::webview::{DownloadEvent, PageLoadEvent, WebviewWindowBuilder};
use tauri::{App, AppHandle, Manager, Runtime, Url, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::DataDir;
use crate::address::ServerAddress;
use crate::lock::{Attempt, Lock, Navigation, Started};
use crate::meta::CheckOptions;
use crate::navigation::{self, Origin, Place, Verdict};
use crate::servers::{ServerList, Store};

/// The label of the app's window, which the picker's capability names.
pub const MAIN_WINDOW: &str = "main";

/// The script that, on iOS, opens what `window.open` asks for in the window, as the webview
/// there has no new windows and drops the call.
#[cfg(target_os = "ios")]
const OPEN_IN_PLACE: &str = include_str!("open-in-place.js");

/// How long the window waits for a server's page to start loading once it has asked for it.
/// The server has just answered the check, within that check's ten seconds: a page that has
/// not begun to arrive three times as long after is not coming, while a slow network or a busy
/// server still has room.
pub const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the app waits on a server.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The check of `/api/v1/meta`.
    pub check: CheckOptions,
    /// The server's page, from the window asking for it until it starts loading.
    pub page: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            check: CheckOptions::default(),
            page: PAGE_TIMEOUT,
        }
    }
}

/// Why the window came back to the picker from a server it was opening: the picker says so as
/// it loads again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interrupted {
    /// The server's origin.
    pub origin: String,
    /// `cancelled`, or `page_timed_out`.
    pub code: &'static str,
    pub message: String,
}

/// What the app holds while it runs.
pub struct Shell {
    pub store: Store,
    list: Mutex<ServerList>,
    pub lock: Lock,
    pub limits: Limits,
    /// Whether the picker has been told which server to open as the app starts, which it is
    /// once: coming back to the picker shows the list.
    launched: AtomicBool,
    /// Where each download the server's page started is being saved, by its URL.
    downloads: Mutex<HashMap<String, PathBuf>>,
    /// Why the window last came back to the picker from a server it was opening, until the
    /// picker has read it.
    interrupted: Mutex<Option<Interrupted>>,
    /// Held while the window is sent to a server or back to the picker, so that the page load
    /// of an attempt to open a server and the way back that stops it are asked for in the
    /// order the lock decided them.
    window_changes: Mutex<()>,
}

impl Shell {
    /// The list, locked for a change.
    pub fn list(&self) -> MutexGuard<'_, ServerList> {
        self.list.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The server to open as the app starts: the one used last, the first time only.
    pub fn take_launch(&self) -> Option<String> {
        if self.launched.swap(true, Ordering::SeqCst) {
            None
        } else {
            self.list().last.clone()
        }
    }

    /// Why the window last came back to the picker from a server it was opening, once.
    pub fn take_interrupted(&self) -> Option<Interrupted> {
        self.interrupted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn interrupt(&self, interrupted: Interrupted) {
        *self
            .interrupted
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(interrupted);
    }

    fn window_changes(&self) -> MutexGuard<'_, ()> {
        self.window_changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn downloads(&self) -> MutexGuard<'_, HashMap<String, PathBuf>> {
        self.downloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reads the server list, puts the state in place and opens the window on the picker.
pub fn setup<R: Runtime>(
    app: &mut App<R>,
    data_dir: &DataDir,
    limits: Limits,
) -> tauri::Result<()> {
    let dir = match data_dir {
        DataDir::App => app.path().app_data_dir()?,
        DataDir::At(dir) => dir.clone(),
    };
    let store = Store::new(&dir);
    let list = store.load();
    log::info!(
        "{} saved server(s) in {}",
        list.servers.len(),
        store.path().display()
    );
    let config = main_config(app.handle())?;
    let origin = app_origin(app);
    let picker = picker_url(&origin, &config).ok_or(tauri::Error::WebviewNotFound)?;
    log::info!("the picker is {picker}");
    app.manage(Shell {
        store,
        list: Mutex::new(list),
        // WebView2 names each message's own page; the others name, or may, the window's.
        lock: Lock::new(origin, picker, !cfg!(windows)),
        limits,
        launched: AtomicBool::new(false),
        downloads: Mutex::new(HashMap::new()),
        interrupted: Mutex::new(None),
        window_changes: Mutex::new(()),
    });
    main_window(app.handle(), &config)?;
    Ok(())
}

/// The window's configuration in tauri.conf.json.
fn main_config<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WindowConfig> {
    app.config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)
}

/// Where the picker is: the window's page on the app's own origin, as Tauri loads it. Worked
/// out from the configuration, as a webview only knows its URL once it has started loading.
fn picker_url(app: &Origin, config: &WindowConfig) -> Option<Url> {
    let page = match &config.url {
        WebviewUrl::App(path) => path.to_string_lossy().into_owned(),
        _ => String::new(),
    };
    Url::parse(&format!("{app}/")).ok()?.join(&page).ok()
}

/// The origin the app's own pages are served from: `tauri://localhost` on macOS, iOS and
/// Linux, `http://tauri.localhost` on Windows and Android, and the development server's
/// while developing, as Tauri serves them.
fn app_origin<R: Runtime>(app: &App<R>) -> Origin {
    let config = app.config();
    #[cfg(dev)]
    if let Some(origin) = config.build.dev_url.as_ref().and_then(Origin::of) {
        return origin;
    }
    let https = config
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .is_some_and(|window| window.use_https_scheme);
    if cfg!(any(windows, target_os = "android")) {
        Origin::new(if https { "https" } else { "http" }, "tauri.localhost")
    } else {
        Origin::new("tauri", "localhost")
    }
}

/// The window, from its configuration, with the origin lock on every navigation, page load,
/// new window and download.
fn main_window<R: Runtime>(
    app: &AppHandle<R>,
    config: &WindowConfig,
) -> tauri::Result<WebviewWindow<R>> {
    let navigation_app = app.clone();
    let builder = WebviewWindowBuilder::from_config(app, config)?
        .on_navigation(move |url| on_navigation(&navigation_app, url))
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Started {
                on_page_started(window.app_handle(), payload.url());
            }
        })
        .on_download(|webview, event| on_download(webview.app_handle(), event));
    #[cfg(target_os = "ios")]
    let builder = builder.initialization_script(OPEN_IN_PLACE);
    #[cfg(desktop)]
    let builder = {
        let new_window_app = app.clone();
        builder
            .on_new_window(move |url, _features| {
                on_new_window(&new_window_app, &url);
                tauri::webview::NewWindowResponse::Deny
            })
            .on_document_title_changed(|window, title| {
                if let Err(error) = window.set_title(&title) {
                    log::warn!("could not set the window's title: {error}");
                }
            })
    };
    let window = builder.build()?;
    #[cfg(target_os = "ios")]
    allow_back_gesture(&window);
    Ok(window)
}

/// What the window does with a navigation to `url` now, as the origin lock decides: what its
/// navigation handler acts on.
pub fn navigation<R: Runtime>(app: &AppHandle<R>, url: &Url) -> Navigation {
    app.state::<Shell>().lock.navigation(url, Instant::now())
}

/// The origin lock of the app's window.
pub fn lock<R: Runtime>(app: &AppHandle<R>) -> &Lock {
    &app.state::<Shell>().inner().lock
}

/// Whether a download from `url` is saved: only the server's are.
pub fn download_allowed<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    app.state::<Shell>().lock.judge(url) == Verdict::Load(Place::Server)
}

fn on_navigation<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    let decision = navigation(app, url);
    log::info!("navigation to {}: {decision}", navigation::describe(url));
    act(app, &decision);
    decision.goes_ahead()
}

/// Does what a decision asks beside cancelling the navigation.
fn act<R: Runtime>(app: &AppHandle<R>, decision: &Navigation) {
    match decision {
        Navigation::OpenOutside(url) => open_outside(app, url),
        Navigation::ShowPicker => later(app, show_picker),
        Navigation::Load | Navigation::Refuse | Navigation::TooSoon => {}
    }
}

/// A page has started loading in the window: the lock is told, and the window does what the
/// lock asks next.
fn on_page_started<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let started = app.state::<Shell>().lock.started(url, Instant::now());
    log::info!("page started on {}", navigation::describe(url));
    match started {
        Started::Stay => {}
        Started::LoadPicker(picker) => {
            log::info!("loading the server picker");
            later(app, move |app| window(app)?.navigate(picker));
        }
        Started::Leave { outside, back } => {
            log::warn!(
                "leaving a page on {}, which the origin lock does not allow",
                navigation::describe(url)
            );
            if let Some(outside) = outside {
                open_outside(app, &outside);
            }
            match back {
                Some(back) => later(app, move |app| window(app)?.navigate(back)),
                None => later(app, show_picker),
            }
        }
    }
}

/// Runs `change` on the window after the webview's callback that asked for it has returned:
/// on the main thread, a navigation would run inside that callback.
fn later<R: Runtime>(
    app: &AppHandle<R>,
    change: impl FnOnce(&AppHandle<R>) -> tauri::Result<()> + Send + 'static,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = change(&app) {
            log::warn!("the window could not load the page: {error}");
        }
    });
}

/// Loads the server's page in the window for `attempt`, opening the lock to its origin first,
/// and gives up on it if it has not started loading within the time limit. Loads nothing, and
/// says so, once the attempt was cancelled or another began.
pub fn open_server<R: Runtime>(
    app: &AppHandle<R>,
    address: &ServerAddress,
    attempt: Attempt,
) -> tauri::Result<bool> {
    let shell = app.state::<Shell>();
    let _changing = shell.window_changes();
    if !shell.lock.open_server(Origin::from(address), attempt) {
        return Ok(false);
    }
    log::info!("opening {address}");
    window(app)?.navigate(address.url().clone())?;
    let app = app.clone();
    let limit = shell.limits.page;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(limit).await;
        if let Err(error) = give_up_on_page(&app, attempt, limit) {
            log::warn!("the window could not load the page: {error}");
        }
    });
    Ok(true)
}

/// Shows the picker again, and why, if the page `attempt` asked for has not started loading.
fn give_up_on_page<R: Runtime>(
    app: &AppHandle<R>,
    attempt: Attempt,
    limit: Duration,
) -> tauri::Result<()> {
    let shell = app.state::<Shell>();
    let _changing = shell.window_changes();
    let Some(origin) = shell.lock.stalled(attempt) else {
        return Ok(());
    };
    let seconds = limit.as_secs_f64().ceil() as u64;
    log::warn!("the page of {origin} did not start loading within {seconds} seconds");
    shell.interrupt(Interrupted {
        origin: origin.to_string(),
        code: "page_timed_out",
        message: format!(
            "{origin} answered the app's check, but its page did not start to arrive within \
             {seconds} seconds. Try again, or ask its administrator to look into it."
        ),
    });
    show_picker_now(app)
}

/// Stops the attempt to open a server under way: true when the window goes back to the
/// picker, which stops the server's page loading; false when only its check was under way,
/// after which the attempt now opens nothing.
pub fn cancel_opening<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    let shell = app.state::<Shell>();
    let _changing = shell.window_changes();
    let Some(origin) = shell.lock.cancel_opening() else {
        log::info!("cancelled opening a server during its check");
        return Ok(false);
    };
    log::info!("stopped opening {origin}");
    shell.interrupt(Interrupted {
        origin: origin.to_string(),
        code: "cancelled",
        message: format!("Stopped opening {origin}."),
    });
    show_picker_now(app)?;
    Ok(true)
}

/// Closes the lock to a server removed from the list.
pub fn forget_server<R: Runtime>(app: &AppHandle<R>, address: &ServerAddress) {
    app.state::<Shell>().lock.forget(&Origin::from(address));
}

/// Shows the picker again: a blank page first, which replaces the server's, and the picker
/// once that has committed.
pub fn show_picker<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let shell = app.state::<Shell>();
    let _changing = shell.window_changes();
    show_picker_now(app)
}

/// [`show_picker`], for a caller that holds the window's changes already.
fn show_picker_now<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let blank = app.state::<Shell>().lock.show_picker();
    log::info!("showing the server picker");
    window(app)?.navigate(blank)
}

pub fn window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or(tauri::Error::WindowNotFound)
}

/// Opens a URL from outside the origin lock in the system's browser, or the app for its scheme.
/// Off the thread that judged it, which on mobile is the one the opener needs to call into.
fn open_outside<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let app = app.clone();
    let url = url.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
            log::warn!("{}", outside_failure(&url, &error));
        }
    });
}

/// The log line for a URL the system could not open: its origin and the kind of failure. The
/// opener's own message can hold the command it ran, the whole URL with it, whose path or
/// query may carry a token.
fn outside_failure(url: &Url, error: &tauri_plugin_opener::Error) -> String {
    let reason = match error {
        tauri_plugin_opener::Error::Io(error) => error.kind().to_string(),
        _ => "the system's opener failed".to_owned(),
    };
    format!(
        "could not open {} outside the app: {reason}",
        navigation::describe(url)
    )
}

/// `window.open` and links with a target: the server's own pages load in the window, as there
/// is one; another origin's open outside, as navigations to them do. A blank window, which a
/// page could only fill itself, opens nowhere.
#[cfg(desktop)]
fn on_new_window<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let decision = if url.scheme() == "about" {
        Navigation::Refuse
    } else {
        navigation(app, url)
    };
    log::info!("new window for {}: {decision}", navigation::describe(url));
    match decision {
        Navigation::Load => {
            let url = url.clone();
            later(app, move |app| window(app)?.navigate(url));
        }
        other => act(app, &other),
    }
}

/// Downloads from the server's origin are saved where the webview proposes, the downloads
/// directory; others are refused. A finished one is shown in its folder on desktop; on mobile
/// the system tells of it, so where it went is of no use there.
#[cfg_attr(
    mobile,
    expect(
        unused_variables,
        reason = "only desktop shows a finished download in its folder"
    )
)]
fn on_download<R: Runtime>(app: &AppHandle<R>, event: DownloadEvent<'_>) -> bool {
    let shell = app.state::<Shell>();
    match event {
        DownloadEvent::Requested { url, destination } => {
            let allowed = download_allowed(app, &url);
            #[cfg(target_os = "ios")]
            if allowed && let Some(name) = destination.file_name().map(ToOwned::to_owned) {
                // iOS has no downloads directory: the app's documents, which the Files app
                // shows, take their place.
                match app.path().document_dir() {
                    Ok(dir) => *destination = dir.join(name),
                    Err(error) => log::warn!("no documents directory: {error}"),
                }
            }
            log::info!(
                "download from {}: {}",
                navigation::describe(&url),
                if allowed { "saving" } else { "refused" }
            );
            if allowed {
                shell
                    .downloads()
                    .insert(url.to_string(), destination.clone());
            }
            allowed
        }
        DownloadEvent::Finished { url, path, success } => {
            let proposed = shell.downloads().remove(url.as_str());
            log::info!(
                "download from {} {}",
                navigation::describe(&url),
                if success { "finished" } else { "failed" }
            );
            #[cfg(desktop)]
            if success
                && let Some(path) = path
                    .filter(|path| !path.as_os_str().is_empty())
                    .or(proposed)
                && let Err(error) = app.opener().reveal_item_in_dir(&path)
            {
                log::warn!("could not show the download in its folder: {error}");
            }
            true
        }
        _ => true,
    }
}

/// iOS has no back button: the edge swipe goes back through the server's pages, and from the
/// first of them to the picker, as Android's back gesture does.
#[cfg(target_os = "ios")]
fn allow_back_gesture<R: Runtime>(window: &WebviewWindow<R>) {
    let result = window.with_webview(|webview| {
        let view = webview.inner().cast::<objc2::runtime::AnyObject>();
        // SAFETY: on iOS the handle is the window's WKWebView, alive while the closure runs
        // on the main thread, and the setter takes a BOOL.
        #[expect(unsafe_code, reason = "objc2::msg_send! is unsafe")]
        unsafe {
            let _: () = objc2::msg_send![&*view, setAllowsBackForwardNavigationGestures: true];
        }
    });
    if let Err(error) = result {
        log::warn!("could not turn on the back gesture: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_is_the_windows_page_on_the_apps_origin() {
        let config = WindowConfig {
            url: WebviewUrl::App("index.html".into()),
            ..WindowConfig::default()
        };
        let development = Url::parse("http://localhost:1420").unwrap();
        for (origin, picker) in [
            (
                Origin::new("tauri", "localhost"),
                "tauri://localhost/index.html",
            ),
            (
                Origin::new("http", "tauri.localhost"),
                "http://tauri.localhost/index.html",
            ),
            (
                Origin::new("https", "tauri.localhost"),
                "https://tauri.localhost/index.html",
            ),
            (
                Origin::of(&development).unwrap(),
                "http://localhost:1420/index.html",
            ),
        ] {
            assert_eq!(
                picker_url(&origin, &config).map(String::from).as_deref(),
                Some(picker)
            );
        }
    }

    #[test]
    fn a_failure_to_open_outside_logs_the_origin_alone() {
        let url = Url::parse("https://client.example/callback?code=secret-code&state=s").unwrap();
        // What the open crate reports when a launcher such as gio refuses the URL.
        let error = tauri_plugin_opener::Error::Io(std::io::Error::other(format!(
            "Launcher \"gio\" [\"open\", \"{url}\"] failed with exit status: 2"
        )));
        let line = outside_failure(&url, &error);
        assert_eq!(
            line,
            "could not open https://client.example outside the app: other error"
        );
        let error = tauri_plugin_opener::Error::ForbiddenUrl {
            url: url.to_string(),
            with: None,
        };
        let line = outside_failure(&url, &error);
        assert!(
            !line.contains("secret") && !line.contains("callback"),
            "{line}"
        );
    }
}
