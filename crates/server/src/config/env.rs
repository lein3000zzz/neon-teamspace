//! `NEON_` environment variables as a configuration layer.

use figment::providers::Env;
use figment::value::{Dict, Map, Uncased, UncasedStr};
use figment::{Error, Metadata, Profile, Provider};
use neon_domain::settings::ENV_PREFIX as SETTINGS_PREFIX;

/// Starts every variable the server reads.
const PREFIX: &str = "NEON_";

/// The bootstrap variables, `__` separating levels: `NEON_DATABASE__URL` sets
/// `database.url`. Blank variables count as unset, so compose files can pass optional
/// ones through. `NEON_CONFIG` names the file instead, and `NEON_SETTINGS__*` are left to
/// the settings registry, which parses them by each setting's type.
pub(super) struct EnvVars;

impl Provider for EnvVars {
    fn metadata(&self) -> Metadata {
        Metadata::named("environment variable")
            .interpolater(|_: &Profile, keys: &[&str]| var_name(&keys.join(".")))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Env::raw().filter_map(bootstrap_key).data()
    }
}

/// The dotted key a variable sets, or `None` when it is not a bootstrap variable.
fn bootstrap_key(name: &UncasedStr) -> Option<Uncased<'_>> {
    let name = name.as_str();
    let rest = strip_prefix(name, PREFIX)?;
    let top = rest.split("__").next().unwrap_or_default();
    if top.eq_ignore_ascii_case("CONFIG") || top.eq_ignore_ascii_case("SETTINGS") {
        return None;
    }
    let value = std::env::var_os(name)?;
    if value.to_string_lossy().trim().is_empty() {
        return None;
    }
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
}
