use std::fmt;
use std::time::Duration;

use serde_json::Value as Json;

use crate::duration;

/// Shown instead of a secret wherever a value is displayed, logged or audited.
pub const REDACTED: &str = "[redacted]";

/// The type of a setting, which decides how raw values parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Integer,
    String,
    /// A string limited to the listed options.
    Enum(&'static [&'static str]),
    /// Written the way people type it: `30s`, `1h 30m`, `7 days`.
    Duration,
    /// Some of the listed options, each at most once, kept in the listed order: a YAML list,
    /// or a comma-separated environment variable. Empty is none of them.
    Set(&'static [&'static str]),
    /// A string that is never displayed, logged or audited.
    Secret,
}

impl Kind {
    /// Parses an environment variable, where every value arrives as text.
    ///
    /// Errors here and in [`parse_json`](Self::parse_json) say what is wanted without
    /// repeating the value: it may be a secret, such as a database URL, set on the wrong
    /// variable, and these messages reach logs.
    pub(crate) fn parse_text(self, raw: &str) -> Result<Value, String> {
        match self {
            Self::Bool => match raw.trim().to_ascii_lowercase().as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(WANT_BOOL.to_owned()),
            },
            Self::Integer => raw
                .trim()
                .parse()
                .map(Value::Integer)
                .map_err(|_| WANT_INTEGER.to_owned()),
            Self::String => Ok(Value::String(raw.trim().to_owned())),
            Self::Enum(options) => parse_option(options, raw),
            Self::Duration => parse_duration(raw),
            Self::Set(options) => parse_set(
                options,
                raw.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty()),
            ),
            Self::Secret => Ok(Value::Secret(Secret::new(raw))),
        }
    }

    /// Parses a JSON value, as the YAML file, the database and the API provide them.
    pub(crate) fn parse_json(self, raw: &Json) -> Result<Value, String> {
        match (self, raw) {
            (Self::Bool, Json::Bool(b)) => Ok(Value::Bool(*b)),
            (Self::Bool, _) => Err(WANT_BOOL.to_owned()),
            (Self::Integer, json) => json
                .as_i64()
                .map(Value::Integer)
                .ok_or_else(|| WANT_INTEGER.to_owned()),
            (Self::String, Json::String(s)) => Ok(Value::String(s.trim().to_owned())),
            (Self::String, _) => Err("want text".to_owned()),
            (Self::Enum(options), Json::String(s)) => parse_option(options, s),
            (Self::Enum(options), _) => Err(want_option(options)),
            (Self::Duration, Json::String(s)) => parse_duration(s),
            (Self::Duration, _) => Err(WANT_DURATION.to_owned()),
            (Self::Set(options), Json::Array(items)) => {
                let texts = items
                    .iter()
                    .map(|item| item.as_str().map(str::trim))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| want_set(options))?;
                parse_set(options, texts.into_iter())
            }
            (Self::Set(options), _) => Err(want_set(options)),
            (Self::Secret, Json::String(s)) => Ok(Value::Secret(Secret::new(s.as_str()))),
            (Self::Secret, _) => Err("want text".to_owned()),
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bool => "bool",
            Self::Integer => "integer",
            Self::String => "string",
            Self::Enum(_) => "enum",
            Self::Duration => "duration",
            Self::Set(_) => "set",
            Self::Secret => "secret",
        })
    }
}

const WANT_BOOL: &str = "want true or false";
const WANT_INTEGER: &str = "want a whole number";
const WANT_DURATION: &str = "want a duration like 30s or 1h 30m";

fn want_option(options: &[&str]) -> String {
    format!("want one of {}", options.join(", "))
}

fn parse_option(options: &[&str], raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if options.contains(&raw) {
        Ok(Value::String(raw.to_owned()))
    } else {
        Err(want_option(options))
    }
}

fn want_set(options: &[&str]) -> String {
    format!("want a list of some of {}", options.join(", "))
}

/// The options named, each once, in the listed order. Naming one twice is a typo-like
/// slip worth saying, not a different value.
fn parse_set<'a>(options: &[&str], items: impl Iterator<Item = &'a str>) -> Result<Value, String> {
    let mut named = Vec::new();
    for item in items {
        if !options.contains(&item) {
            return Err(want_set(options));
        }
        if named.contains(&item) {
            return Err(format!("names {item} twice"));
        }
        named.push(item);
    }
    Ok(Value::List(
        options
            .iter()
            .filter(|option| named.contains(option))
            .map(|option| (*option).to_owned())
            .collect(),
    ))
}

fn parse_duration(raw: &str) -> Result<Value, String> {
    duration::parse(raw)
        .map(Value::Duration)
        .ok_or_else(|| WANT_DURATION.to_owned())
}

/// A secret setting value. `Debug` and `Display` print [`REDACTED`], so a secret cannot
/// reach a log line by accident; [`Secret::expose`] is the one way to read it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// A parsed setting value. Enum settings hold their option as a string, and set settings
/// their options as a list in the listed order.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    String(String),
    Duration(Duration),
    List(Vec<String>),
    Secret(Secret),
}

impl Value {
    /// The JSON form kept in the database. It holds secrets in the clear.
    pub fn to_stored_json(&self) -> Json {
        match self {
            Self::Bool(b) => Json::Bool(*b),
            Self::Integer(n) => Json::from(*n),
            Self::String(s) => Json::String(s.clone()),
            Self::Duration(d) => Json::String(duration::format(*d)),
            Self::List(items) => Json::Array(items.iter().cloned().map(Json::String).collect()),
            Self::Secret(s) => Json::String(s.expose().to_owned()),
        }
    }

    /// The JSON form for display and the audit table, with secrets replaced by [`REDACTED`].
    /// An empty secret stays empty, so a reader can tell whether one is set.
    pub fn to_display_json(&self) -> Json {
        match self {
            Self::Secret(s) if s.expose().is_empty() => Json::String(String::new()),
            Self::Secret(_) => Json::String(REDACTED.to_owned()),
            other => other.to_stored_json(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Integer(n) => write!(f, "{n}"),
            Self::String(s) => f.write_str(s),
            Self::Duration(d) => f.write_str(&duration::format(*d)),
            Self::List(items) => f.write_str(&items.join(", ")),
            Self::Secret(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn text_parses_every_kind() {
        assert_eq!(Kind::Bool.parse_text(" TRUE "), Ok(Value::Bool(true)));
        assert_eq!(Kind::Integer.parse_text("42"), Ok(Value::Integer(42)));
        assert_eq!(
            Kind::String.parse_text("  hi "),
            Ok(Value::String("hi".into()))
        );
        assert_eq!(
            Kind::Enum(&["a", "b"]).parse_text("b"),
            Ok(Value::String("b".into()))
        );
        assert_eq!(
            Kind::Duration.parse_text("1h 30m"),
            Ok(Value::Duration(Duration::from_secs(5400)))
        );
        assert_eq!(
            Kind::Secret.parse_text(" keep spaces "),
            Ok(Value::Secret(Secret::new(" keep spaces ")))
        );
    }

    #[test]
    fn errors_say_what_is_wanted_without_the_value() {
        let url = "postgres://neon:hunter2@db/neon";

        assert_eq!(Kind::Bool.parse_text(url), Err("want true or false".into()));
        assert_eq!(
            Kind::Integer.parse_text(url),
            Err("want a whole number".into())
        );
        assert_eq!(
            Kind::Enum(&["a", "b"]).parse_text(url),
            Err("want one of a, b".into())
        );
        assert_eq!(
            Kind::Duration.parse_text(url),
            Err("want a duration like 30s or 1h 30m".into())
        );
        assert_eq!(
            Kind::Duration.parse_json(&json!(url)),
            Err("want a duration like 30s or 1h 30m".into())
        );
        assert_eq!(
            Kind::Integer.parse_json(&json!(1.5)),
            Err("want a whole number".into())
        );
    }

    #[test]
    fn json_parses_matching_types_only() {
        assert_eq!(Kind::Bool.parse_json(&json!(false)), Ok(Value::Bool(false)));
        assert_eq!(Kind::Integer.parse_json(&json!(-3)), Ok(Value::Integer(-3)));
        assert_eq!(
            Kind::Duration.parse_json(&json!("90s")),
            Ok(Value::Duration(Duration::from_secs(90)))
        );

        assert!(Kind::Bool.parse_json(&json!("true")).is_err());
        assert!(Kind::Integer.parse_json(&json!(1.5)).is_err());
        assert!(Kind::Integer.parse_json(&json!("1")).is_err());
        assert!(Kind::String.parse_json(&json!(1)).is_err());
        assert!(Kind::Secret.parse_json(&json!(null)).is_err());
    }

    #[test]
    fn sets_keep_the_listed_order_and_name_each_option_once() {
        const MODES: Kind = Kind::Set(&["user", "agent"]);
        let list = |items: &[&str]| Value::List(items.iter().map(|i| (*i).to_owned()).collect());

        assert_eq!(
            MODES.parse_text("agent, user"),
            Ok(list(&["user", "agent"]))
        );
        assert_eq!(MODES.parse_text(" agent "), Ok(list(&["agent"])));
        assert_eq!(MODES.parse_text(""), Ok(list(&[])));
        assert_eq!(MODES.parse_json(&json!(["agent"])), Ok(list(&["agent"])));
        assert_eq!(MODES.parse_json(&json!([])), Ok(list(&[])));
        assert_eq!(
            MODES.parse_text("user,bots"),
            Err("want a list of some of user, agent".into())
        );
        assert_eq!(
            MODES.parse_json(&json!("user")),
            Err("want a list of some of user, agent".into())
        );
        assert_eq!(
            MODES.parse_json(&json!(["user", 1])),
            Err("want a list of some of user, agent".into())
        );
        assert_eq!(
            MODES.parse_json(&json!(["user", "user"])),
            Err("names user twice".into())
        );
        let value = list(&["user", "agent"]);
        assert_eq!(value.to_stored_json(), json!(["user", "agent"]));
        assert_eq!(MODES.parse_json(&value.to_stored_json()), Ok(value.clone()));
        assert_eq!(value.to_string(), "user, agent");
        assert_eq!(MODES.to_string(), "set");
    }

    #[test]
    fn durations_round_trip_through_json() {
        let value = Value::Duration(Duration::from_secs(5400));
        assert_eq!(value.to_stored_json(), json!("1h 30m"));
        assert_eq!(
            Kind::Duration.parse_json(&value.to_stored_json()),
            Ok(value)
        );
    }

    #[test]
    fn secrets_are_redacted_everywhere_but_storage() {
        let value = Value::Secret(Secret::new("hunter2"));

        assert_eq!(format!("{value}"), REDACTED);
        assert!(!format!("{value:?}").contains("hunter2"));
        assert_eq!(value.to_display_json(), json!(REDACTED));
        assert_eq!(value.to_stored_json(), json!("hunter2"));
        assert_eq!(
            Value::Secret(Secret::default()).to_display_json(),
            json!("")
        );
    }
}
