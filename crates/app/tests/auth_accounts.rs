//! Signing up, verifying addresses, changing and resetting passwords.

mod support;

use std::time::Duration;

use neon_app::auth::{AuthError, InviteSpec, LoginInput, RegisterInput};
use neon_app::mail::Message;
use neon_app::ratelimit::Bucket;
use neon_domain::auth::{Role, WeakPassword};
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, IP, Options, PASSWORD, renaming, secret};

fn register(email: &str, code: Option<&str>) -> RegisterInput {
    RegisterInput {
        email: email.to_owned(),
        password: secret(PASSWORD),
        display_name: None,
        invite_code: code.map(secret),
        ip: IP.to_owned(),
    }
}

fn login(email: &str, password: &str) -> LoginInput {
    LoginInput {
        email: email.to_owned(),
        password: secret(password),
        ip: IP.to_owned(),
        user_agent: "tests".to_owned(),
    }
}

async fn open(pool: PgPool) -> Harness {
    Harness::with(
        pool,
        Options {
            settings: vec![("auth.registration", json!("open"))],
            ..Options::default()
        },
    )
    .await
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn open_registration_makes_a_verified_account_with_an_argon2id_hash(pool: PgPool) {
    let h = open(pool).await;

    let registration = h
        .auth
        .register(RegisterInput {
            display_name: Some("  Ada Lovelace ".into()),
            ..register(" Ada@Example.com ", None)
        })
        .await
        .expect("registers");

    let user = &registration.user;
    assert_eq!(user.address(), "Ada@Example.com");
    assert_eq!(user.display_name, "Ada Lovelace");
    assert_eq!(user.role, Role::User);
    assert!(user.is_verified());
    assert!(!registration.must_verify);
    assert!(h.mails().await.is_empty());
    let hash: String = h.scalar("SELECT password_hash FROM users").await;
    assert!(hash.starts_with("$argon2id$v=19$"), "{hash}");
    assert!(!hash.contains(PASSWORD));
    let normalized: String = h.scalar("SELECT email_normalized FROM users").await;
    assert_eq!(normalized, "ada@example.com");
    h.sign_in("ada@example.com").await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn closed_registration_refuses_everyone(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.registration", json!("closed")).await;

    let error = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect_err("closed");

    assert!(matches!(error, AuthError::RegistrationClosed), "{error:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invite_registration_takes_one_use_of_an_open_invite(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let created = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                code: Some(secret("Spring-Team-2026")),
                max_uses: Some(2),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");

    for code in [None, Some(""), Some("no-such-code"), Some(&"x".repeat(200))] {
        let error = h
            .auth
            .register(register("ada@example.com", code))
            .await
            .expect_err("no invite");
        assert!(
            matches!(error, AuthError::InvalidInvite),
            "{code:?}: {error:?}"
        );
    }

    // Codes match whatever their case.
    let ada = h
        .auth
        .register(register("ada@example.com", Some("spring-team-2026")))
        .await
        .expect("registers");
    h.auth
        .register(register("grace@example.com", Some("SPRING-TEAM-2026")))
        .await
        .expect("registers");
    let error = h
        .auth
        .register(register("linus@example.com", Some(created.code.expose())))
        .await
        .expect_err("used up");
    assert!(matches!(error, AuthError::InvalidInvite), "{error:?}");

    let uses: i32 = h.scalar("SELECT uses FROM invites").await;
    assert_eq!(uses, 2);
    let first: uuid::Uuid = h
        .scalar("SELECT user_id FROM invite_uses ORDER BY used_at LIMIT 1")
        .await;
    assert_eq!(first, ada.user.id);
    let accounts: i64 = h.scalar("SELECT count(*) FROM users").await;
    assert_eq!(accounts, 3);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sign_ups_racing_for_an_invite_s_last_use_let_one_in(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let created = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("a single-use invite");

    let racing: Vec<_> = (0..12)
        .map(|n| {
            let auth = h.auth.clone();
            let code = created.code.expose().to_owned();
            tokio::spawn(async move {
                auth.register(register(&format!("racer{n}@example.com"), Some(&code)))
                    .await
            })
        })
        .collect();
    let mut admitted = 0;
    for sign_up in racing {
        match sign_up.await.expect("the sign-up ends") {
            Ok(_) => admitted += 1,
            Err(AuthError::InvalidInvite) => {}
            Err(error) => panic!("only the invite may refuse: {error:?}"),
        }
    }

    assert_eq!(admitted, 1);
    let uses: i32 = h.scalar("SELECT uses FROM invites").await;
    let recorded: i64 = h.scalar("SELECT count(*) FROM invite_uses").await;
    let users: i64 = h
        .scalar("SELECT count(*) FROM users WHERE email LIKE 'racer%'")
        .await;
    assert_eq!((uses, recorded, users), (1, 1, 1));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_bound_invite_admits_only_its_address_which_counts_as_verified(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.verify_email", json!(true)).await;
    let admin = h.admin().await;
    let created = h
        .auth
        .invite_by_email(&admin, "Ada@Example.com")
        .await
        .expect("mailed");
    let code = created.code.expose().to_owned();

    let error = h
        .auth
        .register(register("grace@example.com", Some(&code)))
        .await
        .expect_err("for another address");
    assert!(matches!(error, AuthError::InviteForOtherEmail), "{error:?}");
    assert!(!error.to_string().contains("ada@"), "{error}");

    let registration = h
        .auth
        .register(register("ada@example.COM", Some(&code)))
        .await
        .expect("registers");
    assert!(!registration.must_verify);
    assert!(registration.user.is_verified());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn verifying_sign_ups_must_open_the_mailed_link_before_signing_in(pool: PgPool) {
    let h = open(pool).await;
    h.set("auth.verify_email", json!(true)).await;

    let registration = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect("registers");
    assert!(registration.must_verify);
    assert!(!registration.user.is_verified());

    let wrong = h
        .auth
        .login(login("ada@example.com", "wrong password!"))
        .await
        .expect_err("wrong");
    assert!(matches!(wrong, AuthError::InvalidCredentials), "{wrong:?}");
    let unverified = h
        .auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect_err("unverified");
    assert!(
        matches!(unverified, AuthError::EmailNotVerified),
        "{unverified:?}"
    );

    let token = h.token_of("verification").await;
    assert_eq!(token.expose().len(), 43);
    let email = h.auth.verify_email(&token, IP).await.expect("verifies");
    assert_eq!(email, "ada@example.com");
    let again = h.auth.verify_email(&token, IP).await.expect_err("used");
    assert!(matches!(again, AuthError::InvalidLink), "{again:?}");
    h.sign_in("ada@example.com").await;

    let stored: i64 = h
        .scalar("SELECT count(*) FROM email_tokens WHERE used_at IS NOT NULL")
        .await;
    assert_eq!(stored, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_mail_sign_ups_do_not_verify(pool: PgPool) {
    // The settings set mail up, but with no public URL to link to, it cannot go.
    let h = Harness::with(
        pool,
        Options {
            public_url: None,
            settings: vec![
                ("auth.registration", json!("open")),
                ("auth.verify_email", json!(true)),
            ],
            ..Options::default()
        },
    )
    .await;

    let registration = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect("registers");

    assert!(!registration.must_verify && registration.user.is_verified());
    assert!(!h.auth.config().verify_email);
    assert!(h.mails().await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn verifying_sign_ups_cannot_be_turned_on_without_mail(pool: PgPool) {
    let h = open(pool).await;
    h.set("mail.enabled", json!(false)).await;

    let refused = h
        .settings
        .update(
            [(
                "auth.verify_email".to_owned(),
                neon_app::settings::Change::Set(json!(true)),
            )],
            None,
        )
        .await
        .expect_err("needs mail");

    assert!(
        refused.to_string().contains("needs mail set up first"),
        "{refused}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_sign_up_that_fails_queues_no_mail(pool: PgPool) {
    let h = open(pool).await;
    h.set("auth.verify_email", json!(true)).await;
    h.account("grace@example.com", Role::User).await;

    let error = h
        .auth
        .register(register("Grace@example.com", None))
        .await
        .expect_err("taken");

    assert!(matches!(error, AuthError::EmailTaken), "{error:?}");
    let links: i64 = h.scalar("SELECT count(*) FROM email_tokens").await;
    assert_eq!(links, 0);
    assert!(h.mails().await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn verification_links_expire_after_a_day(pool: PgPool) {
    let h = open(pool).await;
    h.set("auth.verify_email", json!(true)).await;
    h.auth
        .register(register("ada@example.com", None))
        .await
        .expect("registers");
    let token = h.token_of("verification").await;

    h.time.advance(Duration::from_secs(24 * 3600));
    let error = h.auth.verify_email(&token, IP).await.expect_err("expired");

    assert!(matches!(error, AuthError::InvalidLink), "{error:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn links_are_mailed_again_at_most_once_a_minute_and_five_times_a_day(pool: PgPool) {
    let h = open(pool).await;
    h.set("auth.verify_email", json!(true)).await;
    h.auth
        .register(register("ada@example.com", None))
        .await
        .expect("registers");
    let first = h.token_of("verification").await;
    let resend = || async {
        h.auth
            .resend_verification("ada@example.com", IP)
            .await
            .expect("answers");
        h.mails().await.len()
    };

    // Within the minute: nothing more.
    assert_eq!(resend().await, 0);
    for _ in 0..4 {
        h.time.advance(Duration::from_secs(61));
        assert_eq!(resend().await, 1);
    }
    h.time.advance(Duration::from_secs(61));
    assert_eq!(resend().await, 0, "five links a day, the first included");

    // The answer is the same for an address without an account.
    h.auth
        .resend_verification("nobody@example.com", IP)
        .await
        .expect("answers");
    // Any link not used or expired still works, and using one ends the others.
    h.auth.verify_email(&first, IP).await.expect("verifies");
    let unused: i64 = h
        .scalar("SELECT count(*) FROM email_tokens WHERE used_at IS NULL")
        .await;
    assert_eq!(unused, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn signing_up_again_replaces_an_unverified_account(pool: PgPool) {
    let h = open(pool).await;
    h.set("auth.verify_email", json!(true)).await;
    let squatter = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect("registers");

    let owner = h
        .auth
        .register(register("ADA@example.com", None))
        .await
        .expect("registers");

    assert_ne!(owner.user.id, squatter.user.id);
    let accounts: i64 = h.scalar("SELECT count(*) FROM users").await;
    assert_eq!(accounts, 1);

    // A verified account keeps its address.
    h.auth
        .verify_email(&h.token_of("verification").await, IP)
        .await
        .expect("verifies");
    let error = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect_err("taken");
    assert!(matches!(error, AuthError::EmailTaken), "{error:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn new_passwords_follow_the_policy(pool: PgPool) {
    let h = open(pool).await;

    let short = h
        .auth
        .register(RegisterInput {
            password: secret("short pw"),
            ..register("ada@example.com", None)
        })
        .await
        .expect_err("too short");
    assert!(
        matches!(short, AuthError::WeakPassword(WeakPassword::TooShort(12))),
        "{short:?}"
    );

    h.set("auth.min_password_length", json!(8)).await;
    h.auth
        .register(RegisterInput {
            password: secret("short pw"),
            ..register("ada@example.com", None)
        })
        .await
        .expect("long enough now");

    let invalid = h
        .auth
        .register(register("not an address", None))
        .await
        .expect_err("invalid");
    assert!(matches!(invalid, AuthError::InvalidEmail(_)), "{invalid:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sign_ups_are_rate_limited_by_ip(pool: PgPool) {
    let h = open(pool).await;
    h.limiter.refuse(Bucket::Register);

    let error = h
        .auth
        .register(register("ada@example.com", None))
        .await
        .expect_err("limited");

    assert!(
        matches!(error, AuthError::RateLimited { retry_after } if retry_after == Duration::from_secs(30)),
        "{error:?}"
    );
    assert_eq!(h.limiter.seen(), [(Bucket::Register, IP.to_owned())]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn changing_the_password_signs_out_the_other_sessions(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (current, current_token) = h.sign_in("ada@example.com").await;
    let (_, other_token) = h.sign_in("ada@example.com").await;

    let wrong = h
        .auth
        .change_password(
            &current,
            &secret("not my password"),
            &secret("a new passphrase"),
        )
        .await
        .expect_err("wrong");
    assert!(matches!(wrong, AuthError::InvalidCredentials), "{wrong:?}");
    let weak = h
        .auth
        .change_password(&current, &secret(PASSWORD), &secret("short"))
        .await
        .expect_err("weak");
    assert!(matches!(weak, AuthError::WeakPassword(_)), "{weak:?}");

    h.auth
        .change_password(&current, &secret(PASSWORD), &secret("a new passphrase"))
        .await
        .expect("changes");
    h.auth.settle().await;

    assert!(
        h.auth
            .authenticate_session(current_token.expose())
            .await
            .is_ok()
    );
    assert!(matches!(
        h.auth.authenticate_session(other_token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(
        h.auth
            .login(login("ada@example.com", "a new passphrase"))
            .await
            .is_ok()
    );
    let mails = h.mails().await;
    assert!(
        mails.iter().any(
            |mail| matches!(mail.message, Message::PasswordChanged { .. })
                && mail.to.address == "ada@example.com"
        ),
        "{mails:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn tokens_cannot_change_passwords(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (session, _) = h.sign_in("ada@example.com").await;
    let (token, _) = h
        .auth
        .create_api_token(&session, "ci", None, &["write"], None)
        .await
        .expect("a token");
    let principal = h
        .auth
        .authenticate_token(token.expose())
        .await
        .expect("works");

    let error = h
        .auth
        .change_password(&principal, &secret(PASSWORD), &secret("a new passphrase"))
        .await
        .expect_err("token");

    assert!(matches!(error, AuthError::Forbidden(_)), "{error:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_reset_link_sets_the_password_verifies_and_signs_out_everywhere(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::User).await;
    let (_, session) = h.sign_in("ada@example.com").await;
    // Resetting verifies an address that was not.
    sqlx::query("UPDATE users SET email_verified_at = NULL WHERE id = $1")
        .bind(ada.id)
        .execute(&h.pool)
        .await
        .unwrap();

    // The same answer whether the address has an account or not.
    h.auth
        .request_password_reset("nobody@example.com", IP)
        .await
        .expect("answers");
    h.auth
        .request_password_reset("ADA@example.com", IP)
        .await
        .expect("answers");
    let token = h.token_of("password_reset").await;

    let weak = h
        .auth
        .reset_password(&token, &secret("short"), IP)
        .await
        .expect_err("weak");
    assert!(matches!(weak, AuthError::WeakPassword(_)), "{weak:?}");
    let email = h
        .auth
        .reset_password(&token, &secret("a brand new passphrase"), IP)
        .await
        .expect("the link still works after a weak password");
    assert_eq!(email, "ada@example.com");
    h.auth.settle().await;

    assert!(matches!(
        h.auth
            .reset_password(&token, &secret("another new passphrase"), IP)
            .await,
        Err(AuthError::InvalidLink)
    ));
    assert!(matches!(
        h.auth.authenticate_session(session.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
    let verified: bool = h
        .scalar("SELECT email_verified_at IS NOT NULL FROM users")
        .await;
    assert!(verified);
    assert!(
        h.auth
            .login(login("ada@example.com", "a brand new passphrase"))
            .await
            .is_ok()
    );
    assert!(
        h.mails()
            .await
            .iter()
            .any(|mail| matches!(mail.message, Message::PasswordChanged { .. }))
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn reset_links_last_an_hour_and_skip_disabled_accounts(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::User).await;
    h.auth
        .request_password_reset("ada@example.com", IP)
        .await
        .expect("answers");
    let token = h.token_of("password_reset").await;

    h.time.advance(Duration::from_secs(3600));
    assert!(matches!(
        h.auth
            .reset_password(&token, &secret("a brand new passphrase"), IP)
            .await,
        Err(AuthError::InvalidLink)
    ));

    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(ada.id)
        .execute(&h.pool)
        .await
        .unwrap();
    h.auth
        .request_password_reset("ada@example.com", IP)
        .await
        .expect("answers");
    assert!(h.mails().await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn resets_are_off_without_mail_or_the_setting(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.password_reset", json!(false)).await;
    assert!(matches!(
        h.auth.request_password_reset("ada@example.com", IP).await,
        Err(AuthError::PasswordResetOff)
    ));

    h.set("auth.password_reset", json!(true)).await;
    h.set("mail.enabled", json!(false)).await;
    assert!(matches!(
        h.auth
            .reset_password(&secret("token"), &secret(PASSWORD), IP)
            .await,
        Err(AuthError::PasswordResetOff)
    ));
    assert!(!h.auth.config().password_reset);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn owners_rename_their_accounts_and_keep_their_preferences(pool: PgPool) {
    use neon_domain::auth::Preferences;
    use neon_domain::auth::preferences::{Mode, Palette, UiFont};

    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;

    let renamed = h
        .auth
        .update_profile(&ada, renaming(" Ada Lovelace "))
        .await
        .expect("renames");
    assert_eq!(renamed.display_name, "Ada Lovelace");
    let error = h
        .auth
        .update_profile(&ada, renaming(&"x".repeat(101)))
        .await
        .expect_err("too long");
    assert!(matches!(error, AuthError::InvalidInput(_)), "{error:?}");

    assert_eq!(
        h.auth.preferences(&ada).await.expect("reads"),
        Preferences::default()
    );
    let chosen =
        Preferences::new(Palette::Warm, Mode::Dark, Some(25), UiFont::Mono).expect("valid");
    h.auth.set_preferences(&ada, chosen).await.expect("saves");
    assert_eq!(h.auth.preferences(&ada).await.expect("reads"), chosen);
}
