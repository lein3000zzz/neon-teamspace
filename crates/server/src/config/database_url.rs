use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use url::Url;

/// Replaces a password in a shown URL.
const REDACTED: &str = "redacted";

/// The Postgres URL. It usually holds a password, so `Debug`, `Display` and `Serialize`
/// all show it with the password replaced; [`expose`](Self::expose) is the one way to
/// read it whole.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct DatabaseUrl(String);

impl DatabaseUrl {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// The URL with its password, for connecting.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The URL with the password in its user part and in any `password` parameter
    /// replaced. Anything but a Postgres URL is hidden whole, since where a password
    /// would sit in it is unknown.
    pub fn redacted(&self) -> String {
        let Some(mut url) = Url::parse(&self.0).ok().filter(is_postgres) else {
            return format!("[{REDACTED}]");
        };
        if url.password().is_some() {
            // Fails only for URLs that cannot have a password, and this one has one.
            let _ = url.set_password(Some(REDACTED));
        }
        if url.query_pairs().any(|(name, _)| is_password(&name)) {
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .map(|(name, value)| {
                    let value = if is_password(&name) {
                        REDACTED.into()
                    } else {
                        value
                    };
                    (name.into_owned(), value.into_owned())
                })
                .collect();
            url.query_pairs_mut().clear().extend_pairs(pairs);
        }
        url.into()
    }

    /// Problems are described without the URL, which may hold a password.
    pub(super) fn check(&self) -> Result<(), String> {
        if self.0.trim().is_empty() {
            return Err("is required".to_owned());
        }
        let url = Url::parse(&self.0).map_err(|error| format!("is not a URL: {error}"))?;
        if !is_postgres(&url) {
            return Err(format!(
                "must start with postgres:// or postgresql://, not {}://",
                url.scheme()
            ));
        }
        Ok(())
    }
}

fn is_postgres(url: &Url) -> bool {
    matches!(url.scheme(), "postgres" | "postgresql")
}

/// Covers libpq's `password` and `sslpassword`.
fn is_password(name: &str) -> bool {
    name.to_ascii_lowercase().contains("password")
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DatabaseUrl({:?})", self.redacted())
    }
}

impl fmt::Display for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

/// Serializes redacted: the configuration is only ever serialized to be shown.
impl Serialize for DatabaseUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_redacted_wherever_the_url_is_shown() {
        let url = DatabaseUrl::new("postgres://neon:hunter2@db.internal:5432/neon");

        assert_eq!(
            url.redacted(),
            "postgres://neon:redacted@db.internal:5432/neon"
        );
        assert_eq!(url.to_string(), url.redacted());
        assert!(!format!("{url:?}").contains("hunter2"));
        assert_eq!(
            url.expose(),
            "postgres://neon:hunter2@db.internal:5432/neon"
        );
    }

    #[test]
    fn password_parameters_are_redacted() {
        let url = DatabaseUrl::new("postgres://db/neon?user=neon&password=hunter2&sslmode=require");

        assert_eq!(
            url.redacted(),
            "postgres://db/neon?user=neon&password=redacted&sslmode=require"
        );
    }

    #[test]
    fn urls_without_secrets_and_unparsable_ones() {
        assert_eq!(
            DatabaseUrl::new("postgres://localhost/neon").redacted(),
            "postgres://localhost/neon"
        );
        assert_eq!(DatabaseUrl::new("neon:hunter2@db").redacted(), "[redacted]");
        assert_eq!(DatabaseUrl::new("not a url").redacted(), "[redacted]");
    }

    #[test]
    fn checks_describe_problems_without_the_url() {
        assert_eq!(DatabaseUrl::default().check(), Err("is required".into()));
        assert_eq!(
            DatabaseUrl::new("mysql://root:hunter2@db/neon").check(),
            Err("must start with postgres:// or postgresql://, not mysql://".into())
        );
        let unparsable = DatabaseUrl::new("hunter2").check().expect_err("not a URL");
        assert!(!unparsable.contains("hunter2"), "{unparsable}");
        assert_eq!(
            DatabaseUrl::new("postgresql:///neon?host=/tmp").check(),
            Ok(())
        );
    }
}
