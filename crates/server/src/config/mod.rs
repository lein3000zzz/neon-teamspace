//! The bootstrap configuration: what the server needs before it can reach its database.
//!
//! Values come from the defaults, then the YAML file, then `NEON_` environment variables,
//! the last one that sets a value winning. The file is `neon.yaml` in the working
//! directory, or the path in `NEON_CONFIG`.
//!
//! The file's `settings` section and the `NEON_SETTINGS__*` variables are runtime
//! settings. The settings registry parses them, and a value set there pins its setting,
//! so the admin page cannot change it.

mod database_url;
mod env;
mod render;
mod yaml;

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use figment::Figment;
use neon_domain::settings::{Pinned, Problem, RawLayers, Registry};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use url::Url;

pub use database_url::DatabaseUrl;

use env::EnvVars;
use yaml::YamlFile;

/// Names the config file. A file named here must exist.
pub const CONFIG_ENV: &str = "NEON_CONFIG";

/// The config file read when `NEON_CONFIG` is unset, if it exists.
pub const DEFAULT_FILE: &str = "neon.yaml";

/// The configuration as the file and the environment set it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Parsed by [`load`] into [`Loaded::settings`].
    #[serde(default)]
    settings: SettingsTree,
    /// Accepted and ignored until the observability configuration is read from it, so
    /// files and variables written for it already load.
    #[serde(default)]
    pub observability: IgnoredAny,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// The address the HTTP API listens on.
    pub listen: SocketAddr,
    /// Where users reach the server, such as `https://neon.example.com`.
    pub public_url: Option<Url>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080)),
            public_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    /// The Postgres URL. Required.
    pub url: DatabaseUrl,
    /// The most connections the pool keeps open.
    pub max_connections: u32,
    /// Whether the server applies pending migrations when it starts.
    pub auto_migrate: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: DatabaseUrl::default(),
            max_connections: 10,
            auto_migrate: true,
        }
    }
}

/// The file's `settings` tree as written. `Debug` lists only its keys, since values of
/// secret settings may be in it.
#[derive(Default, Deserialize)]
#[serde(transparent)]
struct SettingsTree(Map<String, Json>);

impl fmt::Debug for SettingsTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// A loaded and checked configuration.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    /// The runtime settings the environment and the file set.
    pub settings: Pinned,
    /// The config file that was read, if any.
    pub file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{CONFIG_ENV} names {}, which does not exist", .0.display())]
    MissingFile(PathBuf),
    /// The file is not YAML, or a value has the wrong type or an unknown key.
    #[error("{}", list_figment(.0))]
    Read(Box<figment::Error>),
    #[error("invalid configuration:{}", list(.0))]
    Invalid(Vec<Problem>),
}

/// Reads the configuration and checks all of it, reporting every problem at once.
pub fn load(registry: &Registry) -> Result<Loaded, ConfigError> {
    let file = config_file()?;

    let mut figment = Figment::new();
    if let Some(path) = &file {
        figment = figment.merge(YamlFile::new(path.clone()));
    }
    let config: Config = figment
        .merge(EnvVars)
        .extract()
        .map_err(|error| ConfigError::Read(Box::new(error)))?;

    let raw = RawLayers::default()
        .with_env(env::settings_vars())
        .with_yaml(&config.settings.0);
    let mut problems = config.problems();
    let settings = match registry.pin(&raw) {
        Ok(settings) => Some(settings),
        Err(settings_problems) => {
            problems.extend(settings_problems);
            None
        }
    };

    match settings {
        Some(settings) if problems.is_empty() => Ok(Loaded {
            config,
            settings,
            file,
        }),
        _ => Err(ConfigError::Invalid(problems)),
    }
}

/// The file named by `NEON_CONFIG`, which must exist, or else `neon.yaml` if it exists.
fn config_file() -> Result<Option<PathBuf>, ConfigError> {
    let named = std::env::var_os(CONFIG_ENV).filter(|path| !path.is_empty());
    let required = named.is_some();
    let path = named.map_or_else(|| PathBuf::from(DEFAULT_FILE), PathBuf::from);

    // An error other than "not found", such as no permission, surfaces when reading.
    match (path.try_exists(), required) {
        (Ok(false), true) => Err(ConfigError::MissingFile(path)),
        (Ok(false), false) => Ok(None),
        _ => Ok(Some(path)),
    }
}

impl Config {
    fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut report = |key: &str, message: String| {
            problems.push(Problem::new(
                format!("{key} ({})", env::var_name(key)),
                message,
            ));
        };

        if let Err(message) = self.database.url.check() {
            report("database.url", message);
        }
        if self.database.max_connections == 0 {
            report("database.max_connections", "must be at least 1".to_owned());
        }
        if let Some(url) = &self.server.public_url
            && let Err(message) = check_public_url(url)
        {
            report("server.public_url", message);
        }

        problems
    }
}

/// The public URL is an origin: browsers compare it with the `Origin` header, and the
/// server is always served from the root of it.
fn check_public_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("must be http or https, not {}", url.scheme()));
    }
    if !url.has_host() {
        return Err("must name a host".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("must not hold a user name or password".to_owned());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(
            "must be only a scheme, host and port, like https://neon.example.com".to_owned(),
        );
    }
    Ok(())
}

fn list(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|problem| format!("\n  {problem}"))
        .collect()
}

fn list_figment(error: &figment::Error) -> String {
    let lines: Vec<String> = error.clone().into_iter().map(|e| e.to_string()).collect();
    lines.join("\n")
}
