//! The saved servers, kept as `servers.json` in the app's data directory.
//!
//! Each server is its origin, the name someone gave it, and what its last check said: the
//! instance's name and the server's version. Nothing secret is kept: signing in is the server's
//! own pages, whose cookies the webview keeps. The file is replaced whole on each change, by
//! writing a new one beside it and renaming it over the old, so a crash leaves one or the
//! other. A file that cannot be read is moved aside to `servers.json.unreadable` and the list
//! starts empty; an entry whose origin is not one the app would take is dropped.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::address::ServerAddress;
use crate::meta::ServerInfo;

/// The file's name in the app's data directory.
pub const FILE_NAME: &str = "servers.json";

/// The version of the file's format.
const FORMAT: u32 = 1;

/// The longest name someone can give a server, in characters.
pub const MAX_LABEL_CHARS: usize = 80;

/// The most servers kept.
pub const MAX_SERVERS: usize = 50;

/// One saved server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedServer {
    /// Its origin, such as `https://neon.example.com`: the key of the list.
    pub origin: String,
    /// The name someone gave it, shown in place of the instance's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The instance's name, as the last check read it.
    pub name: String,
    /// The server's version, as the last check read it.
    pub version: String,
}

/// The list, in the order the servers were added, and the one used last.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerList {
    pub servers: Vec<SavedServer>,
    /// The origin of the server opened last, which the app opens as it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

/// What a change to the list was refused for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ListError {
    #[error("That server is not in the list.")]
    Unknown,
    #[error("A server's name can be at most {MAX_LABEL_CHARS} characters, with no line breaks.")]
    Label,
    #[error("The list holds at most {MAX_SERVERS} servers: remove one first.")]
    Full,
}

impl ListError {
    /// A stable name for the picker and the tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown_server",
            Self::Label => "label",
            Self::Full => "list_full",
        }
    }
}

impl ServerList {
    /// The server with this origin.
    pub fn get(&self, origin: &str) -> Option<&SavedServer> {
        self.servers.iter().find(|server| server.origin == origin)
    }

    /// Adds the server, or refreshes what its check said when it is in the list already.
    pub fn upsert(
        &mut self,
        address: &ServerAddress,
        info: &ServerInfo,
    ) -> Result<&SavedServer, ListError> {
        let origin = address.origin();
        let at = match self
            .servers
            .iter()
            .position(|server| server.origin == origin)
        {
            Some(at) => {
                let server = &mut self.servers[at];
                server.name.clone_from(&info.name);
                server.version.clone_from(&info.version);
                at
            }
            None if self.servers.len() >= MAX_SERVERS => return Err(ListError::Full),
            None => {
                self.servers.push(SavedServer {
                    origin,
                    label: None,
                    name: info.name.clone(),
                    version: info.version.clone(),
                });
                self.servers.len() - 1
            }
        };
        Ok(&self.servers[at])
    }

    /// Names the server; a blank name goes back to the instance's own.
    pub fn rename(&mut self, origin: &str, label: &str) -> Result<&SavedServer, ListError> {
        let label = label.trim();
        if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
            return Err(ListError::Label);
        }
        let server = self
            .servers
            .iter_mut()
            .find(|server| server.origin == origin)
            .ok_or(ListError::Unknown)?;
        server.label = (!label.is_empty()).then(|| label.to_owned());
        Ok(server)
    }

    /// Forgets the server, and that it was used last.
    pub fn remove(&mut self, origin: &str) -> Result<(), ListError> {
        let before = self.servers.len();
        self.servers.retain(|server| server.origin != origin);
        if self.servers.len() == before {
            return Err(ListError::Unknown);
        }
        if self.last.as_deref() == Some(origin) {
            self.last = None;
        }
        Ok(())
    }

    /// Remembers the server as the one used last.
    pub fn mark_used(&mut self, origin: &str) -> Result<(), ListError> {
        if self.get(origin).is_none() {
            return Err(ListError::Unknown);
        }
        self.last = Some(origin.to_owned());
        Ok(())
    }

    /// Drops what the app would not have written: an origin it would not take, or one twice,
    /// a name too long, and a last server that is not in the list.
    fn sanitise(mut self) -> Self {
        let mut seen = Vec::new();
        self.servers.retain(|server| {
            let canonical = ServerAddress::parse(&server.origin)
                .is_ok_and(|address| address.origin() == server.origin);
            let fresh = !seen.contains(&server.origin);
            seen.push(server.origin.clone());
            canonical && fresh
        });
        self.servers.truncate(MAX_SERVERS);
        for server in &mut self.servers {
            if server
                .label
                .as_deref()
                .is_some_and(|label| label.chars().count() > MAX_LABEL_CHARS || label.is_empty())
            {
                server.label = None;
            }
        }
        if let Some(last) = &self.last
            && self.get(last).is_none()
        {
            self.last = None;
        }
        self
    }
}

/// The file as written: the list and its format's version.
#[derive(Serialize, Deserialize)]
struct File {
    format: u32,
    #[serde(flatten)]
    list: ServerList,
}

/// Where the list is kept.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// The list in `dir`, the app's data directory.
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the list: empty when there is no file yet, or when it cannot be read, which is
    /// then moved aside.
    pub fn load(&self) -> ServerList {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return ServerList::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", self.path.display());
                return ServerList::default();
            }
        };
        match serde_json::from_slice::<File>(&bytes) {
            Ok(file) if file.format == FORMAT => file.list.sanitise(),
            Ok(file) => {
                log::warn!(
                    "{} has format {}, not {FORMAT}: starting with no servers",
                    self.path.display(),
                    file.format
                );
                self.move_aside();
                ServerList::default()
            }
            Err(error) => {
                log::warn!(
                    "{} is unreadable ({error}): starting with no servers",
                    self.path.display()
                );
                self.move_aside();
                ServerList::default()
            }
        }
    }

    /// Writes the list, replacing the file whole.
    pub fn save(&self, list: &ServerList) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = File {
            format: FORMAT,
            list: list.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let temporary = self.path.with_extension("json.new");
        {
            let mut out = fs::File::create(&temporary)?;
            out.write_all(&bytes)?;
            out.sync_all()?;
        }
        fs::rename(&temporary, &self.path)
    }

    fn move_aside(&self) {
        let aside = self.path.with_extension("json.unreadable");
        if let Err(error) = fs::rename(&self.path, &aside) {
            log::warn!("could not move {} aside: {error}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(origin: &str) -> ServerAddress {
        ServerAddress::parse(origin).unwrap()
    }

    fn info(name: &str, version: &str) -> ServerInfo {
        ServerInfo {
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    fn list_of(origins: &[&str]) -> ServerList {
        let mut list = ServerList::default();
        for origin in origins {
            list.upsert(&address(origin), &info("Acme", "0.1.0"))
                .unwrap();
        }
        list
    }

    #[test]
    fn servers_are_added_in_order_and_refreshed_in_place() {
        let mut list = list_of(&["https://a.example", "https://b.example"]);
        list.rename("https://a.example", "Work").unwrap();
        let refreshed = list
            .upsert(&address("https://A.example/x"), &info("Acme Corp", "0.2.0"))
            .unwrap()
            .clone();
        assert_eq!(
            refreshed,
            SavedServer {
                origin: "https://a.example".to_owned(),
                label: Some("Work".to_owned()),
                name: "Acme Corp".to_owned(),
                version: "0.2.0".to_owned(),
            }
        );
        let origins: Vec<&str> = list.servers.iter().map(|s| s.origin.as_str()).collect();
        assert_eq!(origins, ["https://a.example", "https://b.example"]);
    }

    #[test]
    fn the_list_has_a_limit() {
        let mut list = ServerList::default();
        for n in 0..MAX_SERVERS {
            list.upsert(
                &address(&format!("https://s{n}.example")),
                &info("A", "0.1.0"),
            )
            .unwrap();
        }
        assert_eq!(
            list.upsert(&address("https://more.example"), &info("A", "0.1.0")),
            Err(ListError::Full)
        );
        // One already there is still refreshed.
        assert!(
            list.upsert(&address("https://s0.example"), &info("B", "0.2.0"))
                .is_ok()
        );
    }

    #[test]
    fn renaming_trims_and_a_blank_name_goes_back_to_the_instances() {
        let mut list = list_of(&["https://a.example"]);
        assert_eq!(
            list.rename("https://a.example", "  Work  ")
                .unwrap()
                .label
                .as_deref(),
            Some("Work")
        );
        assert_eq!(list.rename("https://a.example", "   ").unwrap().label, None);
        let longest = "n".repeat(MAX_LABEL_CHARS);
        assert!(list.rename("https://a.example", &longest).is_ok());
        assert_eq!(
            list.rename("https://a.example", &format!("{longest}n")),
            Err(ListError::Label)
        );
        assert_eq!(
            list.rename("https://a.example", "two\nlines"),
            Err(ListError::Label)
        );
        assert_eq!(
            list.rename("https://b.example", "Work"),
            Err(ListError::Unknown)
        );
    }

    #[test]
    fn removing_forgets_the_server_and_that_it_was_last() {
        let mut list = list_of(&["https://a.example", "https://b.example"]);
        list.mark_used("https://a.example").unwrap();
        list.remove("https://b.example").unwrap();
        assert_eq!(list.last.as_deref(), Some("https://a.example"));
        list.remove("https://a.example").unwrap();
        assert_eq!(list.last, None);
        assert!(list.servers.is_empty());
        assert_eq!(list.remove("https://a.example"), Err(ListError::Unknown));
    }

    #[test]
    fn only_a_listed_server_is_marked_used() {
        let mut list = list_of(&["https://a.example"]);
        assert_eq!(list.mark_used("https://b.example"), Err(ListError::Unknown));
        list.mark_used("https://a.example").unwrap();
        assert_eq!(list.last.as_deref(), Some("https://a.example"));
    }

    #[test]
    fn the_list_survives_a_save_and_a_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("nested"));
        assert_eq!(store.load(), ServerList::default());

        let mut list = list_of(&["https://a.example", "http://192.168.1.20:3000"]);
        list.rename("https://a.example", "Work").unwrap();
        list.mark_used("http://192.168.1.20:3000").unwrap();
        store.save(&list).unwrap();
        assert_eq!(store.load(), list);
        assert!(!store.path().with_extension("json.new").exists());

        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "format": 1,
                "servers": [
                    {"origin": "https://a.example", "label": "Work", "name": "Acme", "version": "0.1.0"},
                    {"origin": "http://192.168.1.20:3000", "name": "Acme", "version": "0.1.0"},
                ],
                "last": "http://192.168.1.20:3000",
            })
        );
    }

    #[test]
    fn an_unreadable_file_is_moved_aside() {
        for content in [
            "{not json",
            r#"{"format": 2, "servers": []}"#,
            r#"{"servers": 1}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::new(dir.path());
            fs::write(store.path(), content).unwrap();
            assert_eq!(store.load(), ServerList::default(), "{content}");
            assert!(!store.path().exists(), "{content}");
            assert_eq!(
                fs::read_to_string(store.path().with_extension("json.unreadable")).unwrap(),
                content
            );
        }
    }

    #[test]
    fn entries_the_app_would_not_write_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let long = "n".repeat(MAX_LABEL_CHARS + 1);
        let file = serde_json::json!({
            "format": 1,
            "servers": [
                {"origin": "https://a.example", "label": long, "name": "A", "version": "0.1.0"},
                {"origin": "https://a.example", "name": "Twice", "version": "0.1.0"},
                {"origin": "http://public.example", "name": "Plain", "version": "0.1.0"},
                {"origin": "https://B.example/path", "name": "Not canonical", "version": "0.1.0"},
                {"origin": "javascript:alert(1)", "name": "Script", "version": "0.1.0"},
                {"origin": "https://c.example", "label": "", "name": "C", "version": "0.1.0"},
            ],
            "last": "https://gone.example",
        });
        fs::write(store.path(), file.to_string()).unwrap();
        let list = store.load();
        assert_eq!(
            list,
            ServerList {
                servers: vec![
                    SavedServer {
                        origin: "https://a.example".to_owned(),
                        label: None,
                        name: "A".to_owned(),
                        version: "0.1.0".to_owned(),
                    },
                    SavedServer {
                        origin: "https://c.example".to_owned(),
                        label: None,
                        name: "C".to_owned(),
                        version: "0.1.0".to_owned(),
                    },
                ],
                last: None,
            }
        );
    }
}
