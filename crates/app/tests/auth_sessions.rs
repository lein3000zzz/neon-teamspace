//! Signing in, sessions and API tokens.

mod support;

use std::time::{Duration, Instant};

use neon_app::auth::{AuthError, LoginInput, UserUpdate};
use neon_app::crypto::{Argon2Params, hash_secret};
use neon_app::pages::AccessChange;
use neon_app::ratelimit::Bucket;
use neon_domain::auth::{Credential, Role, Scope};
use serde_json::json;
use sqlx::PgPool;
use support::{CHEAP, Harness, IP, Options, PASSWORD, secret};

fn login(email: &str, password: &str) -> LoginInput {
    LoginInput {
        email: email.to_owned(),
        password: secret(password),
        ip: IP.to_owned(),
        user_agent: "Mozilla/5.0 (tests)".to_owned(),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn signing_in_opens_a_session_kept_only_as_a_hash(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::User).await;

    let outcome = h
        .auth
        .login(login(" ADA@example.com ", PASSWORD))
        .await
        .expect("signs in");

    assert_eq!(outcome.user.id, ada.id);
    assert_eq!(outcome.token.expose().len(), 43, "256 bits, base64url");
    let session = &outcome.session;
    assert_eq!(
        session.expires_at,
        support::start() + Duration::from_secs(7 * 24 * 3600)
    );
    assert_eq!(
        session.absolute_expires_at,
        support::start() + Duration::from_secs(30 * 24 * 3600)
    );
    assert_eq!(session.ip, IP);
    assert_eq!(session.user_agent, "Mozilla/5.0 (tests)");
    let stored: Vec<u8> = h.scalar("SELECT token_hash FROM sessions").await;
    assert_eq!(stored, hash_secret(outcome.token.expose()));

    let principal = h
        .auth
        .authenticate_session(outcome.token.expose())
        .await
        .expect("works");
    assert_eq!(principal.user.id, ada.id);
    assert_eq!(principal.credential, Credential::Session { id: session.id });
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn unknown_addresses_and_wrong_passwords_look_the_same(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            params: Argon2Params::DEFAULT,
            ..Options::default()
        },
    )
    .await;
    // The first sign-in upgrades the cheap test hash to the parameters the dummy hash
    // has, as every account's is once it signed in after an upgrade.
    h.account("ada@example.com", Role::User).await;
    h.auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect("upgrades the hash");

    let started = Instant::now();
    let wrong = h
        .auth
        .login(login("ada@example.com", "a wrong password"))
        .await
        .expect_err("wrong");
    let wrong_took = started.elapsed();
    let started = Instant::now();
    let unknown = h
        .auth
        .login(login("nobody@example.com", PASSWORD))
        .await
        .expect_err("unknown");
    let unknown_took = started.elapsed();

    assert!(matches!(wrong, AuthError::InvalidCredentials), "{wrong:?}");
    assert!(
        matches!(unknown, AuthError::InvalidCredentials),
        "{unknown:?}"
    );
    assert_eq!(wrong.to_string(), unknown.to_string());
    // Both cost a full hash. The margin absorbs scheduling noise, not a missing hash,
    // which would be orders of magnitude faster.
    assert!(
        unknown_took * 3 >= wrong_took,
        "{unknown_took:?} against {wrong_took:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_disabled_account_is_only_told_after_the_right_password(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::User).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(ada.id)
        .execute(&h.pool)
        .await
        .unwrap();

    let wrong = h
        .auth
        .login(login("ada@example.com", "a wrong password"))
        .await
        .expect_err("wrong");
    let right = h
        .auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect_err("disabled");

    assert!(matches!(wrong, AuthError::InvalidCredentials), "{wrong:?}");
    assert!(matches!(right, AuthError::UserDisabled), "{right:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn hashes_made_with_old_parameters_are_upgraded_on_sign_in(pool: PgPool) {
    let stronger = Argon2Params {
        iterations: 2,
        ..CHEAP
    };
    let h = Harness::with(
        pool,
        Options {
            params: stronger,
            ..Options::default()
        },
    )
    .await;
    h.account("ada@example.com", Role::User).await;
    let before: String = h.scalar("SELECT password_hash FROM users").await;
    assert!(before.contains("m=64,t=1,p=1"), "{before}");

    h.auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect("signs in");

    let after: String = h.scalar("SELECT password_hash FROM users").await;
    assert!(after.contains("m=64,t=2,p=1"), "{after}");
    h.auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect("the new hash works");
}

/// A reset of the password that commits while a sign-in is between checking the old
/// password and opening its session: the sign-in fails, and the new password stays.
async fn a_reset_under_a_sign_in_wins(h: &Harness) {
    let ada = h.account("ada@example.com", Role::User).await;
    let mut reset = h.pool.begin().await.expect("a transaction");
    sqlx::query("UPDATE users SET password_hash = 'the new hash' WHERE id = $1")
        .bind(ada.id)
        .execute(&mut *reset)
        .await
        .expect("the reset sets the password");

    let auth = h.auth.clone();
    let signing_in =
        tokio::spawn(async move { auth.login(login("ada@example.com", PASSWORD)).await });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&h.pool)
        .await
        .expect("reads the activity");
        if waiting > 0 {
            break;
        }
        assert!(
            !signing_in.is_finished(),
            "the sign-in wrote without waiting for the reset"
        );
        assert!(
            Instant::now() < deadline,
            "the sign-in never reached a write"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(ada.id)
        .execute(&mut *reset)
        .await
        .expect("the reset signs out everywhere");
    reset.commit().await.expect("the reset commits");

    let error = signing_in
        .await
        .expect("the sign-in ends")
        .expect_err("the password it checked is gone");
    assert!(matches!(error, AuthError::InvalidCredentials), "{error:?}");
    let sessions: i64 = h.scalar("SELECT count(*) FROM sessions").await;
    assert_eq!(sessions, 0);
    let hash: String = h.scalar("SELECT password_hash FROM users").await;
    assert_eq!(hash, "the new hash");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_sign_in_opens_no_session_once_the_password_changed_under_it(pool: PgPool) {
    a_reset_under_a_sign_in_wins(&Harness::new(pool).await).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_sign_in_does_not_upgrade_a_hash_that_changed_under_it(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            params: Argon2Params {
                iterations: 2,
                ..CHEAP
            },
            ..Options::default()
        },
    )
    .await;
    a_reset_under_a_sign_in_wins(&h).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sign_in_limits_check_the_ip_first_then_the_address(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.limiter.refuse(Bucket::LoginIp);

    let error = h
        .auth
        .login(login("Ada@Example.com", PASSWORD))
        .await
        .expect_err("limited");

    assert!(matches!(error, AuthError::RateLimited { .. }), "{error:?}");
    // A client stopped by its IP charges nothing else.
    assert_eq!(h.limiter.seen(), [(Bucket::LoginIp, IP.to_owned())]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sign_ins_count_per_ip_and_address_and_per_address(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;

    h.auth
        .login(login("Ada@Example.com", PASSWORD))
        .await
        .expect("signs in");
    let long = format!("{}@example.com", "a".repeat(300));
    let error = h
        .auth
        .login(login(&long, PASSWORD))
        .await
        .expect_err("too long");

    assert!(matches!(error, AuthError::InvalidCredentials), "{error:?}");
    assert_eq!(
        h.limiter.seen(),
        [
            (Bucket::LoginIp, IP.to_owned()),
            (Bucket::LoginIpEmail, format!("{IP} ada@example.com")),
            (Bucket::LoginEmail, "ada@example.com".to_owned()),
            // An address longer than any account's is refused before it is charged.
            (Bucket::LoginIp, IP.to_owned()),
        ]
    );
    h.limiter.refuse(Bucket::LoginEmail);
    assert!(matches!(
        h.auth.login(login("ada@example.com", PASSWORD)).await,
        Err(AuthError::RateLimited { .. })
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn use_slides_the_idle_expiry_at_most_once_a_minute_up_to_the_absolute_one(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.session_idle_ttl", json!("1h")).await;
    h.set("auth.session_absolute_ttl", json!("2h")).await;
    h.account("ada@example.com", Role::User).await;
    let (_, token) = h.sign_in("ada@example.com").await;
    let expiry = || async {
        let at: time::OffsetDateTime = h.scalar("SELECT expires_at FROM sessions").await;
        at
    };
    assert_eq!(expiry().await, support::start() + Duration::from_secs(3600));

    h.time.advance(Duration::from_secs(30));
    h.auth
        .authenticate_session(token.expose())
        .await
        .expect("works");
    assert_eq!(
        expiry().await,
        support::start() + Duration::from_secs(3600),
        "not yet a minute"
    );

    h.time.advance(Duration::from_secs(50 * 60));
    h.auth
        .authenticate_session(token.expose())
        .await
        .expect("works");
    assert_eq!(
        expiry().await,
        support::start() + Duration::from_secs(30 + 50 * 60 + 3600)
    );

    h.time.advance(Duration::from_secs(50 * 60));
    h.auth
        .authenticate_session(token.expose())
        .await
        .expect("works");
    assert_eq!(
        expiry().await,
        support::start() + Duration::from_secs(2 * 3600),
        "capped"
    );

    h.time.advance(Duration::from_secs(30 * 60));
    assert!(matches!(
        h.auth.authenticate_session(token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
    let left: i64 = h.scalar("SELECT count(*) FROM sessions").await;
    assert_eq!(left, 0, "an expired session is deleted when it is seen");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_idle_session_expires(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (_, token) = h.sign_in("ada@example.com").await;

    h.time.advance(Duration::from_secs(7 * 24 * 3600));

    assert!(matches!(
        h.auth.authenticate_session(token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        h.auth.authenticate_session("garbage").await,
        Err(AuthError::Unauthenticated)
    ));
    assert!(matches!(
        h.auth.authenticate_session("").await,
        Err(AuthError::Unauthenticated)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn signing_out_and_revoking_end_sessions(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (first, first_token) = h.sign_in("ada@example.com").await;
    let (second, second_token) = h.sign_in("ada@example.com").await;
    let (third, third_token) = h.sign_in("ada@example.com").await;

    let listed = h.auth.list_sessions(&first).await.expect("lists");
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].id, third.session_id().unwrap(), "newest first");

    h.auth
        .revoke_session(&first, second.session_id().unwrap())
        .await
        .expect("revokes");
    assert!(
        h.auth
            .authenticate_session(second_token.expose())
            .await
            .is_err()
    );
    assert!(matches!(
        h.auth
            .revoke_session(&first, second.session_id().unwrap())
            .await,
        Err(AuthError::NotFound(_))
    ));

    assert_eq!(
        h.auth.revoke_other_sessions(&first).await.expect("revokes"),
        1
    );
    assert!(
        h.auth
            .authenticate_session(third_token.expose())
            .await
            .is_err()
    );

    h.auth
        .logout(first.session_id().unwrap())
        .await
        .expect("signs out");
    h.auth
        .logout(first.session_id().unwrap())
        .await
        .expect("twice is fine");
    assert!(
        h.auth
            .authenticate_session(first_token.expose())
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_disabled_account_s_sessions_stop_working(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::User).await;
    let (_, token) = h.sign_in("ada@example.com").await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(ada.id)
        .execute(&h.pool)
        .await
        .unwrap();

    assert!(matches!(
        h.auth.authenticate_session(token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn api_tokens_are_shown_once_and_kept_as_hashes(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;

    let (secret_token, token) = h
        .auth
        .create_api_token(
            &ada,
            "  CI deploys ",
            None,
            &["write", "read", "write"],
            None,
        )
        .await
        .expect("a token");

    assert!(secret_token.expose().starts_with("nw_"));
    assert_eq!(secret_token.expose().len(), 46);
    assert_eq!(token.prefix, secret_token.expose()[..10]);
    assert_eq!(token.name, "CI deploys");
    assert_eq!(token.scopes, [Scope::Read, Scope::Write]);
    let stored: Vec<u8> = h.scalar("SELECT token_hash FROM api_tokens").await;
    assert_eq!(stored, hash_secret(secret_token.expose()));

    let principal = h
        .auth
        .authenticate_token(secret_token.expose())
        .await
        .expect("works");
    assert!(!principal.via_session());
    assert!(principal.can(Scope::Write) && !principal.can(Scope::Admin));
    let used: bool = h
        .scalar("SELECT last_used_at IS NOT NULL FROM api_tokens")
        .await;
    assert!(used);

    let listed = h.auth.list_api_tokens(&principal).await.expect("lists");
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id, &listed[0].name), (token.id, &token.name));

    // Tokens cannot make or revoke tokens.
    assert!(matches!(
        h.auth
            .create_api_token(&principal, "more", None, &["read"], None)
            .await,
        Err(AuthError::Forbidden(_))
    ));
    assert!(matches!(
        h.auth.revoke_api_token(&principal, token.id).await,
        Err(AuthError::Forbidden(_))
    ));
    h.auth
        .revoke_api_token(&ada, token.id)
        .await
        .expect("revokes");
    assert!(matches!(
        h.auth.authenticate_token(secret_token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_token_never_exceeds_its_owner_s_role(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;
    let admin = h.admin().await;

    let refused = h
        .auth
        .create_api_token(&ada, "admin", None, &["admin"], None)
        .await;
    assert!(
        matches!(refused, Err(AuthError::Forbidden(_))),
        "{refused:?}"
    );
    for bad in [&[][..], &["delete"][..]] {
        let error = h
            .auth
            .create_api_token(&ada, "bad", None, bad, None)
            .await
            .expect_err("invalid");
        assert!(matches!(error, AuthError::InvalidInput(_)), "{error:?}");
    }

    let (secret_token, _) = h
        .auth
        .create_api_token(&admin, "ops", None, &["admin"], None)
        .await
        .expect("an admin's token");
    assert!(
        h.auth
            .authenticate_token(secret_token.expose())
            .await
            .expect("works")
            .is_admin()
    );

    h.account("grace@example.com", Role::Admin).await;
    let (grace, _) = h.sign_in("grace@example.com").await;
    h.auth
        .update_user(
            &grace,
            admin.user.id,
            UserUpdate {
                role: Some(Role::User),
                ..UserUpdate::default()
            },
        )
        .await
        .expect("demotes");
    let demoted = h
        .auth
        .authenticate_token(secret_token.expose())
        .await
        .expect("still works");
    assert!(!demoted.is_admin());
    assert!(matches!(
        &demoted.credential,
        Credential::Token { scopes, .. } if scopes.is_empty()
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn tokens_expire_and_need_their_prefix(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;
    let past = support::start();
    assert!(matches!(
        h.auth
            .create_api_token(&ada, "old", Some(past), &["read"], None)
            .await,
        Err(AuthError::InvalidInput(_))
    ));
    let (secret_token, _) = h
        .auth
        .create_api_token(
            &ada,
            "brief",
            Some(past + Duration::from_secs(3600)),
            &["read"],
            None,
        )
        .await
        .expect("a token");

    assert!(matches!(
        h.auth.authenticate_token(&secret_token.expose()[3..]).await,
        Err(AuthError::Unauthenticated)
    ));
    h.time.advance(Duration::from_secs(3600));
    assert!(matches!(
        h.auth.authenticate_token(secret_token.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn ending_sessions_and_tokens_is_told_and_refreshing_sees_it(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("admin@example.com", Role::Admin).await;
    let ada = h.account("ada@example.com", Role::User).await;
    let (admin_session, _) = h.sign_in("admin@example.com").await;
    let (first, _) = h.sign_in("ada@example.com").await;
    let (second, _) = h.sign_in("ada@example.com").await;
    let (third, _) = h.sign_in("ada@example.com").await;
    let (token_secret, token) = h
        .auth
        .create_api_token(&first, "ci", None, &["read", "write"], None)
        .await
        .expect("created");
    let by_token = h
        .auth
        .authenticate_token(token_secret.expose())
        .await
        .expect("works");
    let account = AccessChange::Account { user_id: ada.id };
    h.changes.take();

    // A session or token that holds refreshes as it stands now.
    let fresh = h.auth.refresh(&second).await.expect("holds");
    assert_eq!(fresh.session_id(), second.session_id());
    let fresh = h.auth.refresh(&by_token).await.expect("holds");
    assert!(fresh.can(Scope::Write) && !fresh.via_session());
    assert_eq!(h.changes.take(), vec![], "refreshing changes nothing");

    h.auth
        .revoke_session(&first, second.session_id().unwrap())
        .await
        .expect("revoked");
    assert_eq!(h.changes.take(), vec![account]);
    assert!(matches!(
        h.auth.refresh(&second).await,
        Err(AuthError::Unauthenticated)
    ));
    h.auth.revoke_other_sessions(&first).await.expect("revoked");
    h.auth
        .logout(first.session_id().unwrap())
        .await
        .expect("signed out");
    h.auth
        .logout(first.session_id().unwrap())
        .await
        .expect("again");
    assert_eq!(
        h.changes.take(),
        vec![account, account],
        "a session already gone tells nobody"
    );
    assert!(h.auth.refresh(&third).await.is_err());

    h.auth
        .revoke_api_token(&admin_session, token.id)
        .await
        .expect_err("not the admin's token");
    let (fourth, _) = h.sign_in("ada@example.com").await;
    h.auth
        .revoke_api_token(&fourth, token.id)
        .await
        .expect("revoked");
    assert_eq!(h.changes.take(), vec![account]);
    assert!(h.auth.refresh(&by_token).await.is_err());

    // Disabling an account tells, and ends what it signed in with.
    h.auth
        .update_user(
            &admin_session,
            ada.id,
            UserUpdate {
                disabled: Some(true),
                ..UserUpdate::default()
            },
        )
        .await
        .expect("disabled");
    assert_eq!(h.changes.take(), vec![account]);
    assert!(h.auth.refresh(&fourth).await.is_err());

    // A session that expired no longer refreshes, though nothing told of it.
    let (admin_later, _) = h.sign_in("admin@example.com").await;
    h.time.advance(Duration::from_secs(7 * 24 * 3600));
    assert!(matches!(
        h.auth.refresh(&admin_later).await,
        Err(AuthError::Unauthenticated)
    ));
}
