use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value as Json;

use super::value::{Kind, Secret, Value};

/// Starts the environment variable of every setting: `instance.name` is read from
/// `NEON_SETTINGS__INSTANCE__NAME`.
pub const ENV_PREFIX: &str = "NEON_SETTINGS__";

/// Starts the YAML path of every setting: `instance.name` is `settings.instance.name`.
pub const YAML_PREFIX: &str = "settings.";

/// A typed handle to a setting. Definitions are built from keys, so a key always reads
/// back as the type it was defined with.
pub struct Key<T: ?Sized> {
    name: &'static str,
    _type: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized> Key<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _type: PhantomData,
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }
}

// Derives would require `T: Clone`, which `str` is not.
impl<T: ?Sized> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Key<T> {}

impl<T: ?Sized> fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({})", self.name)
    }
}

/// The Rust types settings read as.
pub trait SettingType {
    fn from_value(value: &Value) -> Option<&Self>;
}

impl SettingType for bool {
    fn from_value(value: &Value) -> Option<&Self> {
        match value {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl SettingType for i64 {
    fn from_value(value: &Value) -> Option<&Self> {
        match value {
            Value::Integer(n) => Some(n),
            _ => None,
        }
    }
}

impl SettingType for str {
    fn from_value(value: &Value) -> Option<&Self> {
        match value {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl SettingType for Duration {
    fn from_value(value: &Value) -> Option<&Self> {
        match value {
            Value::Duration(d) => Some(d),
            _ => None,
        }
    }
}

impl SettingType for Secret {
    fn from_value(value: &Value) -> Option<&Self> {
        match value {
            Value::Secret(s) => Some(s),
            _ => None,
        }
    }
}

type Check = Arc<dyn Fn(&Value) -> Result<(), String> + Send + Sync>;

/// One runtime setting: its key, type, default and description, plus checks a value must
/// pass beyond its type.
#[derive(Clone)]
pub struct Definition {
    key: &'static str,
    kind: Kind,
    default: Value,
    description: &'static str,
    checks: Vec<Check>,
    browser_only: bool,
}

impl Definition {
    pub fn bool(key: Key<bool>, default: bool, description: &'static str) -> Self {
        Self::new(key.name(), Kind::Bool, Value::Bool(default), description)
    }

    pub fn integer(key: Key<i64>, default: i64, description: &'static str) -> Self {
        Self::new(
            key.name(),
            Kind::Integer,
            Value::Integer(default),
            description,
        )
    }

    pub fn string(key: Key<str>, default: &str, description: &'static str) -> Self {
        Self::new(
            key.name(),
            Kind::String,
            Value::String(default.to_owned()),
            description,
        )
    }

    pub fn enumeration(
        key: Key<str>,
        options: &'static [&'static str],
        default: &'static str,
        description: &'static str,
    ) -> Self {
        Self::new(
            key.name(),
            Kind::Enum(options),
            Value::String(default.to_owned()),
            description,
        )
    }

    pub fn duration(key: Key<Duration>, default: Duration, description: &'static str) -> Self {
        Self::new(
            key.name(),
            Kind::Duration,
            Value::Duration(default),
            description,
        )
    }

    /// A secret starts empty: a default would be a secret in the source code.
    pub fn secret(key: Key<Secret>, description: &'static str) -> Self {
        Self::new(
            key.name(),
            Kind::Secret,
            Value::Secret(Secret::default()),
            description,
        )
    }

    fn new(key: &'static str, kind: Kind, default: Value, description: &'static str) -> Self {
        Self {
            key,
            kind,
            default,
            description,
            checks: Vec::new(),
            browser_only: false,
        }
    }

    /// Marks a setting that says where a secret goes, such as the mail server the mail
    /// password signs in to. Like a secret, it changes only from a browser session: an API
    /// token that leaked could otherwise point the secret, and every mailed link, at a
    /// server of its holder's choosing.
    pub fn browser_only(mut self) -> Self {
        self.browser_only = true;
        self
    }

    /// Adds a check on the parsed value, such as a range. Its error is shown to whoever
    /// typed the value, so it should say what is wanted.
    pub fn check<T, F>(mut self, check: F) -> Self
    where
        T: SettingType + ?Sized,
        F: Fn(&T) -> Result<(), String> + Send + Sync + 'static,
    {
        self.checks.push(Arc::new(move |value| {
            T::from_value(value).map_or(Ok(()), &check)
        }));
        self
    }

    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn default_value(&self) -> &Value {
        &self.default
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    /// Whether only a browser session may change it, never an API token: secrets, and the
    /// settings marked [`browser_only`](Self::browser_only).
    pub fn is_browser_only(&self) -> bool {
        self.browser_only || self.kind == Kind::Secret
    }

    /// The environment variable that sets this setting.
    pub fn env_var(&self) -> String {
        format!(
            "{ENV_PREFIX}{}",
            self.key.to_ascii_uppercase().replace('.', "__")
        )
    }

    /// The setting's path in the YAML file.
    pub fn yaml_path(&self) -> String {
        format!("{YAML_PREFIX}{}", self.key)
    }

    /// Parses and checks an environment variable's value.
    pub fn parse_text(&self, raw: &str) -> Result<Value, String> {
        self.validate(self.kind.parse_text(raw)?)
    }

    /// Parses and checks a JSON value from the YAML file, the database or the API.
    pub fn parse_json(&self, raw: &Json) -> Result<Value, String> {
        self.validate(self.kind.parse_json(raw)?)
    }

    pub(crate) fn validate(&self, value: Value) -> Result<Value, String> {
        for check in &self.checks {
            check(&value)?;
        }
        Ok(value)
    }
}

impl fmt::Debug for Definition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Definition")
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("default", &self.default)
            .field("checks", &self.checks.len())
            .field("browser_only", &self.browser_only)
            .finish_non_exhaustive()
    }
}

/// Reusable checks for [`Definition::check`].
pub mod checks {
    /// Rejects text that is empty or only whitespace.
    pub fn non_blank(value: &str) -> Result<(), String> {
        if value.trim().is_empty() {
            Err("must not be empty".to_owned())
        } else {
            Ok(())
        }
    }

    /// Rejects text longer than `max` characters.
    pub fn max_chars(max: usize) -> impl Fn(&str) -> Result<(), String> + Send + Sync {
        move |value| {
            if value.chars().count() > max {
                Err(format!("must be at most {max} characters"))
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NAME: Key<str> = Key::new("instance.name");
    const LIMIT: Key<i64> = Key::new("workspaces.limit");

    #[test]
    fn names_derive_from_the_key() {
        let def = Definition::string(NAME, "Neon", "The name.");

        assert_eq!(def.env_var(), "NEON_SETTINGS__INSTANCE__NAME");
        assert_eq!(def.yaml_path(), "settings.instance.name");
    }

    #[test]
    fn checks_run_after_parsing() {
        let def = Definition::string(NAME, "Neon", "The name.")
            .check(checks::non_blank)
            .check(checks::max_chars(5));

        assert_eq!(def.parse_text("Neon"), Ok(Value::String("Neon".into())));
        assert_eq!(def.parse_text("   "), Err("must not be empty".into()));
        assert_eq!(
            def.parse_json(&json!("Neon Workspace")),
            Err("must be at most 5 characters".into())
        );
    }

    #[test]
    fn typed_checks_see_their_type() {
        let def = Definition::integer(LIMIT, 10, "A limit.").check(|n: &i64| {
            if (1..=100).contains(n) {
                Ok(())
            } else {
                Err("must be between 1 and 100".to_owned())
            }
        });

        assert_eq!(def.parse_text("100"), Ok(Value::Integer(100)));
        assert_eq!(
            def.parse_json(&json!(0)),
            Err("must be between 1 and 100".into())
        );
    }

    #[test]
    fn secrets_default_to_empty() {
        let def = Definition::secret(Key::new("mail.password"), "The password.");

        assert_eq!(def.default_value(), &Value::Secret(Secret::default()));
        assert_eq!(def.kind(), Kind::Secret);
    }

    #[test]
    fn secrets_and_marked_settings_change_only_from_a_browser() {
        let secret = Definition::secret(Key::new("mail.password"), "The password.");
        let host = Definition::string(Key::new("mail.host"), "", "The host.").browser_only();
        let name = Definition::string(NAME, "Neon", "The name.");

        assert!(secret.is_browser_only());
        assert!(host.is_browser_only());
        assert!(!name.is_browser_only());
    }
}
