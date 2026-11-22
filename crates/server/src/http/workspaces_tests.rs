//! Workspaces over HTTP: the routes, their answers and error codes, the scopes they need,
//! the origin check, and what instance admins see.

use axum::Router;
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN, RETRY_AFTER};
use axum::http::{Method, StatusCode};
use neon_app::crypto::Envelope;
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;

use super::auth_tests::{Answer, admin, call, sign_in, user};
use super::tests::{state, state_with};
use super::*;

/// A signed-in browser: its session cookie.
struct Browser<'a> {
    app: &'a Router,
    cookie: String,
}

impl<'a> Browser<'a> {
    async fn sign_in(app: &'a Router, email: &str) -> Self {
        Self {
            app,
            cookie: sign_in(app, email).await,
        }
    }

    async fn send(&self, method: Method, uri: &str, body: Option<Value>) -> Answer {
        call(
            self.app,
            method,
            uri,
            &[(COOKIE.as_str(), &self.cookie)],
            body,
        )
        .await
    }

    async fn get(&self, uri: &str) -> Answer {
        self.send(Method::GET, uri, None).await
    }

    async fn post(&self, uri: &str, body: Value) -> Answer {
        self.send(Method::POST, uri, Some(body)).await
    }

    /// An API token with `scopes`, as a bearer header.
    async fn token(&self, scopes: &[&str]) -> String {
        let created = self
            .post(
                "/api/v1/account/tokens",
                json!({"name": "ci", "scopes": scopes}),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        format!(
            "Bearer {}",
            created.body["token"].as_str().expect("a token")
        )
    }
}

fn id(answer: &Answer) -> String {
    answer.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id: {}", answer.body))
        .to_owned()
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name"))
        .collect()
}

/// Ada owns Design; Bob is its admin, Cy and Dee its members, all through invitations
/// they accepted.
struct Team<'a> {
    ada: Browser<'a>,
    bob: Browser<'a>,
    cy: Browser<'a>,
    dee: Browser<'a>,
    workspace: String,
}

async fn team<'a>(state: &AppState, app: &'a Router) -> Team<'a> {
    for email in [
        "ada@example.com",
        "bob@example.com",
        "cy@example.com",
        "dee@example.com",
    ] {
        user(state, email).await;
    }
    let ada = Browser::sign_in(app, "ada@example.com").await;
    let created = ada
        .post("/api/v1/workspaces", json!({"name": "Design"}))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let workspace = id(&created);
    let team = Team {
        bob: Browser::sign_in(app, "bob@example.com").await,
        cy: Browser::sign_in(app, "cy@example.com").await,
        dee: Browser::sign_in(app, "dee@example.com").await,
        ada,
        workspace,
    };
    for (who, role) in [
        (&team.bob, "admin"),
        (&team.cy, "member"),
        (&team.dee, "member"),
    ] {
        let me = who.get("/api/v1/account/me").await;
        let email = me.body["email"].as_str().expect("an address");
        let sent = team
            .ada
            .post(
                &format!("/api/v1/workspaces/{}/invitations", team.workspace),
                json!({"email": email, "role": role}),
            )
            .await;
        assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
        let accepted = who
            .post(
                &format!("/api/v1/account/invitations/{}/accept", id(&sent)),
                json!({}),
            )
            .await;
        assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    }
    team
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_workspace_is_created_listed_and_read(pool: PgPool) {
    let state = state(pool).await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let ada = Browser::sign_in(&app, "ada@example.com").await;

    let created = ada
        .post(
            "/api/v1/workspaces",
            json!({"name": "Design Team", "icon": "🚀", "hue_override": 12}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["slug"], "design-team");
    assert_eq!(created.body["role"], "owner");
    assert_eq!(created.body["hue"], 12);
    assert_eq!(created.body["hue_override"], 12);
    assert_eq!(created.body["member_count"], 1);
    assert_eq!(created.body["members_can_create_teamspaces"], true);
    assert_eq!(
        created.body["limits"],
        json!({"members": null, "storage_bytes": null, "agents": null})
    );
    let workspace = id(&created);

    let listed = ada.get("/api/v1/workspaces").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body["workspaces"][0]["id"], workspace);
    assert_eq!(listed.body["workspaces"][0]["role"], "owner");
    let read = ada.get(&format!("/api/v1/workspaces/{workspace}")).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["icon"], "🚀");

    let cleared = ada
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{workspace}"),
            Some(json!({"icon": null, "hue_override": null, "name": "Studio"})),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.body);
    assert_eq!(cleared.body["icon"], Value::Null);
    assert_eq!(cleared.body["hue_override"], Value::Null);
    assert_eq!(cleared.body["name"], "Studio");
    assert_eq!(cleared.body["slug"], "design-team");
    let unknown = ada
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{workspace}"),
            Some(json!({"owner": "me"})),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.code(), "invalid_json");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_invited_account_accepts_and_teamspaces_show_as_their_access_says(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let base = format!("/api/v1/workspaces/{}", t.workspace);

    for (access, by) in [("open", &t.ada), ("closed", &t.ada), ("private", &t.cy)] {
        let created = by
            .post(
                &format!("{base}/teamspaces"),
                json!({"name": access.to_uppercase(), "access": access}),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        assert_eq!(created.body["role"], "owner");
    }

    let by_member = t.dee.get(&format!("{base}/teamspaces")).await;
    assert_eq!(by_member.status, StatusCode::OK);
    assert_eq!(
        names(&by_member.body["teamspaces"]),
        ["General", "CLOSED", "OPEN"],
        "a member outside the private teamspace does not see it"
    );
    assert_eq!(by_member.body["hidden"], json!([]));
    let by_admin = t.bob.get(&format!("{base}/teamspaces")).await;
    assert_eq!(
        names(&by_admin.body["teamspaces"]),
        ["General", "CLOSED", "OPEN"]
    );
    assert_eq!(names(&by_admin.body["hidden"]), ["PRIVATE"]);
    assert_eq!(
        by_admin.body["hidden"][0].as_object().map(|o| o.len()),
        Some(2),
        "its id and name only"
    );
    let private = by_admin.body["hidden"][0]["id"].as_str().expect("an id");
    let peek = t.dee.get(&format!("{base}/teamspaces/{private}")).await;
    assert_eq!(
        (peek.status, peek.code()),
        (StatusCode::NOT_FOUND, "not_found")
    );
    // The admin outside it gets its name and no more, whatever they ask.
    let by_admin_outside = [
        t.bob.get(&format!("{base}/teamspaces/{private}")).await,
        t.bob
            .get(&format!("{base}/teamspaces/{private}/members"))
            .await,
        t.bob
            .send(
                Method::PATCH,
                &format!("{base}/teamspaces/{private}"),
                Some(json!({"icon": "X"})),
            )
            .await,
        t.bob
            .post(
                &format!("{base}/teamspaces/{private}/members"),
                json!({"user_id": t.dee.get("/api/v1/account/me").await.body["id"]}),
            )
            .await,
    ];
    for answer in by_admin_outside {
        assert_eq!(
            (answer.status, answer.code()),
            (StatusCode::NOT_FOUND, "not_found"),
            "{}",
            answer.body
        );
    }
    let joined = t
        .bob
        .post(&format!("{base}/teamspaces/{private}/join"), json!({}))
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert_eq!(joined.body["role"], "member");
    let in_it = t.bob.get(&format!("{base}/teamspaces/{private}")).await;
    assert_eq!(in_it.status, StatusCode::OK, "in it, they see it in full");
    let by_owner_inside = t.cy.get(&format!("{base}/teamspaces/{private}")).await;
    assert_eq!(by_owner_inside.status, StatusCode::OK);
    assert_eq!(by_owner_inside.body["access"], "private");

    let members = t.dee.get(&format!("{base}/members")).await;
    assert_eq!(members.body["members"].as_array().map(Vec::len), Some(4));
    assert_eq!(members.body["members"][0]["role"], "owner");
    assert_eq!(
        members.body["members"][0]["user"]["email"],
        "ada@example.com"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invitations_answer_their_error_codes(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let invitations = format!("/api/v1/workspaces/{}/invitations", t.workspace);

    let nobody = t
        .ada
        .post(
            &invitations,
            json!({"email": "nobody@example.com", "role": "member"}),
        )
        .await;
    assert_eq!(
        (nobody.status, nobody.code()),
        (StatusCode::NOT_FOUND, "no_account")
    );
    let member = t
        .ada
        .post(
            &invitations,
            json!({"email": "cy@example.com", "role": "member"}),
        )
        .await;
    assert_eq!(
        (member.status, member.code()),
        (StatusCode::CONFLICT, "already_member")
    );
    let owner = t
        .ada
        .post(
            &invitations,
            json!({"email": "nobody@example.com", "role": "owner"}),
        )
        .await;
    assert_eq!(
        (owner.status, owner.code()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    let by_member =
        t.cy.post(
            &invitations,
            json!({"email": "nobody@example.com", "role": "guest"}),
        )
        .await;
    assert_eq!(by_member.status, StatusCode::FORBIDDEN);
    let listed = t.ada.get(&invitations).await;
    assert_eq!(listed.body["invitations"], json!([]));
    let mine = t.cy.get("/api/v1/account/invitations").await;
    assert_eq!(mine.body["invitations"], json!([]));
    let stranger =
        t.cy.post(
            &format!(
                "/api/v1/account/invitations/{}/decline",
                uuid::Uuid::now_v7()
            ),
            json!({}),
        )
        .await;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn address_lookups_answer_429_past_the_limit(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let invitations = format!("/api/v1/workspaces/{}/invitations", t.workspace);

    let mut last = None;
    for n in 0..20 {
        let answer = t
            .bob
            .post(
                &invitations,
                json!({"email": format!("probe{n}@example.com"), "role": "member"}),
            )
            .await;
        if answer.status == StatusCode::TOO_MANY_REQUESTS {
            last = Some(answer);
            break;
        }
        assert_eq!(answer.code(), "no_account");
    }
    let limited = last.expect("the lookups are limited");
    assert_eq!(limited.code(), "rate_limited");
    assert!(limited.headers.contains_key(RETRY_AFTER));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invite_links_show_their_token_once_and_admit_signed_in_accounts(pool: PgPool) {
    let state = state_with(pool, keyed(), WebConfig::new(None, Vec::new())).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    user(&state, "eve@example.com").await;
    let eve = Browser::sign_in(&app, "eve@example.com").await;
    let links = format!("/api/v1/workspaces/{}/invite-links", t.workspace);

    let created = t
        .bob
        .post(&links, json!({"role": "guest", "max_uses": 2}))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let token = created.body["token"].as_str().expect("a token").to_owned();
    assert!(token.starts_with("wsl_"));
    assert_eq!(created.body["link"]["status"], "open");
    let link = created.body["link"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let listed = t.ada.get(&links).await;
    assert_eq!(listed.body["links"][0]["id"], link);
    assert!(
        listed.body["links"][0].get("token").is_none(),
        "lists carry no token"
    );
    let copied = t.ada.get(&format!("{links}/{link}/token")).await;
    assert_eq!(copied.body["token"], token);

    let anonymous = call(
        &app,
        Method::POST,
        "/api/v1/workspace-invite-links/join",
        &[],
        Some(json!({"token": token})),
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let joined = eve
        .post(
            "/api/v1/workspace-invite-links/join",
            json!({"token": token}),
        )
        .await;
    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert_eq!(joined.body["role"], "guest");
    assert_eq!(joined.body["id"], t.workspace);
    let twice = eve
        .post(
            "/api/v1/workspace-invite-links/join",
            json!({"token": token}),
        )
        .await;
    assert_eq!(
        (twice.status, twice.code()),
        (StatusCode::CONFLICT, "already_member")
    );
    let guests_list = eve
        .get(&format!("/api/v1/workspaces/{}/members", t.workspace))
        .await;
    assert_eq!(
        guests_list.body["members"].as_array().map(Vec::len),
        Some(1)
    );
    let as_guest = eve
        .get(&format!("/api/v1/workspaces/{}/teamspaces", t.workspace))
        .await;
    assert_eq!(as_guest.body["teamspaces"], json!([]));
    let seen_by_guest = eve
        .get(&format!("/api/v1/workspaces/{}", t.workspace))
        .await;
    assert_eq!(seen_by_guest.body["role"], "guest");
    assert!(
        seen_by_guest.body.get("member_count").is_none(),
        "a guest learns no head count: {}",
        seen_by_guest.body
    );
    let seen_by_member =
        t.cy.get(&format!("/api/v1/workspaces/{}", t.workspace))
            .await;
    assert_eq!(seen_by_member.body["member_count"], 5);

    let revoked = t
        .ada
        .post(&format!("{links}/{link}/revoke"), json!({}))
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let after = t
        .dee
        .post(
            "/api/v1/workspace-invite-links/join",
            json!({"token": token}),
        )
        .await;
    assert_eq!(
        (after.status, after.code()),
        (StatusCode::BAD_REQUEST, "invalid_invite_link")
    );
    let admin_role = t.ada.post(&links, json!({"role": "admin"})).await;
    assert_eq!(admin_role.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_invitation_ends_when_its_account_joins_by_link(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    user(&state, "eve@example.com").await;
    let eve = Browser::sign_in(&app, "eve@example.com").await;
    let eve_id = eve.get("/api/v1/account/me").await.body["id"].clone();
    let base = format!("/api/v1/workspaces/{}", t.workspace);

    let invited = t
        .ada
        .post(
            &format!("{base}/invitations"),
            json!({"email": "eve@example.com", "role": "admin"}),
        )
        .await;
    assert_eq!(invited.status, StatusCode::CREATED, "{}", invited.body);
    let link = t
        .ada
        .post(&format!("{base}/invite-links"), json!({"role": "guest"}))
        .await;
    let joined = eve
        .post(
            "/api/v1/workspace-invite-links/join",
            json!({"token": link.body["token"]}),
        )
        .await;
    assert_eq!(joined.body["role"], "guest");
    let pending = t.ada.get(&format!("{base}/invitations")).await;
    assert_eq!(pending.body["invitations"], json!([]));

    let removed = t
        .ada
        .send(
            Method::DELETE,
            &format!("{base}/members/{}", eve_id.as_str().expect("an id")),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let back = eve
        .post(
            &format!("/api/v1/account/invitations/{}/accept", id(&invited)),
            json!({}),
        )
        .await;
    assert_eq!(
        (back.status, back.code()),
        (StatusCode::NOT_FOUND, "not_found"),
        "{}",
        back.body
    );
}

fn keyed() -> Envelope {
    use base64::Engine as _;
    let entry = format!(
        "k1:{}",
        base64::engine::general_purpose::STANDARD.encode([7u8; 32])
    );
    Envelope::new(neon_app::crypto::Keyring::parse(&[entry]).expect("a valid key"))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn members_and_owners_keep_to_their_roles(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let base = format!("/api/v1/workspaces/{}", t.workspace);
    let ada_id = t.ada.get("/api/v1/account/me").await.body["id"].clone();
    let cy_id = t.cy.get("/api/v1/account/me").await.body["id"].clone();
    let ada_id = ada_id.as_str().expect("an id");
    let cy_id = cy_id.as_str().expect("an id");

    let last = t.ada.post(&format!("{base}/leave"), json!({})).await;
    assert_eq!(
        (last.status, last.code()),
        (StatusCode::CONFLICT, "last_owner")
    );
    let on_owner = t
        .bob
        .send(
            Method::PATCH,
            &format!("{base}/members/{ada_id}"),
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(on_owner.status, StatusCode::FORBIDDEN);
    let promoted = t
        .bob
        .send(
            Method::PATCH,
            &format!("{base}/members/{cy_id}"),
            Some(json!({"role": "admin"})),
        )
        .await;
    assert_eq!(promoted.status, StatusCode::OK, "{}", promoted.body);
    assert_eq!(promoted.body["role"], "admin");
    assert_eq!(promoted.body["user"]["id"], cy_id);

    let transferred = t
        .ada
        .post(&format!("{base}/transfer"), json!({"user_id": cy_id}))
        .await;
    assert_eq!(
        transferred.status,
        StatusCode::NO_CONTENT,
        "{}",
        transferred.body
    );
    let mistyped =
        t.cy.send(
            Method::DELETE,
            &base,
            Some(json!({"confirm_name": "design"})),
        )
        .await;
    assert_eq!(
        (mistyped.status, mistyped.code()),
        (StatusCode::BAD_REQUEST, "confirmation_mismatch")
    );
    let removed =
        t.cy.send(Method::DELETE, &format!("{base}/members/{ada_id}"), None)
            .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let deleted =
        t.cy.send(
            Method::DELETE,
            &base,
            Some(json!({"confirm_name": "Design"})),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(t.cy.get(&base).await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn tokens_need_the_scope_of_the_method_and_cannot_delete(pool: PgPool) {
    let state = state(pool).await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let ada = Browser::sign_in(&app, "ada@example.com").await;
    let reader = ada.token(&["read"]).await;
    let writer = ada.token(&["write"]).await;
    let with = |bearer: &str, method: Method, uri: &str, body: Option<Value>| {
        let app = app.clone();
        let bearer = bearer.to_owned();
        let uri = uri.to_owned();
        async move {
            call(
                &app,
                method,
                &uri,
                &[(AUTHORIZATION.as_str(), &bearer)],
                body,
            )
            .await
        }
    };

    let refused = with(
        &reader,
        Method::POST,
        "/api/v1/workspaces",
        Some(json!({"name": "Design"})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::FORBIDDEN, "insufficient_scope")
    );
    let created = with(
        &writer,
        Method::POST,
        "/api/v1/workspaces",
        Some(json!({"name": "Design"})),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let workspace = id(&created);
    let listed = with(&reader, Method::GET, "/api/v1/workspaces", None).await;
    assert_eq!(listed.status, StatusCode::OK);
    let deleted = with(
        &writer,
        Method::DELETE,
        &format!("/api/v1/workspaces/{workspace}"),
        Some(json!({"confirm_name": "Design"})),
    )
    .await;
    assert_eq!(
        (deleted.status, deleted.code()),
        (StatusCode::FORBIDDEN, "forbidden"),
        "deleting needs a browser session"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn changes_from_another_origin_are_refused(pool: PgPool) {
    let public = Url::parse("https://neon.example.com").expect("a URL");
    let state = state_with(
        pool,
        Envelope::default(),
        WebConfig::new(Some(&public), Vec::new()),
    )
    .await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let cookie = sign_in(&app, "ada@example.com").await;
    let create = |origin: &'static str| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/api/v1/workspaces",
                &[(COOKIE.as_str(), &cookie), (ORIGIN.as_str(), origin)],
                Some(json!({"name": "Design"})),
            )
            .await
        }
    };

    let cross = create("https://evil.example").await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);
    let same = create("https://neon.example.com").await;
    assert_eq!(same.status, StatusCode::CREATED, "{}", same.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn instance_admins_list_limit_and_delete_but_never_enter(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    admin(&state).await;
    let t = team(&state, &app).await;
    let root = Browser::sign_in(&app, "admin@example.com").await;

    let listed = root.get("/api/v1/admin/workspaces").await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let entry = &listed.body["workspaces"][0];
    assert_eq!(entry["name"], "Design");
    assert_eq!(entry["member_count"], 4);
    assert_eq!(entry["owners"][0]["email"], "ada@example.com");
    assert_eq!(
        entry["limits"]["members"],
        json!({"own": null, "effective": null})
    );
    assert_eq!(entry["usage"]["members"], 4);
    assert_eq!(listed.body["next"], Value::Null);

    let limited = root
        .send(
            Method::PATCH,
            &format!("/api/v1/admin/workspaces/{}/limits", t.workspace),
            Some(json!({"members": 4})),
        )
        .await;
    assert_eq!(limited.status, StatusCode::OK, "{}", limited.body);
    assert_eq!(
        limited.body["limits"]["members"],
        json!({"own": 4, "effective": 4})
    );
    let full = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{}/invite-links", t.workspace),
            json!({"role": "member"}),
        )
        .await;
    assert_eq!(
        (full.status, full.code()),
        (StatusCode::CONFLICT, "member_limit")
    );

    for uri in [
        format!("/api/v1/workspaces/{}", t.workspace),
        format!("/api/v1/workspaces/{}/members", t.workspace),
        format!("/api/v1/workspaces/{}/teamspaces", t.workspace),
    ] {
        let inside = root.get(&uri).await;
        assert_eq!(inside.status, StatusCode::NOT_FOUND, "{uri}");
    }
    let by_member = t.ada.get("/api/v1/admin/workspaces").await;
    assert_eq!(by_member.status, StatusCode::FORBIDDEN);

    let deleted = root
        .send(
            Method::DELETE,
            &format!("/api/v1/admin/workspaces/{}", t.workspace),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = t.ada.get("/api/v1/workspaces").await;
    assert_eq!(gone.body["workspaces"], json!([]));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn accounts_show_their_hue_and_may_pin_one(pool: PgPool) {
    let state = state(pool).await;
    user(&state, "ada@example.com").await;
    let app = router(state, false);
    let ada = Browser::sign_in(&app, "ada@example.com").await;

    let me = ada.get("/api/v1/account/me").await;
    let id = uuid::Uuid::parse_str(me.body["id"].as_str().expect("an id")).expect("a UUID");
    assert_eq!(me.body["hue"], neon_domain::hue::hue_for_uuid(id));
    assert_eq!(me.body["hue_override"], Value::Null);

    let pinned = ada
        .send(
            Method::PATCH,
            "/api/v1/account/me",
            Some(json!({"hue_override": 300})),
        )
        .await;
    assert_eq!(pinned.status, StatusCode::OK, "{}", pinned.body);
    assert_eq!(pinned.body["hue"], 300);
    let too_far = ada
        .send(
            Method::PATCH,
            "/api/v1/account/me",
            Some(json!({"hue_override": 360})),
        )
        .await;
    assert_eq!(
        (too_far.status, too_far.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    let renamed = ada
        .send(
            Method::PATCH,
            "/api/v1/account/me",
            Some(json!({"display_name": "Ada L", "hue_override": null})),
        )
        .await;
    assert_eq!(renamed.body["display_name"], "Ada L");
    assert_eq!(renamed.body["hue_override"], Value::Null);

    let half_bad = ada
        .send(
            Method::PATCH,
            "/api/v1/account/me",
            Some(json!({"display_name": "Ada Renamed", "hue_override": 999})),
        )
        .await;
    assert_eq!(
        (half_bad.status, half_bad.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    let after = ada.get("/api/v1/account/me").await;
    assert_eq!(
        after.body["display_name"], "Ada L",
        "a change refused is not made in part"
    );
}
