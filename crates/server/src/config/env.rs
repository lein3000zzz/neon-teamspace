//! `NEON_` environment variables as a configuration layer.

use figment::providers::Env;
use figment::value::{Dict, Map, Uncased, UncasedStr};
use figment::{Error, Metadata, Profile, Provider};
use neon_domain::settings::ENV_PREFIX as SETTINGS_PREFIX;

use super::CONFIG_ENV;

/// Starts every variable the server reads.
const PREFIX: &str = "NEON_";

/// Bootstrap variables, `__` separating levels: `NEON_DATABASE__URL` sets
/// `database.url`. Only the variables named here are read, so a `NEON_` variable that
/// belongs to another tool never reaches the configuration.
pub(super) struct EnvVars {
    names: Vec<String>,
}

impl EnvVars {
    pub(super) fn new(names: Vec<String>) -> Self {
        Self { names }
    }
}

impl Provider for EnvVars {
    fn metadata(&self) -> Metadata {
        Metadata::named("environment variable")
            .interpolater(|_: &Profile, keys: &[&str]| var_name(&keys.join(".")))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        Env::raw().only(&names).filter_map(bootstrap_key).data()
    }
}

/// The `NEON_` variables that may set a bootstrap key, by name. Blank ones count as
/// unset, so compose files can pass optional ones through. `NEON_CONFIG` names the file
/// instead, and `NEON_SETTINGS__*` are left to the settings registry, which parses them
/// by each setting's type.
pub(super) fn candidates() -> Vec<String> {
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let rest = strip_prefix(&name, PREFIX)?;
            let skip = rest.is_empty()
                || name.eq_ignore_ascii_case(CONFIG_ENV)
                || strip_prefix(&name, SETTINGS_PREFIX).is_some()
                || value.to_string_lossy().trim().is_empty();
            (!skip).then_some(name)
        })
        .collect();
    names.sort();
    names
}

/// The dotted key a bootstrap variable sets.
fn bootstrap_key(name: &UncasedStr) -> Option<Uncased<'_>> {
    let rest = strip_prefix(name.as_str(), PREFIX)?;
    Some(rest.replace("__", ".").into())
}

/// The `NEON_SETTINGS__*` variables, for the settings registry.
pub(super) fn settings_vars() -> impl Iterator<Item = (String, String)> {
    std::env::vars_os().filter_map(|(name, value)| {
        let name = name.into_string().ok()?;
        strip_prefix(&name, SETTINGS_PREFIX)?;
        Some((name.to_ascii_uppercase(), value.into_string().ok()?))
    })
}

/// The variable that sets a dotted key: `database.url` is `NEON_DATABASE__URL`.
pub(super) fn var_name(key: &str) -> String {
    format!("{PREFIX}{}", key.to_ascii_uppercase().replace('.', "__"))
}

/// Strips `prefix` ignoring case, as the variables are matched.
fn strip_prefix<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let head = name.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &name[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_names_follow_the_key() {
        assert_eq!(var_name("database.url"), "NEON_DATABASE__URL");
        assert_eq!(
            var_name("database.max_connections"),
            "NEON_DATABASE__MAX_CONNECTIONS"
        );
    }

    #[test]
    fn prefixes_match_ignoring_case() {
        assert_eq!(
            strip_prefix("neon_server__listen", PREFIX),
            Some("server__listen")
        );
        assert_eq!(strip_prefix("NEON", PREFIX), None);
        assert_eq!(strip_prefix("OTHER_X", PREFIX), None);
    }

    #[test]
    fn keys_nest_at_double_underscores() {
        assert_eq!(
            bootstrap_key(UncasedStr::new("NEON_DATABASE__MAX_CONNECTIONS")),
            Some(Uncased::from("DATABASE.MAX_CONNECTIONS"))
        );
    }
}
