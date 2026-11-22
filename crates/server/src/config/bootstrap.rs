//! The bootstrap keys for accounts and secrets: the first admin, the keys that seal
//! secrets in the database, and the proxies whose `X-Forwarded-For` is believed.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr as _;

use ipnet::IpNet;
use neon_app::crypto::{KeyError, Keyring};
use serde::de::{self, Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::database_url::REDACTED;

/// Text that is never shown: `Debug` and `Serialize` print `[redacted]` in its place, and
/// [`expose`](Self::expose) is the one way to read it.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{REDACTED}]")
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("[{REDACTED}]"))
    }
}

/// The first admin, made as the server starts while no admin can sign in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    pub email: Option<String>,
    pub password: Option<SecretString>,
}

/// Secrets kept in the database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecretsConfig {
    /// `id:base64key` entries, the current key first, written as a list or as one string
    /// separated by commas.
    #[serde(deserialize_with = "key_list")]
    pub keys: Vec<SecretString>,
}

impl SecretsConfig {
    /// The keys, or `None` when there are none.
    pub fn keyring(&self) -> Result<Option<Keyring>, KeyError> {
        let entries: Vec<&str> = self.keys.iter().map(SecretString::expose).collect();
        Keyring::parse(&entries)
    }
}

/// Shows each key's id, which is no secret, and hides its material. An entry without an
/// id shows nothing of it.
impl Serialize for SecretsConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Shown {
            keys: Vec<String>,
        }
        let keys = self
            .keys
            .iter()
            .map(|entry| match entry.expose().split_once(':') {
                Some((id, _)) => format!("{}:[{REDACTED}]", id.trim()),
                None => format!("[{REDACTED}]"),
            })
            .collect();
        Shown { keys }.serialize(serializer)
    }
}

fn key_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<SecretString>, D::Error> {
    Ok(string_list(deserializer)?
        .into_iter()
        .map(SecretString::new)
        .collect())
}

/// Address ranges whose `X-Forwarded-For` the server believes unless told otherwise:
/// loopback and the private networks a reverse proxy in front usually sits on.
pub fn default_trusted_proxies() -> Vec<IpNet> {
    [
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "::1/128",
        "fc00::/7",
    ]
    .into_iter()
    .filter_map(|range| range.parse().ok())
    .collect()
}

/// Ranges written like `10.0.0.0/8`, or single addresses, as a list or one string
/// separated by commas.
pub(super) fn proxy_list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<IpNet>, D::Error> {
    string_list(deserializer)?
        .iter()
        .map(|text| {
            IpNet::from_str(text)
                .or_else(|_| IpAddr::from_str(text).map(IpNet::from))
                .map_err(|_| D::Error::custom("want address ranges like 10.0.0.0/8, or addresses"))
        })
        .collect()
}

/// A list of strings, or one string whose entries are separated by commas, as an
/// environment variable can only give. Blank entries are skipped.
fn string_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    struct List;

    impl<'de> Visitor<'de> for List {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list of strings, or a string of entries separated by commas")
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
            Ok(text
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element::<String>()? {
                let entry = entry.trim();
                if !entry.is_empty() {
                    entries.push(entry.to_owned());
                }
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_any(List)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_are_read_from_a_list_or_a_comma_separated_string() {
        let listed: SecretsConfig =
            serde_json::from_value(json!({"keys": ["a:x", " b:y "]})).expect("a list");
        let joined: SecretsConfig =
            serde_json::from_value(json!({"keys": "a:x, b:y,"})).expect("a string");

        assert_eq!(listed, joined);
        assert_eq!(
            listed.keys,
            [SecretString::new("a:x"), SecretString::new("b:y")]
        );
    }

    #[test]
    fn keys_are_shown_by_id_alone() {
        let secrets = SecretsConfig {
            keys: vec![
                SecretString::new("2026:c2VjcmV0"),
                SecretString::new("no-colon"),
            ],
        };

        let shown = serde_json::to_value(&secrets).expect("serializes");

        assert_eq!(shown, json!({"keys": ["2026:[redacted]", "[redacted]"]}));
        assert!(!format!("{secrets:?}").contains("c2VjcmV0"));
    }

    #[test]
    fn proxies_are_ranges_or_addresses() {
        #[derive(Deserialize)]
        struct Proxies {
            #[serde(deserialize_with = "proxy_list")]
            list: Vec<IpNet>,
        }

        let parsed: Proxies =
            serde_json::from_value(json!({"list": "10.0.0.0/8, 192.0.2.1, ::1"})).expect("valid");
        assert_eq!(
            parsed
                .list
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["10.0.0.0/8", "192.0.2.1/32", "::1/128"]
        );
        assert!(serde_json::from_value::<Proxies>(json!({"list": ["nope"]})).is_err());
        assert_eq!(default_trusted_proxies().len(), 6);
    }

    #[test]
    fn passwords_never_show() {
        let admin = AdminConfig {
            email: Some("root@example.com".into()),
            password: Some(SecretString::new("hunter2hunter2")),
        };

        assert!(!format!("{admin:?}").contains("hunter2"));
        let shown = serde_json::to_string(&admin).expect("serializes");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("root@example.com"), "{shown}");
    }
}
