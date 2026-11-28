//! What `neon-updater serve` reads from its environment.

use std::net::SocketAddr;

use anyhow::{Context as _, anyhow, bail};

use crate::server::Token;

/// The token the server sends, at least 32 characters.
pub const TOKEN_VAR: &str = "NEON_UPDATER_TOKEN";
/// The address to listen on.
pub const ADDR_VAR: &str = "NEON_UPDATER_ADDR";
/// The Compose services to update, separated by commas.
pub const SERVICES_VAR: &str = "NEON_UPDATER_SERVICES";
/// The stack's Compose project, the updater's own unless set.
pub const PROJECT_VAR: &str = "NEON_UPDATER_PROJECT";
/// The image an update runs in, the updater's own unless set.
pub const IMAGE_VAR: &str = "NEON_UPDATER_IMAGE";
/// Whether an update checks the images' signatures, true unless set to false.
pub const VERIFY_VAR: &str = "NEON_UPDATER_VERIFY";

const DEFAULT_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_SERVICES: &str = "server,web";

/// The settings of `neon-updater serve`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub token: Token,
    pub addr: SocketAddr,
    pub services: Vec<String>,
    pub project: Option<String>,
    pub image: Option<String>,
    pub verify: bool,
}

impl ServeConfig {
    /// Reads the process's environment.
    ///
    /// # Errors
    ///
    /// When a variable's value is not one the updater takes, or the token is missing or too
    /// short.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// Reads the variables `var` gives. A blank variable counts as unset.
    ///
    /// # Errors
    ///
    /// As [`ServeConfig::from_env`].
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let get = |name: &str| var(name).filter(|value| !value.trim().is_empty());

        let token = get(TOKEN_VAR).ok_or_else(|| {
            anyhow!("{TOKEN_VAR} is not set: the updater serves only the holder of its token")
        })?;
        let token = Token::new(token.trim()).context(TOKEN_VAR)?;

        let addr = get(ADDR_VAR).unwrap_or_else(|| DEFAULT_ADDR.to_owned());
        let addr = addr
            .trim()
            .parse()
            .with_context(|| format!("{ADDR_VAR}: {addr:?} is not an address and a port"))?;

        let services: Vec<String> = get(SERVICES_VAR)
            .unwrap_or_else(|| DEFAULT_SERVICES.to_owned())
            .split(',')
            .map(str::trim)
            .filter(|service| !service.is_empty())
            .map(str::to_owned)
            .collect();
        if services.is_empty() {
            bail!("{SERVICES_VAR} names no service");
        }
        if let Some(bad) = services.iter().find(|service| !is_service_name(service)) {
            bail!("{SERVICES_VAR}: {bad:?} is not a Compose service's name");
        }

        let project = get(PROJECT_VAR).map(|project| project.trim().to_owned());
        if let Some(bad) = project
            .as_deref()
            .filter(|project| !is_project_name(project))
        {
            bail!("{PROJECT_VAR}: {bad:?} is not a Compose project's name");
        }
        let image = get(IMAGE_VAR).map(|image| image.trim().to_owned());
        if let Some(bad) = image.as_deref().filter(|image| image.starts_with('-')) {
            bail!("{IMAGE_VAR}: {bad:?} is not an image");
        }

        let verify = match get(VERIFY_VAR) {
            None => true,
            Some(value) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                _ => bail!("{VERIFY_VAR}: {value:?} is neither true nor false"),
            },
        };

        Ok(Self {
            token,
            addr,
            services,
            project,
            image,
            verify,
        })
    }
}

/// A Compose service's name: a letter or digit, then letters, digits, `_`, `.` and `-`.
fn is_service_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// A Compose project's name: a lowercase letter or digit, then those, `_` and `-`.
fn is_project_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const TOKEN: &str = "test-token-not-a-secret-0123456789";

    fn read(vars: &[(&str, &str)]) -> anyhow::Result<ServeConfig> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        ServeConfig::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn defaults() {
        let config = read(&[(TOKEN_VAR, TOKEN)]).unwrap();
        assert!(config.token.matches(TOKEN));
        assert_eq!(config.addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.services, ["server", "web"]);
        assert_eq!(config.project, None);
        assert_eq!(config.image, None);
        assert!(config.verify);
    }

    #[test]
    fn every_variable() {
        let config = read(&[
            (TOKEN_VAR, TOKEN),
            (ADDR_VAR, "127.0.0.1:9000"),
            (SERVICES_VAR, " server , web,, worker "),
            (PROJECT_VAR, "neon-prod"),
            (
                IMAGE_VAR,
                "ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.0",
            ),
            (VERIFY_VAR, "False"),
        ])
        .unwrap();
        assert_eq!(config.addr, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(config.services, ["server", "web", "worker"]);
        assert_eq!(config.project.as_deref(), Some("neon-prod"));
        assert_eq!(
            config.image.as_deref(),
            Some("ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.0")
        );
        assert!(!config.verify);

        let blank = read(&[(TOKEN_VAR, TOKEN), (SERVICES_VAR, " "), (VERIFY_VAR, "")]).unwrap();
        assert_eq!(blank.services, ["server", "web"], "blank is unset");
        assert!(blank.verify);
    }

    #[test]
    fn refuses_to_start_without_a_long_token() {
        let error = read(&[]).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("NEON_UPDATER_TOKEN is not set"),
            "{error}"
        );
        let error = read(&[(TOKEN_VAR, "test-token-of-31-characters-xyz")]).unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "NEON_UPDATER_TOKEN: the token must be at least 32 characters, such as the output of openssl rand -hex 32"
        );
    }

    #[test]
    fn refuses_values_it_cannot_use() {
        for (var, value) in [
            (ADDR_VAR, ":8080"),
            (SERVICES_VAR, ","),
            (SERVICES_VAR, "server,--file=/etc/passwd"),
            (SERVICES_VAR, "server web"),
            (PROJECT_VAR, "Neon"),
            (PROJECT_VAR, "-p"),
            (IMAGE_VAR, "--privileged"),
            (VERIFY_VAR, "maybe"),
        ] {
            let error = read(&[(TOKEN_VAR, TOKEN), (var, value)]).unwrap_err();
            assert!(error.to_string().starts_with(var), "{var}={value}: {error}");
        }
    }
}
