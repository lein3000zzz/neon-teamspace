use std::fmt;
use std::time::Duration;

use serde_json::Value as Json;

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
    /// Written the way people type it: `30s`, `1h 30m`, `7days`.
    Duration,
    /// A string that is never displayed, logged or audited.
    Secret,
}

impl Kind {
    /// Parses an environment variable, where every value arrives as text.
    pub(crate) fn parse_text(self, raw: &str) -> Result<Value, String> {
        match self {
            Self::Bool => match raw.trim().to_ascii_lowercase().as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(format!("{raw:?} is not true or false")),
            },
            Self::Integer => raw
                .trim()
                .parse()
                .map(Value::Integer)
                .map_err(|_| format!("{raw:?} is not a whole number")),
            Self::String => Ok(Value::String(raw.trim().to_owned())),
            Self::Enum(options) => parse_option(options, raw),
            Self::Duration => parse_duration(raw),
            Self::Secret => Ok(Value::Secret(Secret::new(raw))),
        }
    }

    /// Parses a JSON value, as the YAML file, the database and the API provide them.
    pub(crate) fn parse_json(self, raw: &Json) -> Result<Value, String> {
        match (self, raw) {
            (Self::Bool, Json::Bool(b)) => Ok(Value::Bool(*b)),
            (Self::Bool, _) => Err("want true or false".to_owned()),
            (Self::Integer, Json::Number(n)) => n
                .as_i64()
                .map(Value::Integer)
                .ok_or_else(|| format!("{n} is not a whole number")),
            (Self::Integer, _) => Err("want a whole number".to_owned()),
            (Self::String, Json::String(s)) => Ok(Value::String(s.trim().to_owned())),
            (Self::String, _) => Err("want text".to_owned()),
            (Self::Enum(options), Json::String(s)) => parse_option(options, s),
            (Self::Enum(options), _) => Err(format!("want one of {}", options.join(", "))),
            (Self::Duration, Json::String(s)) => parse_duration(s),
            (Self::Duration, _) => Err("want a duration like 30s or 1h 30m".to_owned()),
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
            Self::Secret => "secret",
        })
    }
}

fn parse_option(options: &[&str], raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if options.contains(&raw) {
        Ok(Value::String(raw.to_owned()))
    } else {
        Err(format!("{raw:?} is not one of {}", options.join(", ")))
    }
}

fn parse_duration(raw: &str) -> Result<Value, String> {
    humantime::parse_duration(raw.trim())
        .map(Value::Duration)
        .map_err(|_| format!("{raw:?} is not a duration like 30s or 1h 30m"))
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

/// A parsed setting value. Enum settings hold their option as a string.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    String(String),
    Duration(Duration),
    Secret(Secret),
}

impl Value {
    /// The JSON form kept in the database. It holds secrets in the clear.
    pub fn to_stored_json(&self) -> Json {
        match self {
            Self::Bool(b) => Json::Bool(*b),
            Self::Integer(n) => Json::from(*n),
            Self::String(s) => Json::String(s.clone()),
            Self::Duration(d) => Json::String(humantime::format_duration(*d).to_string()),
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
            Self::Duration(d) => write!(f, "{}", humantime::format_duration(*d)),
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
        assert_eq!(Kind::String.parse_text("  hi "), Ok(Value::String("hi".into())));
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
    fn text_errors_quote_the_value() {
        assert_eq!(
            Kind::Bool.parse_text("yes"),
            Err("\"yes\" is not true or false".into())
        );
        assert_eq!(
            Kind::Integer.parse_text("1.5"),
            Err("\"1.5\" is not a whole number".into())
        );
        assert_eq!(
            Kind::Enum(&["a", "b"]).parse_text("c"),
            Err("\"c\" is not one of a, b".into())
        );
        assert_eq!(
            Kind::Duration.parse_text("soon"),
            Err("\"soon\" is not a duration like 30s or 1h 30m".into())
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
    fn durations_round_trip_through_json() {
        let value = Value::Duration(Duration::from_secs(5400));
        assert_eq!(value.to_stored_json(), json!("1h 30m"));
        assert_eq!(Kind::Duration.parse_json(&value.to_stored_json()), Ok(value));
    }

    #[test]
    fn secrets_are_redacted_everywhere_but_storage() {
        let value = Value::Secret(Secret::new("hunter2"));

        assert_eq!(format!("{value}"), REDACTED);
        assert!(!format!("{value:?}").contains("hunter2"));
        assert_eq!(value.to_display_json(), json!(REDACTED));
        assert_eq!(value.to_stored_json(), json!("hunter2"));
        assert_eq!(Value::Secret(Secret::default()).to_display_json(), json!(""));
    }
}
