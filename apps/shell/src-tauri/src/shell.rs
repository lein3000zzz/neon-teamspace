//! The app's state and its one window, whose webview the origin lock guards.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use tauri::webview::{DownloadEvent, WebviewWindowBuilder};
use tauri::{App, AppHandle, Manager, Runtime, Url, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::DataDir;
use crate::address::ServerAddress;
use crate::meta::CheckOptions;
use crate::navigation::{self, Origin, Place, Verdict};
use crate::servers::{ServerList, Store};

/// The label of the app's window, which the picker's capability names.
pub const MAIN_WINDOW: &str = "main";

/// What the app holds while it runs.
pub struct Shell {
    pub store: Store,
    pub list: Mutex<ServerList>,
    pub lock: Arc<Lock>,
    pub options: CheckOptions,
    /// Whether the picker has been told which server to open as the app starts, which it is
    /// once: coming back to the picker shows the list.
    launched: AtomicBool,
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
}

/// The origin lock's state: the app's own origin, where the picker is, and the servers'.
pub struct Lock {
    app: Origin,
    /// Where the picker is, for the desktop menu's way back to it; on mobile the back gesture
    /// returns to it through the webview's history.
    #[cfg(desktop)]
    picker: RwLock<Option<Url>>,
    /// The origins of the servers opened since the app started, the one in use last.
    servers: RwLock<Vec<Origin>>,
    /// Where each download the server's page started is being saved, by its URL.
    downloads: Mutex<HashMap<String, PathBuf>>,
}

impl Lock {
    fn new(app: Origin) -> Self {
        Self {
            app,
            #[cfg(desktop)]
            picker: RwLock::new(None),
            servers: RwLock::new(Vec::new()),
            downloads: Mutex::new(HashMap::new()),
        }
    }

    /// Where a navigation to `url` goes now.
    pub fn judge(&self, url: &Url) -> Verdict {
        let servers = self.servers.read().unwrap_or_else(PoisonError::into_inner);
        navigation::judge(url, &self.app, &servers)
    }

    fn open_server(&self, address: &ServerAddress) {
        let Some(origin) = Origin::of(address.url()) else {
            return;
        };
        let mut servers = self.servers.write().unwrap_or_else(PoisonError::into_inner);
        servers.retain(|opened| *opened != origin);
        servers.push(origin);
    }

    /// Closes the lock to a server removed from the list.
    pub fn forget(&self, address: &ServerAddress) {
        if let Some(origin) = Origin::of(address.url()) {
            let mut servers = self.servers.write().unwrap_or_else(PoisonError::into_inner);
            servers.retain(|opened| *opened != origin);
        }
    }

    #[cfg(desktop)]
    fn picker(&self) -> Option<Url> {
        self.picker
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Reads the server list, puts the state in place and opens the window on the picker.
pub fn setup<R: Runtime>(app: &mut App<R>, data_dir: &DataDir) -> tauri::Result<()> {
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
    let lock = Arc::new(Lock::new(app_origin(app)));
    app.manage(Shell {
        store,
        list: Mutex::new(list),
        lock: Arc::clone(&lock),
        options: CheckOptions::default(),
        launched: AtomicBool::new(false),
    });

    let window = main_window(app.handle(), Arc::clone(&lock))?;
    let picker = window.url()?;
    log::info!("the picker is at {}", navigation::describe(&picker));
    #[cfg(desktop)]
    {
        *lock.picker.write().unwrap_or_else(PoisonError::into_inner) = Some(picker);
    }
    Ok(())
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

/// The window, from its configuration, with the origin lock on every navigation, new window
/// and download.
fn main_window<R: Runtime>(app: &AppHandle<R>, lock: Arc<Lock>) -> tauri::Result<WebviewWindow<R>> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;

    let navigation_lock = Arc::clone(&lock);
    let navigation_app = app.clone();
    let builder = WebviewWindowBuilder::from_config(app, &config)?
        .on_navigation(move |url| {
            let verdict = navigation_lock.judge(url);
            log::info!("navigation to {}: {verdict}", navigation::describe(url));
            if verdict == Verdict::OpenOutside {
                open_outside(&navigation_app, url);
            }
            matches!(verdict, Verdict::Load(_))
        })
        .on_download(move |webview, event| on_download(webview.app_handle(), &lock, event));
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

/// Loads the server's page in the window, opening the lock to its origin first.
pub fn open_server<R: Runtime>(app: &AppHandle<R>, address: &ServerAddress) -> tauri::Result<()> {
    let shell = app.state::<Shell>();
    shell.lock.open_server(address);
    log::info!("opening {address}");
    window(app)?.navigate(address.url().clone())
}

/// Shows the picker again.
#[cfg(desktop)]
pub fn show_picker<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let shell = app.state::<Shell>();
    let picker = shell.lock.picker().ok_or(tauri::Error::WebviewNotFound)?;
    log::info!("showing the server picker");
    window(app)?.navigate(picker)
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
            log::warn!(
                "could not open {} outside the app: {error}",
                navigation::describe(&url)
            );
        }
    });
}

/// `window.open` and links with a target: the server's own pages load in the window, as there
/// is one; another origin's open outside, as navigations to them do.
#[cfg(desktop)]
fn on_new_window<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    let verdict = app.state::<Shell>().lock.judge(url);
    log::info!("new window for {}: {verdict}", navigation::describe(url));
    match verdict {
        Verdict::Load(Place::Server) => {
            if let Err(error) = window(app).and_then(|window| window.navigate(url.clone())) {
                log::warn!("could not load {}: {error}", navigation::describe(url));
            }
        }
        Verdict::OpenOutside => open_outside(app, url),
        Verdict::Load(Place::App) | Verdict::Refuse => {}
    }
}

/// Downloads from the server's origin are saved where the webview proposes, the downloads
/// directory; others are refused. A finished one is shown in its folder on desktop.
#[cfg_attr(target_os = "android", allow(unused_variables))]
fn on_download<R: Runtime>(app: &AppHandle<R>, lock: &Lock, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let allowed = lock.judge(&url) == Verdict::Load(Place::Server);
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
                lock.downloads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(url.to_string(), destination.clone());
            }
            allowed
        }
        DownloadEvent::Finished { url, path, success } => {
            let proposed = lock
                .downloads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(url.as_str());
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
        #[allow(unsafe_code)]
        unsafe {
            let _: () = objc2::msg_send![&*view, setAllowsBackForwardNavigationGestures: true];
        }
    });
    if let Err(error) = result {
        log::warn!("could not turn on the back gesture: {error}");
    }
}
