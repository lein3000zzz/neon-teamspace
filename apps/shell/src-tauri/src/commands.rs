//! What the bundled picker asks of the app. Only the picker's capability grants these: the
//! server's pages, on another origin, can call none of them.

use serde::Serialize;
use tauri::{AppHandle, Runtime, State};

use crate::address::{AddressError, ServerAddress};
use crate::meta::{self, CheckError};
use crate::servers::{ListError, SavedServer, ServerList};
use crate::shell::{self, Shell};

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
}

#[tauri::command]
pub fn startup(shell: State<'_, Shell>) -> Startup {
    let open = shell.take_launch();
    Startup {
        list: ListView::from(&*shell.list()),
        open,
    }
}

/// Checks a new address, saves it and opens it. An `http://` address is refused with the code
/// `insecure`, and the warning as its message, until the picker asks again with
/// `accept_insecure`, once the person has read it.
#[tauri::command]
pub async fn connect<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
    address: String,
    accept_insecure: bool,
) -> Result<ListView, Failure> {
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
    let info = meta::check(&address, shell.options).await?;
    log::info!("{address} is Neon Teamspace {}", info.version);
    let view = {
        let mut list = shell.list();
        let origin = list.upsert(&address, &info)?.origin.clone();
        list.mark_used(&origin)?;
        shell.store.save(&list).map_err(|error| storage(&error))?;
        ListView::from(&*list)
    };
    shell::open_server(&app, &address)?;
    Ok(view)
}

/// Checks a saved server again, which may have moved or changed, and opens it.
#[tauri::command]
pub async fn open_server<R: Runtime>(
    app: AppHandle<R>,
    shell: State<'_, Shell>,
    origin: String,
) -> Result<ListView, Failure> {
    if shell.list().get(&origin).is_none() {
        return Err(ListError::Unknown.into());
    }
    let address = ServerAddress::parse(&origin)?;
    let info = meta::check(&address, shell.options).await?;
    log::info!("{address} is Neon Teamspace {}", info.version);
    let view = {
        let mut list = shell.list();
        list.upsert(&address, &info)?;
        list.mark_used(&origin)?;
        shell.store.save(&list).map_err(|error| storage(&error))?;
        ListView::from(&*list)
    };
    shell::open_server(&app, &address)?;
    Ok(view)
}

#[tauri::command]
pub fn rename_server(
    shell: State<'_, Shell>,
    origin: String,
    label: String,
) -> Result<ListView, Failure> {
    let mut list = shell.list();
    list.rename(&origin, &label)?;
    shell.store.save(&list).map_err(|error| storage(&error))?;
    Ok(ListView::from(&*list))
}

/// Forgets a server, and loads none of its pages in the app any more.
#[tauri::command]
pub fn remove_server(shell: State<'_, Shell>, origin: String) -> Result<ListView, Failure> {
    let mut list = shell.list();
    list.remove(&origin)?;
    shell.store.save(&list).map_err(|error| storage(&error))?;
    if let Ok(address) = ServerAddress::parse(&origin) {
        shell.lock.forget(&address);
    }
    Ok(ListView::from(&*list))
}
