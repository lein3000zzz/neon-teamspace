//! The desktop apps' updates, through Tauri's updater: the check of the latest release's
//! `latest.json`, the offer, and the download, signature check, install and restart that
//! follow **Install and Restart**. While the update downloads and installs, the menu's
//! **Check for Updates…** says how far it is and cannot be chosen, and the Dock or the taskbar
//! shows the progress where the platform has a place for it; the window stays in use.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager as _, Runtime, Url};
use tauri_plugin_updater::{Error, Update, UpdaterExt as _};

use super::dialog::{self, Dialog};
use super::{Asked, Install, UpdateError, Updates, offer_text};
use crate::{menu, shell};

/// How long the check of `latest.json` may take. The download has no limit, as a slow network
/// takes its time over the app's few megabytes.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// The title of the dialog that says an update failed.
const FAILED: &str = "Could Not Update Neon Teamspace";

/// What the updater was doing when it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Check,
    Download,
    Install,
}

/// Asks the updater for the latest release: `None` when it is no newer than the app.
pub(super) async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, UpdateError> {
    let updater = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| logged(&error, Step::Check))?;
    updater
        .check()
        .await
        .map_err(|error| logged(&error, Step::Check))
}

/// The version `update` brings.
pub(super) fn version(update: &Update) -> String {
    update.version.clone()
}

/// Offers `update` in a dialog, and installs it, or opens its release page for an install the
/// updater does not replace, when the answer is to go ahead. **Later** puts the offers off for
/// the rest of the run.
pub(super) async fn offer<R: Runtime>(app: &AppHandle<R>, update: Update, _asked: Asked) {
    let install = Install::this();
    let (message, go) = offer_message(
        install,
        &update.version,
        &update.current_version,
        update.body.as_deref(),
    );
    if !dialog::show(app, Dialog::offer(message, go)).await {
        log::info!("{} was offered, and put off until later", update.version);
        app.state::<Updates>().decline();
        return;
    }
    match install {
        Install::Replaceable => self::install(app, &update).await,
        Install::Package(_) | Install::Elsewhere => match release_page(&update.version) {
            Ok(page) => shell::open_outside(app, &page),
            Err(error) => log::warn!("no release page for {}: {error}", update.version),
        },
    }
}

/// What the offer of release `version` to someone who has `current` says for `install`, and the
/// label of its button that goes ahead.
fn offer_message(
    install: Install,
    version: &str,
    current: &str,
    notes: Option<&str>,
) -> (String, &'static str) {
    let mut message = offer_text(version, current, notes);
    let go = match install {
        Install::Replaceable => "Install and Restart",
        Install::Package(kind) => {
            message.push_str(&format!(
                "\n\nThis copy was installed from a {kind} package, which the system's package \
                 manager updates: download the new {kind} from the release page, and install it \
                 as you installed this one."
            ));
            "Open Release Page"
        }
        Install::Elsewhere => {
            message.push_str(
                "\n\nThis copy was not installed from one of the release's installers, so it does \
                 not update itself: the release page has them.",
            );
            "Open Release Page"
        }
    };
    (message, go)
}

/// The page of release `version` on GitHub.
fn release_page(version: &str) -> Result<Url, url::ParseError> {
    Url::parse(&format!(
        "https://github.com/{}/releases/tag/v{version}",
        crate::releases::REPOSITORY
    ))
}

/// Downloads `update`, which the updater checks against the app's key, installs it and
/// restarts the app; or says in a dialog what went wrong. On Windows the installer closes the
/// app itself, and opens the new version once it is done.
async fn install<R: Runtime>(app: &AppHandle<R>, update: &Update) {
    log::info!("downloading {}", update.version);
    let progress = Progress::start(app);
    let mut received: u64 = 0;
    let downloaded = update
        .download(
            |chunk, total| {
                received = received.saturating_add(chunk as u64);
                progress.downloading(received, total);
            },
            || progress.installing(),
        )
        .await;
    let bytes = match downloaded {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(progress);
            return failed(app, &error, Step::Download).await;
        }
    };
    progress.installing();
    log::info!("installing {}", update.version);
    let installing = update.clone();
    let installed = tauri::async_runtime::spawn_blocking(move || installing.install(bytes)).await;
    drop(progress);
    match installed {
        Ok(Ok(())) => {
            log::info!("installed {}; restarting", update.version);
            app.request_restart();
        }
        Ok(Err(error)) => failed(app, &error, Step::Install).await,
        Err(error) => {
            log::warn!("the install of {} did not finish: {error}", update.version);
            let failure = UpdateError::Install("it stopped before it was done".to_owned());
            dialog::show(app, Dialog::failed(FAILED, &failure)).await;
        }
    }
}

/// Says in a dialog why the update failed at `step`, and logs the updater's own words.
async fn failed<R: Runtime>(app: &AppHandle<R>, error: &Error, step: Step) {
    let failure = logged(error, step);
    dialog::show(app, Dialog::failed(FAILED, &failure)).await;
}

/// [`failure`], with the updater's own words in the log.
fn logged(error: &Error, step: Step) -> UpdateError {
    let failure = failure(error, step);
    log::warn!("the updater failed ({step:?}, {}): {error}", failure.code());
    failure
}

/// The updater's error at `step` in words for the person.
fn failure(error: &Error, step: Step) -> UpdateError {
    match (error, step) {
        (Error::Reqwest(error), Step::Check) if error.is_timeout() => UpdateError::TimedOut,
        (Error::Reqwest(_), Step::Check) => UpdateError::Unreachable,
        (Error::Reqwest(error), _) if error.is_timeout() => {
            UpdateError::Download("it took too long".to_owned())
        }
        (Error::Reqwest(_), _) => UpdateError::Download("the connection broke off".to_owned()),
        (Error::Network(message), _) => UpdateError::Download(match message.rsplit_once(": ") {
            Some((_, status)) => format!("GitHub answered {status}"),
            None => "GitHub did not send it".to_owned(),
        }),
        (Error::ReleaseNotFound, _) => UpdateError::NoRelease,
        (Error::Serialization(_) | Error::Semver(_) | Error::UrlParse(_), _) => {
            UpdateError::Unreadable
        }
        (Error::TargetNotFound(_) | Error::TargetsNotFound(_), _) => UpdateError::NoPlatform,
        (
            Error::Minisign(_)
            | Error::Base64(_)
            | Error::SignatureUtf8(_)
            | Error::SignedVersionMismatch { .. }
            | Error::MissingSignedVersion,
            _,
        ) => UpdateError::Signature,
        (
            Error::EmptyEndpoints
            | Error::InsecureTransportProtocol
            | Error::UnsupportedArch
            | Error::UnsupportedOs
            | Error::FailedToDetermineExtractPath,
            _,
        ) => UpdateError::Unsupported,
        (Error::Io(error), Step::Download | Step::Install) => {
            UpdateError::Install(error.kind().to_string())
        }
        (Error::PreviousAppNotRestored(path), _) => UpdateError::Install(format!(
            "the version it replaced could not be put back either, and was left at {}",
            path.display()
        )),
        (Error::InvalidUpdaterFormat | Error::BinaryNotFoundInArchive, _) => {
            UpdateError::Install("the download is not a file this platform installs".to_owned())
        }
        (Error::TempDirNotFound | Error::TempDirNotOnSameMountPoint, _) => {
            UpdateError::Install("there was no room to unpack it beside the app".to_owned())
        }
        (Error::AuthenticationFailed, _) => {
            UpdateError::Install("the system did not give the permission it needs".to_owned())
        }
        (_, Step::Check) => UpdateError::Unreadable,
        (_, Step::Download) => UpdateError::Download("something unexpected went wrong".to_owned()),
        (_, Step::Install) => UpdateError::Install("something unexpected went wrong".to_owned()),
    }
}

/// The download's and install's progress, in the menu and on the Dock or the taskbar, until it
/// drops.
struct Progress<R: Runtime> {
    app: AppHandle<R>,
    /// One more than the percentage shown last, or 0 before any.
    shown: AtomicU64,
}

impl<R: Runtime> Progress<R> {
    fn start(app: &AppHandle<R>) -> Self {
        let progress = Self {
            app: app.clone(),
            shown: AtomicU64::new(0),
        };
        progress.show("Downloading Update…", None);
        progress
    }

    /// `received` bytes of `total` have arrived: shown once for each percent.
    fn downloading(&self, received: u64, total: Option<u64>) {
        let Some(total) = total.filter(|total| *total > 0) else {
            return;
        };
        let percent = (received.saturating_mul(100) / total).min(100);
        if self.shown.swap(percent + 1, Ordering::SeqCst) != percent + 1 {
            self.show(&format!("Downloading Update… {percent}%"), Some(percent));
        }
    }

    fn installing(&self) {
        self.show("Installing Update…", None);
    }

    fn show(&self, text: &str, percent: Option<u64>) {
        if let Some(item) = menu::check_for_updates(&self.app) {
            let shown = item.set_text(text).and_then(|()| item.set_enabled(false));
            if let Err(error) = shown {
                log::warn!("could not show the update's progress in the menu: {error}");
            }
        }
        self.progress_bar(ProgressBarState {
            status: Some(if percent.is_some() {
                ProgressBarStatus::Normal
            } else {
                ProgressBarStatus::Indeterminate
            }),
            progress: percent,
        });
    }

    fn progress_bar(&self, state: ProgressBarState) {
        // Only some Linux desktops have a place for it, and the others say nothing.
        if let Ok(window) = shell::window(&self.app)
            && let Err(error) = window.set_progress_bar(state)
        {
            log::debug!("no progress bar for the update: {error}");
        }
    }
}

impl<R: Runtime> Drop for Progress<R> {
    fn drop(&mut self) {
        if let Some(item) = menu::check_for_updates(&self.app) {
            let restored = item
                .set_text(menu::CHECK_FOR_UPDATES_LABEL)
                .and_then(|()| item.set_enabled(true));
            if let Err(error) = restored {
                log::warn!("could not restore the menu's Check for Updates…: {error}");
            }
        }
        self.progress_bar(ProgressBarState {
            status: Some(ProgressBarStatus::None),
            progress: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn an_install_the_updater_replaces_is_offered_to_install_and_restart() {
        let (message, go) = offer_message(
            Install::Replaceable,
            "0.2.0",
            "0.1.0",
            Some("Apps that update themselves.\n\nWhat changed: https://example.com"),
        );
        assert_eq!(go, "Install and Restart");
        assert_eq!(
            message,
            "Neon Teamspace 0.2.0 is available. You have 0.1.0.\n\nApps that update \
             themselves.\nWhat changed: https://example.com"
        );
    }

    #[test]
    fn a_linux_package_is_pointed_to_the_release_page() {
        for kind in [".deb", ".rpm"] {
            let (message, go) = offer_message(Install::Package(kind), "0.2.0", "0.1.0", None);
            assert_eq!(go, "Open Release Page");
            assert!(
                message.ends_with(&format!(
                    "This copy was installed from a {kind} package, which the system's package \
                     manager updates: download the new {kind} from the release page, and install \
                     it as you installed this one."
                )),
                "{message}"
            );
        }
        let (message, go) = offer_message(Install::Elsewhere, "0.2.0", "0.1.0", None);
        assert_eq!(go, "Open Release Page");
        assert!(message.contains("does not update itself"), "{message}");
    }

    #[test]
    fn a_release_page_is_its_tags() {
        assert_eq!(
            release_page("0.2.0").unwrap().as_str(),
            "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn the_updaters_errors_are_put_in_plain_words() {
        for (error, step, expected) in [
            (Error::ReleaseNotFound, Step::Check, UpdateError::NoRelease),
            (
                Error::TargetsNotFound(vec!["darwin-aarch64-app".to_owned()]),
                Step::Check,
                UpdateError::NoPlatform,
            ),
            (
                Error::MissingSignedVersion,
                Step::Download,
                UpdateError::Signature,
            ),
            (
                Error::SignedVersionMismatch {
                    signed: "0.1.0".to_owned(),
                    announced: "0.2.0".to_owned(),
                },
                Step::Download,
                UpdateError::Signature,
            ),
            (
                Error::SignatureUtf8("x".to_owned()),
                Step::Download,
                UpdateError::Signature,
            ),
            (
                Error::Network("Download request failed with status: 404 Not Found".to_owned()),
                Step::Download,
                UpdateError::Download("GitHub answered 404 Not Found".to_owned()),
            ),
            (
                Error::Io(std::io::ErrorKind::PermissionDenied.into()),
                Step::Install,
                UpdateError::Install("permission denied".to_owned()),
            ),
            (
                Error::PreviousAppNotRestored(PathBuf::from("/tmp/old.app")),
                Step::Install,
                UpdateError::Install(
                    "the version it replaced could not be put back either, and was left at \
                     /tmp/old.app"
                        .to_owned(),
                ),
            ),
            (
                Error::InvalidUpdaterFormat,
                Step::Install,
                UpdateError::Install(
                    "the download is not a file this platform installs".to_owned(),
                ),
            ),
            (
                Error::InsecureTransportProtocol,
                Step::Check,
                UpdateError::Unsupported,
            ),
            (
                Error::Io(std::io::ErrorKind::NotFound.into()),
                Step::Check,
                UpdateError::Unreadable,
            ),
        ] {
            assert_eq!(failure(&error, step), expected, "{error:?} at {step:?}");
        }
    }
}
