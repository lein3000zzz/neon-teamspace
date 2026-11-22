//! The admin's work on accounts, the first admin, and the hourly cleanup.

mod support;

use std::time::Duration;

use neon_app::auth::{AuthError, CleanupReport, InviteSpec, NewAccount, RegisterInput, UserUpdate};
use neon_domain::auth::Role;
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, IP, PASSWORD, secret};

fn demote() -> UserUpdate {
    UserUpdate {
        role: Some(Role::User),
        ..UserUpdate::default()
    }
}

fn disable() -> UserUpdate {
    UserUpdate {
        disabled: Some(true),
        ..UserUpdate::default()
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_last_active_admin_can_be_neither_demoted_nor_disabled(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;

    for update in [demote(), disable()] {
        let error = h
            .auth
            .update_user(&admin, admin.user.id, update)
            .await
            .expect_err("last admin");
        assert!(matches!(error, AuthError::LastAdmin), "{error:?}");
    }

    // A disabled admin does not count.
    let grace = h.account("grace@example.com", Role::Admin).await;
    h.auth
        .update_user(&admin, grace.id, disable())
        .await
        .expect("disables");
    assert!(matches!(
        h.auth.update_user(&admin, admin.user.id, demote()).await,
        Err(AuthError::LastAdmin)
    ));
    // With another active admin, either may step down.
    h.auth
        .update_user(
            &admin,
            grace.id,
            UserUpdate {
                disabled: Some(false),
                ..UserUpdate::default()
            },
        )
        .await
        .expect("enables");
    let demoted = h
        .auth
        .update_user(&admin, admin.user.id, demote())
        .await
        .expect("demotes");
    assert_eq!(demoted.role, Role::User);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn two_admins_demoting_each_other_at_once_leave_one(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.account("ada@example.com", Role::Admin).await;
    let grace = h.account("grace@example.com", Role::Admin).await;
    let (ada_session, _) = h.sign_in("ada@example.com").await;
    let (grace_session, _) = h.sign_in("grace@example.com").await;

    let (first, second) = tokio::join!(
        h.auth.update_user(&ada_session, grace.id, demote()),
        h.auth.update_user(&grace_session, ada.id, demote()),
    );

    let failures = [&first, &second]
        .iter()
        .filter(|result| matches!(result, Err(AuthError::LastAdmin)))
        .count();
    assert_eq!(failures, 1, "{first:?} {second:?}");
    let admins: i64 = h
        .scalar("SELECT count(*) FROM users WHERE role = 'admin' AND disabled_at IS NULL")
        .await;
    assert_eq!(admins, 1);
}

/// Bob's principal is read as his request begins, so it may predate the admin's change.
#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_rename_that_began_before_a_disable_keeps_it(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let bob = h.account("bob@example.com", Role::User).await;
    let (stale, _) = h.sign_in("bob@example.com").await;

    h.auth
        .update_user(&admin, bob.id, disable())
        .await
        .expect("disables");
    let renamed = h.auth.update_profile(&stale, "Bob").await.expect("renames");

    assert_eq!(renamed.display_name, "Bob");
    assert!(
        renamed.is_disabled(),
        "the answer shows the account as it is"
    );
    let disabled: bool = h
        .scalar("SELECT disabled_at IS NOT NULL FROM users WHERE email = 'bob@example.com'")
        .await;
    assert!(disabled);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_rename_that_began_before_a_promotion_cannot_leave_no_admin(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let bob = h.account("bob@example.com", Role::User).await;
    let (stale, _) = h.sign_in("bob@example.com").await;

    // The only admin makes Bob one, then steps down, which leaves Bob.
    h.auth
        .update_user(
            &admin,
            bob.id,
            UserUpdate {
                role: Some(Role::Admin),
                ..UserUpdate::default()
            },
        )
        .await
        .expect("promotes");
    h.auth
        .update_user(&admin, admin.user.id, demote())
        .await
        .expect("steps down");
    let renamed = h.auth.update_profile(&stale, "Bob").await.expect("renames");

    assert_eq!(renamed.role, Role::Admin);
    let admins: i64 = h
        .scalar("SELECT count(*) FROM users WHERE role = 'admin' AND disabled_at IS NULL")
        .await;
    assert_eq!(admins, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn renaming_in_a_loop_with_a_token_does_not_undo_a_disable(pool: PgPool) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let bob = h.account("bob@example.com", Role::User).await;
    let (session, _) = h.sign_in("bob@example.com").await;
    let (token, _) = h
        .auth
        .create_api_token(&session, "loop", None, &["read", "write"])
        .await
        .expect("a token");
    let token = token.expose().to_owned();

    let stop = Arc::new(AtomicBool::new(false));
    let loops: Vec<_> = (0..8)
        .map(|_| {
            let auth = h.auth.clone();
            let token = token.clone();
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(principal) = auth.authenticate_token(&token).await {
                        // Whether each rename went through does not matter, only the disable.
                        let _ = auth.update_profile(&principal, "Bob").await;
                    }
                }
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.auth
        .update_user(&admin, bob.id, disable())
        .await
        .expect("disables");
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.store(true, Ordering::Relaxed);
    for task in loops {
        task.await.expect("the loop ends");
    }

    let disabled: bool = h
        .scalar("SELECT disabled_at IS NOT NULL FROM users WHERE email = 'bob@example.com'")
        .await;
    assert!(disabled);
    assert!(h.auth.authenticate_token(&token).await.is_err());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn disabling_signs_out_everywhere_and_verifying_is_one_way(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let ada = h.account("ada@example.com", Role::User).await;
    let (_, token) = h.sign_in("ada@example.com").await;
    sqlx::query("UPDATE users SET email_verified_at = NULL WHERE id = $1")
        .bind(ada.id)
        .execute(&h.pool)
        .await
        .unwrap();

    let disabled = h
        .auth
        .update_user(&admin, ada.id, disable())
        .await
        .expect("disables");
    assert!(disabled.is_disabled());
    let sessions: i64 = h.scalar("SELECT count(*) FROM sessions WHERE user_id <> (SELECT id FROM users WHERE email = 'admin@example.com')").await;
    assert_eq!(sessions, 0);
    assert!(h.auth.authenticate_session(token.expose()).await.is_err());

    let verified = h
        .auth
        .update_user(
            &admin,
            ada.id,
            UserUpdate {
                email_verified: Some(true),
                ..UserUpdate::default()
            },
        )
        .await
        .expect("verifies");
    assert!(verified.is_verified());
    let error = h
        .auth
        .update_user(
            &admin,
            ada.id,
            UserUpdate {
                email_verified: Some(false),
                ..UserUpdate::default()
            },
        )
        .await
        .expect_err("one way");
    assert!(matches!(error, AuthError::InvalidInput(_)), "{error:?}");
    assert!(matches!(
        h.auth
            .update_user(&admin, uuid::Uuid::now_v7(), disable())
            .await,
        Err(AuthError::NotFound(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_make_accounts_in_any_mode_and_list_them_in_pages(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.registration", json!("closed")).await;
    let admin = h.admin().await;
    for n in 0..4 {
        h.time.advance(Duration::from_millis(1));
        let user = h
            .auth
            .create_user(
                &admin,
                NewAccount {
                    email: format!("user{n}@example.com"),
                    password: secret(PASSWORD),
                    display_name: Some(format!("User {n}")),
                    role: Role::User,
                },
            )
            .await
            .expect("makes an account");
        assert!(user.is_verified());
    }
    let taken = h
        .auth
        .create_user(
            &admin,
            NewAccount {
                email: "USER0@example.com".into(),
                password: secret(PASSWORD),
                display_name: None,
                role: Role::User,
            },
        )
        .await
        .expect_err("taken");
    assert!(matches!(taken, AuthError::EmailTaken), "{taken:?}");

    let first = h
        .auth
        .list_users(&admin, None, Some(3))
        .await
        .expect("lists");
    assert_eq!(first.users.len(), 3);
    assert_eq!(first.users[0].email.as_str(), "admin@example.com");
    let next = first.next.expect("more");
    let second = h
        .auth
        .list_users(&admin, Some(next), Some(3))
        .await
        .expect("lists");
    assert_eq!(second.users.len(), 2);
    assert_eq!(second.users[1].email.as_str(), "user3@example.com");
    assert_eq!(second.next, None);
    assert!(matches!(
        h.auth.list_users(&admin, None, Some(0)).await,
        Err(AuthError::InvalidInput(_))
    ));

    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;
    assert!(matches!(
        h.auth.list_users(&ada, None, None).await,
        Err(AuthError::Forbidden(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_first_admin_is_made_only_while_no_admin_can_sign_in(pool: PgPool) {
    let h = Harness::new(pool).await;

    let first = h
        .auth
        .ensure_admin("root@example.com", &secret(PASSWORD))
        .await
        .expect("makes one");
    let user = first.expect("made");
    assert_eq!(user.role, Role::Admin);
    assert!(user.is_verified());
    assert!(
        h.auth
            .ensure_admin("other@example.com", &secret(PASSWORD))
            .await
            .expect("checks")
            .is_none()
    );

    sqlx::query("UPDATE users SET disabled_at = now()")
        .execute(&h.pool)
        .await
        .unwrap();
    let again = h
        .auth
        .ensure_admin("rescue@example.com", &secret(PASSWORD))
        .await
        .expect("makes one");
    assert!(again.is_some(), "a disabled admin cannot sign in");

    let weak = h
        .auth
        .ensure_admin("weak@example.com", &secret("short"))
        .await;
    assert!(weak.expect("an admin exists").is_none());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_cleanup_removes_what_expired_and_keeps_the_rest(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.registration", json!("open")).await;
    h.set("auth.verify_email", json!(true)).await;
    let admin = h.admin().await;
    h.auth
        .register(RegisterInput {
            email: "squatter@example.com".into(),
            password: secret(PASSWORD),
            display_name: None,
            invite_code: None,
            ip: IP.into(),
        })
        .await
        .expect("registers");
    let short = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                expires_at: Some(support::start() + Duration::from_secs(3600)),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");
    let lasting = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                expires_at: Some(support::start() + Duration::from_secs(300 * 24 * 3600)),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");

    assert_eq!(
        h.auth.cleanup().await.expect("cleans"),
        CleanupReport::default()
    );

    // Forty days on: the admin's session, the unverified account and its link, and the
    // short invite are all past their keeping.
    h.time.advance(Duration::from_secs(40 * 24 * 3600));
    let report = h.auth.cleanup().await.expect("cleans");

    assert_eq!(
        report,
        CleanupReport {
            sessions: 1,
            email_tokens: 1,
            unverified_users: 1,
            invites: 1,
            join_requests: 0,
        }
    );
    let invites: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM invites")
        .fetch_all(&h.pool)
        .await
        .unwrap();
    assert_eq!(invites, [lasting.invite.id]);
    assert_ne!(short.invite.id, lasting.invite.id);
    let accounts: i64 = h.scalar("SELECT count(*) FROM users").await;
    assert_eq!(accounts, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_cleanup_removes_old_requests_to_join(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("auth.requests", json!(true)).await;
    let admin = h.admin().await;
    for email in ["unconfirmed@example.com", "denied@example.com"] {
        h.auth.request_to_join(email, "", IP).await.expect("taken");
    }
    h.auth.settle().await;
    let mails = h.mails().await;
    let denied_token = mails
        .iter()
        .find(|mail| mail.to.address == "denied@example.com")
        .map(|mail| match &mail.message {
            neon_app::mail::Message::JoinConfirmation { token, .. } => token.clone(),
            other => panic!("{other:?}"),
        })
        .expect("mailed");
    h.auth
        .confirm_join(&denied_token, IP)
        .await
        .expect("confirms");
    let id = h.auth.list_join_requests(&admin).await.expect("lists")[0].id;
    h.auth.deny_join(&admin, id).await.expect("denies");
    h.auth.settle().await;

    // A day after its link stopped working, an unconfirmed request goes.
    h.time.advance(Duration::from_secs(3 * 24 * 3600));
    assert_eq!(
        h.auth.cleanup().await.expect("cleans").join_requests,
        1,
        "the unconfirmed one"
    );
    h.time.advance(Duration::from_secs(28 * 24 * 3600));
    assert_eq!(
        h.auth.cleanup().await.expect("cleans").join_requests,
        1,
        "the denied one"
    );
    let left: i64 = h.scalar("SELECT count(*) FROM join_requests").await;
    assert_eq!(left, 0);
}
