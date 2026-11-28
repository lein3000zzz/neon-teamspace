//! The native dialogs that answer a check for updates and offer a newer release, which the app
//! shows from Rust: rfd's on the desktop, which Tauri's dialog plugin shows there too, attached
//! to the app's window; the dialog plugin's on Android.

use tauri::{AppHandle, Runtime};
use tokio::sync::oneshot;

use super::UpdateError;

/// The look of a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Info,
    Error,
}

/// A dialog: a title, a message, and either OK alone or a choice of two buttons, the first of
/// which goes ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dialog {
    pub title: String,
    pub message: String,
    pub kind: Kind,
    /// The labels of the button that goes ahead and of the one that does not.
    pub choice: Option<(String, String)>,
}

impl Dialog {
    /// The answer to a check of the menu's when the app is the latest release, `current`.
    pub(crate) fn up_to_date(current: &str) -> Self {
        Self {
            title: "Neon Teamspace Is Up to Date".to_owned(),
            message: format!("Neon Teamspace {current} is the latest version."),
            kind: Kind::Info,
            choice: None,
        }
    }

    /// The answer to a check of the menu's that failed.
    pub(crate) fn check_failed(error: &UpdateError) -> Self {
        Self {
            title: "Could Not Check for Updates".to_owned(),
            message: error.to_string(),
            kind: Kind::Error,
            choice: None,
        }
    }

    /// An offer of a newer release, saying `message`, whose first button is `go`.
    pub(crate) fn offer(message: String, go: &str) -> Self {
        Self {
            title: "Update Available".to_owned(),
            message,
            kind: Kind::Info,
            choice: Some((go.to_owned(), "Later".to_owned())),
        }
    }

    /// A failure after the offer was taken, such as a download that broke off.
    #[cfg_attr(
        target_os = "android",
        expect(dead_code, reason = "the browser downloads the APK on Android")
    )]
    pub(crate) fn failed(title: &str, error: &UpdateError) -> Self {
        Self {
            title: title.to_owned(),
            message: error.to_string(),
            kind: Kind::Error,
            choice: None,
        }
    }
}

/// Shows `dialog` and waits for its answer: true when the button that goes ahead was chosen,
/// false for the other, for closing the dialog, and when it could not be shown, which is
/// logged.
#[cfg(desktop)]
pub(crate) async fn show<R: Runtime>(app: &AppHandle<R>, dialog: Dialog) -> bool {
    use rfd::{AsyncMessageDialog, MessageButtons, MessageLevel};

    let (sender, answer) = oneshot::channel();
    let parent = crate::shell::window(app).ok();
    let go = dialog.choice.as_ref().map(|(go, _)| go.clone());
    let buttons = match dialog.choice {
        Some((go, later)) => MessageButtons::OkCancelCustom(go, later),
        None => MessageButtons::Ok,
    };
    let level = match dialog.kind {
        Kind::Info => MessageLevel::Info,
        Kind::Error => MessageLevel::Error,
    };
    // As the dialog plugin does: rfd builds the dialog on the main thread, and a thread of its
    // own waits for the answer.
    let shown = app.run_on_main_thread(move || {
        let mut message = AsyncMessageDialog::new()
            .set_title(&dialog.title)
            .set_description(&dialog.message)
            .set_level(level)
            .set_buttons(buttons);
        if let Some(window) = &parent {
            message = message.set_parent(window);
        }
        let shown = message.show();
        std::thread::spawn(move || {
            let _ = sender.send(tauri::async_runtime::block_on(shown));
        });
    });
    if let Err(error) = shown {
        log::warn!("could not show a dialog: {error}");
        return false;
    }
    answer
        .await
        .is_ok_and(|answer| goes_ahead(&answer, go.as_deref()))
}

/// Whether rfd's `answer` is the button that goes ahead, labelled `go`: macOS and Windows name
/// the button, while GTK answers OK or Cancel for the first and the second.
#[cfg(desktop)]
fn goes_ahead(answer: &rfd::MessageDialogResult, go: Option<&str>) -> bool {
    match answer {
        rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes => true,
        rfd::MessageDialogResult::Custom(label) => Some(label.as_str()) == go,
        rfd::MessageDialogResult::No | rfd::MessageDialogResult::Cancel => false,
    }
}

/// [`show`], on Android: the dialog plugin's dialog, whose answer comes back on a thread of the
/// plugin's.
#[cfg(target_os = "android")]
pub(crate) async fn show<R: Runtime>(app: &AppHandle<R>, dialog: Dialog) -> bool {
    use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

    let (sender, answer) = oneshot::channel();
    let buttons = match dialog.choice {
        Some((go, later)) => MessageDialogButtons::OkCancelCustom(go, later),
        None => MessageDialogButtons::Ok,
    };
    let kind = match dialog.kind {
        Kind::Info => MessageDialogKind::Info,
        Kind::Error => MessageDialogKind::Error,
    };
    app.dialog()
        .message(dialog.message)
        .title(dialog.title)
        .kind(kind)
        .buttons(buttons)
        .show(move |went_ahead| {
            let _ = sender.send(went_ahead);
        });
    answer.await.unwrap_or(false)
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;

    #[test]
    fn the_first_button_goes_ahead_whatever_the_platform_answers() {
        use rfd::MessageDialogResult as Answer;

        let go = Some("Install and Restart");
        assert!(goes_ahead(
            &Answer::Custom("Install and Restart".to_owned()),
            go
        ));
        assert!(goes_ahead(&Answer::Ok, go));
        assert!(!goes_ahead(&Answer::Custom("Later".to_owned()), go));
        assert!(!goes_ahead(&Answer::Cancel, go));
        assert!(!goes_ahead(&Answer::No, go));
    }

    #[test]
    fn a_failed_check_says_why_in_an_error_dialog() {
        let dialog = Dialog::check_failed(&UpdateError::Unreachable);
        assert_eq!(dialog.kind, Kind::Error);
        assert_eq!(dialog.choice, None);
        assert!(dialog.message.contains("could not reach GitHub"));
        let current = Dialog::up_to_date("0.1.0");
        assert_eq!(
            current.message,
            "Neon Teamspace 0.1.0 is the latest version."
        );
    }
}
