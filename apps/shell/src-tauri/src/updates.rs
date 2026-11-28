//! The app's own updates.
//!
//! - **macOS, Windows and Linux**: Tauri's updater reads `latest.json` from the repository's
//!   latest release, the endpoint in `tauri.conf.json`, which names a file and its signature for
//!   each platform. Installing a newer release downloads the file, which the updater checks
//!   against the public key the app carries, and the version that key signed, before it installs
//!   it; then the app restarts. A copy installed from a `.deb` or `.rpm` package, which the
//!   system's package manager owns, is pointed to the release page instead: the updater would
//!   install the package as root, through `pkexec` or a password prompt of its own.
//! - **Android**, which Tauri's updater does not cover: [`crate::releases`] asks GitHub for the
//!   latest release, and a dialog offers its APK, which the browser downloads.
//! - **iOS**: nothing. The app does not update itself there.
//!
//! The app checks quietly [`FIRST_CHECK`] after it starts and every [`RECHECK`] after, and
//! whenever it is asked: by **Check for Updates…** in the desktop menu, or the picker's
//! **Check for updates** on Android. A quiet check only logs what goes wrong; one that was
//! asked for says it. **Later** stops the quiet checks offering anything for the rest of the
//! run. One check, with what follows it, runs at a time: the menu's, chosen while a quiet one
//! is under way, has that one answer with a dialog as the menu's would. Everything runs in
//! Rust, through the plugins' Rust APIs: no page, the picker included, may call the updater or
//! the dialogs.

#[cfg(target_os = "android")]
mod android;
#[cfg(desktop)]
mod desktop;
#[cfg(any(desktop, target_os = "android"))]
mod dialog;

#[cfg(not(target_os = "ios"))]
use std::sync::Arc;
#[cfg(not(target_os = "ios"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
#[cfg(not(target_os = "ios"))]
use tauri::Manager as _;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Runtime};

use crate::releases::ReleaseError;

#[cfg(target_os = "android")]
use android as platform;
#[cfg(desktop)]
use desktop as platform;

/// How long after the app starts it first checks for updates: long enough for the window, and
/// the server opened last, to have had the network first.
pub const FIRST_CHECK: Duration = Duration::from_secs(5);

/// How long the app waits between its quiet checks while it runs.
pub const RECHECK: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether the picker offers its own check for updates: on Android, where the app has no menu.
pub const PICKER_CHECKS: bool = cfg!(target_os = "android");

/// The most lines of a release's notes a dialog shows.
const NOTES_LINES: usize = 3;

/// The most characters of a release's notes a dialog shows.
const NOTES_CHARS: usize = 300;

/// The updates' state while the app runs: whether a check, or what follows it, is under way,
/// and whether someone answered **Later**.
#[cfg(not(target_os = "ios"))]
#[derive(Debug, Default)]
pub(crate) struct Updates {
    under_way: Arc<UnderWay>,
    declined: AtomicBool,
}

/// What a check under way shares with the [`Busy`] that ends it.
#[cfg(not(target_os = "ios"))]
#[derive(Debug, Default)]
struct UnderWay {
    /// Whether a check, or what follows it, is under way.
    busy: AtomicBool,
    /// Whether the menu's check was chosen while it was, for it to answer as the menu's would.
    menu: AtomicBool,
}

/// A check under way, until it drops.
#[cfg(not(target_os = "ios"))]
#[derive(Debug)]
pub(crate) struct Busy(Arc<UnderWay>);

#[cfg(not(target_os = "ios"))]
impl Busy {
    /// Whether the menu's check was chosen since the check began: once.
    pub(crate) fn asked_by_menu(&self) -> bool {
        self.0.menu.swap(false, Ordering::SeqCst)
    }
}

#[cfg(not(target_os = "ios"))]
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.menu.store(false, Ordering::SeqCst);
        self.0.busy.store(false, Ordering::SeqCst);
    }
}

#[cfg(not(target_os = "ios"))]
impl Updates {
    /// Starts a check, unless one is under way. `asked` by the menu while one is, it has that
    /// one answer as the menu's check would.
    pub(crate) fn begin(&self, asked: Asked) -> Option<Busy> {
        if self.under_way.busy.swap(true, Ordering::SeqCst) {
            if asked == Asked::Menu {
                self.under_way.menu.store(true, Ordering::SeqCst);
            }
            None
        } else {
            Some(Busy(Arc::clone(&self.under_way)))
        }
    }

    /// Someone answered **Later**: the quiet checks offer nothing more in this run.
    pub(crate) fn decline(&self) {
        self.declined.store(true, Ordering::SeqCst);
    }

    /// Whether someone answered **Later** in this run.
    pub(crate) fn declined(&self) -> bool {
        self.declined.load(Ordering::SeqCst)
    }
}

/// Who asked for a check, which decides what it says.
#[cfg(not(target_os = "ios"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    /// The app itself, as it starts and once a day: it logs what goes wrong, and offers a newer
    /// release only until someone answers **Later**.
    Quietly,
    /// The desktop menu: a dialog answers every outcome.
    Menu,
    /// The picker, which shows what the check found itself: only a newer release gets a dialog.
    Picker,
}

/// What a check found, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateCheck {
    /// The app is the latest release, `version`.
    Current { version: String },
    /// Release `version` is newer than the app's, `current`, and a dialog offers it.
    Available { version: String, current: String },
}

/// Why a check, or the update after it, failed, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error(
        "The app could not reach GitHub, where its updates are published. Check the connection, \
         then try again."
    )]
    Unreachable,
    #[error("GitHub did not answer in time. Try again later.")]
    TimedOut,
    #[error("GitHub takes only so many questions an hour from one network. Try again later.")]
    RateLimited,
    #[error("GitHub answered {0} when asked for the latest release. Try again later.")]
    Status(u16),
    #[error("The latest release has no update information yet. Try again later.")]
    NoRelease,
    #[error("The latest release's update information could not be read.")]
    Unreadable,
    #[error("The latest release has no update for this platform.")]
    NoPlatform,
    #[error(
        "The update's signature does not match the key this app trusts, so nothing was installed."
    )]
    Signature,
    #[error("The update could not be downloaded: {0}.")]
    Download(String),
    #[error("The update could not be installed: {0}. This version stays as it was.")]
    Install(String),
    #[error("The app is checking for updates already.")]
    Busy,
    #[error("This app does not update itself here.")]
    Unsupported,
}

impl UpdateError {
    /// A stable name for the picker and the tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::TimedOut => "timed_out",
            Self::RateLimited => "rate_limited",
            Self::Status(_) => "status",
            Self::NoRelease => "no_release",
            Self::Unreadable => "unreadable",
            Self::NoPlatform => "no_platform",
            Self::Signature => "signature",
            Self::Download(_) => "download",
            Self::Install(_) => "install",
            Self::Busy => "busy",
            Self::Unsupported => "unsupported",
        }
    }
}

/// A failure of GitHub's API, as the Android app's check meets it.
impl From<ReleaseError> for UpdateError {
    fn from(error: ReleaseError) -> Self {
        match error {
            ReleaseError::Unreachable { .. } => Self::Unreachable,
            ReleaseError::TimedOut { .. } => Self::TimedOut,
            ReleaseError::RateLimited => Self::RateLimited,
            ReleaseError::NoRelease => Self::NoRelease,
            ReleaseError::Status { status } => Self::Status(status),
            ReleaseError::Unreadable { .. } => Self::Unreadable,
        }
    }
}

/// How this copy of the app was installed, which decides how it updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// From a release's macOS app, Windows setup or AppImage, whose files the updater replaces.
    Replaceable,
    /// From a Linux package, `.deb` or `.rpm`, which the system's package manager owns.
    Package(&'static str),
    /// From an `.msi`, which no release has, or from no installer at all, as a development
    /// build: there is nothing the updater should replace.
    Elsewhere,
}

impl Install {
    /// The install a build of `bundle`, as Tauri's bundler marks the binary of each, makes.
    pub fn of(bundle: Option<BundleType>) -> Self {
        match bundle {
            Some(BundleType::App | BundleType::Dmg | BundleType::Nsis | BundleType::AppImage) => {
                Self::Replaceable
            }
            Some(BundleType::Deb) => Self::Package(".deb"),
            Some(BundleType::Rpm) => Self::Package(".rpm"),
            Some(BundleType::Msi) | None => Self::Elsewhere,
        }
    }

    /// This copy's: none of a development build, whose binary on macOS passes for an app's.
    pub fn this() -> Self {
        if cfg!(dev) {
            Self::Elsewhere
        } else {
            Self::of(tauri::utils::platform::bundle_type())
        }
    }
}

/// What a dialog that offers release `version` says to someone who has `current`, with the
/// first lines of its `notes`.
pub fn offer_text(version: &str, current: &str, notes: Option<&str>) -> String {
    let mut text = format!("Neon Teamspace {version} is available. You have {current}.");
    if let Some(summary) = notes.and_then(summary) {
        text.push_str("\n\n");
        text.push_str(&summary);
    }
    text
}

/// The first lines of a release's notes, as a dialog shows them: at most [`NOTES_LINES`] lines
/// that are not blank, of [`NOTES_CHARS`] characters together, without control characters.
fn summary(notes: &str) -> Option<String> {
    let lines: Vec<String> = notes
        .lines()
        .map(|line| line.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .take(NOTES_LINES)
        .collect();
    if lines.is_empty() {
        return None;
    }
    let joined = lines.join("\n");
    if joined.chars().count() <= NOTES_CHARS {
        return Some(joined);
    }
    // Cut at the last space that fits, so that no word is left in half.
    let mut short: String = joined.chars().take(NOTES_CHARS - 1).collect();
    if let Some(space) = short.rfind(char::is_whitespace) {
        short.truncate(space);
    }
    short.truncate(short.trim_end().len());
    short.push('…');
    Some(short)
}

/// Checks for updates quietly [`FIRST_CHECK`] after the app starts and every [`RECHECK`] after,
/// until someone answers **Later**: what [`crate::run`] starts.
#[cfg(not(target_os = "ios"))]
pub(crate) fn watch<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if app.state::<Updates>().declined() {
                log::info!("no more update checks in this run: the answer was Later");
                return;
            }
            // What goes wrong is logged; nothing more is needed here.
            let _ = check(&app, Asked::Quietly).await;
            tokio::time::sleep(RECHECK).await;
        }
    });
}

/// The picker's **Check for updates**: what the check found, while a dialog offers a newer
/// release.
#[cfg(not(target_os = "ios"))]
pub(crate) async fn from_picker<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<UpdateCheck, UpdateError> {
    check(app, Asked::Picker).await
}

/// The picker's check, which the iOS app does not make: it does not update itself.
#[cfg(target_os = "ios")]
pub(crate) async fn from_picker<R: Runtime>(
    _app: &AppHandle<R>,
) -> Result<UpdateCheck, UpdateError> {
    Err(UpdateError::Unsupported)
}

/// Checks for updates as `asked`: what the check found, while a dialog offers a newer release
/// and does what the answer says. A dialog also answers the menu's check when the app is the
/// latest release or the check failed.
#[cfg(not(target_os = "ios"))]
pub(crate) async fn check<R: Runtime>(
    app: &AppHandle<R>,
    asked: Asked,
) -> Result<UpdateCheck, UpdateError> {
    let Some(busy) = app.state::<Updates>().begin(asked) else {
        log::info!("a check for updates is under way already");
        return Err(UpdateError::Busy);
    };
    let current = app.package_info().version.to_string();
    log::info!("checking for updates to {current} ({asked:?})");
    let found = platform::find(app).await;
    let asked = if busy.asked_by_menu() {
        Asked::Menu
    } else {
        asked
    };
    match found {
        Ok(None) => {
            log::info!("{current} is the latest release");
            if asked == Asked::Menu {
                dialog::show(app, dialog::Dialog::up_to_date(&current)).await;
            }
            Ok(UpdateCheck::Current { version: current })
        }
        Ok(Some(offer)) => {
            let version = platform::version(&offer);
            log::info!("{version} is newer than {current}");
            if asked == Asked::Quietly && app.state::<Updates>().declined() {
                return Ok(UpdateCheck::Available { version, current });
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                platform::offer(&app, offer, asked).await;
                drop(busy);
            });
            Ok(UpdateCheck::Available { version, current })
        }
        Err(error) => {
            log::warn!("the check for updates failed ({}): {error}", error.code());
            if asked == Asked::Menu {
                dialog::show(app, dialog::Dialog::check_failed(&error)).await;
            }
            Err(error)
        }
    }
}

/// The menu's **Check for Updates…**, which runs while the menu goes on.
#[cfg(desktop)]
pub(crate) fn check_from_menu<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // A dialog says what went wrong. A check under way already answers with a dialog now,
        // and an offer, a download or an install has one of its own.
        let _ = check(&app, Asked::Menu).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_names_both_versions_and_the_first_lines_of_the_notes() {
        assert_eq!(
            offer_text("0.2.0", "0.1.0", None),
            "Neon Teamspace 0.2.0 is available. You have 0.1.0."
        );
        assert_eq!(
            offer_text(
                "0.2.0",
                "0.1.0",
                Some(
                    "\n\nApps that update themselves.\n\n  Faster search.\nA new board view.\nMore."
                )
            ),
            "Neon Teamspace 0.2.0 is available. You have 0.1.0.\n\nApps that update \
             themselves.\nFaster search.\nA new board view."
        );
        assert_eq!(
            offer_text("0.2.0", "0.1.0", Some(" \n\t\n")),
            "Neon Teamspace 0.2.0 is available. You have 0.1.0."
        );
    }

    #[test]
    fn long_notes_are_cut_short_and_lose_their_control_characters() {
        let long = "word ".repeat(100);
        let short = summary(&long).unwrap();
        assert!(short.chars().count() <= NOTES_CHARS, "{short}");
        assert!(short.ends_with(" word…"), "{short}");
        assert_eq!(summary("a\u{7}b\u{1b}[31m").unwrap(), "ab[31m");
    }

    #[test]
    fn the_install_decides_how_the_app_updates() {
        for bundle in [
            BundleType::App,
            BundleType::Dmg,
            BundleType::Nsis,
            BundleType::AppImage,
        ] {
            assert_eq!(
                Install::of(Some(bundle.clone())),
                Install::Replaceable,
                "{bundle:?}"
            );
        }
        assert_eq!(Install::of(Some(BundleType::Deb)), Install::Package(".deb"));
        assert_eq!(Install::of(Some(BundleType::Rpm)), Install::Package(".rpm"));
        assert_eq!(Install::of(Some(BundleType::Msi)), Install::Elsewhere);
        assert_eq!(Install::of(None), Install::Elsewhere);
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn one_check_runs_at_a_time() {
        let updates = Updates::default();
        let first = updates
            .begin(Asked::Quietly)
            .expect("the first check starts");
        for asked in [Asked::Quietly, Asked::Menu, Asked::Picker] {
            assert!(
                updates.begin(asked).is_none(),
                "a check {asked:?} started beside it"
            );
        }
        drop(first);
        assert!(
            updates.begin(Asked::Quietly).is_some(),
            "no check starts once the first ended"
        );
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn the_menus_check_chosen_meanwhile_has_the_quiet_one_answer_for_it() {
        let updates = Updates::default();
        let quiet = updates
            .begin(Asked::Quietly)
            .expect("the quiet check starts");
        assert!(!quiet.asked_by_menu());
        assert!(updates.begin(Asked::Picker).is_none());
        assert!(!quiet.asked_by_menu(), "the picker's check is no menu's");
        assert!(updates.begin(Asked::Menu).is_none());
        assert!(quiet.asked_by_menu());
        assert!(!quiet.asked_by_menu(), "the menu's choice is taken once");
        // Chosen during an offer or an install, it is forgotten with the check.
        assert!(updates.begin(Asked::Menu).is_none());
        drop(quiet);
        let next = updates
            .begin(Asked::Quietly)
            .expect("the next check starts");
        assert!(!next.asked_by_menu());
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn later_is_kept_for_the_run() {
        let updates = Updates::default();
        assert!(!updates.declined());
        updates.decline();
        assert!(updates.declined());
    }

    #[test]
    fn githubs_failures_keep_their_sense() {
        for (error, expected) in [
            (
                ReleaseError::Unreachable {
                    reason: "connection refused".to_owned(),
                },
                UpdateError::Unreachable,
            ),
            (
                ReleaseError::TimedOut { seconds: 15 },
                UpdateError::TimedOut,
            ),
            (ReleaseError::RateLimited, UpdateError::RateLimited),
            (ReleaseError::NoRelease, UpdateError::NoRelease),
            (
                ReleaseError::Status { status: 502 },
                UpdateError::Status(502),
            ),
            (
                ReleaseError::Unreadable {
                    what: "its tag is not a version".to_owned(),
                },
                UpdateError::Unreadable,
            ),
        ] {
            assert_eq!(UpdateError::from(error.clone()), expected, "{error:?}");
        }
    }

    #[test]
    fn the_picker_reads_what_a_check_found() {
        assert_eq!(
            serde_json::to_value(UpdateCheck::Current {
                version: "0.1.0".to_owned()
            })
            .unwrap(),
            serde_json::json!({"state": "current", "version": "0.1.0"})
        );
        assert_eq!(
            serde_json::to_value(UpdateCheck::Available {
                version: "0.2.0".to_owned(),
                current: "0.1.0".to_owned()
            })
            .unwrap(),
            serde_json::json!({"state": "available", "version": "0.2.0", "current": "0.1.0"})
        );
    }

    #[test]
    fn every_failure_has_a_code_and_a_sentence() {
        let errors = [
            UpdateError::Unreachable,
            UpdateError::TimedOut,
            UpdateError::RateLimited,
            UpdateError::Status(502),
            UpdateError::NoRelease,
            UpdateError::Unreadable,
            UpdateError::NoPlatform,
            UpdateError::Signature,
            UpdateError::Download("the connection broke off".to_owned()),
            UpdateError::Install("permission denied".to_owned()),
            UpdateError::Busy,
            UpdateError::Unsupported,
        ];
        let mut codes: Vec<&str> = errors.iter().map(UpdateError::code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), errors.len(), "two failures share a code");
        for error in errors {
            let message = error.to_string();
            assert!(message.ends_with('.'), "{message}");
            assert!(message.starts_with(char::is_uppercase), "{message}");
        }
    }
}
