//! The bootstrap configuration: what the server needs before it can reach its database.
//!
//! Values come from the defaults, then the YAML file, then `NEON_` environment variables,
//! the last one that sets a value winning. The file is `neon.yaml` in the working
//! directory, or the path in `NEON_CONFIG`. An unknown key in the file fails the load,
//! since it is almost always a typo. An unknown `NEON_` variable is only reported: other
//! tools use the prefix too, such as Neon's Postgres tooling with `NEON_API_KEY`.
//!
//! Neither errors nor `Debug` output show a configured value, since one set under the
//! wrong key may be the database URL with its password.
//!
//! The file's `settings` section and the `NEON_SETTINGS__*` variables are runtime
//! settings. The settings registry parses them, and a value set there pins its setting,
//! so the admin page cannot change it.
//!
//! The `observability` section is `neon-observability`'s own configuration. Its errors
//! fail the load with the others; its warnings are logged once telemetry is installed.
//! The server gives its `logging.level` a default of its own, [`DEFAULT_LOG_LEVEL`].
//!
//! The `admin` section names the first admin, made as the server starts while no admin can
//! sign in, and `secrets.keys` the keys that seal secrets kept in the database.

mod bootstrap;
mod database_url;
mod env;
mod render;
mod yaml;

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use figment::Figment;
use figment::error::{Actual, Kind, OneOf};
use figment::providers::Serialized;
use ipnet::IpNet;
use neon_app::jobs::LISTEN_CONNECTIONS;
use neon_domain::auth::Email;
use neon_domain::settings::{Pinned, Problem, RawLayers, Registry};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value as Json};
use url::Url;

pub use bootstrap::{AdminConfig, SecretString, SecretsConfig, default_trusted_proxies};
pub use database_url::DatabaseUrl;

use env::EnvVars;
use yaml::YamlFile;

/// Names the config file. A file named here must exist.
pub const CONFIG_ENV: &str = "NEON_CONFIG";

/// The config file read when `NEON_CONFIG` is unset, if it exists.
pub const DEFAULT_FILE: &str = "neon.yaml";

/// `observability.logging.level` when neither the file nor the environment sets it.
/// Postgres notices, such as "relation already exists, skipping" on every start, and
/// OpenTelemetry's own messages, such as the one as its tracer is installed, are only
/// worth seeing when they warn.
pub const DEFAULT_LOG_LEVEL: &str = "info,sqlx::postgres::notice=warn,opentelemetry=warn";

/// The fewest connections the server can work with: the job worker keeps its own to
/// listen for new jobs, and queries need at least one more.
pub const MIN_CONNECTIONS: u32 = LISTEN_CONNECTIONS + 1;

/// The configuration as the file and the environment set it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    /// The first admin.
    #[serde(default)]
    pub admin: AdminConfig,
    /// The keys that seal secrets kept in the database.
    #[serde(default)]
    pub secrets: SecretsConfig,
    /// Parsed by [`load`] into [`Loaded::settings`].
    #[serde(default)]
    settings: SettingsTree,
    /// Logs, metrics, traces, profiles and the admin listener.
    #[serde(default)]
    pub observability: neon_observability::Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// The address the HTTP API listens on.
    pub listen: SocketAddr,
    /// Where users reach the server, such as `https://neon.example.com`. Browsers' `Origin`
    /// must match it on requests that change something, and over https the session cookie
    /// is `Secure` and named `__Host-neon_session`.
    #[serde(deserialize_with = "optional_url")]
    pub public_url: Option<Url>,
    /// Peers whose `X-Forwarded-For` names the client, such as the reverse proxy in front.
    /// Anyone else's is ignored, so a client cannot pick the address rate limits count.
    #[serde(deserialize_with = "bootstrap::proxy_list")]
    pub trusted_proxies: Vec<IpNet>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080)),
            public_url: None,
            trusted_proxies: default_trusted_proxies(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    /// The Postgres URL. Required.
    pub url: DatabaseUrl,
    /// The most connections the server opens to Postgres, counting the one the job
    /// worker listens on. At least [`MIN_CONNECTIONS`].
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
    /// `NEON_` variables that set no configuration key, by name. They are ignored, and
    /// listed so a misspelt one does not go unnoticed.
    pub ignored_vars: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{CONFIG_ENV} names {}, which does not exist", .0.display())]
    MissingFile(PathBuf),
    /// The file is not YAML, or a value has the wrong type or an unknown key. The errors
    /// describe values by their type only.
    #[error("{}", list_figment(.0))]
    Read(Vec<figment::Error>),
    #[error("invalid configuration:{}", list(.0))]
    Invalid(Vec<Problem>),
}

/// Reads the configuration and checks all of it, reporting every problem at once.
pub fn load(registry: &Registry) -> Result<Loaded, ConfigError> {
    let file = config_file()?;
    let (vars, ignored_vars) = env::candidates()
        .into_iter()
        .partition(|name| sets_a_key(name));

    let mut figment = Figment::from(Serialized::default(
        "observability.logging.level",
        DEFAULT_LOG_LEVEL,
    ));
    if let Some(path) = &file {
        figment = figment.merge(YamlFile::new(path.clone()));
    }
    let config: Config = figment
        .merge(EnvVars::new(vars))
        .extract()
        .map_err(read_error)?;

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
            ignored_vars,
        }),
        _ => Err(ConfigError::Invalid(problems)),
    }
}

/// Whether a `NEON_` variable sets a configuration key. Each one is tried on its own, so
/// the configuration types decide which keys exist and a new section needs no list here.
/// A value of the wrong type still sets a key; the load then reports it.
fn sets_a_key(name: &str) -> bool {
    match Figment::from(EnvVars::new(vec![name.to_owned()])).extract::<Config>() {
        Ok(_) => true,
        Err(error) => !error
            .into_iter()
            .any(|error| matches!(error.kind, Kind::UnknownField(..))),
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
        if self.database.max_connections < MIN_CONNECTIONS {
            report(
                "database.max_connections",
                format!(
                    "must be at least {MIN_CONNECTIONS}: the job worker keeps one connection \
                     to listen for new jobs, and queries need at least one more"
                ),
            );
        }
        if let Some(url) = &self.server.public_url
            && let Err(message) = check_public_url(url)
        {
            report("server.public_url", message);
        }
        match (&self.admin.email, &self.admin.password) {
            (Some(email), Some(_)) => {
                if Email::parse(email).is_err() {
                    report(
                        "admin.email",
                        "is not an email address like name@example.com".to_owned(),
                    );
                }
            }
            (Some(_), None) => report(
                "admin.password",
                "is required with admin.email: set both or neither".to_owned(),
            ),
            (None, Some(_)) => report(
                "admin.email",
                "is required with admin.password: set both or neither".to_owned(),
            ),
            (None, None) => {}
        }
        if let Err(error) = self.secrets.keyring() {
            report("secrets.keys", error.to_string());
        }
        // Its warnings are logged as telemetry starts, which checks the section again.
        if let Err(invalid) = self.observability.validate() {
            for problem in invalid.errors {
                report(problem.field, problem.message);
            }
        }

        problems
    }
}

/// Parses a URL as `Url`'s own `Deserialize` does, except that the error leaves out the
/// text, which `url` quotes. Custom errors are passed on as they are, so every type read
/// from the configuration must keep values out of its own.
fn optional_url<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Url>, D::Error> {
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    Url::parse(&text)
        .map(Some)
        .map_err(|error| D::Error::custom(format!("not a URL: {error}")))
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

fn list_figment(errors: &[figment::Error]) -> String {
    let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
    lines.join("\n")
}

/// Splits figment's chain of errors and replaces every value they quote with its type.
/// The key path and the file or variable it came from are kept.
fn read_error(error: figment::Error) -> ConfigError {
    ConfigError::Read(
        error
            .into_iter()
            .map(|mut error| {
                error.kind = without_value(error.kind);
                error
            })
            .collect(),
    )
}

fn without_value(kind: Kind) -> Kind {
    match kind {
        Kind::InvalidType(actual, expected) => Kind::InvalidType(type_of(actual), expected),
        Kind::InvalidValue(actual, expected) => Kind::InvalidValue(type_of(actual), expected),
        Kind::Unsupported(actual) => Kind::Unsupported(type_of(actual)),
        Kind::UnsupportedKey(actual, expected) => Kind::UnsupportedKey(type_of(actual), expected),
        Kind::UnknownVariant(_, expected) => {
            Kind::Message(format!("unknown value: expected {}", OneOf(expected)))
        }
        Kind::ISizeOutOfRange(_) | Kind::USizeOutOfRange(_) => {
            Kind::Message("a number is out of range".to_owned())
        }
        other => other,
    }
}

fn type_of(actual: Actual) -> Actual {
    let name = match actual {
        Actual::Bool(_) => "a boolean",
        Actual::Unsigned(_) | Actual::Signed(_) | Actual::Float(_) => "a number",
        Actual::Char(_) => "a character",
        Actual::Str(_) => "a string",
        Actual::Bytes(_) => "bytes",
        Actual::Other(_) => "a value",
        other => return other,
    };
    Actual::Other(name.to_owned())
}
