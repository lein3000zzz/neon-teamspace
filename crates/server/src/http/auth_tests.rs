//! Signing in over HTTP: the cookie, bearer tokens, the origin check, the 401 and 403
//! answers, rate limits, and the account and admin routes.

use axum::Router;
use axum::body::Body;
use axum::http::header::{
    AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN, REFERER, RETRY_AFTER, SET_COOKIE,
};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http_body_util::BodyExt as _;
use neon_app::auth::{NewAccount, RegisterInput};
use neon_app::crypto::{Envelope, Keyring};
use neon_app::settings::Change;
use neon_domain::auth::Role;
use neon_domain::settings::Secret;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;
use url::Url;

use super::tests::{state, state_with};
use super::*;

pub(super) const PASSWORD: &str = "correct horse battery";

pub(super) struct Answer {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Value,
}

impl Answer {
    pub(super) fn code(&self) -> &str {
        self.body["error"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("not the error shape: {}", self.body))
    }

    fn cookie(&self) -> &str {
        self.headers
            .get(SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("a Set-Cookie header")
    }

    /// The `name=value` part of the cookie set.
    fn cookie_pair(&self) -> String {
        self.cookie().split(';').next().expect("a pair").to_owned()
    }
}

pub(super) async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = match body {
        Some(json) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("a request"))
        .await
        .expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)))
    };
    Answer {
        status,
        headers,
        body,
    }
}

pub(super) async fn admin(state: &AppState) {
    state
        .auth
        .bootstrap_admin(NewAccount {
            email: "admin@example.com".into(),
            password: Secret::new(PASSWORD),
            display_name: None,
            role: Role::Admin,
        })
        .await
        .expect("an admin");
}

/// A user who signed up while registration was open, which it is no longer.
pub(super) async fn user(state: &AppState, email: &str) {
    let mode = |mode: &str| {
        state.settings.update(
            [("auth.registration".to_owned(), Change::Set(json!(mode)))],
            None,
        )
    };
    mode("open").await.expect("saved");
    state
        .auth
        .register(RegisterInput {
            email: email.into(),
            password: Secret::new(PASSWORD),
            display_name: None,
            invite_code: None,
            ip: "unknown".into(),
        })
        .await
        .expect("registers");
    mode("invite").await.expect("saved");
}

pub(super) async fn sign_in(app: &Router, email: &str) -> String {
    let answer = call(
        app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    answer.cookie_pair()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_session_cookie_and_a_token_both_reach_the_account(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    let app = router(state, false);

    let login = call(
        &app,
        Method::POST,
        "/api/v1/auth/login",
        &[("user-agent", "curl/8")],
        Some(json!({"email": "Admin@Example.com", "password": PASSWORD})),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    let cookie = login.cookie();
    assert!(cookie.starts_with("neon_session="), "{cookie}");
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
        assert!(cookie.contains(flag), "{cookie}");
    }
    assert!(!cookie.contains("Secure"), "plain http: {cookie}");
    assert_eq!(login.body["user"]["email"], "admin@example.com");
    let session = login.cookie_pair();

    let me = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(COOKIE.as_str(), &session)],
        None,
    )
    .await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["email"], "admin@example.com");
    assert_eq!(me.body["role"], "admin");
    assert_eq!(me.body["credential"], "session");
    assert_eq!(me.body["scopes"], json!(["read", "write", "admin"]));

    let created = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &[(COOKIE.as_str(), &session)],
        Some(json!({"name": "ci", "scopes": ["read"]})),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let token = created.body["token"].as_str().expect("a token").to_owned();
    assert!(token.starts_with("nw_"));
    let bearer = format!("Bearer {token}");

    let me = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(AUTHORIZATION.as_str(), &bearer)],
        None,
    )
    .await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.body["credential"], "token");
    assert_eq!(me.body["scopes"], json!(["read"]));

    // Read only: changing anything is refused, and so are the admin routes.
    let rename = call(
        &app,
        Method::PATCH,
        "/api/v1/account/me",
        &[(AUTHORIZATION.as_str(), &bearer)],
        Some(json!({"display_name": "Root"})),
    )
    .await;
    assert_eq!(
        (rename.status, rename.code()),
        (StatusCode::FORBIDDEN, "insufficient_scope")
    );
    let users = call(
        &app,
        Method::GET,
        "/api/v1/admin/users",
        &[(AUTHORIZATION.as_str(), &bearer)],
        None,
    )
    .await;
    assert_eq!(
        (users.status, users.code()),
        (StatusCode::FORBIDDEN, "insufficient_scope")
    );

    let logout = call(
        &app,
        Method::POST,
        "/api/v1/auth/logout",
        &[(COOKIE.as_str(), &session)],
        None,
    )
    .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert!(logout.cookie().contains("Max-Age=0"), "{}", logout.cookie());
    let after = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(COOKIE.as_str(), &session)],
        None,
    )
    .await;
    assert_eq!(
        (after.status, after.code()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
    assert!(
        after.cookie().contains("Max-Age=0"),
        "a stale cookie is cleared"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn requests_that_change_something_must_come_from_the_server_s_origin(pool: PgPool) {
    let public = Url::parse("https://neon.example.com").expect("a URL");
    let state = state_with(
        pool,
        Envelope::default(),
        WebConfig::new(Some(&public), Vec::new()),
    )
    .await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    assert!(session.starts_with("__Host-neon_session="), "{session}");
    let rename = |headers: Vec<(&'static str, String)>| {
        let app = app.clone();
        let session = session.clone();
        async move {
            let mut all: Vec<(&str, String)> = vec![(COOKIE.as_str(), session)];
            all.extend(headers);
            let borrowed: Vec<(&str, &str)> = all.iter().map(|(n, v)| (*n, v.as_str())).collect();
            call(
                &app,
                Method::PATCH,
                "/api/v1/account/me",
                &borrowed,
                Some(json!({"display_name": "Root"})),
            )
            .await
        }
    };

    let cross = rename(vec![(ORIGIN.as_str(), "https://evil.example".into())]).await;
    assert_eq!(
        (cross.status, cross.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    let referred = rename(vec![(REFERER.as_str(), "https://evil.example/page".into())]).await;
    assert_eq!(referred.status, StatusCode::FORBIDDEN);
    let other_scheme = rename(vec![(ORIGIN.as_str(), "http://neon.example.com".into())]).await;
    assert_eq!(other_scheme.status, StatusCode::FORBIDDEN);

    let same = rename(vec![(ORIGIN.as_str(), "https://neon.example.com".into())]).await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.body);
    let from_a_page = rename(vec![(
        REFERER.as_str(),
        "https://neon.example.com/settings".into(),
    )])
    .await;
    assert_eq!(from_a_page.status, StatusCode::OK);
    // Without either header, SameSite=Lax keeps the cookie off cross-site posts.
    let neither = rename(Vec::new()).await;
    assert_eq!(neither.status, StatusCode::OK);

    // Reading is safe, and a bearer token is never sent by a browser on its own.
    let read = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[
            (COOKIE.as_str(), &session),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        None,
    )
    .await;
    assert_eq!(read.status, StatusCode::OK);
    let token = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &[
            (COOKIE.as_str(), &session),
            (ORIGIN.as_str(), "https://neon.example.com"),
        ],
        Some(json!({"name": "ci", "scopes": ["write"]})),
    )
    .await;
    let bearer = format!("Bearer {}", token.body["token"].as_str().expect("a token"));
    let with_token = call(
        &app,
        Method::PATCH,
        "/api/v1/account/me",
        &[
            (AUTHORIZATION.as_str(), &bearer),
            (ORIGIN.as_str(), "https://evil.example"),
        ],
        Some(json!({"display_name": "Root"})),
    )
    .await;
    assert_eq!(with_token.status, StatusCode::OK, "{}", with_token.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn https_cookies_are_secure_and_host_prefixed(pool: PgPool) {
    let public = Url::parse("https://neon.example.com").expect("a URL");
    let state = state_with(
        pool,
        Envelope::default(),
        WebConfig::new(Some(&public), Vec::new()),
    )
    .await;
    admin(&state).await;
    let app = router(state, false);

    let login = call(
        &app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        Some(json!({"email": "admin@example.com", "password": PASSWORD})),
    )
    .await;

    let cookie = login.cookie();
    assert!(cookie.starts_with("__Host-neon_session="), "{cookie}");
    assert!(
        cookie.contains("; Secure") && cookie.contains("Path=/"),
        "{cookie}"
    );
    // The plain name is not read over https.
    let value = login
        .cookie_pair()
        .split_once('=')
        .expect("a pair")
        .1
        .to_owned();
    let plain = format!("neon_session={value}");
    let me = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(COOKIE.as_str(), &plain)],
        None,
    )
    .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn unauthenticated_and_unauthorized_answers_use_the_error_shape(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);

    let anonymous = call(&app, Method::GET, "/api/v1/account/me", &[], None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.body,
        json!({"error": {"code": "unauthenticated", "message": "sign in first"}})
    );
    let bad_header = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(AUTHORIZATION.as_str(), "Basic abc")],
        None,
    )
    .await;
    assert_eq!(
        (bad_header.status, bad_header.code()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
    let forged = call(
        &app,
        Method::GET,
        "/api/v1/account/me",
        &[(AUTHORIZATION.as_str(), "Bearer nw_forged")],
        None,
    )
    .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    let ada = sign_in(&app, "ada@example.com").await;
    let users = call(
        &app,
        Method::GET,
        "/api/v1/admin/users",
        &[(COOKIE.as_str(), &ada)],
        None,
    )
    .await;
    assert_eq!(users.status, StatusCode::FORBIDDEN);
    assert_eq!(
        users.body,
        json!({"error": {"code": "forbidden", "message": "only an admin may do this"}})
    );

    let wrong = call(
        &app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        Some(json!({"email": "ada@example.com", "password": "not the password"})),
    )
    .await;
    let unknown = call(
        &app,
        Method::POST,
        "/api/v1/auth/login",
        &[],
        Some(json!({"email": "nobody@example.com", "password": "not the password"})),
    )
    .await;
    assert_eq!(
        (wrong.status, wrong.code()),
        (StatusCode::UNAUTHORIZED, "invalid_credentials")
    );
    assert_eq!(wrong.body, unknown.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn too_many_sign_ins_answer_429_with_retry_after(pool: PgPool) {
    let app = router(state(pool).await, false);
    let attempt = || {
        call(
            &app,
            Method::POST,
            "/api/v1/auth/login",
            &[],
            Some(json!({"email": "ada@example.com", "password": "not the password"})),
        )
    };

    for _ in 0..10 {
        assert_eq!(attempt().await.status, StatusCode::UNAUTHORIZED);
    }
    let limited = attempt().await;

    assert_eq!(
        (limited.status, limited.code()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    );
    let wait: u64 = limited.headers[RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=30).contains(&wait), "{wait}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sign_ups_follow_the_registration_mode(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);

    let config = call(&app, Method::GET, "/api/v1/auth/config", &[], None).await;
    assert_eq!(
        config.body,
        json!({
            "registration": "invite",
            "min_password_length": 12,
            "verify_email": false,
            "password_reset": false,
            "requests": false,
        })
    );
    let refused = call(
        &app,
        Method::POST,
        "/api/v1/auth/register",
        &[],
        Some(json!({"email": "ada@example.com", "password": PASSWORD})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "invalid_invite")
    );

    state
        .settings
        .update(
            [("auth.registration".to_owned(), Change::Set(json!("open")))],
            None,
        )
        .await
        .expect("saved");
    let registered = call(
        &app,
        Method::POST,
        "/api/v1/auth/register",
        &[],
        Some(json!({"email": "ada@example.com", "password": PASSWORD, "display_name": "Ada"})),
    )
    .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.body
    );
    assert_eq!(registered.body["user"]["display_name"], "Ada");
    assert_eq!(registered.body["must_verify"], false);
    assert!(registered.body.get("sent").is_none());
    assert!(
        registered.headers.get(SET_COOKIE).is_none(),
        "signing up signs nobody in"
    );

    let unknown_field = call(
        &app,
        Method::POST,
        "/api/v1/auth/register",
        &[],
        Some(json!({"email": "bob@example.com", "password": PASSWORD, "admin": true})),
    )
    .await;
    assert_eq!(unknown_field.code(), "invalid_json");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn preferences_are_kept_and_checked(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let cookie = [(COOKIE.as_str(), session.as_str())];

    let defaults = call(
        &app,
        Method::GET,
        "/api/v1/account/preferences",
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        defaults.body,
        json!({"palette": "nothing", "mode": "system", "accent_hue": null, "ui_font": "default"})
    );
    let chosen = json!({"palette": "warm", "mode": "dark", "accent_hue": 25, "ui_font": "mono"});
    let saved = call(
        &app,
        Method::PUT,
        "/api/v1/account/preferences",
        &cookie,
        Some(chosen.clone()),
    )
    .await;
    assert_eq!((saved.status, &saved.body), (StatusCode::OK, &chosen));
    let read = call(
        &app,
        Method::GET,
        "/api/v1/account/preferences",
        &cookie,
        None,
    )
    .await;
    assert_eq!(read.body, chosen);
    let following =
        json!({"palette": "warm", "mode": "dark", "accent_hue": null, "ui_font": "mono"});
    let unset = call(
        &app,
        Method::PUT,
        "/api/v1/account/preferences",
        &cookie,
        Some(following.clone()),
    )
    .await;
    assert_eq!((unset.status, &unset.body), (StatusCode::OK, &following));

    let hue = call(
        &app,
        Method::PUT,
        "/api/v1/account/preferences",
        &cookie,
        Some(json!({"palette": "warm", "mode": "dark", "accent_hue": 360, "ui_font": "mono"})),
    )
    .await;
    assert_eq!(
        (hue.status, hue.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    let palette = call(
        &app,
        Method::PUT,
        "/api/v1/account/preferences",
        &cookie,
        Some(json!({"palette": "sepia", "mode": "dark", "accent_hue": 1, "ui_font": "mono"})),
    )
    .await;
    assert_eq!(palette.code(), "invalid_json");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_read_and_change_settings_with_their_sources(pool: PgPool) {
    let state = state(pool).await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let cookie = [(COOKIE.as_str(), session.as_str())];

    let listed = call(&app, Method::GET, "/api/v1/admin/settings", &cookie, None).await;
    assert_eq!(listed.status, StatusCode::OK);
    let registration = listed.body["settings"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|setting| setting["key"] == "auth.registration")
        .expect("listed")
        .clone();
    assert_eq!(registration["kind"], "enum");
    assert_eq!(registration["options"], json!(["open", "invite", "closed"]));
    assert_eq!(registration["value"], "invite");
    assert_eq!(registration["source"], "default");
    assert_eq!(registration["pinned"], false);

    let refused = call(
        &app,
        Method::PATCH,
        "/api/v1/admin/settings",
        &cookie,
        Some(json!({"changes": {"auth.min_password_length": 4, "auth.requests": true, "auth.registration": "open"}})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "invalid_settings")
    );
    assert_eq!(
        refused.body["error"]["problems"],
        json!([{"key": "auth.min_password_length", "message": "must be between 8 and 128"}])
    );

    let saved = call(
        &app,
        Method::PATCH,
        "/api/v1/admin/settings",
        &cookie,
        Some(json!({"changes": {"auth.invite_ttl": "3d", "instance.name": null}})),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    let ttl = saved.body["settings"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|setting| setting["key"] == "auth.invite_ttl")
        .expect("listed")
        .clone();
    assert_eq!(
        (ttl["value"].clone(), ttl["source"].clone()),
        (json!("3d"), json!("database"))
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_make_invites_and_copy_their_codes(pool: PgPool) {
    let key = format!("k1:{}", STANDARD.encode([9u8; 32]));
    let envelope = Envelope::new(Keyring::parse(&[key]).expect("a key"));
    let state = state_with(pool, envelope, WebConfig::new(None, Vec::new())).await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let cookie = [(COOKIE.as_str(), session.as_str())];

    let created = call(
        &app,
        Method::POST,
        "/api/v1/admin/invites",
        &cookie,
        Some(json!({"code": "Team-Spring-2026", "max_uses": 3})),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["code"], "Team-Spring-2026");
    assert_eq!(created.body["invite"]["status"], "open");
    assert!(created.body.get("queued").is_none());
    let id = created.body["invite"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let copied = call(
        &app,
        Method::GET,
        &format!("/api/v1/admin/invites/{id}/code"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(copied.body, json!({"code": "Team-Spring-2026"}));
    let checked = call(
        &app,
        Method::POST,
        "/api/v1/admin/invites/check",
        &cookie,
        Some(json!({"code": "team-spring-2026"})),
    )
    .await;
    assert_eq!(
        checked.body,
        json!({"problem": "another invite has this code"})
    );
    let listed = call(&app, Method::GET, "/api/v1/admin/invites", &cookie, None).await;
    assert!(
        listed.body["invites"][0].get("code").is_none(),
        "lists carry no codes"
    );
    assert_eq!(listed.body["by_email"], false);

    let lookup = call(
        &app,
        Method::POST,
        "/api/v1/auth/invite",
        &[],
        Some(json!({"code": "TEAM-spring-2026"})),
    )
    .await;
    assert_eq!(lookup.body, json!({"email": null}));
    let mailed = call(
        &app,
        Method::POST,
        "/api/v1/admin/invites",
        &cookie,
        Some(json!({"email": "ada@example.com"})),
    )
    .await;
    assert_eq!(
        (mailed.status, mailed.code()),
        (StatusCode::FORBIDDEN, "invite_email_off")
    );

    let revoked = call(
        &app,
        Method::POST,
        &format!("/api/v1/admin/invites/{id}/revoke"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let gone = call(
        &app,
        Method::POST,
        "/api/v1/auth/invite",
        &[],
        Some(json!({"code": "Team-Spring-2026"})),
    )
    .await;
    assert_eq!(
        (gone.status, gone.code()),
        (StatusCode::BAD_REQUEST, "invalid_invite")
    );
}

pub(super) fn keyed() -> Envelope {
    let key = format!("k1:{}", STANDARD.encode([9u8; 32]));
    Envelope::new(Keyring::parse(&[key]).expect("a key"))
}

/// The settings of `key` in a settings list.
fn setting<'a>(list: &'a Value, key: &str) -> &'a Value {
    list["settings"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|setting| setting["key"] == key)
        .unwrap_or_else(|| panic!("{key} is not listed"))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_set_mail_up_and_send_themselves_a_test(pool: PgPool) {
    let state = state_with(pool.clone(), keyed(), WebConfig::new(None, Vec::new())).await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let cookie = [(COOKIE.as_str(), session.as_str())];

    let listed = call(&app, Method::GET, "/api/v1/admin/settings", &cookie, None).await;
    assert_eq!(
        listed.body["mail"],
        json!({"configured": false, "problem": "mail.enabled is off", "last_sent_at": null, "last_failure": null})
    );
    assert_eq!(setting(&listed.body, "mail.host")["browser_only"], true);
    assert_eq!(setting(&listed.body, "mail.password")["browser_only"], true);
    assert_eq!(setting(&listed.body, "mail.from")["browser_only"], false);
    let off = call(&app, Method::POST, "/api/v1/admin/mail/test", &cookie, None).await;
    assert_eq!(
        (off.status, off.code()),
        (StatusCode::CONFLICT, "mail_not_set_up")
    );
    assert_eq!(
        off.body["error"]["message"],
        "mail is not set up: mail.enabled is off"
    );

    let saved = call(
        &app,
        Method::PATCH,
        "/api/v1/admin/settings",
        &cookie,
        Some(json!({"changes": {
            "mail.enabled": true,
            "mail.host": "smtp.example.com",
            "mail.username": "neon",
            "mail.password": "hunter2-smtp",
            "mail.from": "neon@example.com",
        }})),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.body["mail"]["configured"], true);
    assert_eq!(setting(&saved.body, "mail.password")["value"], "[redacted]");
    let stored: Value =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'mail.password'")
            .fetch_one(&pool)
            .await
            .expect("saved");
    assert!(!stored.to_string().contains("hunter2"), "{stored}");

    let sent = call(&app, Method::POST, "/api/v1/admin/mail/test", &cookie, None).await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{}", sent.body);
    assert_eq!(
        sent.body,
        json!({"queued": true, "to": "admin@example.com"})
    );
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'mail.send'")
        .fetch_one(&pool)
        .await
        .expect("counted");
    assert_eq!(queued, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn api_tokens_cannot_change_where_secrets_go(pool: PgPool) {
    let state = state_with(pool, keyed(), WebConfig::new(None, Vec::new())).await;
    admin(&state).await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let created = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &[(COOKIE.as_str(), &session)],
        Some(json!({"name": "ops", "scopes": ["read", "write", "admin"]})),
    )
    .await;
    let bearer = format!(
        "Bearer {}",
        created.body["token"].as_str().expect("a token")
    );
    let token = [(AUTHORIZATION.as_str(), bearer.as_str())];

    let refused = call(
        &app,
        Method::PATCH,
        "/api/v1/admin/settings",
        &token,
        Some(json!({"changes": {
            "instance.name": "Renamed",
            "mail.host": "smtp.attacker.example",
            "mail.password": "theirs",
        }})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    let keys: Vec<&str> = refused.body["error"]["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .map(|problem| problem["key"].as_str().expect("a key"))
        .collect();
    assert_eq!(keys, ["mail.host", "mail.password"]);
    let listed = call(&app, Method::GET, "/api/v1/admin/settings", &token, None).await;
    assert_eq!(
        setting(&listed.body, "instance.name")["value"],
        "Neon Teamspace",
        "nothing was saved"
    );

    let renamed = call(
        &app,
        Method::PATCH,
        "/api/v1/admin/settings",
        &token,
        Some(json!({"changes": {"instance.name": "Renamed", "mail.from": "neon@example.com"}})),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
}

/// The settings that say where secrets go change only at a browser, so a token must not
/// make an account to sign in to one with, nor promote one it made through an invite.
#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn api_tokens_cannot_make_an_admin_to_sign_in_with(pool: PgPool) {
    let state = state_with(pool, keyed(), WebConfig::new(None, Vec::new())).await;
    admin(&state).await;
    user(&state, "bob@example.com").await;
    let app = router(state, false);
    let session = sign_in(&app, "admin@example.com").await;
    let created = call(
        &app,
        Method::POST,
        "/api/v1/account/tokens",
        &[(COOKIE.as_str(), &session)],
        Some(json!({"name": "ops", "scopes": ["read", "write", "admin"]})),
    )
    .await;
    // The scheme is case-insensitive.
    let bearer = format!(
        "bearer {}",
        created.body["token"].as_str().expect("a token")
    );
    let token = [(AUTHORIZATION.as_str(), bearer.as_str())];

    for role in ["admin", "user"] {
        let made = call(
            &app,
            Method::POST,
            "/api/v1/admin/users",
            &token,
            Some(json!({"email": "mallory@example.com", "password": PASSWORD, "role": role})),
        )
        .await;
        assert_eq!(
            (made.status, made.code()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{role}"
        );
    }
    let listed = call(&app, Method::GET, "/api/v1/admin/users", &token, None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let users = listed.body["users"].as_array().expect("users");
    assert_eq!(users.len(), 2, "nobody was made");
    let bob = users
        .iter()
        .find(|user| user["email"] == "bob@example.com")
        .expect("bob");
    let path = format!("/api/v1/admin/users/{}", bob["id"].as_str().expect("an id"));

    let promoted = call(
        &app,
        Method::PATCH,
        &path,
        &token,
        Some(json!({"role": "admin"})),
    )
    .await;
    assert_eq!(
        (promoted.status, promoted.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    let verified = call(
        &app,
        Method::PATCH,
        &path,
        &token,
        Some(json!({"email_verified": true})),
    )
    .await;
    assert_eq!(verified.status, StatusCode::OK, "{}", verified.body);
    assert_eq!(verified.body["role"], "user");

    let by_browser = call(
        &app,
        Method::PATCH,
        &path,
        &[(COOKIE.as_str(), &session)],
        Some(json!({"role": "admin"})),
    )
    .await;
    assert_eq!(by_browser.status, StatusCode::OK, "{}", by_browser.body);
    assert_eq!(by_browser.body["role"], "admin");
}
