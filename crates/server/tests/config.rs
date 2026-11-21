//! Loading the configuration from the defaults, the YAML file and the environment.
//!
//! These tests sit in their own binary: `figment::Jail` changes the process's working
//! directory and environment, which tests running beside them would see.

// `figment::Jail` takes closures returning `figment::Result`, whose error is large.
#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::Jail;
use neon_app::settings::{INSTANCE_NAME, registry};
use neon_domain::settings::{Problem, Source, Stored};
use neon_server::config::{self, ConfigError, DatabaseConfig, Loaded, ServerConfig};

const URL: &str = "postgres://neon:hunter2@127.0.0.1:55432/neon";

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

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("a socket address")
}

#[test]
fn defaults_apply_without_a_file() {
    jailed(|jail| {
        jail.set_env("NEON_DATABASE__URL", URL);

        let loaded = load().expect("valid configuration");

        assert_eq!(loaded.file, None);
        assert_eq!(loaded.config.server.listen, addr("0.0.0.0:8080"));
        assert_eq!(loaded.config.server.public_url, None);
        assert_eq!(loaded.config.database.url.expose(), URL);
        assert_eq!(loaded.config.database.max_connections, 10);
        assert!(loaded.config.database.auto_migrate);
        assert_eq!(
            instance_name(&loaded),
            ("Neon Workspace".to_owned(), Source::Default)
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
            ("Neon Workspace".to_owned(), Source::Default)
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
                    "must be at least 1"
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

        jail.set_env("NEON_DATABASE__MAX_CONNECTIONS", "");
        jail.set_env("NEON_DATABSE__URL", URL);
        let message = read_error(load());
        assert!(message.contains("databse"), "{message}");
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
fn the_observability_section_is_accepted() {
    jailed(|jail| {
        jail.create_file(
            "neon.yaml",
            "observability:\n  logging:\n    level: debug\n",
        )?;
        jail.set_env("NEON_DATABASE__URL", URL);
        jail.set_env("NEON_OBSERVABILITY__ADMIN__ADDR", "127.0.0.1:9090");

        load().expect("valid configuration");
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

        assert!(!format!("{loaded:?}").contains("hunter2"));
        assert!(!shown.contains("hunter2"), "{shown}");
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
        assert_eq!(
            loaded.config.database,
            DatabaseConfig {
                url: loaded.config.database.url.clone(),
                ..DatabaseConfig::default()
            }
        );
        assert_eq!(instance_name(&loaded).1, Source::Default);
        Ok(())
    });
}
