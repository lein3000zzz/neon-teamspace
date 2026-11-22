//! Invites and requests to join.

mod support;

use std::time::Duration;

use neon_app::auth::{AuthError, InviteSpec, RegisterInput};
use neon_app::crypto::{Envelope, hash_secret};
use neon_app::mail::Message;
use neon_app::settings::Change;
use neon_domain::auth::{InviteStatus, JoinRequestStatus, Role};
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, IP, Options, PASSWORD, secret};

fn register(email: &str, code: &str) -> RegisterInput {
    RegisterInput {
        email: email.to_owned(),
        password: secret(PASSWORD),
        display_name: None,
        invite_code: Some(secret(code)),
        ip: IP.to_owned(),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn generated_codes_are_long_random_and_kept_hashed_and_sealed(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;

    let created = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("an invite");

    let code = created.code.expose();
    assert!(code.starts_with("inv_") && code.len() == 4 + 43, "{code}");
    let invite = &created.invite;
    assert_eq!((invite.max_uses, invite.uses), (1, 0));
    assert_eq!(
        invite.expires_at,
        support::start() + Duration::from_secs(7 * 24 * 3600)
    );
    assert!(invite.code_kept);
    let hash: Vec<u8> = h.scalar("SELECT code_hash FROM invites").await;
    assert_eq!(hash, hash_secret(&code.to_lowercase()));
    let sealed: Vec<u8> = h.scalar("SELECT code_sealed FROM invites").await;
    assert!(!sealed.windows(code.len()).any(|w| w == code.as_bytes()));

    let copied = h
        .auth
        .invite_code_of(&admin, invite.id)
        .await
        .expect("opens");
    assert_eq!(copied.expose(), code);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_secret_key_a_code_shows_only_once(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            envelope: Envelope::default(),
            ..Options::default()
        },
    )
    .await;
    let admin = h.admin().await;

    let created = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("an invite");

    assert!(!created.invite.code_kept);
    let error = h
        .auth
        .invite_code_of(&admin, created.invite.id)
        .await
        .expect_err("not kept");
    assert!(matches!(error, AuthError::CodeUnavailable(_)), "{error:?}");
    // The invite works all the same.
    h.auth
        .register(register("ada@example.com", created.code.expose()))
        .await
        .expect("registers");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn typed_codes_are_checked_and_unique_whatever_their_case(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let spec = |code: &str| InviteSpec {
        code: Some(secret(code)),
        ..InviteSpec::default()
    };

    for (code, want) in [
        ("short", Some("a code has 8 to 64 characters")),
        (
            "has spaces in it",
            Some("a code has letters, digits, - and _ only"),
        ),
        ("Team-Spring_2026", None),
        ("", None),
    ] {
        assert_eq!(
            h.auth
                .check_invite_code(&admin, &secret(code))
                .await
                .expect("checks"),
            want,
            "{code}"
        );
    }
    let error = h
        .auth
        .create_invite(&admin, spec("short"))
        .await
        .expect_err("invalid");
    assert!(matches!(error, AuthError::InvalidInput(_)), "{error:?}");

    h.auth
        .create_invite(&admin, spec(" Team-Spring_2026 "))
        .await
        .expect("an invite");
    assert_eq!(
        h.auth
            .check_invite_code(&admin, &secret("team-spring_2026"))
            .await
            .expect("checks"),
        Some("another invite has this code")
    );
    let taken = h
        .auth
        .create_invite(&admin, spec("TEAM-SPRING_2026"))
        .await
        .expect_err("taken");
    assert!(matches!(taken, AuthError::InviteCodeTaken), "{taken:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn uses_and_ends_stay_within_bounds(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let now = support::start();

    for spec in [
        InviteSpec {
            max_uses: Some(0),
            ..InviteSpec::default()
        },
        InviteSpec {
            max_uses: Some(1001),
            ..InviteSpec::default()
        },
        InviteSpec {
            expires_at: Some(now),
            ..InviteSpec::default()
        },
        InviteSpec {
            expires_at: Some(now + Duration::from_secs(366 * 24 * 3600)),
            ..InviteSpec::default()
        },
    ] {
        let error = h
            .auth
            .create_invite(&admin, spec.clone())
            .await
            .expect_err("out of bounds");
        assert!(
            matches!(error, AuthError::InvalidInput(_)),
            "{spec:?}: {error:?}"
        );
    }

    let created = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                max_uses: Some(1000),
                expires_at: Some(now + Duration::from_secs(3600)),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");
    assert_eq!(created.invite.expires_at, now + Duration::from_secs(3600));

    h.time.advance(Duration::from_secs(3600));
    let error = h
        .auth
        .register(register("ada@example.com", created.code.expose()))
        .await
        .expect_err("expired");
    assert!(matches!(error, AuthError::InvalidInvite), "{error:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn lists_show_sign_ups_and_revoked_invites_admit_no_one(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let first = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                max_uses: Some(5),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");
    let ada = h
        .auth
        .register(register("ada@example.com", first.code.expose()))
        .await
        .expect("registers");
    h.time.advance(Duration::from_secs(1));
    let second = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("an invite");

    h.auth
        .revoke_invite(&admin, first.invite.id)
        .await
        .expect("revokes");
    h.auth
        .revoke_invite(&admin, first.invite.id)
        .await
        .expect("twice is fine");

    let listed = h.auth.list_invites(&admin).await.expect("lists");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].invite.id, second.invite.id, "newest first");
    assert_eq!(
        listed[1].invite.status(h.time.clock().now()),
        InviteStatus::Revoked
    );
    assert_eq!(listed[1].signups.len(), 1);
    assert_eq!(listed[1].signups[0].user_id, Some(ada.user.id));
    assert_eq!(
        listed[1].signups[0].email.as_deref(),
        Some("ada@example.com")
    );
    let error = h
        .auth
        .register(register("grace@example.com", first.code.expose()))
        .await
        .expect_err("revoked");
    assert!(matches!(error, AuthError::InvalidInvite), "{error:?}");
    assert!(matches!(
        h.auth.revoke_invite(&admin, uuid::Uuid::now_v7()).await,
        Err(AuthError::NotFound(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_admins_manage_invites(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;

    let error = h
        .auth
        .create_invite(&ada, InviteSpec::default())
        .await
        .expect_err("not an admin");
    assert!(matches!(error, AuthError::Forbidden(_)), "{error:?}");
    assert!(matches!(
        h.auth.list_invites(&ada).await,
        Err(AuthError::Forbidden(_))
    ));
    assert!(matches!(
        h.auth.invite_by_email(&ada, "grace@example.com").await,
        Err(AuthError::Forbidden(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_sign_up_form_looks_an_invite_up_for_its_address(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let mailed = h
        .auth
        .invite_by_email(&admin, "ada@example.com")
        .await
        .expect("mailed");
    let open = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("an invite");

    let found = h.auth.invite_for(&mailed.code, IP).await.expect("found");
    assert_eq!(found.email.as_deref(), Some("ada@example.com"));
    assert_eq!(
        h.auth
            .invite_for(&open.code, IP)
            .await
            .expect("found")
            .email,
        None
    );
    for code in ["", "inv_nothing", &"x".repeat(129)] {
        assert!(matches!(
            h.auth.invite_for(&secret(code), IP).await,
            Err(AuthError::InvalidInvite)
        ));
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mailed_invites_name_the_admin_and_refuse_existing_accounts(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    h.account("grace@example.com", Role::User).await;

    let created = h
        .auth
        .invite_by_email(&admin, "Ada@Example.com")
        .await
        .expect("mailed");

    assert_eq!(created.invite.email.as_deref(), Some("Ada@Example.com"));
    assert_eq!(created.invite.max_uses, 1);
    let mails = h.mails().await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to.address, "Ada@Example.com");
    match &mails[0].message {
        Message::Invite {
            inviter_email,
            code,
            expires_at,
            ..
        } => {
            assert_eq!(inviter_email, "admin@example.com");
            assert_eq!(code, &created.code);
            assert_eq!(*expires_at, created.invite.expires_at);
        }
        other => panic!("not an invite: {other:?}"),
    }
    let taken = h
        .auth
        .invite_by_email(&admin, "grace@example.com")
        .await
        .expect_err("an account");
    assert!(matches!(taken, AuthError::EmailTaken), "{taken:?}");

    assert!(h.mails().await.is_empty(), "nothing for a refused address");

    h.set("mail.enabled", json!(false)).await;
    assert!(matches!(
        h.auth.invite_by_email(&admin, "linus@example.com").await,
        Err(AuthError::InviteEmailOff)
    ));
}

async fn requests_on(pool: PgPool) -> Harness {
    Harness::with(
        pool,
        Options {
            settings: vec![("auth.requests", json!(true))],
            ..Options::default()
        },
    )
    .await
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn requests_are_open_only_with_the_setting_invite_mode_and_mail(pool: PgPool) {
    let h = Harness::new(pool).await;
    assert!(matches!(
        h.auth.request_to_join("ada@example.com", "", IP).await,
        Err(AuthError::RequestsOff)
    ));
    h.set("mail.enabled", json!(false)).await;
    let refused = h
        .settings
        .update(
            [("auth.requests".to_owned(), Change::Set(json!(true)))],
            None,
        )
        .await
        .expect_err("needs mail");
    assert!(
        refused.to_string().contains("needs mail set up first"),
        "{refused}"
    );

    h.set("mail.enabled", json!(true)).await;
    h.set("auth.requests", json!(true)).await;
    assert!(h.auth.requests_open());
    assert!(h.auth.config().requests);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_public_url_requests_stay_closed(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            public_url: None,
            settings: vec![("auth.requests", json!(true))],
            ..Options::default()
        },
    )
    .await;

    assert!(!h.auth.requests_open(), "mail cannot link anywhere");
    assert!(matches!(
        h.auth.request_to_join("ada@example.com", "", IP).await,
        Err(AuthError::RequestsOff)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_request_is_confirmed_by_mail_then_approved_with_a_bound_invite(pool: PgPool) {
    let h = requests_on(pool).await;
    let admin = h.admin().await;

    h.auth
        .request_to_join("Ada@Example.com", "  From the hackathon team ", IP)
        .await
        .expect("taken");
    h.auth.settle().await;
    let token = h.token_of("join_confirmation").await;
    assert!(
        h.auth
            .list_join_requests(&admin)
            .await
            .expect("lists")
            .is_empty(),
        "unconfirmed"
    );

    h.auth.confirm_join(&token, IP).await.expect("confirms");
    h.auth.settle().await;
    assert!(matches!(
        h.auth.confirm_join(&token, IP).await,
        Err(AuthError::InvalidLink)
    ));
    let told = h.mails().await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].to.address, "admin@example.com");
    assert!(matches!(
        &told[0].message,
        Message::JoinRequested { requester, note }
            if requester == "Ada@Example.com" && note == "From the hackathon team"
    ));

    let listed = h.auth.list_join_requests(&admin).await.expect("lists");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status(), JoinRequestStatus::Pending);
    let (created, queued) = h
        .auth
        .approve_join(&admin, listed[0].id)
        .await
        .expect("approves");
    assert!(queued);
    assert_eq!(created.invite.email.as_deref(), Some("Ada@Example.com"));
    assert_eq!(created.invite.max_uses, 1);
    assert!(matches!(
        h.auth.approve_join(&admin, listed[0].id).await,
        Err(AuthError::NotFound(_))
    ));

    let code = h.token_of("invite").await;
    let registration = h
        .auth
        .register(register("ada@example.com", code.expose()))
        .await
        .expect("registers");
    assert!(registration.user.is_verified());
    let approved = h.auth.list_join_requests(&admin).await.expect("lists");
    assert_eq!(approved[0].status(), JoinRequestStatus::Approved);
    assert_eq!(approved[0].invite_id, Some(created.invite.id));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_answer_is_the_same_whatever_comes_of_a_request(pool: PgPool) {
    let h = requests_on(pool).await;
    h.account("grace@example.com", Role::User).await;

    // An address with an account gets nothing, and its asker the same answer.
    h.auth
        .request_to_join("grace@example.com", "", IP)
        .await
        .expect("answers");
    h.auth.settle().await;
    assert!(h.mails().await.is_empty());
    let kept: i64 = h.scalar("SELECT count(*) FROM join_requests").await;
    assert_eq!(kept, 0);

    let invalid = h
        .auth
        .request_to_join("not an address", "", IP)
        .await
        .expect_err("invalid");
    assert!(matches!(invalid, AuthError::InvalidEmail(_)), "{invalid:?}");
    let long = h
        .auth
        .request_to_join("ada@example.com", &"x".repeat(501), IP)
        .await
        .expect_err("a long note");
    assert!(matches!(long, AuthError::InvalidInput(_)), "{long:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_address_has_one_open_request_mailed_at_most_five_times(pool: PgPool) {
    let h = requests_on(pool).await;
    let ask = || async {
        h.auth
            .request_to_join("ada@example.com", "", IP)
            .await
            .expect("answers");
        h.auth.settle().await;
        h.mails().await.len()
    };

    assert_eq!(ask().await, 1);
    assert_eq!(ask().await, 0, "within the minute");
    for _ in 0..4 {
        h.time.advance(Duration::from_secs(60));
        assert_eq!(ask().await, 1);
    }
    h.time.advance(Duration::from_secs(60));
    assert_eq!(ask().await, 0, "five links in all");
    let kept: i64 = h.scalar("SELECT count(*) FROM join_requests").await;
    let mails: i32 = h.scalar("SELECT mails FROM join_requests").await;
    assert_eq!((kept, mails), (1, 5));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn open_requests_are_capped(pool: PgPool) {
    let h = requests_on(pool).await;
    sqlx::query(
        "INSERT INTO join_requests (id, email, email_normalized, link_expires_at, mailed_at, created_at)
         SELECT gen_random_uuid(), n || '@example.com', n || '@example.com', now(), now(), now()
         FROM generate_series(1, 200) AS n",
    )
    .execute(&h.pool)
    .await
    .unwrap();

    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("the same answer");
    h.auth.settle().await;

    assert!(h.mails().await.is_empty());
    let kept: i64 = h.scalar("SELECT count(*) FROM join_requests").await;
    assert_eq!(kept, 200);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_denied_address_is_told_and_waits_30_days(pool: PgPool) {
    let h = requests_on(pool).await;
    let admin = h.admin().await;
    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("taken");
    h.auth.settle().await;
    h.auth
        .confirm_join(&h.token_of("join_confirmation").await, IP)
        .await
        .expect("confirms");
    h.auth.settle().await;
    h.mails().await;
    let id = h.auth.list_join_requests(&admin).await.expect("lists")[0].id;

    h.auth.deny_join(&admin, id).await.expect("denies");
    h.auth.settle().await;

    let mails = h.mails().await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to.address, "ada@example.com");
    assert!(matches!(
        mails[0].message,
        Message::JoinDenied { ask_again_after } if ask_again_after == Duration::from_secs(30 * 24 * 3600)
    ));
    assert!(matches!(
        h.auth.deny_join(&admin, id).await,
        Err(AuthError::NotFound(_))
    ));

    h.time.advance(Duration::from_secs(29 * 24 * 3600));
    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("the same answer");
    h.auth.settle().await;
    assert!(h.mails().await.is_empty(), "still blocked");

    h.time.advance(Duration::from_secs(2 * 24 * 3600));
    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("taken");
    h.auth.settle().await;
    assert_eq!(h.mails().await.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn denials_go_unmailed_while_the_setting_is_off(pool: PgPool) {
    let h = requests_on(pool).await;
    h.set("auth.request_denied_mail", json!(false)).await;
    let admin = h.admin().await;
    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("taken");
    h.auth.settle().await;
    h.auth
        .confirm_join(&h.token_of("join_confirmation").await, IP)
        .await
        .expect("confirms");
    h.auth.settle().await;
    h.mails().await;
    let id = h.auth.list_join_requests(&admin).await.expect("lists")[0].id;

    h.auth.deny_join(&admin, id).await.expect("denies");
    h.auth.settle().await;

    assert!(h.mails().await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn confirmation_links_last_a_day(pool: PgPool) {
    let h = requests_on(pool).await;
    h.auth
        .request_to_join("ada@example.com", "", IP)
        .await
        .expect("taken");
    h.auth.settle().await;
    let token = h.token_of("join_confirmation").await;

    h.time.advance(Duration::from_secs(24 * 3600));

    assert!(matches!(
        h.auth.confirm_join(&token, IP).await,
        Err(AuthError::InvalidLink)
    ));
    assert!(matches!(
        h.auth.confirm_join(&secret(""), IP).await,
        Err(AuthError::InvalidLink)
    ));
}
