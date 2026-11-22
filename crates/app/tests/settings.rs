use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_app::crypto::{Envelope, Keyring};
use neon_app::settings::{Change, SettingsService, UpdateError};
use neon_domain::settings::{
    Definition, Key, Pinned, Problem, REDACTED, RawLayers, Registry, Secret, Source, checks,
};
use serde_json::{Value as Json, json};
use sqlx::PgPool;
use uuid::Uuid;

const NAME: Key<str> = Key::new("instance.name");
const VERIFY: Key<bool> = Key::new("auth.verify_email");
const HOST: Key<str> = Key::new("mail.host");
const PASSWORD: Key<Secret> = Key::new("mail.password");
const LIMIT: Key<i64> = Key::new("workspaces.limit");
const IDLE: Key<Duration> = Key::new("auth.session_idle");

/// Settings shaped like the server's, with a rule across two of them.
fn registry() -> Registry {
    Registry::builder()
        .define(Definition::string(NAME, "Neon Workspace", "The name.").check(checks::non_blank))
        .define(Definition::bool(VERIFY, false, "Verify sign-ups by mail."))
        .define(Definition::string(HOST, "", "The mail server."))
        .define(Definition::secret(PASSWORD, "The mail server's password."))
        .define(Definition::integer(LIMIT, 10, "Workspaces per user."))
        .define(Definition::duration(
            IDLE,
            Duration::from_secs(3600),
            "Idle sign-out.",
        ))
        .rule(VERIFY.name(), |s| {
            if *s.get(VERIFY) && s.get(HOST).is_empty() {
                Err("needs a mail server: set mail.host first".to_owned())
            } else {
                Ok(())
            }
        })
        .build()
        .expect("valid registry")
}

fn pinned(env: &[(&str, &str)], yaml: Json) -> Pinned {
    let Json::Object(tree) = yaml else {
        panic!("the YAML settings are a mapping")
    };
    let raw = RawLayers::default()
        .with_env(env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())))
        .with_yaml(&tree);
    registry().pin(&raw).expect("valid layers")
}

/// An envelope with one key of `byte` repeated.
fn envelope(byte: u8) -> Envelope {
    let entry = format!("k1:{}", STANDARD.encode([byte; 32]));
    Envelope::new(Keyring::parse(&[entry]).expect("a valid key"))
}

async fn service(pool: &PgPool, pinned: Pinned) -> SettingsService {
    service_with(pool, pinned, envelope(1)).await
}

async fn service_with(pool: &PgPool, pinned: Pinned, envelope: Envelope) -> SettingsService {
    let (service, warnings) = SettingsService::load(pool.clone(), registry(), pinned, envelope)
        .await
        .expect("settings load");
    assert!(warnings.is_empty(), "{warnings:?}");
    service
}

fn set(key: &str, value: Json) -> (String, Change) {
    (key.to_owned(), Change::Set(value))
}

fn invalid(result: Result<impl std::fmt::Debug, UpdateError>) -> Vec<Problem> {
    match result {
        Err(UpdateError::Invalid(problems)) => problems,
        other => panic!("expected invalid settings, got {other:?}"),
    }
}

async fn saved(pool: &PgPool) -> Vec<(String, Json)> {
    sqlx::query_as("SELECT key, value FROM settings ORDER BY key")
        .fetch_all(pool)
        .await
        .expect("saved settings")
}

async fn audit(pool: &PgPool) -> Vec<(String, Json, Json, Option<Uuid>)> {
    sqlx::query_as("SELECT key, old_value, new_value, changed_by FROM settings_audit ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("audit rows")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn values_resolve_env_then_yaml_then_database_then_default(pool: PgPool) {
    for (key, value) in [
        ("instance.name", json!("From the database")),
        ("mail.host", json!("db.example.com")),
        ("workspaces.limit", json!(25)),
    ] {
        neon_store::settings::upsert(&pool, key, &value, None)
            .await
            .unwrap();
    }

    let service = service(
        &pool,
        pinned(
            &[("NEON_SETTINGS__INSTANCE__NAME", "From the environment")],
            json!({"instance": {"name": "From YAML"}, "mail": {"host": "yaml.example.com"}}),
        ),
    )
    .await;

    let current = service.current();
    let source = |key: &str| current.resolved(key).map(|r| r.source);
    assert_eq!(current.get(NAME), "From the environment");
    assert_eq!(source("instance.name"), Some(Source::Env));
    assert_eq!(current.get(HOST), "yaml.example.com");
    assert_eq!(source("mail.host"), Some(Source::Yaml));
    assert_eq!(*current.get(LIMIT), 25);
    assert_eq!(source("workspaces.limit"), Some(Source::Database));
    assert_eq!(*current.get(IDLE), Duration::from_secs(3600));
    assert_eq!(source("auth.session_idle"), Some(Source::Default));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn saved_values_that_no_longer_parse_are_warnings(pool: PgPool) {
    neon_store::settings::upsert(&pool, "workspaces.limit", &json!("ten"), None)
        .await
        .unwrap();
    neon_store::settings::upsert(&pool, "removed.setting", &json!(1), None)
        .await
        .unwrap();

    let (service, warnings) =
        SettingsService::load(pool.clone(), registry(), Pinned::default(), envelope(1))
            .await
            .unwrap();

    assert_eq!(*service.current().get(LIMIT), 10);
    assert_eq!(
        warnings,
        [
            Problem::new("removed.setting", "saved value ignored: no such setting"),
            Problem::new(
                "workspaces.limit",
                "saved value ignored: want a whole number"
            ),
        ]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn pinned_settings_cannot_be_written(pool: PgPool) {
    let service = service(
        &pool,
        pinned(
            &[("NEON_SETTINGS__INSTANCE__NAME", "Pinned")],
            json!({"mail": {"host": "yaml.example.com"}}),
        ),
    )
    .await;

    let problems = invalid(
        service
            .update(
                [
                    set("instance.name", json!("Changed")),
                    ("mail.host".to_owned(), Change::Reset),
                ],
                None,
            )
            .await,
    );

    assert_eq!(
        problems,
        [
            Problem::new(
                "instance.name",
                "set by the environment variable NEON_SETTINGS__INSTANCE__NAME; change it there"
            ),
            Problem::new(
                "mail.host",
                "set in the config file as settings.mail.host; change it there"
            ),
        ]
    );
    assert_eq!(service.current().get(NAME), "Pinned");
    assert!(saved(&pool).await.is_empty());
    assert!(audit(&pool).await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_write_saves_the_value_and_its_audit_row_and_applies_at_once(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;
    let admin = Uuid::now_v7();

    let snapshot = service
        .update([set("instance.name", json!("Acme"))], Some(admin))
        .await
        .unwrap();

    assert_eq!(snapshot.get(NAME), "Acme");
    assert_eq!(service.current().get(NAME), "Acme");
    assert_eq!(
        service.current().resolved("instance.name").unwrap().source,
        Source::Database
    );
    assert_eq!(
        saved(&pool).await,
        [("instance.name".into(), json!("Acme"))]
    );
    assert_eq!(
        audit(&pool).await,
        [(
            "instance.name".into(),
            json!("Neon Workspace"),
            json!("Acme"),
            Some(admin)
        )]
    );

    // Saving the same value again changes nothing and records nothing.
    service
        .update([set("instance.name", json!("Acme"))], Some(admin))
        .await
        .unwrap();
    assert_eq!(audit(&pool).await.len(), 1);

    service
        .update([("instance.name".to_owned(), Change::Reset)], None)
        .await
        .unwrap();
    assert_eq!(service.current().get(NAME), "Neon Workspace");
    assert!(saved(&pool).await.is_empty());
    assert_eq!(
        audit(&pool).await[1],
        (
            "instance.name".into(),
            json!("Acme"),
            json!("Neon Workspace"),
            None
        )
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_failed_audit_write_rolls_back_the_value(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;
    sqlx::query("ALTER TABLE settings_audit ADD CONSTRAINT refuse_all CHECK (false) NOT VALID")
        .execute(&pool)
        .await
        .unwrap();

    let result = service
        .update([set("instance.name", json!("Acme"))], None)
        .await;

    assert!(
        matches!(result, Err(UpdateError::Database(_))),
        "{result:?}"
    );
    assert!(saved(&pool).await.is_empty());
    assert_eq!(service.current().get(NAME), "Neon Workspace");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_invalid_value_is_reported_and_nothing_is_saved(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;

    let problems = invalid(
        service
            .update(
                [
                    set("instance.name", json!("  ")),
                    set("workspaces.limit", json!("many")),
                    set("auth.session_idle", json!("2h")),
                    set("no.such", json!(1)),
                ],
                None,
            )
            .await,
    );

    assert_eq!(
        problems,
        [
            Problem::new("instance.name", "must not be empty"),
            Problem::new("no.such", "no such setting"),
            Problem::new("workspaces.limit", "want a whole number"),
        ]
    );
    assert!(saved(&pool).await.is_empty());
    assert_eq!(*service.current().get(IDLE), Duration::from_secs(3600));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn rules_check_the_settings_as_they_would_be(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;

    let refused = invalid(
        service
            .update([set("auth.verify_email", json!(true))], None)
            .await,
    );
    assert_eq!(
        refused,
        [Problem::new(
            "auth.verify_email",
            "needs a mail server: set mail.host first"
        )]
    );
    assert!(saved(&pool).await.is_empty());

    service
        .update(
            [
                set("auth.verify_email", json!(true)),
                set("mail.host", json!("smtp.example.com")),
            ],
            None,
        )
        .await
        .unwrap();
    assert!(*service.current().get(VERIFY));

    let refused = invalid(
        service
            .update([("mail.host".to_owned(), Change::Reset)], None)
            .await,
    );
    assert_eq!(refused[0].key, "auth.verify_email");
    assert_eq!(service.current().get(HOST), "smtp.example.com");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn secrets_are_redacted_in_the_audit_and_for_display(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;

    service
        .update([set("mail.password", json!("hunter2"))], None)
        .await
        .unwrap();

    assert_eq!(service.current().get(PASSWORD).expose(), "hunter2");
    assert!(!format!("{:?}", service.current()).contains("hunter2"));
    assert_eq!(
        audit(&pool).await,
        [("mail.password".into(), json!(""), json!(REDACTED), None)]
    );
    let entry = service
        .entries()
        .into_iter()
        .find(|entry| entry.key == "mail.password")
        .unwrap();
    assert_eq!(entry.value, json!(REDACTED));
    assert_eq!(entry.env_var, "NEON_SETTINGS__MAIL__PASSWORD");
    assert_eq!(entry.source, Source::Database);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn secrets_are_saved_sealed_and_read_back(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;

    service
        .update([set("mail.password", json!("hunter2"))], None)
        .await
        .unwrap();

    let rows = saved(&pool).await;
    assert_eq!(rows.len(), 1);
    let stored = rows[0].1.to_string();
    assert!(!stored.contains("hunter2"), "{stored}");
    assert!(rows[0].1["sealed"].is_string(), "{stored}");

    let reloaded = service_with(&pool, Pinned::default(), envelope(1)).await;
    assert_eq!(reloaded.current().get(PASSWORD).expose(), "hunter2");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn secrets_are_refused_without_a_key(pool: PgPool) {
    let service = service_with(&pool, Pinned::default(), Envelope::default()).await;

    let problems = invalid(
        service
            .update(
                [
                    set("mail.password", json!("hunter2")),
                    set("instance.name", json!("Acme")),
                ],
                None,
            )
            .await,
    );

    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "mail.password");
    assert!(
        problems[0]
            .message
            .contains("set secrets.keys (NEON_SECRETS__KEYS) first"),
        "{}",
        problems[0].message
    );
    assert!(saved(&pool).await.is_empty());
    assert_eq!(service.current().get(NAME), "Neon Workspace");

    // An empty secret hides nothing, so it needs no key.
    service
        .update([set("mail.password", json!(""))], None)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn secrets_that_do_not_open_are_warnings(pool: PgPool) {
    let service = service(&pool, Pinned::default()).await;
    service
        .update([set("mail.password", json!("hunter2"))], None)
        .await
        .unwrap();

    let (other_key, warnings) =
        SettingsService::load(pool.clone(), registry(), Pinned::default(), envelope(2))
            .await
            .unwrap();
    assert_eq!(other_key.current().get(PASSWORD).expose(), "");
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .message
            .starts_with("saved value ignored: it does not decrypt")
    );

    let (no_key, warnings) = SettingsService::load(
        pool.clone(),
        registry(),
        Pinned::default(),
        Envelope::default(),
    )
    .await
    .unwrap();
    assert_eq!(no_key.current().get(PASSWORD).expose(), "");
    assert_eq!(warnings[0].key, "mail.password");

    // A secret written in the clear, past this service, is never used.
    neon_store::settings::upsert(&pool, "mail.password", &json!("in the clear"), None)
        .await
        .unwrap();
    let (_, warnings) =
        SettingsService::load(pool.clone(), registry(), Pinned::default(), envelope(1))
            .await
            .unwrap();
    assert_eq!(
        warnings,
        [Problem::new(
            "mail.password",
            "saved value ignored: a secret saved without encryption is not used"
        )]
    );
    for warning in &warnings {
        assert!(!warning.message.contains("hunter2"));
    }
}
