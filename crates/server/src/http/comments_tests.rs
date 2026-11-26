//! Comments and the inbox over HTTP: the routes, their answers and error codes, a share to
//! comment, a guest who sees no addresses, the inbox read and marked, mail preferences and
//! the link that stops mail without signing in, and both live: a page's connection that
//! hears its threads change, and the inbox's feed.

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::auth_tests::{call, keyed};
use super::pages_tests::{create, general, guest};
use super::sync_tests::Server;
use super::tests::{state, state_with};
use super::workspaces_tests::{Browser, team};
use super::*;

const PATIENCE: Duration = Duration::from_secs(10);

fn mention(id: &str, name: &str) -> String {
    format!("@[{name}](user:{id})")
}

async fn me(who: &Browser<'_>) -> String {
    who.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn comments_are_started_replied_to_changed_resolved_and_deleted(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Notes").await;
    let cy = me(&t.cy).await;

    let started = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/comments"),
            json!({"markdown": format!("**Look**, {}", mention(&cy, "cy"))}),
        )
        .await;
    assert_eq!(started.status, StatusCode::CREATED, "{}", started.body);
    let thread = &started.body["thread"];
    assert_eq!(thread["anchor"], Value::Null);
    assert_eq!(
        thread["comments"][0]["markdown"],
        format!("**Look**, {}", mention(&cy, "cy"))
    );
    assert_eq!(thread["comments"][0]["content"][0]["styles"]["bold"], true);
    assert_eq!(thread["comments"][0]["content"][2]["type"], "mention");
    assert_eq!(started.body["unnotified"], json!([]));
    let thread_id = thread["id"].as_str().expect("an id").to_owned();

    let replied =
        t.cy.post(
            &format!("/api/v1/threads/{thread_id}/comments"),
            json!({"markdown": "On it"}),
        )
        .await;
    assert_eq!(replied.status, StatusCode::CREATED, "{}", replied.body);
    let reply = replied.body["comment_id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let listed = t.dee.get(&format!("/api/v1/pages/{page}/comments")).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    assert_eq!(listed.body["may_comment"], true);
    assert_eq!(listed.body["following"], false);
    assert_eq!(
        listed.body["threads"][0]["comments"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    let refused = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/comments/{reply}"),
            Some(json!({"markdown": "Mine"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let edited =
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/comments/{reply}"),
            Some(json!({"markdown": "On it, today"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert!(edited.body["thread"]["comments"][1]["edited_at"].is_string());

    let resolved = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/threads/{thread_id}"),
            Some(json!({"resolved": true})),
        )
        .await;
    assert_eq!(resolved.status, StatusCode::OK, "{}", resolved.body);
    assert!(resolved.body["resolved_at"].is_string());

    let followed = t
        .dee
        .send(
            Method::PUT,
            &format!("/api/v1/pages/{page}/follow"),
            Some(json!({"following": true})),
        )
        .await;
    assert_eq!(followed.body, json!({"following": true}));

    let deleted =
        t.cy.send(Method::DELETE, &format!("/api/v1/comments/{reply}"), None)
            .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let both = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/comments"),
            json!({"markdown": "x", "block_id": "b", "anchor": {"start": "AA==", "end": "AA==", "quote": "x"}}),
        )
        .await;
    assert_eq!(both.status, StatusCode::BAD_REQUEST);
    assert_eq!(both.code(), "invalid_input");
    let empty = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/comments"),
            json!({"markdown": " "}),
        )
        .await;
    assert_eq!(empty.code(), "invalid_input");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_share_to_comment_lets_a_guest_comment_who_sees_no_addresses(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Notes").await;
    super::auth_tests::user(&state, "gus@example.com").await;
    let gus = guest(&app, &t.ada, ws, "gus@example.com").await;
    let gus_id = me(&gus).await;
    let comments = format!("/api/v1/pages/{page}/comments");

    assert_eq!(gus.get(&comments).await.status, StatusCode::NOT_FOUND);
    let shared = t
        .ada
        .post(
            &format!("/api/v1/pages/{page}/shares"),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    t.ada.post(&comments, json!({"markdown": "From Ada"})).await;
    let read = gus.get(&comments).await;
    assert_eq!(read.body["may_comment"], false);
    let author = &read.body["threads"][0]["comments"][0]["author"];
    assert_eq!(author["display_name"], "ada");
    assert_eq!(author["email"], Value::Null, "{author}");
    let refused = gus.post(&comments, json!({"markdown": "Hi"})).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    let changed = t
        .ada
        .send(
            Method::PATCH,
            &format!("/api/v1/pages/{page}/shares/{gus_id}"),
            Some(json!({"access": "can_comment"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["access"], "can_comment");
    assert_eq!(
        gus.get(&format!("/api/v1/pages/{page}")).await.body["access"],
        "comment"
    );
    let posted = gus.post(&comments, json!({"markdown": "Hi"})).await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    let candidates = gus.get(&format!("/api/v1/pages/{page}/mentions?q=")).await;
    assert_eq!(
        candidates.body["people"],
        json!([]),
        "a guest finds no people"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_inbox_is_read_counted_and_marked(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Notes").await;
    let cy = me(&t.cy).await;
    let posted = t
        .dee
        .post(
            &format!("/api/v1/pages/{page}/comments"),
            json!({"markdown": format!("{} please", mention(&cy, "cy"))}),
        )
        .await;
    let thread = posted.body["thread"]["id"].clone();

    let inbox = t.cy.get("/api/v1/inbox").await;
    assert_eq!(inbox.status, StatusCode::OK, "{}", inbox.body);
    let first = &inbox.body["notifications"][0];
    assert_eq!(first["kind"], "mention");
    assert_eq!(first["thread_id"], thread);
    assert_eq!(first["page"]["title"], "Notes");
    assert_eq!(first["workspace"]["name"], "Design");
    assert_eq!(first["actor"]["display_name"], "dee");
    assert_eq!(first["excerpt"], "@cy please");
    let notification = first["id"].as_str().expect("an id").to_owned();
    // Ada made the page: she follows it.
    let ada = t.ada.get("/api/v1/inbox?filter=unread").await;
    assert_eq!(ada.body["notifications"][0]["kind"], "comment");
    assert_eq!(
        t.cy.get(&format!("/api/v1/inbox/count?workspace_id={ws}"))
            .await
            .body,
        json!({"unread": 1})
    );

    let marked =
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/notifications/{notification}"),
            Some(json!({"read": true})),
        )
        .await;
    assert_eq!(marked.status, StatusCode::NO_CONTENT);
    assert_eq!(
        t.cy.get("/api/v1/inbox/count").await.body,
        json!({"unread": 0})
    );
    let others = t
        .dee
        .send(
            Method::PATCH,
            &format!("/api/v1/notifications/{notification}"),
            Some(json!({"archived": true})),
        )
        .await;
    assert_eq!(others.status, StatusCode::NOT_FOUND);
    let all = t.ada.post("/api/v1/inbox/read", json!({})).await;
    assert_eq!(all.body, json!({"marked": 1}));

    let settings = t.cy.get("/api/v1/me/notifications").await;
    assert_eq!(settings.body["mail"], "each");
    assert_eq!(settings.body["mail_available"], false);
    let changed =
        t.cy.send(
            Method::PUT,
            "/api/v1/me/notifications",
            Some(json!({"mail": "digest"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["mail"], "digest");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_link_in_a_mail_stops_it_without_signing_in(pool: PgPool) {
    let state = state_with(pool, keyed(), WebConfig::new(None, Vec::new())).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let cy = me(&t.cy).await;
    let token = state
        .inbox
        .unsubscribe_token(cy.parse().expect("a uuid"))
        .expect("signed");
    // As a mail client's one click posts it: no session, no origin, a form body.
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(format!("/api/v1/unsubscribe?token={token}"))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(axum::body::Body::from("List-Unsubscribe=One-Click"))
        .expect("a request");
    let response = tower::ServiceExt::oneshot(app.clone(), request)
        .await
        .expect("answered");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        t.cy.get("/api/v1/me/notifications").await.body["mail"],
        "none"
    );

    let forged = call(
        &app,
        Method::POST,
        &format!("/api/v1/unsubscribe?token={}x", token),
        &[],
        None,
    )
    .await;
    assert_eq!(forged.status, StatusCode::BAD_REQUEST);
    assert_eq!(forged.code(), "invalid_token");
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn socket(addr: SocketAddr, path: &str, cookie: &str) -> Socket {
    let mut request = format!("ws://{addr}{path}")
        .into_client_request()
        .expect("a request");
    request.headers_mut().insert(
        HeaderName::from_static("cookie"),
        HeaderValue::from_str(cookie).expect("a cookie"),
    );
    let (socket, _) = timeout(PATIENCE, tokio_tungstenite::connect_async(request))
        .await
        .expect("the handshake ends")
        .expect("upgraded");
    socket
}

/// Reads until a frame `found` takes, and returns what it took from it.
async fn until<T>(socket: &mut Socket, mut found: impl FnMut(&Frame) -> Option<T>) -> T {
    timeout(PATIENCE, async {
        loop {
            let frame = socket.next().await.expect("open").expect("a frame");
            if let Some(found) = found(&frame) {
                return found;
            }
        }
    })
    .await
    .expect("it came")
}

/// The JSON of a page connection's message about its comments, of type 100.
fn comments_message(frame: &Frame) -> Option<Value> {
    use yrs::updates::decoder::Decode as _;
    let Frame::Binary(data) = frame else {
        return None;
    };
    match yrs::sync::Message::decode_v1(data).ok()? {
        yrs::sync::Message::Custom(100, json) => serde_json::from_slice(&json).ok(),
        _ => None,
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn open_pages_and_inboxes_hear_their_changes_live(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let general = general(&t.ada, ws).await;
    let page = create(&t.ada, ws, json!({"teamspace_id": general}), "Notes").await;
    let server = Server::start(app.clone()).await;
    let cy = me(&t.cy).await;

    let mut editor = socket(
        server.addr,
        &format!("/ws/pages/{page}?comments=true"),
        t.ada.cookie(),
    )
    .await;
    // The connection is set up once the document's sync step 1 comes.
    until(&mut editor, |frame| {
        matches!(frame, Frame::Binary(_)).then_some(())
    })
    .await;
    let mut inbox = socket(server.addr, "/ws/inbox", t.cy.cookie()).await;
    let ready = until(&mut inbox, |frame| match frame {
        Frame::Text(text) => Some(text.to_string()),
        _ => None,
    })
    .await;
    assert_eq!(ready, r#"{"type":"ready"}"#);

    let posted = t
        .dee
        .post(
            &format!("/api/v1/pages/{page}/comments"),
            json!({"markdown": format!("{} look", mention(&cy, "cy"))}),
        )
        .await;
    let thread = posted.body["thread"]["id"].clone();
    let told = until(&mut editor, comments_message).await;
    assert_eq!(told, json!({"type": "comments", "thread_id": thread}));
    let heard = until(&mut inbox, |frame| match frame {
        Frame::Text(text) => Some(text.to_string()),
        _ => None,
    })
    .await;
    assert_eq!(heard, r#"{"type":"inbox"}"#);

    // A connection that did not ask hears nothing of comments.
    let mut plain = socket(server.addr, &format!("/ws/pages/{page}"), t.dee.cookie()).await;
    t.dee
        .send(
            Method::PATCH,
            &format!("/api/v1/threads/{}", thread.as_str().expect("an id")),
            Some(json!({"resolved": true})),
        )
        .await;
    until(&mut editor, comments_message).await;
    let quiet = timeout(Duration::from_millis(300), async {
        loop {
            let frame = plain.next().await.expect("open").expect("a frame");
            if comments_message(&frame).is_some() {
                return;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "a stock client is told nothing of comments");
    drop((editor, inbox, plain));
    state.sync.shutdown().await;
    state.inbox_feeds.shutdown().await;
    server.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn hidden_pages_have_no_comments_to_find(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let private = create(&t.ada, ws, json!({"private": true}), "Mine").await;
    let made = t
        .ada
        .post(
            &format!("/api/v1/pages/{private}/comments"),
            json!({"markdown": "Note"}),
        )
        .await;
    let thread = made.body["thread"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let comment = made.body["comment_id"].as_str().expect("an id").to_owned();
    for (method, path, body) in [
        (
            Method::GET,
            format!("/api/v1/pages/{private}/comments"),
            None,
        ),
        (
            Method::POST,
            format!("/api/v1/threads/{thread}/comments"),
            Some(json!({"markdown": "x"})),
        ),
        (
            Method::PATCH,
            format!("/api/v1/threads/{thread}"),
            Some(json!({"resolved": true})),
        ),
        (Method::DELETE, format!("/api/v1/comments/{comment}"), None),
        (
            Method::GET,
            format!("/api/v1/pages/{private}/mentions?q=a"),
            None,
        ),
    ] {
        let answer = t.cy.send(method.clone(), &path, body).await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            answer.body
        );
    }
    let candidates = t
        .ada
        .get(&format!("/api/v1/pages/{private}/mentions?q="))
        .await;
    let sees: Vec<(Value, Value)> = candidates.body["people"]
        .as_array()
        .expect("people")
        .iter()
        .map(|found| {
            (
                found["person"]["display_name"].clone(),
                found["sees_page"].clone(),
            )
        })
        .collect();
    assert!(sees.contains(&(json!("ada"), json!(true))), "{sees:?}");
    assert!(sees.contains(&(json!("cy"), json!(false))), "{sees:?}");
}
