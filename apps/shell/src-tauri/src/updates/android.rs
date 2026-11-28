//! The Android app's updates, which Tauri's updater does not cover: the latest release from
//! GitHub's API, and a dialog that offers its APK, which the browser downloads as it would any
//! link the app opens outside, for the system's installer to install over the app.

use semver::Version;
use tauri::{AppHandle, Manager as _, Runtime};

use super::dialog::{self, Dialog};
use super::{Asked, UpdateError, Updates, offer_text};
use crate::releases::{self, Release, ReleaseOptions};
use crate::shell;

/// Asks GitHub for the latest release: `None` when it is no newer than the app.
pub(super) async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Release>, UpdateError> {
    let release = releases::latest(&ReleaseOptions::default())
        .await
        .map_err(|error| {
            log::warn!(
                "GitHub's latest release could not be had ({}): {error}",
                error.code()
            );
            UpdateError::from(error)
        })?;
    let current: &Version = &app.package_info().version;
    Ok(releases::is_newer(&release.version, current).then_some(release))
}

/// The version `release` brings.
pub(super) fn version(release: &Release) -> String {
    release.version.to_string()
}

/// Offers `release` in a dialog: its APK, to download, or, asked for by the picker, its page
/// when it has none. A quiet check offers a release only with its APK.
pub(super) async fn offer<R: Runtime>(app: &AppHandle<R>, release: Release, asked: Asked) {
    let version = release.version.to_string();
    let current = app.package_info().version.to_string();
    let mut message = offer_text(&version, &current, None);
    let (go, open) = match release.apk {
        Some(apk) => {
            message.push_str(
                "\n\nDownload the new APK, then open it to install it over this app. Your \
                 servers stay as they are.",
            );
            ("Download", apk)
        }
        None if asked == Asked::Quietly => {
            log::info!("{version} has no APK for this app yet: nothing to offer");
            return;
        }
        None => {
            message.push_str("\n\nIts release has no APK for this app yet.");
            ("Open Release Page", release.page)
        }
    };
    if dialog::show(app, Dialog::offer(message, go)).await {
        shell::open_outside(app, &open);
    } else {
        log::info!("{version} was offered, and put off until later");
        app.state::<Updates>().decline();
    }
}
