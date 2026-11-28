//! Loading the configuration from the defaults, the YAML file and the environment.
//!
//! These tests sit in their own binary: `figment::Jail` changes the process's working
//! directory and environment, which tests running beside them would see.

#![expect(
    clippy::result_large_err,
    reason = "figment::Jail's closures return figment::Result, whose error is large"
)]

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::Jail;
use neon_app::settings::{INSTANCE_NAME, registry};
use neon_domain::settings::{Problem, Source, Stored};
use neon_observability::config::{LogFormat, LoggingConfig, ProfilingMode};
use neon_server::config::{
    self, AdminConfig, ConfigError, DEFAULT_LOG_LEVEL, DatabaseConfig, Loaded, MIN_CONNECTIONS,
    SecretsConfig, ServerConfig, UpdatesConfig,
};

const URL: &str = "postgres://neon:hunter2@127.0.0.1:55432/neon";

/// The database password in [`URL`], which no error or debug output may show.
const PASSWORD: &str = "hunter2";

/// Runs `test` in a temporary working directory with an empty environment.
fn jailed(test: impl FnOnce(&mut Jail) -> figment::Result<()>) {
    Jail::expect_with(|jail| {
        jail.clear_env();
        test(jail)
    });
}

fn load() -> Result<Loaded, ConfigError> {
    config::load(&registry())
}

fn invalid(result: Result<Loaded, ConfigError>) -> Vec<Problem> {
    match result {
        Err(ConfigError::Invalid(problems)) => problems,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

fn read_error(result: Result<Loaded, ConfigError>) -> String {
    match result {
        Err(error @ ConfigError::Read(_)) => error.to_string(),
        other => panic!("expected a read error, got {other:?}"),
    }
}

fn instance_name(loaded: &Loaded) -> (String, Source) {
    let snapshot = registry().resolve(&loaded.settings, &Stored::new());
    let source = snapshot
        .resolved(INSTANCE_NAME.name())
        .map(|resolved| resolved.source)
        .expect("instance.name is registered");
    (snapshot.get(INSTANCE_NAME).to_owned(), source)
}

/// The observability section the server reads when nothing sets it.
fn default_observability() -> neon_observability::Config {
    neon_observability::Config {
        logging: LoggingConfig {
            level: DEFAULT_LOG_LEVEL.to_owned(),
            ..LoggingConfig::default()
        },
        ..neon_observability::Config::default()
    }
}

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("a socket address")
}

#[test]
fn defaults_apply_without_a_file() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.file, None);
        assert!(loaded.ignored_vars.is_empty());
        assert_eq!(loaded.config.server.listen, addr("0.0.0.0:8080"));
        assert_eq!(loaded.config.server.public_url, None);
        assert_eq!(loaded.config.database.url.expose(), URL);
        assert_eq!(loaded.config.database.max_connections, 10);
        assert!(loaded.config.database.auto_migrate);
        assert_eq!(loaded.config.observability, default_observability());
        assert_eq!(
            instance_name(&loaded),
            ("Neon Teamspace".to_owned(), Source::Default)
        );
        Ok(())
    });
}

#[test]
fn the_file_sets_values_and_pins_settings() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            &format!(
                "server:\n  listen: 127.0.0.1:9000\n  public_url: https://neon.example.com\n\
                 database:\n  url: {URL}\n  max_connections: 5\n  auto_migrate: false\n\
                 settings:\n  instance:\n    name: From YAML\n"
            ),
        )?;

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.file, Some(PathBuf::from("neon.yaml")));
        assert_eq!(loaded.config.server.listen, addr("127.0.0.1:9000"));
        assert_eq!(
            loaded
                .config
                .server
                .public_url
                .as_ref()
                .map(|url| url.as_str()),
            Some("https://neon.example.com/")
        );
        assert_eq!(loaded.config.database.url.expose(), URL);
        assert_eq!(loaded.config.database.max_connections, 5);
        assert!(!loaded.config.database.auto_migrate);
        assert_eq!(
            instance_name(&loaded),
            ("From YAML".to_owned(), Source::Yaml)
        );
        Ok(())
    });
}

#[test]
fn the_environment_wins_over_the_file() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            &format!(
                "server:\n  listen: 127.0.0.1:9000\ndatabase:\n  url: {URL}\n  max_connections: 5\n\
                 settings:\n  instance:\n    name: From YAML\n"
            ),
        )?;
        jail.set_env("NEON_SERVER__LISTEN", "127.0.0.1:9100");
        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", 20);
        jail.set_env("NEON_SETTINGS__INSTANCE__NAME", "From the environment");

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.config.server.listen, addr("127.0.0.1:9100"));
        assert_eq!(loaded.config.database.max_connections, 20);
        assert_eq!(loaded.config.database.url.expose(), URL);
        assert_eq!(
            instance_name(&loaded),
            ("From the environment".to_owned(), Source::Env)
        );
        Ok(())
    });
}

#[test]
fn blank_variables_and_empty_keys_count_as_unset() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            "server:\n  public_url:\nsettings:\n  instance:\n    # name: commented out\n",
        )?;
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_SERVER__LISTEN", " ");
        jail.set_env("NEON_SETTINGS__INSTANCE__NAME", "");

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.config.server, ServerConfig::default());
        assert_eq!(
            instance_name(&loaded),
            ("Neon Teamspace".to_owned(), Source::Default)
        );
        Ok(())
    });
}

#[test]
fn neon_config_names_the_file() {
    jailed(|jail| {
        jail.create_file("neon.yaml", "server:\n  listen: 127.0.0.1:1111\n")?;
        let dir = jail.create_dir("etc")?;
        jail.create_file(
            dir.join("neon.yaml"),
            &format!("server:\n  listen: 127.0.0.1:2222\ndatabase:\n  url: {URL}\n"),
        )?;
        jail.set_env("NEON_CONFIG", "etc/neon.yaml");

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.file, Some(PathBuf::from("etc/neon.yaml")));
        assert_eq!(loaded.config.server.listen, addr("127.0.0.1:2222"));
        Ok(())
    });
}

#[test]
fn a_file_neon_config_names_must_exist() {
    jailed(|jail| {
        jail.set_env("NEON_CONFIG", "missing.yaml");
        jail.set_env("NEON_DATABASE__URL", URL);

        let error = load().expect_err("the file is missing");

        assert!(matches!(error, ConfigError::MissingFile(_)), "{error:?}");
        assert_eq!(
            error.to_string(),
            "NEON_CONFIG names missing.yaml, which does not exist"
        );
        Ok(())
    });
}

#[test]
fn every_problem_is_reported_at_once() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            "server:\n  public_url: https://neon.example.com/app\n\
             database:\n  url: mysql://root:hunter2@db/neon\n  max_connections: 0\n\
             settings:\n  instance:\n    name: '  '\n    nmae: typo\n",
        )?;
        jail.set_env("NEON_SETTINGS__INSTANCE__NOPE", "1");

        let result = load();
        let message = result.as_ref().expect_err("invalid").to_string();
        let problems = invalid(result);

        assert_eq!(
            problems,
            [
                Problem::new(
                    "database.url (NEON_DATABASE__URL)",
                    "must start with postgres:// or postgresql://, not mysql://"
                ),
                Problem::new(
                    "database.max_connections (NEON_DATABASE__MAX_CONNECTIONS)",
                    "must be at least 2: the job worker keeps one connection to listen \
                     for new jobs, and queries need at least one more"
                ),
                Problem::new(
                    "server.public_url (NEON_SERVER__PUBLIC_URL)",
                    "must be only a scheme, host and port, like https://neon.example.com"
                ),
                Problem::new("NEON_SETTINGS__INSTANCE__NOPE", "no such setting"),
                Problem::new("settings.instance.name", "must not be empty"),
                Problem::new("settings.instance.nmae", "no such setting"),
            ]
        );
        assert!(
            message.starts_with("invalid configuration:\n  database.url"),
            "{message}"
        );
        assert!(!message.contains("hunter2"), "{message}");
        Ok(())
    });
}

#[test]
fn the_database_url_is_required() {
    jailed(|_| {
        assert_eq!(
            invalid(load()),
            [Problem::new(
                "database.url (NEON_DATABASE__URL)",
                "is required"
            )]
        );
        Ok(())
    });
}

#[test]
fn unknown_keys_and_wrong_types_are_errors() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);

        jail.create_file("neon.yaml", "servr:\n  listen: 127.0.0.1:9000\n")?;
        let message = read_error(load());
        assert!(message.contains("servr"), "{message}");
        assert!(message.contains("YAML file"), "{message}");

        jail.create_file("neon.yaml", "server:\n  listen: everywhere\n")?;
        let message = read_error(load());
        assert!(message.contains("server.listen"), "{message}");

        jail.create_file("neon.yaml", "")?;
        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", "many");
        let message = read_error(load());
        assert!(
            message.contains("NEON_DATABASE__MAX_CONNECTIONS"),
            "{message}"
        );

        Ok(())
    });
}

#[test]
fn the_pool_needs_a_connection_beside_the_listener() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);

        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", MIN_CONNECTIONS - 1);
        let problems = invalid(load());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(
            problems[0].key,
            "database.max_connections (NEON_DATABASE__MAX_CONNECTIONS)"
        );

        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", MIN_CONNECTIONS);
        load().expect("valid configuration");
        Ok(())
    });
}

#[test]
fn neon_variables_that_set_no_key_are_ignored_and_listed() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        // Neon's Postgres tooling, a Kubernetes service named `neon`, and a typo.
        jail.set_env("NEON_API_KEY", "napi_hunter2");
        jail.set_env("NEON_PROJECT_ID", "hunter2-project");
        jail.set_env("NEON_PORT", "tcp://10.96.0.12:8080");
        jail.set_env("NEON_SERVICE_HOST", "10.96.0.12");
        jail.set_env("NEON_DATABASE__URLL", "hunter2");
        jail.set_env("NEON_SERVER__LISTEN", "127.0.0.1:9100");
        let registry = registry();

        let loaded = config::load(&registry).expect("valid configuration");
        let shown = loaded.render(&registry).expect("renders");

        assert_eq!(
            loaded.ignored_vars,
            [
                "NEON_API_KEY",
                "NEON_DATABASE__URLL",
                "NEON_PORT",
                "NEON_PROJECT_ID",
                "NEON_SERVICE_HOST",
            ]
        );
        assert_eq!(loaded.config.server.listen, addr("127.0.0.1:9100"));
        assert!(
            shown.contains(
                "# Ignored, as they set no configuration key: NEON_API_KEY, \
                 NEON_DATABASE__URLL, NEON_PORT, NEON_PROJECT_ID, NEON_SERVICE_HOST."
            ),
            "{shown}"
        );
        assert!(!shown.contains(PASSWORD), "{shown}");
        assert!(!format!("{loaded:?}").contains(PASSWORD));
        Ok(())
    });
}

#[test]
fn unknown_settings_variables_are_errors_that_do_not_show_their_value() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        // A typo of mail.password: the value is likely a secret.
        jail.set_env("NEON_SETTINGS__MAIL__PASSWROD", PASSWORD);

        let result = load();
        let message = result.as_ref().expect_err("invalid").to_string();

        assert_eq!(
            invalid(result),
            [Problem::new(
                "NEON_SETTINGS__MAIL__PASSWROD",
                "no such setting"
            )]
        );
        assert!(!message.contains(PASSWORD), "{message}");
        Ok(())
    });
}

#[test]
fn errors_describe_values_without_showing_them() {
    jailed(|jail| {
        let check = |result: Result<Loaded, ConfigError>, key: &str| {
            let debug = format!("{result:?}");
            let message = read_error(result);
            assert!(!message.contains(PASSWORD), "{message}");
            assert!(!debug.contains(PASSWORD), "{debug}");
            assert!(message.contains(key), "{message}");
            message
        };

        // The URL set on the section instead of on its key.
        jail.set_env("NEON_DATABASE", URL);
        let message = check(load(), "NEON_DATABASE");
        assert!(
            message.starts_with(
                "invalid type: found a string, expected struct DatabaseConfig for key"
            ),
            "{message}"
        );
        jail.set_env("NEON_DATABASE", "");

        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", URL);
        check(load(), "NEON_DATABASE__MAX_CONNECTIONS");
        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", "");

        jail.set_env("NEON_SERVER__PUBLIC_URL", format!("neon {PASSWORD}"));
        check(load(), "NEON_SERVER__PUBLIC_URL");
        jail.set_env("NEON_SERVER__PUBLIC_URL", "");

        jail.set_env("NEON_UPDATES__GITHUB_API_URL", format!("api {PASSWORD}"));
        check(load(), "NEON_UPDATES__GITHUB_API_URL");
        jail.set_env("NEON_UPDATES__GITHUB_API_URL", "");

        jail.create_file("neon.yaml", &format!("database: {URL}\n"))?;
        check(load(), "database");

        jail.create_file("neon.yaml", &format!("database:\n  auto_migrate: {URL}\n"))?;
        check(load(), "database.auto_migrate");

        jail.create_file("neon.yaml", &format!("{URL}\n"))?;
        check(load(), "neon.yaml");
        Ok(())
    });
}

#[test]
fn the_release_check_asks_github_unless_told_to_ask_another() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);

        let loaded = load().expect("valid configuration");
        assert_eq!(loaded.config.updates, UpdatesConfig::default());
        assert_eq!(
            loaded.config.updates.github_api_url.as_str(),
            "https://api.github.com/"
        );
        let shown = loaded.render(&registry()).expect("renders");
        assert!(
            shown.contains("updates:\n  github_api_url: https://api.github.com/\n"),
            "{shown}"
        );

        jail.set_env(
            "NEON_UPDATES__GITHUB_API_URL",
            "http://receiver:8080/github",
        );
        let loaded = load().expect("valid configuration");
        assert_eq!(
            loaded.config.updates.github_api_url.as_str(),
            "http://receiver:8080/github"
        );

        for (url, message) in [
            (
                "ftp://mirror.example.com",
                "must be an http or https URL, like https://api.github.com",
            ),
            (
                "https://ci:hunter2@mirror.example.com",
                "must not hold a user name or password",
            ),
            (
                "https://mirror.example.com/?token=1",
                "must not hold a query or a fragment",
            ),
        ] {
            jail.set_env("NEON_UPDATES__GITHUB_API_URL", url);
            assert_eq!(
                invalid(load()),
                [Problem::new(
                    "updates.github_api_url (NEON_UPDATES__GITHUB_API_URL)",
                    message
                )],
                "{url}"
            );
        }
        Ok(())
    });
}

#[test]
fn yaml_syntax_errors_name_the_file() {
    jailed(|jail| {
        jail.create_file("neon.yaml", "server: [unclosed\n")?;

        let message = read_error(load());

        assert!(message.contains("neon.yaml"), "{message}");
        Ok(())
    });
}

#[test]
fn the_observability_section_is_read_from_the_file_and_the_environment() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            "observability:\n  logging:\n    level: debug\n  tracing:\n    sample_ratio: 0.5\n\
             \x20   trust_incoming: false\n  profiling:\n    mode: off\n",
        )?;
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_OBSERVABILITY__ADMIN__ADDR", "0.0.0.0:9191");
        jail.set_env("NEON_OBSERVABILITY__TRACING__TRUST_INCOMING", "true");
        jail.set_env("NEON_OBSERVABILITY__LOGGING__FORMAT", "pretty");
        jail.set_env("NEON_OBSERVABILITY__PROFILING__PUSH__TAGS__ENV", "prod");

        let loaded = load().expect("valid configuration");

        assert!(loaded.ignored_vars.is_empty(), "{:?}", loaded.ignored_vars);
        let observability = &loaded.config.observability;
        assert_eq!(observability.logging.level, "debug");
        assert_eq!(observability.logging.format, LogFormat::Pretty);
        assert!((observability.tracing.sample_ratio - 0.5).abs() < f64::EPSILON);
        assert!(observability.tracing.trust_incoming);
        assert_eq!(observability.profiling.mode, ProfilingMode::Off);
        assert_eq!(
            observability
                .profiling
                .push
                .tags
                .get("env")
                .map(String::as_str),
            Some("prod")
        );
        assert_eq!(observability.admin.addr, addr("0.0.0.0:9191"));
        assert!(observability.admin.enabled);
        Ok(())
    });
}

#[test]
fn observability_errors_are_reported_with_the_others() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", 1);
        jail.set_env("NEON_OBSERVABILITY__TRACING__SAMPLE_RATIO", 2);
        jail.set_env("NEON_OBSERVABILITY__PROFILING__MODE", "push");

        assert_eq!(
            invalid(load()),
            [
                Problem::new("database.url (NEON_DATABASE__URL)", "is required"),
                Problem::new(
                    "database.max_connections (NEON_DATABASE__MAX_CONNECTIONS)",
                    "must be at least 2: the job worker keeps one connection to listen \
                     for new jobs, and queries need at least one more"
                ),
                Problem::new(
                    "observability.tracing.sample_ratio \
                     (NEON_OBSERVABILITY__TRACING__SAMPLE_RATIO)",
                    "must be between 0 and 1, got 2"
                ),
                Problem::new(
                    "observability.profiling.push.server_address \
                     (NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS)",
                    "is required in push mode"
                ),
            ]
        );
        Ok(())
    });
}

#[test]
fn observability_keys_are_checked_like_the_others() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_OBSERVABILITY__ADMN__ADDR", "127.0.0.1:9191");

        let loaded = load().expect("valid configuration");
        assert_eq!(loaded.ignored_vars, ["NEON_OBSERVABILITY__ADMN__ADDR"]);
        assert_eq!(
            loaded.config.observability.admin.addr,
            addr("127.0.0.1:9090")
        );

        jail.create_file(
            "neon.yaml",
            "observability:\n  tracing:\n    sampel_ratio: 1\n",
        )?;
        let message = read_error(load());
        assert!(message.contains("sampel_ratio"), "{message}");

        jail.create_file("neon.yaml", "")?;
        jail.set_env("NEON_OBSERVABILITY__ADMIN__ADDR", "everywhere");
        let message = read_error(load());
        assert!(
            message.contains("NEON_OBSERVABILITY__ADMIN__ADDR"),
            "{message}"
        );
        Ok(())
    });
}

#[test]
fn observability_errors_do_not_show_urls() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_OBSERVABILITY__TRACING__ENDPOINT", URL);
        jail.set_env("NEON_OBSERVABILITY__PROFILING__MODE", "push");
        jail.set_env(
            "NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS",
            format!("https://neon:{PASSWORD}@pyroscope:4040"),
        );

        let result = load();
        let message = result.as_ref().expect_err("invalid").to_string();
        let keys: Vec<String> = invalid(result).into_iter().map(|p| p.key).collect();

        assert_eq!(
            keys,
            [
                "observability.tracing.endpoint (NEON_OBSERVABILITY__TRACING__ENDPOINT)",
                "observability.profiling.push.server_address \
                 (NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS)",
            ]
        );
        assert!(!message.contains(PASSWORD), "{message}");
        Ok(())
    });
}

#[test]
fn config_check_shows_the_observability_section_and_its_warnings() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env(
            "NEON_OBSERVABILITY__TRACING__ENDPOINT",
            "http://otel-lgtm:4318",
        );
        jail.set_env("NEON_OBSERVABILITY__TRACING__TRUST_INCOMING", "true");
        let registry = registry();

        let loaded = config::load(&registry).expect("valid configuration");
        let shown = loaded.render(&registry).expect("renders");

        assert!(
            shown.contains(
                "# Warning: observability.tracing.endpoint: is ignored while \
                 tracing.enabled is false.\n"
            ),
            "{shown}"
        );
        for line in [
            "observability:",
            "    level: info,sqlx::postgres::notice=warn,opentelemetry=warn",
            "    endpoint: http://otel-lgtm:4318",
            "    trust_incoming: true",
            "    addr: 127.0.0.1:9090",
        ] {
            assert!(
                shown.lines().any(|shown| shown == line),
                "no {line:?} in {shown}"
            );
        }
        Ok(())
    });
}

#[test]
fn config_check_hides_credentials_in_observability_urls() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_OBSERVABILITY__TRACING__ENABLED", "true");
        jail.set_env(
            "NEON_OBSERVABILITY__TRACING__ENDPOINT",
            format!("http://neon:{PASSWORD}@collector:4318"),
        );
        jail.set_env("NEON_OBSERVABILITY__PROFILING__MODE", "push");
        jail.set_env(
            "NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS",
            format!("http://{PASSWORD}@pyroscope:4040"),
        );
        let registry = registry();

        let loaded = config::load(&registry).expect("valid configuration");
        let shown = loaded.render(&registry).expect("renders");

        assert!(!shown.contains(PASSWORD), "{shown}");
        for line in [
            "    endpoint: http://neon:redacted@collector:4318/",
            "      server_address: http://redacted@pyroscope:4040/",
        ] {
            assert!(
                shown.lines().any(|shown| shown == line),
                "no {line:?} in {shown}"
            );
        }
        // Debug output, as an error or a log line could show it, redacts them the same way.
        let debug = format!("{loaded:?}");
        assert!(!debug.contains(PASSWORD), "{debug}");
        assert!(
            debug.contains(r#"endpoint: Some("http://neon:redacted@collector:4318/")"#),
            "{debug}"
        );
        assert!(
            debug.contains(r#"server_address: Some("http://redacted@pyroscope:4040/")"#),
            "{debug}"
        );
        Ok(())
    });
}

#[test]
fn secrets_are_redacted_when_the_configuration_is_shown() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_SETTINGS__INSTANCE__NAME", "Acme");
        let registry = registry();

        let loaded = config::load(&registry).expect("valid configuration");
        let shown = loaded.render(&registry).expect("renders");

        assert!(!format!("{loaded:?}").contains(PASSWORD));
        assert!(!format!("{:?}", loaded.config).contains(PASSWORD));
        assert!(!format!("{:?}", loaded.config.database).contains(PASSWORD));
        assert!(!format!("{:?}", loaded.config.database.url).contains(PASSWORD));
        assert!(!shown.contains(PASSWORD), "{shown}");
        assert!(
            shown.contains("postgres://neon:redacted@127.0.0.1:55432/neon"),
            "{shown}"
        );
        assert!(
            shown.contains(r#"instance.name: "Acme" # set by NEON_SETTINGS__INSTANCE__NAME"#),
            "{shown}"
        );
        Ok(())
    });
}

#[test]
fn the_example_file_holds_the_defaults() {
    jailed(|jail| {
        jail.create_file("neon.yaml", include_str!("../../../neon.example.yaml"))?;
        jail.set_env("NEON_DATABASE__URL", URL);

        let loaded = load().expect("the example file is valid");

        assert_eq!(loaded.config.server, ServerConfig::default());
        assert_eq!(loaded.config.admin, AdminConfig::default());
        assert_eq!(loaded.config.secrets, SecretsConfig::default());
        assert_eq!(loaded.config.updates, UpdatesConfig::default());
        assert_eq!(
            loaded.config.database,
            DatabaseConfig {
                url: loaded.config.database.url.clone(),
                ..DatabaseConfig::default()
            }
        );
        assert_eq!(loaded.config.observability, default_observability());
        assert_eq!(instance_name(&loaded).1, Source::Default);
        Ok(())
    });
}

/// A 32-byte key in standard base64.
const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

#[test]
fn the_first_admin_and_the_secret_keys_come_from_the_environment() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_ADMIN__EMAIL", "root@example.com");
        // Digits stay digits, and a leading zero stays: the password is read as typed.
        jail.set_env("NEON_ADMIN__PASSWORD", "000123456789012");
        jail.set_env("NEON_SECRETS__KEYS", format!("2026:{KEY}, 2025:{KEY}"));

        let loaded = load().expect("valid configuration");

        let admin = &loaded.config.admin;
        assert_eq!(admin.email.as_deref(), Some("root@example.com"));
        assert_eq!(
            admin.password.as_ref().map(|password| password.expose()),
            Some("000123456789012")
        );
        let keyring = loaded
            .config
            .secrets
            .keyring()
            .expect("valid keys")
            .expect("two keys");
        assert_eq!(keyring.ids().collect::<Vec<_>>(), ["2026", "2025"]);
        Ok(())
    });
}

#[test]
fn the_file_lists_secret_keys_and_trusted_proxies() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            &format!(
                "server:\n  trusted_proxies: [10.1.0.0/16, 192.0.2.7]\n\
                 database:\n  url: {URL}\nsecrets:\n  keys:\n    - k1:{KEY}\n"
            ),
        )?;

        let loaded = load().expect("valid configuration");

        let proxies: Vec<String> = loaded
            .config
            .server
            .trusted_proxies
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(proxies, ["10.1.0.0/16", "192.0.2.7/32"]);
        assert!(loaded.config.secrets.keyring().expect("valid").is_some());
        Ok(())
    });
}

#[test]
fn the_admin_needs_both_keys_and_malformed_secret_keys_are_named() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_ADMIN__EMAIL", "root@example.com");
        jail.set_env("NEON_SECRETS__KEYS", "k1:aGVsbG8=, missing-colon");

        let problems = invalid(load());

        assert_eq!(
            problems,
            [
                Problem::new(
                    "admin.password (NEON_ADMIN__PASSWORD)",
                    "is required with admin.email: set both or neither"
                ),
                Problem::new(
                    "secrets.keys (NEON_SECRETS__KEYS)",
                    "key \"k1\": want 32 bytes in standard base64, such as `openssl rand -base64 32` prints"
                ),
            ]
        );
        assert!(!format!("{problems:?}").contains("aGVsbG8"));
        Ok(())
    });
}

#[test]
fn the_admin_address_must_be_one() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_ADMIN__EMAIL", "root");
        jail.set_env("NEON_ADMIN__PASSWORD", "hunter2hunter2");

        let problems = invalid(load());

        assert_eq!(
            problems,
            [Problem::new(
                "admin.email (NEON_ADMIN__EMAIL)",
                "is not an email address like name@example.com"
            )]
        );
        Ok(())
    });
}

#[test]
fn config_check_hides_the_admin_password_and_the_key_material() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_ADMIN__EMAIL", "root@example.com");
        jail.set_env("NEON_ADMIN__PASSWORD", "hunter2hunter2");
        jail.set_env("NEON_SECRETS__KEYS", format!("2026:{KEY}"));
        let registry = registry();

        let loaded = config::load(&registry).expect("valid configuration");
        let shown = loaded.render(&registry).expect("renders");

        for secret in ["hunter2", KEY] {
            assert!(!shown.contains(secret), "{shown}");
            assert!(!format!("{loaded:?}").contains(secret));
        }
        assert!(shown.contains("root@example.com"), "{shown}");
        assert!(shown.contains("2026:[redacted]"), "{shown}");
        Ok(())
    });
}
