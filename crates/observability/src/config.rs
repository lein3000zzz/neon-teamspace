//! The `observability` section of the configuration file.
//!
//! Every struct accepts a partial section: missing keys take their defaults and unknown keys
//! are rejected, so a typo fails loudly instead of silently keeping a default.
//!
//! The collector and Pyroscope URLs may carry a user name and password, so `Debug` shows them
//! through [`redact_url`].

use std::collections::BTreeMap;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};

use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use url::Url;

/// Replaces a credential in a URL that is shown.
pub const REDACTED: &str = "redacted";

/// Settings for logs, metrics, traces, profiles and the admin listener.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Log level and output format.
    pub logging: LoggingConfig,
    /// The Prometheus recorder.
    pub metrics: MetricsConfig,
    /// Trace sampling, export over OTLP and trust in incoming trace context.
    pub tracing: TracingConfig,
    /// CPU and heap profiles.
    pub profiling: ProfilingConfig,
    /// The internal listener for metrics, probes and profiles.
    pub admin: AdminConfig,
}

/// Log level and output format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// An `EnvFilter` directive such as `info` or `info,sqlx=warn`. It only filters log
    /// output: traces always record spans at `info` and above, so lowering the log volume
    /// never breaks trace ids or `traceparent` propagation. A more verbose directive, such as
    /// `info,neon_app=debug`, lowers the traced level to its most verbose level for every
    /// target, so lines logged inside `debug` and `trace` spans carry trace ids too.
    pub level: String,
    /// JSON lines for machines or multi-line text for people.
    pub format: LogFormat,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            format: LogFormat::Json,
        }
    }
}

/// How log records are written to stdout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// One JSON object per line, with `trace_id` and `span_id` inside spans.
    #[default]
    Json,
    /// Indented text for local development.
    Pretty,
}

/// The Prometheus recorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    /// Installs the recorder and serves `/metrics` on the admin listener. When off, every
    /// metric call is a no-op.
    pub enabled: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Trace sampling, export over OTLP and trust in incoming trace context. `Debug` redacts the
/// credentials in `endpoint`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TracingConfig {
    /// Exports spans. Spans and trace ids exist either way, so logs stay correlated and
    /// `traceparent` keeps propagating while export is off.
    pub enabled: bool,
    /// Base URL of the collector, for example `http://otel-lgtm:4318`. `/v1/traces` is
    /// appended unless the path already ends with it. Unset falls back to the standard
    /// `OTEL_EXPORTER_OTLP_*` variables, then to `http://localhost:4318`.
    pub endpoint: Option<String>,
    /// The OTLP transport.
    pub protocol: OtlpProtocol,
    /// Share of new traces to sample, from 0 to 1. A continued trace, from a job row or from
    /// a trusted caller (see `trust_incoming`), keeps the sampling decision it started with.
    pub sample_ratio: f64,
    /// Continues the trace and keeps the baggage that HTTP callers send in `traceparent` and
    /// `baggage`.
    ///
    /// Off by default because the app port is public. A trusted `traceparent` lets any client
    /// force sampling past `sample_ratio` with its sampled flag and attach spans to someone
    /// else's trace, and trusted baggage is copied into job rows and outgoing webhooks. While
    /// off, every request starts a new trace that records the caller's context, when valid,
    /// as a span link, and incoming baggage is dropped. Turn it on only when every caller is
    /// trusted, for example behind a proxy that removes or rewrites these headers.
    pub trust_incoming: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            protocol: OtlpProtocol::Http,
            sample_ratio: 1.0,
            trust_incoming: false,
        }
    }
}

/// The OTLP transport.
///
/// Only HTTP with protobuf bodies is built in. gRPC would add tonic and a second HTTP/2
/// stack for no gain over pointing the exporter at a local collector, so it is accepted by
/// the parser only to fail validation with a clear message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OtlpProtocol {
    /// OTLP/HTTP with protobuf bodies, usually on port 4318.
    #[default]
    Http,
    /// OTLP/gRPC, usually on port 4317. Not built in.
    Grpc,
}

/// CPU and heap profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfilingConfig {
    /// Where profiles go. Pull and push exclude each other, because two CPU profilers in one
    /// process break each other's signal handling.
    pub mode: ProfilingMode,
    /// The profiles to collect.
    pub profiles: Vec<Profile>,
    /// Pyroscope settings, read in push mode only.
    pub push: PushConfig,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            mode: ProfilingMode::Off,
            profiles: vec![Profile::Cpu],
            push: PushConfig::default(),
        }
    }
}

/// Where profiles go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfilingMode {
    /// No profiling.
    #[default]
    Off,
    /// Serve `/debug/pprof/*` on the admin listener for a scraper or `go tool pprof`.
    Pull,
    /// Send profiles to a Pyroscope server from inside the process.
    Push,
}

/// A kind of profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// Sampled CPU time.
    Cpu,
    /// Live heap allocations, from jemalloc on Linux.
    Heap,
}

/// Pyroscope settings for push mode. `Debug` redacts the credentials in `server_address`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PushConfig {
    /// Pyroscope base URL, for example `http://pyroscope:4040`.
    pub server_address: Option<String>,
    /// Seconds between uploads.
    pub upload_interval_seconds: u64,
    /// Labels attached to every profile, such as `env: prod`.
    pub tags: BTreeMap<String, String>,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            server_address: None,
            upload_interval_seconds: 10,
            tags: BTreeMap::new(),
        }
    }
}

impl fmt::Debug for TracingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Destructured, so a new field cannot be left out of the output by accident.
        let Self {
            enabled,
            endpoint,
            protocol,
            sample_ratio,
            trust_incoming,
        } = self;
        f.debug_struct("TracingConfig")
            .field("enabled", enabled)
            .field("endpoint", &endpoint.as_deref().map(redact_url))
            .field("protocol", protocol)
            .field("sample_ratio", sample_ratio)
            .field("trust_incoming", trust_incoming)
            .finish()
    }
}

impl fmt::Debug for PushConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            server_address,
            upload_interval_seconds,
            tags,
        } = self;
        f.debug_struct("PushConfig")
            .field("server_address", &server_address.as_deref().map(redact_url))
            .field("upload_interval_seconds", upload_interval_seconds)
            .field("tags", tags)
            .finish()
    }
}

/// The URL with the password in its user part replaced by [`REDACTED`], or the user name when
/// there is no password, since a user name alone may be a token. An HTTP URL without either is
/// returned as written. Anything else is hidden whole, since where a secret would sit in it is
/// unknown: text that does not parse, or parses with another scheme, as `user:secret@host`
/// does. `neon config check` shows the collector and Pyroscope URLs the same way.
#[must_use]
pub fn redact_url(text: &str) -> String {
    let Some(mut url) = Url::parse(text)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
    else {
        return format!("[{REDACTED}]");
    };
    // Both fail only for URLs that cannot have a user part, and this one has one.
    if url.password().is_some() {
        let _ = url.set_password(Some(REDACTED));
    } else if !url.username().is_empty() {
        let _ = url.set_username(REDACTED);
    } else {
        // Parsing would add a trailing slash to a bare host.
        return text.to_owned();
    }
    url.into()
}

/// The internal listener for metrics, probes and profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    /// Starts the listener.
    pub enabled: bool,
    /// Bind address. Keep it on loopback or an internal network: profiles expose memory
    /// contents and a CPU profile holds the process for up to a minute.
    pub addr: SocketAddr,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 9090)),
        }
    }
}

/// One configuration problem, tied to the key it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Dotted path of the key, such as `observability.tracing.sample_ratio`.
    pub field: &'static str,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// The configuration cannot be used. Lists every error, plus the warnings found with them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid observability config: {}", join(.errors))]
pub struct InvalidConfig {
    /// Problems that prevent startup.
    pub errors: Vec<Problem>,
    /// Problems that only degrade a feature.
    pub warnings: Vec<Problem>,
}

fn join(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl Config {
    /// Checks the whole section at once, so every mistake surfaces in one run.
    ///
    /// Returns the warnings when the configuration is usable.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidConfig`] when at least one setting cannot work.
    pub fn validate(&self) -> Result<Vec<Problem>, InvalidConfig> {
        let mut checks = Checks::default();
        self.logging.check(&mut checks);
        if self.metrics.enabled && !self.admin.enabled {
            checks.warning(
                "observability.metrics.enabled",
                "metrics are recorded but nothing serves /metrics while admin.enabled is false",
            );
        }
        self.tracing.check(&mut checks);
        self.profiling.check(&self.admin, &mut checks);
        checks.finish()
    }
}

#[derive(Default)]
struct Checks {
    errors: Vec<Problem>,
    warnings: Vec<Problem>,
}

impl Checks {
    fn error(&mut self, field: &'static str, message: impl Into<String>) {
        self.errors.push(Problem {
            field,
            message: message.into(),
        });
    }

    fn warning(&mut self, field: &'static str, message: impl Into<String>) {
        self.warnings.push(Problem {
            field,
            message: message.into(),
        });
    }

    fn finish(self) -> Result<Vec<Problem>, InvalidConfig> {
        if self.errors.is_empty() {
            Ok(self.warnings)
        } else {
            Err(InvalidConfig {
                errors: self.errors,
                warnings: self.warnings,
            })
        }
    }
}

impl LoggingConfig {
    fn check(&self, checks: &mut Checks) {
        const FIELD: &str = "observability.logging.level";
        if self.level.trim().is_empty() {
            checks.error(FIELD, "must not be empty, use for example \"info\"");
        } else if let Err(error) = EnvFilter::builder().parse(&self.level) {
            checks.error(FIELD, format!("is not a valid filter directive: {error}"));
        }
    }
}

impl TracingConfig {
    fn check(&self, checks: &mut Checks) {
        if self.protocol == OtlpProtocol::Grpc {
            checks.error(
                "observability.tracing.protocol",
                "grpc is not built in; use http and the collector's OTLP/HTTP port (usually 4318)",
            );
        }
        if !(0.0..=1.0).contains(&self.sample_ratio) {
            checks.error(
                "observability.tracing.sample_ratio",
                format!("must be between 0 and 1, got {}", self.sample_ratio),
            );
        }
        if let Some(endpoint) = &self.endpoint {
            if let Err(message) = check_http_url(endpoint) {
                checks.error("observability.tracing.endpoint", message);
            } else if !self.enabled {
                checks.warning(
                    "observability.tracing.endpoint",
                    "is ignored while tracing.enabled is false",
                );
            }
        }
    }
}

impl ProfilingConfig {
    fn check(&self, admin: &AdminConfig, checks: &mut Checks) {
        if self.mode == ProfilingMode::Off {
            if self.push.server_address.is_some() {
                checks.warning(
                    "observability.profiling.push",
                    "is ignored unless profiling.mode is push",
                );
            }
            return;
        }
        if self.profiles.is_empty() {
            checks.error(
                "observability.profiling.profiles",
                "lists no profiles; add cpu, heap or both, or set mode to off",
            );
        }
        if self.profiles.contains(&Profile::Heap) && !cfg!(target_os = "linux") {
            checks.warning(
                "observability.profiling.profiles",
                "heap profiles need jemalloc, which this build only links on Linux",
            );
        }
        match self.mode {
            ProfilingMode::Off => {}
            ProfilingMode::Pull => {
                if !admin.enabled {
                    checks.error(
                        "observability.profiling.mode",
                        "pull serves profiles on the admin listener, which is disabled",
                    );
                }
                if self.push.server_address.is_some() {
                    checks.warning(
                        "observability.profiling.push",
                        "is ignored unless profiling.mode is push",
                    );
                }
            }
            ProfilingMode::Push => self.push.check(checks),
        }
    }
}

impl PushConfig {
    fn check(&self, checks: &mut Checks) {
        const ADDRESS: &str = "observability.profiling.push.server_address";
        match self.server_address.as_deref().map(str::trim) {
            None | Some("") => checks.error(ADDRESS, "is required in push mode"),
            Some(address) => {
                if let Err(message) = check_http_url(address) {
                    checks.error(ADDRESS, message);
                }
            }
        }
        if self.upload_interval_seconds == 0 {
            checks.error(
                "observability.profiling.push.upload_interval_seconds",
                "must be at least 1",
            );
        }
        for key in self.tags.keys() {
            if !is_label_name(key) {
                checks.error(
                    "observability.profiling.push.tags",
                    format!(
                        "tag {key:?} must start with a letter or underscore, contain only \
                         letters, digits and underscores, and not start with \"__\""
                    ),
                );
            }
        }
    }
}

/// Exporters only speak plain HTTP, so TLS belongs to a collector on the internal network.
///
/// Messages leave the value out: a URL may carry credentials, and a value set under the wrong
/// key may be another service's URL with its password.
fn check_http_url(value: &str) -> Result<(), String> {
    let uri: http::Uri = value
        .parse()
        .map_err(|error| format!("is not a URL: {error}"))?;
    match uri.scheme_str() {
        Some("http") if uri.authority().is_some() => Ok(()),
        Some("http") => Err("has no host".to_owned()),
        Some("https") => Err("uses https, which is not built in; send plain http to a \
             collector or agent on the internal network and let it handle TLS"
            .to_owned()),
        _ => Err("must start with http://".to_owned()),
    }
}

/// Pyroscope tags become Prometheus-style labels, which reserve names starting with `__`.
fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    starts_well && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with("__")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(problems: &[Problem]) -> Vec<&'static str> {
        problems.iter().map(|problem| problem.field).collect()
    }

    #[test]
    fn defaults_are_valid_and_quiet() {
        let config = Config::default();
        assert_eq!(config.validate(), Ok(Vec::new()));
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.logging.format, LogFormat::Json);
        assert!(config.metrics.enabled);
        assert!(!config.tracing.enabled);
        assert!(!config.tracing.trust_incoming);
        assert_eq!(config.profiling.mode, ProfilingMode::Off);
        assert!(config.admin.enabled);
        assert_eq!(config.admin.addr.to_string(), "127.0.0.1:9090");
    }

    #[test]
    fn partial_sections_keep_defaults() {
        let config: Config = serde_json::from_str(
            r#"{"tracing": {"enabled": true, "sample_ratio": 0.25, "trust_incoming": true},
                "profiling": {"mode": "pull"}}"#,
        )
        .unwrap();
        assert!(config.tracing.enabled);
        assert!(config.tracing.trust_incoming);
        assert!((config.tracing.sample_ratio - 0.25).abs() < f64::EPSILON);
        assert_eq!(config.tracing.protocol, OtlpProtocol::Http);
        assert_eq!(config.profiling.mode, ProfilingMode::Pull);
        assert_eq!(config.profiling.profiles, vec![Profile::Cpu]);
        assert_eq!(config.admin, AdminConfig::default());
    }

    #[test]
    fn unknown_keys_and_values_are_rejected() {
        assert!(serde_json::from_str::<Config>(r#"{"tracing": {"sampel_ratio": 1}}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"tracing": {"protocol": "udp"}}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"profiling": {"mode": "both"}}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"admin": {"addr": "localhost"}}"#).is_err());
    }

    #[test]
    fn round_trips_through_serde() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }

    #[test]
    fn reports_every_error_at_once() {
        let mut config = Config::default();
        config.logging.level = "info,=nonsense=".to_owned();
        config.tracing.protocol = OtlpProtocol::Grpc;
        config.tracing.sample_ratio = 1.5;
        config.tracing.endpoint = Some("https://collector:4318".to_owned());
        config.profiling.mode = ProfilingMode::Push;
        config.profiling.push.upload_interval_seconds = 0;
        config
            .profiling
            .push
            .tags
            .insert("__reserved".to_owned(), "x".to_owned());

        let invalid = config.validate().unwrap_err();
        assert_eq!(
            fields(&invalid.errors),
            vec![
                "observability.logging.level",
                "observability.tracing.protocol",
                "observability.tracing.sample_ratio",
                "observability.tracing.endpoint",
                "observability.profiling.push.server_address",
                "observability.profiling.push.upload_interval_seconds",
                "observability.profiling.push.tags",
            ]
        );
        let message = invalid.to_string();
        assert!(message.contains("sample_ratio: must be between 0 and 1, got 1.5"));
        assert!(message.contains("server_address: is required in push mode"));
    }

    #[test]
    fn rejects_empty_level_and_nan_ratio() {
        let mut config = Config::default();
        config.logging.level = "  ".to_owned();
        config.tracing.sample_ratio = f64::NAN;
        let invalid = config.validate().unwrap_err();
        assert_eq!(
            fields(&invalid.errors),
            vec![
                "observability.logging.level",
                "observability.tracing.sample_ratio"
            ]
        );
    }

    #[test]
    fn pull_needs_the_admin_listener_and_a_profile() {
        let mut config = Config::default();
        config.profiling.mode = ProfilingMode::Pull;
        config.profiling.profiles.clear();
        config.admin.enabled = false;
        let invalid = config.validate().unwrap_err();
        assert_eq!(
            fields(&invalid.errors),
            vec![
                "observability.profiling.profiles",
                "observability.profiling.mode"
            ]
        );
    }

    #[test]
    fn valid_push_config_passes() {
        let mut config = Config::default();
        config.profiling.mode = ProfilingMode::Push;
        config.profiling.push.server_address = Some("http://pyroscope:4040".to_owned());
        config
            .profiling
            .push
            .tags
            .insert("env".to_owned(), "prod".to_owned());
        assert_eq!(config.validate(), Ok(Vec::new()));
    }

    #[test]
    fn heap_off_linux_is_a_warning_not_an_error() {
        let mut config = Config::default();
        config.profiling.mode = ProfilingMode::Pull;
        config.profiling.profiles = vec![Profile::Cpu, Profile::Heap];
        let warnings = config.validate().unwrap();
        if cfg!(target_os = "linux") {
            assert!(warnings.is_empty());
        } else {
            assert_eq!(fields(&warnings), vec!["observability.profiling.profiles"]);
        }
    }

    #[test]
    fn ignored_settings_are_warnings() {
        let mut config = Config::default();
        config.tracing.endpoint = Some("http://collector:4318".to_owned());
        config.profiling.push.server_address = Some("http://pyroscope:4040".to_owned());
        let warnings = config.validate().unwrap();
        assert_eq!(
            fields(&warnings),
            vec![
                "observability.tracing.endpoint",
                "observability.profiling.push"
            ]
        );
    }

    #[test]
    fn metrics_without_the_admin_listener_is_a_warning() {
        let mut config = Config::default();
        config.admin.enabled = false;
        let warnings = config.validate().unwrap();
        assert_eq!(fields(&warnings), vec!["observability.metrics.enabled"]);

        config.metrics.enabled = false;
        assert_eq!(config.validate(), Ok(Vec::new()));
    }

    #[test]
    fn urls_must_be_plain_http_with_a_host() {
        assert!(check_http_url("http://otel-lgtm:4318").is_ok());
        assert!(check_http_url("http://10.0.0.5:4318/v1/traces").is_ok());
        assert!(check_http_url("https://otel-lgtm:4318").is_err());
        assert!(check_http_url("otel-lgtm:4318").is_err());
        assert!(check_http_url("http://").is_err());
        assert!(check_http_url("not a url").is_err());
    }

    #[test]
    fn url_errors_do_not_show_the_url() {
        for value in [
            "postgres://neon:hunter2@db/neon",
            "https://user:hunter2@collector:4318",
            "http://user:hunter2@",
            "hunter2 is not a url",
        ] {
            let message = check_http_url(value).unwrap_err();
            assert!(!message.contains("hunter2"), "{message}");
        }
    }

    #[test]
    fn credentials_in_urls_are_redacted() {
        assert_eq!(
            redact_url("http://user:hunter2@collector:4318"),
            "http://user:redacted@collector:4318/"
        );
        assert_eq!(
            redact_url("http://hunter2@pyroscope:4040/base"),
            "http://redacted@pyroscope:4040/base"
        );
        assert_eq!(redact_url("http://otel-lgtm:4318"), "http://otel-lgtm:4318");
        assert_eq!(redact_url("hunter2@collector"), "[redacted]");
        // A URL with another scheme: "user" is the scheme, "hunter2@collector" its path.
        assert_eq!(redact_url("user:hunter2@collector"), "[redacted]");
    }

    #[test]
    fn debug_output_hides_credentials_in_urls() {
        let mut config = Config::default();
        config.tracing.endpoint = Some("http://user:hunter2@collector:4318".to_owned());
        config.profiling.push.server_address = Some("http://hunter2@pyroscope:4040".to_owned());
        config
            .profiling
            .push
            .tags
            .insert("env".to_owned(), "prod".to_owned());

        let shown = format!("{config:?}");

        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(
            shown.contains(r#"endpoint: Some("http://user:redacted@collector:4318/")"#),
            "{shown}"
        );
        assert!(
            shown.contains(r#"server_address: Some("http://redacted@pyroscope:4040/")"#),
            "{shown}"
        );
        // The other fields are still shown.
        assert!(shown.contains("sample_ratio: 1.0"), "{shown}");
        assert!(shown.contains("upload_interval_seconds: 10"), "{shown}");
        assert!(shown.contains(r#""env": "prod""#), "{shown}");
    }

    #[test]
    fn debug_output_shows_urls_without_credentials_as_written() {
        let tracing = TracingConfig {
            endpoint: Some("http://otel-lgtm:4318".to_owned()),
            ..TracingConfig::default()
        };

        assert!(format!("{tracing:?}").contains(r#"Some("http://otel-lgtm:4318")"#));
        assert!(format!("{:?}", PushConfig::default()).contains("server_address: None"));
    }

    #[test]
    fn label_names_follow_prometheus_rules() {
        assert!(is_label_name("env"));
        assert!(is_label_name("_private"));
        assert!(is_label_name("region_1"));
        assert!(!is_label_name(""));
        assert!(!is_label_name("1st"));
        assert!(!is_label_name("with-dash"));
        assert!(!is_label_name("__name__"));
    }
}
