use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value as Json};

use super::definition::{ENV_PREFIX, Key, SettingType};
use super::value::Value;

/// Where a setting's value came from, highest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Env,
    Yaml,
    Database,
    Default,
}

impl Source {
    /// Values from the environment or the YAML file win over the database, so the admin
    /// page cannot change them.
    pub fn is_pinned(self) -> bool {
        matches!(self, Self::Env | Self::Yaml)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Yaml => "yaml",
            Self::Database => "database",
            Self::Default => "default",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A setting's value and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub value: Value,
    pub source: Source,
}

/// Every setting's resolved value at one moment. It never changes: a write makes a new
/// snapshot, so a reader holding one sees a consistent set.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub(crate) values: BTreeMap<&'static str, Resolved>,
}

impl Snapshot {
    /// Reads a setting as its type.
    ///
    /// # Panics
    ///
    /// When the key is not registered. Keys are constants defined next to the registry,
    /// and a test resolves every one of them, so this only fires on a programming error.
    pub fn get<T: SettingType + ?Sized>(&self, key: Key<T>) -> &T {
        self.values
            .get(key.name())
            .and_then(|resolved| T::from_value(&resolved.value))
            .unwrap_or_else(|| panic!("setting {} is not registered", key.name()))
    }

    /// The resolved value of a key, if it is registered.
    pub fn resolved(&self, key: &str) -> Option<&Resolved> {
        self.values.get(key)
    }

    /// Every setting in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Resolved)> {
        self.values.iter().map(|(key, resolved)| (*key, resolved))
    }
}

/// Something wrong with a setting, keyed by where to fix it: an environment variable, a
/// YAML path, or a setting key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub key: String,
    pub message: String,
}

impl Problem {
    pub fn new(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.message)
    }
}

/// Setting values from the environment and the YAML file, not yet parsed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawLayers {
    /// By environment variable name, such as `NEON_SETTINGS__INSTANCE__NAME`.
    pub env: BTreeMap<String, String>,
    /// By setting key, such as `instance.name`.
    pub yaml: BTreeMap<String, Json>,
}

impl RawLayers {
    /// Keeps the variables that start with [`ENV_PREFIX`]. An empty variable counts as
    /// unset, so compose files can pass optional variables through.
    pub fn with_env(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env = vars
            .into_iter()
            .filter(|(name, value)| name.starts_with(ENV_PREFIX) && !value.trim().is_empty())
            .collect();
        self
    }

    /// Flattens the `settings` tree of the YAML file into dotted keys. A key with no value
    /// after it counts as unset.
    pub fn with_yaml(mut self, tree: &Map<String, Json>) -> Self {
        fn walk(prefix: &str, tree: &Map<String, Json>, out: &mut BTreeMap<String, Json>) {
            for (name, value) in tree {
                let key = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                match value {
                    Json::Object(inner) => walk(&key, inner, out),
                    Json::Null => {}
                    leaf => {
                        out.insert(key, leaf.clone());
                    }
                }
            }
        }

        self.yaml.clear();
        walk("", tree, &mut self.yaml);
        self
    }
}

/// Parsed values from the environment and the YAML file. They are read once at start and
/// pin their settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pinned {
    pub(crate) env: BTreeMap<&'static str, Value>,
    pub(crate) yaml: BTreeMap<&'static str, Value>,
}

/// Values saved through the admin page, by key.
pub type Stored = BTreeMap<&'static str, Value>;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn env_keeps_prefixed_non_empty_variables() {
        let raw = RawLayers::default().with_env([
            ("NEON_SETTINGS__INSTANCE__NAME".into(), "Neon".into()),
            ("NEON_SETTINGS__MAIL__HOST".into(), "  ".into()),
            ("NEON_DATABASE__URL".into(), "postgres://".into()),
            ("HOME".into(), "/home/neon".into()),
        ]);

        assert_eq!(
            raw.env.into_iter().collect::<Vec<_>>(),
            [("NEON_SETTINGS__INSTANCE__NAME".to_owned(), "Neon".to_owned())]
        );
    }

    #[test]
    fn yaml_flattens_nested_keys_and_skips_nulls() {
        let tree = json!({
            "instance": { "name": "Neon", "tagline": null },
            "auth": { "session": { "idle": "7days" } },
        });
        let Json::Object(tree) = tree else {
            unreachable!()
        };

        let raw = RawLayers::default().with_yaml(&tree);

        assert_eq!(
            raw.yaml,
            BTreeMap::from([
                ("auth.session.idle".to_owned(), json!("7days")),
                ("instance.name".to_owned(), json!("Neon")),
            ])
        );
    }
}
