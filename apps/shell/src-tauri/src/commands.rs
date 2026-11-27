//! What the bundled picker asks of the app. Only the picker's capability grants these: the
//! server's pages, on another origin, can call none of them.

use serde::Serialize;
use tauri::{AppHandle, Runtime, State};

use crate::address::{AddressError, ServerAddress};
use crate::lock::Attempt;
use crate::meta::{self, CheckError, ServerInfo};
use crate::servers::{ListError, SavedServer, ServerList};
use crate::shell::{self, Interrupted, Shell};

/// A failure the picker shows: a stable code and a sentence for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl From<AddressError> for Failure {
    fn from(error: AddressError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl From<CheckError> for Failure {
    fn from(error: CheckError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl From<ListError> for Failure {
    fn from(error: ListError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl From<tauri::Error> for Failure {
    fn from(error: tauri::Error) -> Self {
        log::error!("the window could not load the page: {error}");
        Self {
            code: "window",
            message: format!("The app could not load the page: {error}."),
        }
    }
}

fn storage(error: &std::io::Error) -> Failure {
    log::error!("could not save the server list: {error}");
    Failure {
        code: "storage",
        message: format!("The app could not save its list of servers: {error}."),
    }
}

/// A saved server as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub saved: SavedServer,
    /// Whether it is reached over https; the picker marks the others.
    pub encrypted: bool,
}

/// The list as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListView {
    pub servers: Vec<ServerView>,
    pub last: Option<String>,
}

impl From<&ServerList> for ListView {
    fn from(list: &ServerList) -> Self {
        Self {
            servers: list
                .servers
                .iter()
                .map(|saved| ServerView {
                    encrypted: saved.origin.starts_with("https://"),
                    saved: saved.clone(),
                })
                .collect(),
            last: list.last.clone(),
        }
    }
}

/// What the picker shows as it loads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Startup {
    #[serde(flatten)]
    pub list: ListView,
    /// The server to open at once: the one used last, the first time the picker loads only.
    pub open: Option<String>,
    /// Why the window came back to the picker from a server it was opening, if it did.
    pub interrupted: Option<Interrupted>,
}

/// Refuses a command unless the page last committed in the window is the app's own: a
/// server's page that is still there while the picker loads may not pass as the picker,
/// whatever URL its messages carry.
fn from_the_picker(shell: &Shell, command: &str) -> Result<(), Failure> {
    if shell.lock.on_app_page() {
        Ok(())
    } else {
        log::warn!("refused {command}, as the window shows none of the app's own pages");
        Err(Failure {
            code: "not_the_picker",
            message: "Only the server picker can do this.".to_owned(),
        })
    }
}

/// Checks the server, and logs what the check found, with the technical cause of a failure,
/// which the picker's message leaves out.
async fn check(address: &ServerAddress, shell: &Shell) -> Result<ServerInfo, Failure> {
    match meta::check(address, shell.limits.check).await {
        Ok(info) => {
            log::info!("{address} is Neon Teamspace {}", info.version);
            Ok(info)
        }
        Err(error) => {
            log::warn!("{}", check_failure(address, &error));
            Err(error.into())
        }
    }
}

/// The failure of an attempt to open `address` that was cancelled, or that a newer one
/// replaced: the picker that made it has moved on.
fn cancelled(address: &ServerAddress) -> Failure {
    Failure {
        code: "cancelled",
        message: format!("Stopped opening {address}."),
    }
}

/// Fails once `attempt` is no longer the latest attempt to open a server.
fn still_opening(shell: &Shell, attempt: Attempt, address: &ServerAddress) -> Result<(), Failure> {
    if shell.lock.still_opening(attempt) {
        Ok(())
    } else {
        Err(cancelled(address))
    }
}

/// The log line of a failed check.
fn check_failure(address: &ServerAddress, error: &CheckError) -> String {
    let line = format!("{address} failed its check ({}): {error}", error.code());
    match error.detail() {
        Some(detail) => format!("{line} The cause: {detail}"),
        None => line,
    }
}

#[tauri::command]
pub fn startup(shell: State<'_, Shell>) -> Result<Startup, Failure> {
    from_the_picker(&shell, "startup")?;
    let open = shell.take_launch();
    Ok(Startup {
        list: ListView::from(&*shell.list()),
        open,
        interrupted: shell.take_interrupted(),
    })
}

/// Checks a new address, saves it and opens it. An `http://` address is refused with the code
/// `insecure`, and the warning as its message, until the picker asks again with
/// `accept_insecure`, once the person has read it. Once [`cancel_opening`] stopped it, or
/// another attempt began, it saves and opens nothing, and fails with `cancelled`.
#[tauri::command]
pub async fn connect<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
    address: String,
    accept_insecure: bool,
) -> Result<ListView, Failure> {
    from_the_picker(&shell, "connect")?;
    let address = ServerAddress::parse(&address)?;
    if !address.is_encrypted() && !accept_insecure {
        return Err(Failure {
            code: "insecure",
            message: format!(
                "{address} is reached over plain http://, so anyone on the network between you \
                 and it can read what you send, your password included, and change what you see. \
                 Use it only on this device or a network you trust."
            ),
        });
    }
    let attempt = shell.lock.begin_opening();
    let info = check(&address, &shell).await;
    still_opening(&shell, attempt, &address)?;
    let info = info?;
    let view = {
        let mut list = shell.list();
        let origin = list.upsert(&address, &info)?.origin.clone();
        list.mark_used(&origin)?;
        shell.store.save(&list).map_err(|error| storage(&error))?;
        ListView::from(&*list)
    };
    if !shell::open_server(&app, &address, attempt)? {
        return Err(cancelled(&address));
    }
    Ok(view)
}

/// Checks a saved server again, which may have moved or changed, and opens it; or, as
/// [`connect`], nothing once cancelled.
#[tauri::command]
pub async fn open_server<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
    origin: String,
) -> Result<ListView, Failure> {
    from_the_picker(&shell, "open_server")?;
    if shell.list().get(&origin).is_none() {
        return Err(ListError::Unknown.into());
    }
    let address = ServerAddress::parse(&origin)?;
    let attempt = shell.lock.begin_opening();
    let info = check(&address, &shell).await;
    still_opening(&shell, attempt, &address)?;
    let info = info?;
    let view = {
        let mut list = shell.list();
        list.upsert(&address, &info)?;
        list.mark_used(&origin)?;
        shell.store.save(&list).map_err(|error| storage(&error))?;
        ListView::from(&*list)
    };
    if !shell::open_server(&app, &address, attempt)? {
        return Err(cancelled(&address));
    }
    Ok(view)
}

/// Stops opening a server: its check, whose command then fails with `cancelled`, or its page
/// loading, which showing the picker again stops. True when the window goes back to the
/// picker, which says why as it loads again; false when the picker stays as it is.
#[tauri::command]
pub fn cancel_opening<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
) -> Result<bool, Failure> {
    from_the_picker(&shell, "cancel_opening")?;
    Ok(shell::cancel_opening(&app)?)
}

#[tauri::command]
pub fn rename_server(
    shell: State<'_, Shell>,
    origin: String,
    label: String,
) -> Result<ListView, Failure> {
    from_the_picker(&shell, "rename_server")?;
    let mut list = shell.list();
    list.rename(&origin, &label)?;
    shell.store.save(&list).map_err(|error| storage(&error))?;
    Ok(ListView::from(&*list))
}

/// Forgets a server, and loads none of its pages in the app any more.
#[tauri::command]
pub fn remove_server<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
    origin: String,
) -> Result<ListView, Failure> {
    from_the_picker(&shell, "remove_server")?;
    let mut list = shell.list();
    list.remove(&origin)?;
    shell.store.save(&list).map_err(|error| storage(&error))?;
    if let Ok(address) = ServerAddress::parse(&origin) {
        shell::forget_server(&app, &address);
    }
    Ok(ListView::from(&*list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_line_of_a_failed_check_keeps_its_technical_cause() {
        let address = ServerAddress::parse("https://10.0.2.2:59992").unwrap();
        let error = CheckError::Unreachable {
            origin: address.origin(),
            reason: "it answered without encryption, as a server on plain http does".to_owned(),
            hint: String::new(),
            detail: "received corrupt message of type InvalidContentType".to_owned(),
        };
        assert_eq!(
            check_failure(&address, &error),
            "https://10.0.2.2:59992 failed its check (unreachable): Could not reach \
             https://10.0.2.2:59992: it answered without encryption, as a server on plain http \
             does. The cause: received corrupt message of type InvalidContentType"
        );
        assert!(!Failure::from(error).message.contains("InvalidContentType"));
    }
}
