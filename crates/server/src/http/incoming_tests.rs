//! Incoming webhooks over HTTP: who makes them, the integration each acts as with the least
//! access its target needs, what they take at `/hooks/in/{token}` for each target, with and
//! without signatures, and every refusal: an unknown URL, a bad signature, a body too large
//! or not JSON, a payload that does not read, their rates, a suspended integration, a hook
//! turned off. The path passes the origin check and takes no cookie.

use axum::body::Body;
use axum::http::header::{CONTENT_TYPE, COOKIE, ORIGIN, RETRY_AFTER};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt as _;
use neon_app::settings::Change;
use neon_app::webhooks::signature;
use neon_blocks::BlockKind;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt as _;
use uuid::Uuid;

use super::auth_tests::Answer;
use super::databases_tests::{database, database_id, property};
use super::pages_tests::{create, general};
use super::tests::state_for_webhooks;
use super::workspaces_tests::{Browser, id, team};
use super::*;
use crate::webhooks::HttpTransport;

async fn keyed(pool: PgPool) -> AppState {
    state_for_webhooks(pool, Arc::new(HttpTransport::default())).await
}

/// Posts `body` to a webhook's URL path with `headers`, as a sender does.
async fn post(app: &Router, path: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Answer {
    let mut request = Request::builder().method(Method::POST).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).expect("a request"))
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
    Answer {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

const JSON: (&str, &str) = ("content-type", "application/json");

async fn send(app: &Router, path: &str, payload: &Value) -> Answer {
    post(app, path, &[JSON], payload.to_string().into_bytes()).await
}

/// Signs and posts a payload with `secret`, its timestamp `skew` seconds off.
async fn signed(app: &Router, path: &str, payload: &Value, secret: &str, skew: i64) -> Answer {
    let body = payload.to_string().into_bytes();
    let at = OffsetDateTime::now_utc().unix_timestamp() + skew;
    let signature = signature::header(&[secret.as_bytes()], at, &body);
    let at = at.to_string();
    post(
        app,
        path,
        &[
            JSON,
            ("neon-webhook-timestamp", &at),
            ("neon-webhook-signature", &signature),
        ],
        body,
    )
    .await
}

/// The path of a webhook's URL.
fn path_of(created: &Answer) -> String {
    let url = created.body["url"].as_str().expect("a URL");
    assert!(url.starts_with("/hooks/in/nwh_"), "{url}");
    url.to_owned()
}

/// Makes an incoming webhook as `by`, on `target`.
async fn hook(by: &Browser<'_>, ws: &str, name: &str, target: &str, signed: bool) -> Answer {
    by.post(
        &format!("/api/v1/workspaces/{ws}/incoming-webhooks"),
        json!({"name": name, "target_id": target, "signed": signed}),
    )
    .await
}

fn create_row(values: Value) -> Value {
    json!({"version": 1, "action": "create_row", "values": values})
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_make_incoming_webhooks_each_with_an_integration_of_least_access(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let tasks = database(&t.ada, ws).await;
    let db = database_id(&tasks);

    let refused = hook(&t.cy, ws, "CI", &db, false).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let made = hook(&t.bob, ws, "CI", &db, false).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    path_of(&made);
    assert_eq!(made.body["signing_secret"], Value::Null);
    let created = &made.body["hook"];
    assert_eq!(created["name"], "CI");
    assert_eq!(created["target"]["kind"], "database");
    assert_eq!(created["target"]["title"], "Tasks");
    assert_eq!(created["signed"], false);
    assert_eq!(created["enabled"], true);
    assert_eq!(created["integration"]["status"], "active");
    assert_eq!(created["integration"]["owner"]["display_name"], "bob");
    let integration = created["integration"]["id"].as_str().expect("an id");

    // It is an integration of the workspace, owned by its maker, in no teamspace, with its
    // target shared with it to edit.
    let agents = t.ada.get(&format!("/api/v1/workspaces/{ws}/agents")).await;
    let listed = &agents.body["agents"][0];
    assert_eq!(listed["id"], integration);
    assert_eq!(listed["kind"], "integration");
    let general = general(&t.ada, ws).await;
    let members = t
        .ada
        .get(&format!(
            "/api/v1/workspaces/{ws}/teamspaces/{general}/members"
        ))
        .await;
    assert!(
        !members.body.to_string().contains(integration),
        "{}",
        members.body
    );
    let shares = t.ada.get(&format!("/api/v1/pages/{db}/shares")).await;
    let share = shares.body["shares"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|share| share["user"]["id"] == integration)
        .cloned()
        .expect("shared with it");
    assert_eq!(share["access"], "can_edit");
    assert_eq!(share["user"]["kind"], "integration");

    // It counts against the agent limit.
    state
        .settings
        .update(
            [("workspaces.limits.agents".to_owned(), Change::Set(json!(1)))],
            None,
        )
        .await
        .expect("saved");
    let over = hook(&t.bob, ws, "Second", &db, false).await;
    assert_eq!(
        (over.status, over.code()),
        (StatusCode::CONFLICT, "agent_limit"),
        "{}",
        over.body
    );

    // Its target is a database or a page of the workspace that its maker may share.
    let row = t
        .ada
        .post(
            &format!("/api/v1/databases/{db}/rows"),
            json!({"values": {"Name": "A row"}}),
        )
        .await;
    state
        .settings
        .update(
            [("workspaces.limits.agents".to_owned(), Change::Reset)],
            None,
        )
        .await
        .expect("saved");
    let on_row = hook(&t.bob, ws, "Row", &id(&row), false).await;
    assert_eq!(on_row.status, StatusCode::BAD_REQUEST, "{}", on_row.body);
    let nowhere = hook(&t.bob, ws, "Nowhere", &Uuid::now_v7().to_string(), false).await;
    assert_eq!(nowhere.status, StatusCode::NOT_FOUND, "{}", nowhere.body);
    let listed = t
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/incoming-webhooks"))
        .await;
    assert_eq!(listed.body["hooks"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        t.cy.get(&format!("/api/v1/workspaces/{ws}/incoming-webhooks"))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_database_hook_makes_rows_and_changes_them_by_unique_id(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let tasks = database(&t.ada, ws).await;
    let db = database_id(&tasks);
    let prefixed = t
        .ada
        .post(
            &format!("/api/v1/databases/{db}/properties"),
            json!({"name": "ID", "type": "unique_id", "prefix": "nw"}),
        )
        .await;
    assert_eq!(prefixed.status, StatusCode::CREATED, "{}", prefixed.body);
    let made = hook(&t.ada, ws, "Deploys", &db, false).await;
    let path = path_of(&made);
    let integration = made.body["hook"]["integration"]["id"].clone();

    let created = send(
        &app,
        &path,
        &create_row(json!({"Name": "Ship it", "Status": "Not started"})),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    assert_eq!(created.body["ok"], true);
    assert_eq!(created.body["action"], "create_row");
    assert_eq!(created.body["row"]["unique_id"], "NW-1");
    let row = created.body["row"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let read = t.ada.get(&format!("/api/v1/rows/{row}")).await;
    assert_eq!(read.body["title"], "Ship it");
    assert_eq!(read.body["created_by"], integration);

    let changed = send(
        &app,
        &path,
        &json!({"version": 1, "action": "update_row", "unique_id": "nw-1", "values": {"Status": "Done"}}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["action"], "update_row");
    let read = t.ada.get(&format!("/api/v1/rows/{row}")).await;
    let status = property(&tasks, "Status");
    assert!(read.body["values"][&status].is_string());
    assert_eq!(read.body["last_edited_by"], integration);

    // A value its property does not take, and a row it does not have, are the database's
    // refusals; it reaches no other database.
    let bad = send(&app, &path, &create_row(json!({"Nope": 1}))).await;
    assert_eq!(
        (bad.status, bad.code()),
        (StatusCode::BAD_REQUEST, "invalid_input")
    );
    let missing = send(
        &app,
        &path,
        &json!({"version": 1, "action": "update_row", "unique_id": "NW-99", "values": {"Name": "x"}}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);

    let log = t
        .ada
        .get(&format!(
            "/api/v1/workspaces/{ws}/incoming-webhooks/{}/requests",
            made.body["hook"]["id"].as_str().expect("an id")
        ))
        .await;
    let requests = log.body["requests"].as_array().expect("a log");
    let outcomes: Vec<(&str, &str)> = requests
        .iter()
        .map(|request| {
            (
                request["outcome"].as_str().expect("an outcome"),
                request["action"].as_str().unwrap_or("-"),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("not_found", "update_row"),
            ("invalid_input", "create_row"),
            ("ok", "update_row"),
            ("ok", "create_row")
        ]
    );
    assert_eq!(requests[3]["status_code"], 200);
    assert_eq!(requests[3]["client_ip"], "unknown");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_hook_appends_markdown_through_the_live_document(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Changelog").await;
    let made = hook(&t.ada, ws, "Releases", &page, false).await;
    assert_eq!(made.body["hook"]["target"]["kind"], "page");
    let path = path_of(&made);

    let appended = send(
        &app,
        &path,
        &json!({"version": 1, "action": "append", "markdown": "## v1.2\n\n- Faster boards"}),
    )
    .await;
    assert_eq!(appended.status, StatusCode::OK, "{}", appended.body);
    assert_eq!(appended.body, json!({"ok": true, "action": "append"}));
    send(
        &app,
        &path,
        &json!({"version": 1, "action": "append", "markdown": "Second release"}),
    )
    .await;

    let page_id = Uuid::try_parse(&page).expect("an id");
    let content = state.sync.content(page_id).await.expect("its content");
    let kinds: Vec<&BlockKind> = content.blocks.iter().map(|block| &block.kind).collect();
    assert_eq!(kinds.len(), 3, "{content:?}");
    assert!(matches!(kinds[0], BlockKind::Heading { .. }));
    assert!(content.plain_text().contains("Faster boards"));
    assert!(content.plain_text().ends_with("Second release"));

    // Create_row is a database's, and a page takes nothing else.
    let wrong = send(&app, &path, &create_row(json!({"Name": "x"}))).await;
    assert_eq!(
        (wrong.status, wrong.code()),
        (StatusCode::BAD_REQUEST, "invalid_payload")
    );

    // Signed only, it adds what is signed with its secret alone.
    let hook_id = made.body["hook"]["id"].as_str().expect("an id");
    let signing = t
        .ada
        .post(
            &format!("/api/v1/workspaces/{ws}/incoming-webhooks/{hook_id}/secret"),
            json!({}),
        )
        .await;
    assert_eq!(signing.status, StatusCode::OK, "{}", signing.body);
    let secret = signing.body["signing_secret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    let note = json!({"version": 1, "action": "append", "markdown": "Signed note"});
    assert_eq!(
        send(&app, &path, &note).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        signed(&app, &path, &note, &secret, 0).await.status,
        StatusCode::OK
    );
    let content = state.sync.content(page_id).await.expect("its content");
    assert!(content.plain_text().ends_with("Signed note"), "{content:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn markdown_nested_too_deep_is_refused_and_the_server_keeps_answering(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Changelog").await;
    let path = path_of(&hook(&t.ada, ws, "Releases", &page, false).await);

    // Five thousand quotes deep, read on the test's thread, whose stack is a runtime
    // worker's: two MiB.
    let deep = json!({"version": 1, "action": "append", "markdown": "> ".repeat(5000)});
    let answer = send(&app, &path, &deep).await;
    assert_eq!(
        (answer.status, answer.code()),
        (StatusCode::BAD_REQUEST, "invalid_payload"),
        "{}",
        answer.body
    );
    assert!(
        answer.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("32 levels deep")),
        "{}",
        answer.body
    );

    let page_id = Uuid::try_parse(&page).expect("an id");
    let content = state.sync.content(page_id).await.expect("its content");
    assert!(content.blocks.is_empty(), "{content:?}");
    let still = send(
        &app,
        &path,
        &json!({"version": 1, "action": "append", "markdown": "Still here"}),
    )
    .await;
    assert_eq!(still.status, StatusCode::OK, "{}", still.body);
    let content = state.sync.content(page_id).await.expect("its content");
    assert_eq!(content.plain_text(), "Still here");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_signed_hook_takes_only_requests_signed_with_its_secret(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let db = database_id(&database(&t.ada, ws).await);
    let made = hook(&t.ada, ws, "Signed", &db, true).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["hook"]["signed"], true);
    let secret = made.body["signing_secret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    assert!(secret.starts_with("whsec_"));
    let path = path_of(&made);
    let hook_id = made.body["hook"]["id"].as_str().expect("an id").to_owned();
    let one = format!("/api/v1/workspaces/{ws}/incoming-webhooks/{hook_id}");
    let payload = create_row(json!({"Name": "Signed"}));

    let unsigned = send(&app, &path, &payload).await;
    assert_eq!(
        (unsigned.status, unsigned.code()),
        (StatusCode::UNAUTHORIZED, "invalid_signature")
    );
    let forged = signed(&app, &path, &payload, "whsec_another", 0).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    let replayed = signed(&app, &path, &payload, &secret, -600).await;
    assert_eq!(replayed.status, StatusCode::UNAUTHORIZED);
    assert!(
        replayed.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("5 minutes"))
    );
    let good = signed(&app, &path, &payload, &secret, 0).await;
    assert_eq!(good.status, StatusCode::OK, "{}", good.body);

    // Rotated, both secrets verify for a while.
    let rotated = t.ada.post(&format!("{one}/secret"), json!({})).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    let new = rotated.body["signing_secret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    assert_ne!(new, secret);
    assert!(rotated.body["hook"]["previous_secret_expires_at"].is_string());
    assert_eq!(
        signed(&app, &path, &payload, &new, 0).await.status,
        StatusCode::OK
    );
    assert_eq!(
        signed(&app, &path, &payload, &secret, 0).await.status,
        StatusCode::OK
    );

    // No longer checked, unsigned requests work again.
    let removed = t
        .ada
        .send(Method::DELETE, &format!("{one}/secret"), None)
        .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    assert_eq!(removed.body["signed"], false);
    assert_eq!(send(&app, &path, &payload).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn requests_are_refused_saying_why(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let db = database_id(&database(&t.ada, ws).await);
    let made = hook(&t.bob, ws, "CI", &db, false).await;
    let path = path_of(&made);
    let hook_id = made.body["hook"]["id"].as_str().expect("an id").to_owned();
    let one = format!("/api/v1/workspaces/{ws}/incoming-webhooks/{hook_id}");
    let payload = create_row(json!({"Name": "x"}));
    let refused = |answer: &Answer| (answer.status, answer.code().to_owned());

    for unknown in ["/hooks/in/nwh_unknown", "/hooks/in/not-even-a-token"] {
        assert_eq!(
            refused(&send(&app, unknown, &payload).await),
            (StatusCode::NOT_FOUND, "not_found".to_owned())
        );
    }
    let plain = post(
        &app,
        &path,
        &[("content-type", "text/plain")],
        payload.to_string().into_bytes(),
    )
    .await;
    assert_eq!(
        refused(&plain),
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type".to_owned()
        )
    );
    let large = post(
        &app,
        &path,
        &[JSON],
        vec![b' '; neon_app::webhooks::MAX_INCOMING_BODY + 1],
    )
    .await;
    assert_eq!(
        refused(&large),
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large".to_owned()
        )
    );
    for (body, mentions) in [
        (
            json!({"version": 2, "action": "create_row", "values": {}}),
            "unknown version",
        ),
        (
            json!({"version": 1, "action": "drop_table"}),
            "unknown variant",
        ),
        (
            json!({"version": 1, "action": "create_row", "values": {}, "x": 1}),
            "unknown field",
        ),
    ] {
        let answer = send(&app, &path, &body).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer.code(), "invalid_payload");
        assert!(
            answer.body["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(mentions)),
            "{}",
            answer.body
        );
    }
    let garbled = post(&app, &path, &[JSON], b"{not json".to_vec()).await;
    assert_eq!(garbled.code(), "invalid_payload");

    // Turned off, it refuses; on again, it takes.
    let off = t
        .bob
        .send(Method::PATCH, &one, Some(json!({"enabled": false})))
        .await;
    assert_eq!(off.body["enabled"], false);
    assert_eq!(
        refused(&send(&app, &path, &payload).await),
        (StatusCode::FORBIDDEN, "hook_disabled".to_owned())
    );
    t.bob
        .send(Method::PATCH, &one, Some(json!({"enabled": true})))
        .await;
    assert_eq!(send(&app, &path, &payload).await.status, StatusCode::OK);

    // A new URL: the old one is gone at once.
    let rotated = t.bob.post(&format!("{one}/token"), json!({})).await;
    let new_path = path_of(&rotated);
    assert_ne!(new_path, path);
    assert_eq!(
        send(&app, &path, &payload).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(send(&app, &new_path, &payload).await.status, StatusCode::OK);

    // Its owner leaves: its integration is suspended, and refuses.
    let left = t
        .bob
        .post(&format!("/api/v1/workspaces/{ws}/leave"), json!({}))
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    assert_eq!(
        refused(&send(&app, &new_path, &payload).await),
        (StatusCode::FORBIDDEN, "integration_suspended".to_owned())
    );
    let read = t.ada.get(&one).await;
    assert_eq!(read.body["integration"]["status"], "suspended");

    // Deleted, with its integration.
    let deleted = t.ada.send(Method::DELETE, &one, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(
        send(&app, &new_path, &payload).await.status,
        StatusCode::NOT_FOUND
    );
    let agents = t.ada.get(&format!("/api/v1/workspaces/{ws}/agents")).await;
    assert_eq!(agents.body["agents"][0]["status"], "deleted");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_hook_and_a_client_are_each_held_to_their_rate(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let db = database_id(&database(&t.ada, ws).await);
    let made = hook(&t.ada, ws, "Busy", &db, false).await;
    let path = path_of(&made);
    let hook_id = made.body["hook"]["id"].as_str().expect("an id").to_owned();

    // Bodies refused as they are not JSON, cheap to send, until the hook's rate is spent.
    // Each second gives one more, so the loop goes well past the burst under load.
    let burst = 60;
    let mut limited = None;
    for sent in 1..=burst * 4 {
        let answer = post(
            &app,
            &path,
            &[("content-type", "text/plain")],
            b"x".to_vec(),
        )
        .await;
        if answer.status == StatusCode::TOO_MANY_REQUESTS {
            limited = Some((sent, answer));
            break;
        }
        assert_eq!(answer.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let (sent, answer) = limited.expect("the hook's rate ran out");
    assert!(sent > burst, "limited after {sent}");
    assert_eq!(answer.code(), "rate_limited");
    assert!(answer.headers.contains_key(RETRY_AFTER));
    // Requests refused for their rate are not logged.
    let log = t
        .ada
        .get(&format!(
            "/api/v1/workspaces/{ws}/incoming-webhooks/{hook_id}/requests"
        ))
        .await;
    let logged = log.body["requests"].as_array().expect("a log");
    assert!(
        logged
            .iter()
            .all(|request| request["outcome"] != "rate_limited")
    );

    // A client sending to URLs that do not work runs out too.
    let mut limited = false;
    for _ in 0..600 {
        let answer = send(&app, "/hooks/in/nwh_guessing", &create_row(json!({}))).await;
        if answer.status == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(answer.status, StatusCode::NOT_FOUND);
    }
    assert!(limited, "a client guessing URLs is limited");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_hook_path_passes_the_origin_check_and_takes_no_cookie(pool: PgPool) {
    let state = keyed(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let db = database_id(&database(&t.ada, ws).await);
    let made = hook(&t.ada, ws, "Forms", &db, false).await;
    let path = path_of(&made);
    let integration = made.body["hook"]["integration"]["id"].clone();
    let body = create_row(json!({"Name": "From a form"})).to_string();

    // From another origin, with Ada's cookie: taken, and as the integration, not as her.
    let taken = post(
        &app,
        &path,
        &[
            JSON,
            (ORIGIN.as_str(), "https://elsewhere.example"),
            (COOKIE.as_str(), t.ada.cookie()),
        ],
        body.clone().into_bytes(),
    )
    .await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.body);
    let row = taken.body["row"]["id"].as_str().expect("an id");
    let read = t.ada.get(&format!("/api/v1/rows/{row}")).await;
    assert_eq!(read.body["created_by"], integration);

    // A cookie opens nothing a token does not.
    let cookie_alone = post(
        &app,
        "/hooks/in/nwh_none",
        &[JSON, (COOKIE.as_str(), t.ada.cookie())],
        body.clone().into_bytes(),
    )
    .await;
    assert_eq!(cookie_alone.status, StatusCode::NOT_FOUND);

    // The exemption is for incoming webhooks' path alone.
    for elsewhere in [
        "/hooks/out",
        "/hooks/inbox",
        &format!("/api/v1/databases/{db}/rows"),
    ] {
        let refused = post(
            &app,
            elsewhere,
            &[
                JSON,
                (ORIGIN.as_str(), "https://elsewhere.example"),
                (COOKIE.as_str(), t.ada.cookie()),
            ],
            json!({"values": {"Name": "x"}}).to_string().into_bytes(),
        )
        .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{elsewhere}");
    }
    let wrong_method = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(&path)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("a response");
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
}
