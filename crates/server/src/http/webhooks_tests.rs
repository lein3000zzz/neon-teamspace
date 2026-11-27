//! Outgoing webhooks over HTTP: who registers and manages them, their URLs, secrets and
//! logs, and their deliveries through the job queue to a receiver on loopback, signed, for
//! each event, retried, given up on, turning themselves off, skipped once their identity, or
//! whoever set their URL, no longer sees the row, and turned off once whoever set it may no
//! longer register it.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use neon_app::jobs::{Worker, WorkerConfig, WorkerHandle};
use neon_app::settings::Change;
use neon_app::webhooks::{DeliveryJob, signature};
use neon_domain::jobs::Backoff;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::agents_tests::{Bearer, agent, agent_token};
use super::auth_tests::{Answer, user};
use super::databases_tests::{database, database_id, option, property};
use super::pages_tests::guest;
use super::tests::{state, state_for_webhooks};
use super::workspaces_tests::{Browser, id, team};
use super::*;
use crate::webhooks::HttpTransport;
use crate::webhooks::receiver::{PATIENCE, Received, Receiver, Reply, ScriptedResolver};

/// The state with a secret key, sending webhooks as the server does, to private addresses
/// too, as the receiver is on loopback.
pub(super) async fn delivering(pool: PgPool) -> AppState {
    let state = state_for_webhooks(pool, Arc::new(HttpTransport::default())).await;
    allow_private(&state, true).await;
    state
}

async fn allow_private(state: &AppState, allowed: bool) {
    state
        .settings
        .update(
            [(
                "webhooks.allow_private".to_owned(),
                Change::Set(json!(allowed)),
            )],
            None,
        )
        .await
        .expect("saved");
}

/// A job worker running deliveries, retrying them within milliseconds.
pub(super) async fn worker(pool: &PgPool, state: &AppState) -> WorkerHandle {
    Worker::new(
        pool.clone(),
        WorkerConfig {
            poll_interval: Duration::from_millis(20),
            backoff: Backoff {
                base: Duration::from_millis(5),
                max: Duration::from_millis(20),
            },
            ..WorkerConfig::default()
        },
    )
    .handler(DeliveryJob::new(state.webhooks.clone()))
    .start()
    .await
    .expect("the worker starts")
}

async fn me(by: &Browser<'_>) -> String {
    id(&by.get("/api/v1/account/me").await)
}

/// Registers a webhook as `by`, answering what the server said.
async fn register(by: &Browser<'_>, ws: &str, user: &str, url: &str, events: Value) -> Answer {
    by.post(
        &format!("/api/v1/workspaces/{ws}/webhooks"),
        json!({"user_id": user, "url": url, "events": events}),
    )
    .await
}

/// A webhook `by` registers: its id and secret.
async fn webhook(by: &Browser<'_>, ws: &str, user: &str, url: &str) -> (String, String) {
    let made = register(
        by,
        ws,
        user,
        url,
        json!(["row.assigned", "row.status_changed"]),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    (
        made.body["webhook"]["id"]
            .as_str()
            .expect("an id")
            .to_owned(),
        made.body["secret"].as_str().expect("a secret").to_owned(),
    )
}

/// A database in General with an Assignee person property: its id and body.
pub(super) async fn tasks(by: &Browser<'_>, ws: &str) -> (String, Value) {
    let made = database(by, ws).await;
    let db = database_id(&made);
    let added = by
        .post(
            &format!("/api/v1/databases/{db}/properties"),
            json!({"name": "Assignee", "type": "person"}),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    let read = by.get(&format!("/api/v1/databases/{db}")).await.body;
    (db, read)
}

/// A private teamspace with `members` in it, besides its maker `by`, and a database there
/// with an Assignee person property: the teamspace's members path and the database's id.
async fn private_tasks(
    by: &Browser<'_>,
    ws: &str,
    members: &[&str],
    title: &str,
) -> (String, String) {
    let made = by
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Vault", "access": "private"}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let teamspace = id(&made);
    let path = format!("/api/v1/workspaces/{ws}/teamspaces/{teamspace}/members");
    for member in members {
        let added = by.post(&path, json!({"user_id": member})).await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    }
    let db = by
        .post(
            &format!("/api/v1/workspaces/{ws}/databases"),
            json!({"teamspace_id": teamspace, "title": title}),
        )
        .await;
    assert_eq!(db.status, StatusCode::CREATED, "{}", db.body);
    let db = database_id(&db.body);
    let person = by
        .post(
            &format!("/api/v1/databases/{db}/properties"),
            json!({"name": "Assignee", "type": "person"}),
        )
        .await;
    assert_eq!(person.status, StatusCode::CREATED, "{}", person.body);
    (path, db)
}

async fn row(by: &Browser<'_>, db: &str, values: Value) -> String {
    let made = by
        .post(
            &format!("/api/v1/databases/{db}/rows"),
            json!({"values": values}),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    id(&made)
}

async fn change(by: &Browser<'_>, row: &str, values: Value) {
    let changed = by
        .send(
            Method::PATCH,
            &format!("/api/v1/rows/{row}"),
            Some(json!({"values": values})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
}

/// A webhook's log once `done` holds for it, or the test fails.
async fn log_until(
    by: &Browser<'_>,
    ws: &str,
    webhook: &str,
    done: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let log = by
            .get(&format!(
                "/api/v1/workspaces/{ws}/webhooks/{webhook}/deliveries"
            ))
            .await;
        assert_eq!(log.status, StatusCode::OK, "{}", log.body);
        let attempts = log.body["attempts"].as_array().cloned().unwrap_or_default();
        if done(&attempts) {
            return attempts;
        }
        assert!(Instant::now() < deadline, "the log stayed {attempts:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn outcomes(attempts: &[Value]) -> Vec<&str> {
    attempts
        .iter()
        .map(|attempt| attempt["outcome"].as_str().expect("an outcome"))
        .collect()
}

/// How many deliveries are queued and not finished.
async fn queued(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'webhooks.deliver'")
        .fetch_one(pool)
        .await
        .expect("counted")
}

pub(super) fn signed_with(received: &Received, secrets: &[&str]) -> bool {
    let secrets: Vec<&[u8]> = secrets.iter().map(|secret| secret.as_bytes()).collect();
    signature::verify(
        &secrets,
        received.header("neon-webhook-signature"),
        received.header("neon-webhook-timestamp"),
        &received.body,
        OffsetDateTime::now_utc(),
    )
    .is_ok()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn webhooks_are_registered_for_whom_the_rules_let_and_managed_by_them(pool: PgPool) {
    let state = delivering(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let list = format!("/api/v1/workspaces/{ws}/webhooks");
    let receiver = Receiver::start().await;
    let url = receiver.url("/hook");
    let (cy, dee) = (me(&t.cy).await, me(&t.dee).await);
    let scout = agent(&t.cy, ws, "Scout").await;

    // A member registers for themselves and their agent, never for someone else.
    let own = register(&t.cy, ws, &cy, &url, json!(["row.assigned"])).await;
    assert_eq!(own.status, StatusCode::CREATED, "{}", own.body);
    let secret = own.body["secret"].as_str().expect("a secret");
    assert!(secret.starts_with("whsec_") && secret.len() == 6 + 43);
    let made = &own.body["webhook"];
    assert_eq!(made["identity"]["id"], cy.as_str());
    assert_eq!(made["identity"]["email"], Value::Null);
    assert_eq!(made["events"], json!(["row.assigned"]));
    assert_eq!(made["enabled"], true);
    assert_eq!(made["include_own"], false);
    assert_eq!(made["created_by"]["display_name"], "cy");
    assert!(made.get("secret").is_none());
    let mine = made["id"].as_str().expect("an id").to_owned();
    let for_agent = register(&t.cy, ws, &scout, &url, json!(["row.status_changed"])).await;
    assert_eq!(for_agent.status, StatusCode::CREATED, "{}", for_agent.body);
    let theirs = register(&t.cy, ws, &dee, &url, json!(["row.assigned"])).await;
    assert_eq!(theirs.status, StatusCode::FORBIDDEN, "{}", theirs.body);
    // An admin registers for anyone.
    let by_admin = register(&t.bob, ws, &dee, &url, json!(["row.assigned"])).await;
    assert_eq!(by_admin.status, StatusCode::CREATED, "{}", by_admin.body);

    // Each sees what they may manage.
    let ids = |answer: &Answer| -> Vec<String> {
        answer.body["webhooks"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|webhook| {
                webhook["identity"]["id"]
                    .as_str()
                    .expect("an id")
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(ids(&t.cy.get(&list).await), [cy.clone(), scout.clone()]);
    assert_eq!(ids(&t.dee.get(&list).await), std::slice::from_ref(&dee));
    assert_eq!(ids(&t.bob.get(&list).await).len(), 3);
    assert_eq!(
        ids(&t.bob.get(&format!("{list}?user_id={scout}")).await),
        std::slice::from_ref(&scout)
    );
    let one = format!("{list}/{mine}");
    assert_eq!(t.dee.get(&one).await.status, StatusCode::NOT_FOUND);
    assert_eq!(t.ada.get(&one).await.status, StatusCode::OK);

    // A guest registers none; an agent manages none, its own included.
    user(&state, "eve@example.com").await;
    let visitor = guest(&app, &t.ada, ws, "eve@example.com").await;
    let visitor_id = me(&visitor).await;
    let as_guest = register(&visitor, ws, &visitor_id, &url, json!(["row.assigned"])).await;
    assert_eq!(as_guest.status, StatusCode::FORBIDDEN, "{}", as_guest.body);
    assert_eq!(
        register(&visitor, ws, &cy, &url, json!(["row.assigned"]))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let token = Bearer::new(
        &app,
        agent_token(&t.cy, ws, &scout, &["read", "write"]).await,
    );
    assert_eq!(token.get(&list).await.body["webhooks"], json!([]));
    let by_agent = token
        .post(
            &list,
            json!({"user_id": scout, "url": url, "events": ["row.assigned"]}),
        )
        .await;
    assert_eq!(by_agent.status, StatusCode::FORBIDDEN, "{}", by_agent.body);
    assert_eq!(by_agent.code(), "forbidden");

    // Changing it: events, own changes, off and on again.
    let changed =
        t.cy.send(
            Method::PATCH,
            &one,
            Some(json!({"events": ["row.status_changed", "row.assigned"], "include_own": true})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(
        changed.body["events"],
        json!(["row.assigned", "row.status_changed"])
    );
    assert_eq!(changed.body["include_own"], true);
    let off =
        t.cy.send(Method::PATCH, &one, Some(json!({"enabled": false})))
            .await;
    assert_eq!(off.body["enabled"], false);
    assert_eq!(off.body["disabled_reason"], "turned off by cy");
    let on =
        t.cy.send(Method::PATCH, &one, Some(json!({"enabled": true})))
            .await;
    assert_eq!(on.body["enabled"], true);
    assert_eq!(on.body["disabled_reason"], Value::Null);
    let none =
        t.cy.send(Method::PATCH, &one, Some(json!({"events": []})))
            .await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST);

    // Rotating: a new secret, the old one still signing for a day.
    let rotated = t.cy.post(&format!("{one}/secret"), json!({})).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.body);
    assert_ne!(rotated.body["secret"], secret);
    assert!(rotated.body["webhook"]["previous_secret_expires_at"].is_string());

    // Deleting it.
    let deleted = t.cy.send(Method::DELETE, &one, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(t.cy.get(&one).await.status, StatusCode::NOT_FOUND);
    assert!(receiver.received().is_empty(), "registering sends nothing");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn urls_must_reach_public_addresses_unless_private_ones_are_allowed(pool: PgPool) {
    let state = state_for_webhooks(pool, Arc::new(HttpTransport::default())).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let cy = me(&t.cy).await;
    let events = json!(["row.assigned"]);

    for (url, code) in [
        ("http://127.0.0.1:9/hook", "address_refused"),
        ("http://[::ffff:10.0.0.1]/", "address_refused"),
        ("http://169.254.169.254/latest/meta-data", "address_refused"),
        ("http://0x7f.1/", "address_refused"),
        ("ftp://example.com/", "invalid_url"),
        ("https://user:pass@example.com/", "invalid_url"),
        ("not a url", "invalid_url"),
    ] {
        let refused = register(&t.cy, ws, &cy, url, events.clone()).await;
        assert_eq!(
            (refused.status, refused.code()),
            (StatusCode::BAD_REQUEST, code),
            "{url}: {}",
            refused.body
        );
    }

    allow_private(&state, true).await;
    let local = register(&t.cy, ws, &cy, "http://127.0.0.1:9/hook", events.clone()).await;
    assert_eq!(local.status, StatusCode::CREATED, "{}", local.body);
    // The metadata service stays out of reach whatever the setting says.
    let metadata = register(&t.cy, ws, &cy, "http://169.254.169.254/", events.clone()).await;
    assert_eq!(metadata.code(), "address_refused");
    let webhook = local.body["webhook"]["id"].as_str().expect("an id");
    allow_private(&state, false).await;
    let moved =
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}/webhooks/{webhook}"),
            Some(json!({"url": "http://192.168.1.10/hook"})),
        )
        .await;
    assert_eq!(moved.code(), "address_refused", "{}", moved.body);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn webhooks_need_a_secret_key_to_keep_their_secrets(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let cy = me(&t.cy).await;

    let refused = register(
        &t.cy,
        &t.workspace,
        &cy,
        "https://hooks.example.com/",
        json!(["row.assigned"]),
    )
    .await;

    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::CONFLICT, "no_secret_key"),
        "{}",
        refused.body
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn assignments_and_status_changes_are_delivered_signed(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let cy = me(&t.cy).await;
    let (hook, secret) = webhook(&t.cy, ws, &cy, &receiver.url("/hook")).await;
    let (db, read) = tasks(&t.ada, ws).await;
    let assignee = property(&read, "Assignee");
    let status = property(&read, "Status");
    let started = option(&read, "Status", "In progress");
    let worker = worker(&pool, &state).await;

    // Made with cy in it: assigned. Then its status changes: told too.
    let task = row(&t.ada, &db, json!({"Name": "Fix login", "Assignee": [cy]})).await;
    let first = receiver.wait_for(1).await;
    change(&t.ada, &task, json!({"Status": "In progress"})).await;
    let both = receiver.wait_for(2).await;

    let assigned = &first[0];
    assert!(signed_with(assigned, &[&secret]), "signed with its secret");
    assert!(!signed_with(assigned, &["whsec_another"]));
    assert_eq!(assigned.header("neon-webhook-event"), Some("row.assigned"));
    assert!(assigned.header("neon-webhook-delivery").is_some());
    assert_eq!(assigned.header("content-type"), Some("application/json"));
    let body = assigned.json();
    assert_eq!(body["version"], 1);
    assert_eq!(body["type"], "row.assigned");
    assert_eq!(body["workspace"]["id"], ws.as_str());
    assert_eq!(
        body["identity"],
        json!({"id": cy, "kind": "human", "name": "cy"})
    );
    assert_eq!(body["row"]["id"], task.as_str());
    assert_eq!(body["row"]["title"], "Fix login");
    assert_eq!(body["row"]["unique_id"], "1");
    assert_eq!(body["row"]["database"], json!({"id": db, "title": "Tasks"}));
    assert_eq!(
        body["property"],
        json!({"id": assignee, "name": "Assignee", "type": "person"})
    );
    assert!(body["occurred_at"].is_string());
    assert!(body.get("change").is_none());

    let moved = &both[1];
    assert!(signed_with(moved, &[&secret]));
    let body = moved.json();
    assert_eq!(body["type"], "row.status_changed");
    assert_eq!(body["property"]["id"], status.as_str());
    assert_eq!(body["change"]["from"], Value::Null);
    assert_eq!(
        body["change"]["to"],
        json!({"id": started, "name": "In progress"})
    );
    assert_eq!(body["row"]["status"]["name"], "In progress");

    // The two deliveries are separate jobs, and the log lists attempts as they end, so
    // either may come first.
    let attempts = log_until(&t.cy, ws, &hook, |attempts| attempts.len() == 2).await;
    assert_eq!(outcomes(&attempts), ["delivered", "delivered"]);
    let mut events: Vec<&str> = attempts
        .iter()
        .map(|attempt| attempt["event_type"].as_str().expect("an event type"))
        .collect();
    events.sort_unstable();
    assert_eq!(events, ["row.assigned", "row.status_changed"]);
    for attempt in &attempts {
        assert_eq!(attempt["status_code"], 200);
        assert!(attempt["latency_ms"].is_number());
    }
    // A delivery's job is deleted once it is done, after its attempt is logged.
    let deadline = Instant::now() + PATIENCE;
    while queued(&pool).await != 0 {
        assert!(Instant::now() < deadline, "the done deliveries stay queued");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn moving_a_card_to_a_persons_column_assigns_them(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    webhook(&t.bob, ws, &dee, &receiver.url("/hook")).await;
    let (db, read) = tasks(&t.ada, ws).await;
    let assignee = property(&read, "Assignee");
    let board = t
        .ada
        .post(
            &format!("/api/v1/databases/{db}/views"),
            json!({"name": "By person", "layout": "board", "group": {"property": assignee}}),
        )
        .await;
    assert_eq!(board.status, StatusCode::CREATED, "{}", board.body);
    let view = id(&board);
    let task = row(&t.ada, &db, json!({"Name": "Review"})).await;
    let worker = worker(&pool, &state).await;

    let moved = t
        .ada
        .post(
            &format!("/api/v1/rows/{task}/move"),
            json!({"group": {"view_id": view, "from": null, "to": dee}}),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);

    let received = receiver.wait_for(1).await;
    assert_eq!(received[0].json()["type"], "row.assigned");
    assert_eq!(received[0].json()["identity"]["id"], dee.as_str());
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn own_changes_are_told_only_when_asked(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let cy = me(&t.cy).await;
    let (hook, _) = webhook(&t.cy, ws, &cy, &receiver.url("/hook")).await;
    let (db, _) = tasks(&t.ada, ws).await;

    // cy assigns herself: nothing is even queued.
    let task = row(&t.cy, &db, json!({"Name": "Mine", "Assignee": [cy]})).await;
    change(&t.cy, &task, json!({"Status": "Done"})).await;
    assert_eq!(queued(&pool).await, 0);

    let asked =
        t.cy.send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}/webhooks/{hook}"),
            Some(json!({"include_own": true})),
        )
        .await;
    assert_eq!(asked.status, StatusCode::OK, "{}", asked.body);
    change(&t.cy, &task, json!({"Status": "In progress"})).await;
    assert_eq!(queued(&pool).await, 1);
    let worker = worker(&pool, &state).await;
    let received = receiver.wait_for(1).await;
    assert_eq!(received[0].json()["type"], "row.status_changed");
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn failed_deliveries_are_retried_then_given_up_on(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    receiver.always(Reply {
        status: 503,
        headers: Vec::new(),
        body: b"down for maintenance".to_vec(),
    });
    let dee = me(&t.dee).await;
    let (hook, _) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "Flaky", "Assignee": [dee]})).await;
    let worker = worker(&pool, &state).await;

    let attempts = log_until(&t.dee, ws, &hook, |attempts| {
        attempts
            .first()
            .is_some_and(|last| last["outcome"] == "failed")
    })
    .await;

    let max = usize::try_from(neon_app::webhooks::MAX_ATTEMPTS).expect("a count");
    assert_eq!(attempts.len(), max);
    let mut expected = vec!["retrying"; max - 1];
    expected.insert(0, "failed");
    assert_eq!(outcomes(&attempts), expected);
    assert_eq!(attempts[0]["attempt"], neon_app::webhooks::MAX_ATTEMPTS);
    assert_eq!(attempts[max - 1]["attempt"], 1);
    assert_eq!(attempts[0]["status_code"], 503);
    assert_eq!(attempts[0]["error_kind"], "http_status");
    assert_eq!(attempts[0]["excerpt"], "down for maintenance");
    let delivery = &attempts[0]["delivery_id"];
    assert!(
        attempts
            .iter()
            .all(|attempt| &attempt["delivery_id"] == delivery)
    );
    assert_eq!(receiver.received().len(), max);
    // Given up on once, it stays on, counting the failure.
    let read = t
        .dee
        .get(&format!("/api/v1/workspaces/{ws}/webhooks/{hook}"))
        .await;
    assert_eq!(read.body["enabled"], true);
    assert_eq!(read.body["failures"], 1);
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_failed_delivery_is_tried_again_until_it_goes_through(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    receiver.script([Reply::status(500), Reply::status(502)]);
    let dee = me(&t.dee).await;
    let (hook, secret) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    let (db, _) = tasks(&t.ada, ws).await;
    row(
        &t.ada,
        &db,
        json!({"Name": "Eventually", "Assignee": [dee]}),
    )
    .await;
    let worker = worker(&pool, &state).await;

    let attempts = log_until(&t.dee, ws, &hook, |attempts| {
        attempts
            .first()
            .is_some_and(|last| last["outcome"] == "delivered")
    })
    .await;

    assert_eq!(outcomes(&attempts), ["delivered", "retrying", "retrying"]);
    assert_eq!(attempts[1]["status_code"], 502);
    let received = receiver.received();
    assert_eq!(received.len(), 3);
    // Every attempt is the same event, signed afresh.
    let ids: Vec<Value> = received
        .iter()
        .map(|got| got.json()["id"].clone())
        .collect();
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    assert!(received.iter().all(|got| signed_with(got, &[&secret])));
    let deliveries: Vec<Option<&str>> = received
        .iter()
        .map(|got| got.header("neon-webhook-delivery"))
        .collect();
    assert!(deliveries.iter().all(|delivery| *delivery == deliveries[0]));
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_webhook_turns_itself_off_after_deliveries_keep_failing(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    receiver.always(Reply::status(500));
    let dee = me(&t.dee).await;
    let (hook, _) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    let one = format!("/api/v1/workspaces/{ws}/webhooks/{hook}");
    let (db, _) = tasks(&t.ada, ws).await;
    let limit = usize::try_from(neon_app::webhooks::DISABLE_AFTER).expect("a count");
    for n in 0..limit {
        row(
            &t.ada,
            &db,
            json!({"Name": format!("Task {n}"), "Assignee": [dee]}),
        )
        .await;
    }
    let worker = worker(&pool, &state).await;

    let deadline = Instant::now() + PATIENCE * 3;
    let off = loop {
        let read = t.dee.get(&one).await;
        if read.body["enabled"] == false {
            break read.body;
        }
        assert!(Instant::now() < deadline, "still on: {}", read.body);
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(off["failures"], neon_app::webhooks::DISABLE_AFTER);
    let reason = off["disabled_reason"].as_str().expect("why");
    assert!(
        reason.starts_with(
            "turned off after 5 deliveries in a row failed; the last: the receiver answered 500"
        ),
        "{reason}"
    );
    worker.stop().await;

    // Off, it is told of nothing; on again, its failures start over.
    row(&t.ada, &db, json!({"Name": "Later", "Assignee": [dee]})).await;
    assert_eq!(queued(&pool).await, 0);
    let on = t
        .dee
        .send(Method::PATCH, &one, Some(json!({"enabled": true})))
        .await;
    assert_eq!(on.body["failures"], 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_delivery_is_skipped_once_its_identity_no_longer_sees_the_row(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    let (hook, _) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    // A private teamspace with dee in it, and its database.
    let (members, db) = private_tasks(&t.ada, ws, &[&dee], "Secret plans").await;

    // Assigned while in it; out of it before the delivery runs.
    row(
        &t.ada,
        &db,
        json!({"Name": "Launch day", "Assignee": [dee]}),
    )
    .await;
    assert_eq!(queued(&pool).await, 1);
    let removed = t
        .ada
        .send(Method::DELETE, &format!("{members}/{dee}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    let worker = worker(&pool, &state).await;

    let attempts = log_until(&t.dee, ws, &hook, |attempts| !attempts.is_empty()).await;
    assert_eq!(outcomes(&attempts), ["skipped"]);
    assert_eq!(attempts[0]["error_kind"], "no_access");
    assert_eq!(attempts[0]["status_code"], Value::Null);
    assert!(
        receiver.received().is_empty(),
        "nothing of the row was sent"
    );
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_hears_of_its_work_while_its_owner_is_in_the_workspace(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let scout = agent(&t.dee, ws, "Scout").await;
    let (hook, secret) = webhook(&t.dee, ws, &scout, &receiver.url("/agent")).await;
    let (db, _) = tasks(&t.ada, ws).await;
    let worker = worker(&pool, &state).await;

    let task = row(&t.ada, &db, json!({"Name": "Crawl", "Assignee": [scout]})).await;
    let received = receiver.wait_for(1).await;
    assert!(signed_with(&received[0], &[&secret]));
    assert_eq!(
        received[0].json()["identity"],
        json!({"id": scout, "kind": "agent", "name": "Scout"})
    );

    // Its owner leaves: it is suspended, and hears nothing more. Its webhook, whose URL
    // they set, turns off; a delivery queued before they left is skipped.
    worker.stop().await;
    change(&t.ada, &task, json!({"Status": "In progress"})).await;
    let left = t
        .dee
        .post(&format!("/api/v1/workspaces/{ws}/leave"), json!({}))
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    change(&t.ada, &task, json!({"Status": "Done"})).await;
    let again = self::worker(&pool, &state).await;
    let attempts = log_until(&t.ada, ws, &hook, |attempts| attempts.len() == 2).await;
    again.stop().await;
    assert_eq!(outcomes(&attempts), ["skipped", "delivered"]);
    assert_eq!(attempts[0]["error_kind"], "disabled");
    assert_eq!(receiver.received().len(), 1);
    let off = t
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/webhooks/{hook}"))
        .await
        .body;
    assert_eq!(off["enabled"], false, "{off}");
    assert_eq!(queued(&pool).await, 0);
}

/// Resolves the receiver's name to a public address when the webhook is registered, then to
/// the receiver itself.
#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_name_that_rebinds_to_a_private_address_is_never_reached(pool: PgPool) {
    let receiver = Receiver::start().await;
    let port = receiver.addr().port();
    let public = SocketAddr::from(([93, 184, 215, 14], port));
    let resolver = ScriptedResolver::new(vec![vec![public], vec![receiver.addr()]]);
    let transport = HttpTransport::new(resolver.clone(), Arc::new(crate::webhooks::SystemDialer));
    let state = state_for_webhooks(pool.clone(), Arc::new(transport)).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let dee = me(&t.dee).await;
    let url = format!("http://hooks.example.com:{port}/hook");
    let (hook, _) = webhook(&t.dee, ws, &dee, &url).await;
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "Rebind", "Assignee": [dee]})).await;
    let worker = worker(&pool, &state).await;

    log_until(&t.dee, ws, &hook, |attempts| !attempts.is_empty()).await;
    // Stopped, it makes no more attempts while the log is read.
    worker.stop().await;
    let attempts = log_until(&t.dee, ws, &hook, |_| true).await;

    assert!(
        attempts
            .iter()
            .all(|attempt| attempt["error_kind"] == "address_refused"
                && attempt["status_code"] == Value::Null),
        "{attempts:?}"
    );
    assert_eq!(
        resolver.asked().len(),
        1 + attempts.len(),
        "once to register, then once for each attempt"
    );
    assert!(receiver.received().is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_test_event_is_sent_signed_and_logged_and_rotation_signs_with_both(pool: PgPool) {
    let state = delivering(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    let (hook, old) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    let one = format!("/api/v1/workspaces/{ws}/webhooks/{hook}");

    let tested = t.dee.post(&format!("{one}/test"), json!({})).await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.body);
    assert_eq!(tested.body["outcome"], "delivered");
    assert_eq!(tested.body["status_code"], 200);
    assert_eq!(tested.body["event_type"], "webhook.test");
    assert_eq!(tested.body["delivery_id"], Value::Null);
    let received = receiver.wait_for(1).await;
    assert!(signed_with(&received[0], &[&old]));
    let body = received[0].json();
    assert_eq!(body["type"], "webhook.test");
    assert_eq!(body["row"], Value::Null);
    assert_eq!(body["identity"]["id"], dee.as_str());

    let rotated = t.dee.post(&format!("{one}/secret"), json!({})).await;
    let new = rotated.body["secret"]
        .as_str()
        .expect("a secret")
        .to_owned();
    t.dee.post(&format!("{one}/test"), json!({})).await;
    let received = receiver.wait_for(2).await;
    let header = received[1]
        .header("neon-webhook-signature")
        .expect("signed");
    assert_eq!(header.matches("v1=").count(), 2, "{header}");
    assert!(signed_with(&received[1], &[&new]));
    assert!(signed_with(&received[1], &[&old]));

    // A receiver that fails: the test says so, and nothing is retried.
    receiver.always(Reply::status(404));
    let failed = t.dee.post(&format!("{one}/test"), json!({})).await;
    assert_eq!(failed.body["outcome"], "failed");
    assert_eq!(failed.body["error_kind"], "http_status");
    let log = t.dee.get(&format!("{one}/deliveries")).await;
    assert_eq!(
        outcomes(log.body["attempts"].as_array().expect("a log")),
        ["failed", "delivered", "delivered"]
    );
    let read = t.dee.get(&one).await;
    assert_eq!(read.body["failures"], 0);
}

/// One webhook's log, read once.
async fn log_of(by: &Browser<'_>, ws: &str, webhook: &str) -> Vec<Value> {
    log_until(by, ws, webhook, |_| true).await
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_admin_hears_only_of_rows_they_may_see_too(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    // bob, an admin outside the private teamspace, registers a webhook for dee, and points
    // dee's own at a URL of his.
    let (registered, _) = webhook(&t.bob, ws, &dee, &receiver.url("/bob")).await;
    let (repointed, _) = webhook(&t.dee, ws, &dee, &receiver.url("/dee")).await;
    let moved = t
        .bob
        .send(
            Method::PATCH,
            &format!("/api/v1/workspaces/{ws}/webhooks/{repointed}"),
            Some(json!({"url": receiver.url("/bob-repointed")})),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    let (_, vault) = private_tasks(&t.ada, ws, &[&dee], "Vault plans").await;
    let hidden = t.bob.get(&format!("/api/v1/databases/{vault}")).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.body);
    let worker = worker(&pool, &state).await;

    // A row of the vault, assigned to dee, which bob may not read: skipped for both.
    let secret = row(
        &t.ada,
        &vault,
        json!({"Name": "Acquire Initech", "Assignee": [dee]}),
    )
    .await;
    assert_eq!(
        t.bob.get(&format!("/api/v1/rows/{secret}")).await.status,
        StatusCode::NOT_FOUND
    );
    for hook in [&registered, &repointed] {
        let attempts = log_until(&t.bob, ws, hook, |attempts| !attempts.is_empty()).await;
        assert_eq!(outcomes(&attempts), ["skipped"]);
        assert_eq!(attempts[0]["error_kind"], "no_access");
    }
    assert!(receiver.received().is_empty(), "{:?}", receiver.received());

    // A row everyone may read reaches both of bob's URLs; they stay on.
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "Ship it", "Assignee": [dee]})).await;
    let received = receiver.wait_for(2).await;
    let mut paths: Vec<&str> = received.iter().map(|got| got.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["/bob", "/bob-repointed"]);
    assert!(
        received
            .iter()
            .all(|got| got.json()["row"]["title"] == "Ship it")
    );
    for hook in [&registered, &repointed] {
        let read = t
            .bob
            .get(&format!("/api/v1/workspaces/{ws}/webhooks/{hook}"))
            .await;
        assert_eq!(read.body["enabled"], true, "{}", read.body);
    }
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn webhooks_turn_off_once_whoever_set_their_url_may_no_longer_register_them(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let (bob, dee) = (me(&t.bob).await, me(&t.dee).await);
    let member = format!("/api/v1/workspaces/{ws}/members/{bob}");
    let read = |hook: &str| format!("/api/v1/workspaces/{ws}/webhooks/{hook}");

    // Demoted, bob may no longer register for dee: what he registered turns off at once.
    let (demoted, _) = webhook(&t.bob, ws, &dee, &receiver.url("/bob")).await;
    let changed = t
        .ada
        .send(Method::PATCH, &member, Some(json!({"role": "member"})))
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    let off = t.ada.get(&read(&demoted)).await.body;
    assert_eq!(off["enabled"], false, "{off}");
    assert_eq!(
        off["disabled_reason"],
        "turned off: bob set its URL, and may no longer register webhooks for dee; setting its \
         URL again turns it back on"
    );
    // Turned on again as it is, it is refused; with its URL set again, it is ada's.
    let refused = t
        .ada
        .send(
            Method::PATCH,
            &read(&demoted),
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::CONFLICT, "url_setter_lapsed"),
        "{}",
        refused.body
    );
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "While off", "Assignee": [dee]})).await;
    assert_eq!(queued(&pool).await, 0);
    let again = t
        .ada
        .send(
            Method::PATCH,
            &read(&demoted),
            Some(json!({"enabled": true, "url": receiver.url("/ada")})),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body["enabled"], true);

    // Removed, the same.
    let promoted = t
        .ada
        .send(Method::PATCH, &member, Some(json!({"role": "admin"})))
        .await;
    assert_eq!(promoted.status, StatusCode::OK, "{}", promoted.body);
    let (removed, _) = webhook(&t.bob, ws, &dee, &receiver.url("/bob")).await;
    let gone = t.ada.send(Method::DELETE, &member, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.body);
    assert_eq!(t.ada.get(&read(&removed)).await.body["enabled"], false);
    assert_eq!(t.ada.get(&read(&demoted)).await.body["enabled"], true);

    // Only ada's URL hears of dee's next assignment.
    let worker = worker(&pool, &state).await;
    row(&t.ada, &db, json!({"Name": "After", "Assignee": [dee]})).await;
    let received = receiver.wait_for(1).await;
    assert_eq!(received[0].path, "/ada");
    log_until(&t.ada, ws, &demoted, |attempts| !attempts.is_empty()).await;
    assert!(log_of(&t.ada, ws, &removed).await.is_empty());
    worker.stop().await;
    assert_eq!(receiver.received().len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_webhook_whose_setter_lost_the_right_unnoticed_turns_off_at_its_delivery(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let (bob, dee) = (me(&t.bob).await, me(&t.dee).await);
    let (hook, _) = webhook(&t.bob, ws, &dee, &receiver.url("/bob")).await;
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "Queued", "Assignee": [dee]})).await;
    assert_eq!(queued(&pool).await, 1);
    // bob's account is disabled after the delivery is queued, which no workspace hears of.
    let uuid = uuid::Uuid::try_parse(&bob).expect("an id");
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(uuid)
        .execute(&pool)
        .await
        .expect("disabled");
    let worker = worker(&pool, &state).await;

    let attempts = log_until(&t.ada, ws, &hook, |attempts| !attempts.is_empty()).await;
    worker.stop().await;
    assert_eq!(outcomes(&attempts), ["skipped"]);
    assert_eq!(attempts[0]["error_kind"], "lapsed");
    let off = t
        .ada
        .get(&format!("/api/v1/workspaces/{ws}/webhooks/{hook}"))
        .await
        .body;
    assert_eq!(off["enabled"], false, "{off}");
    assert!(
        off["disabled_reason"]
            .as_str()
            .is_some_and(|why| why.starts_with("turned off: bob, who set its URL, is disabled")),
        "{off}"
    );
    assert!(receiver.received().is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_handed_on_tells_its_former_owner_nothing_more(pool: PgPool) {
    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    let scout = agent(&t.cy, ws, "Scout").await;
    let (cy_hook, _) = webhook(&t.cy, ws, &scout, &receiver.url("/cy")).await;
    let (bob_hook, _) = webhook(&t.bob, ws, &scout, &receiver.url("/bob")).await;

    let handed = t
        .bob
        .post(
            &format!("/api/v1/workspaces/{ws}/agents/{scout}/transfer"),
            json!({"owner_id": dee}),
        )
        .await;
    assert_eq!(handed.status, StatusCode::OK, "{}", handed.body);
    assert_eq!(
        t.cy.get(&format!("/api/v1/workspaces/{ws}/webhooks"))
            .await
            .body["webhooks"],
        json!([])
    );
    // dee, its owner now, sees cy's off, saying why; the admin's stays on.
    let listed = t
        .dee
        .get(&format!("/api/v1/workspaces/{ws}/webhooks"))
        .await;
    let webhooks = listed.body["webhooks"].as_array().expect("a list");
    let of = |id: &str| {
        webhooks
            .iter()
            .find(|webhook| webhook["id"] == id)
            .unwrap_or_else(|| panic!("{id} in {webhooks:?}"))
    };
    assert_eq!(of(&cy_hook)["enabled"], false);
    assert_eq!(
        of(&cy_hook)["disabled_reason"],
        "turned off: cy set its URL, and may no longer register webhooks for Scout; setting \
         its URL again turns it back on"
    );
    assert_eq!(of(&bob_hook)["enabled"], true);

    // A row in a teamspace cy is not in, assigned to Scout, reaches the admin's URL alone.
    let (members, vault) = private_tasks(&t.ada, ws, &[&dee], "Vault plans").await;
    let added = t.ada.post(&members, json!({"user_id": scout})).await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    let bob_in = t
        .ada
        .post(&members, json!({"user_id": me(&t.bob).await}))
        .await;
    assert_eq!(bob_in.status, StatusCode::CREATED, "{}", bob_in.body);
    let worker = worker(&pool, &state).await;
    row(
        &t.ada,
        &vault,
        json!({"Name": "Board seat", "Assignee": [scout]}),
    )
    .await;
    let received = receiver.wait_for(1).await;
    worker.stop().await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/bob");
    assert!(log_of(&t.dee, ws, &cy_hook).await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn webhooks_are_capped_per_identity_and_per_workspace(pool: PgPool) {
    use neon_app::webhooks::{MAX_PER_IDENTITY, MAX_PER_WORKSPACE};

    let state = delivering(pool.clone()).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let (bob, cy, dee) = (me(&t.bob).await, me(&t.cy).await, me(&t.dee).await);
    let url = "http://127.0.0.1:9/hook";
    let events = json!(["row.assigned"]);
    for _ in 0..MAX_PER_IDENTITY {
        let made = register(&t.cy, ws, &cy, url, events.clone()).await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    }

    let refused = register(&t.cy, ws, &cy, url, events.clone()).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::CONFLICT, "webhook_limit"),
        "{}",
        refused.body
    );
    assert!(
        refused.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("this identity has 10 webhooks")),
        "{}",
        refused.body
    );
    // One change concerns each identity's webhooks, no more.
    let (db, _) = tasks(&t.ada, ws).await;
    row(&t.ada, &db, json!({"Name": "Fan out", "Assignee": [cy]})).await;
    assert_eq!(queued(&pool).await, MAX_PER_IDENTITY);

    // The workspace's own limit: dee's, filled in directly, take what is left of it.
    let workspace = uuid::Uuid::try_parse(ws).expect("an id");
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM webhooks WHERE workspace_id = $1")
        .bind(workspace)
        .fetch_one(&pool)
        .await
        .expect("counted");
    sqlx::query(
        "INSERT INTO webhooks (id, workspace_id, user_id, url, events, secret, created_at,
                               updated_at)
         SELECT gen_random_uuid(), w.workspace_id, $2, w.url, w.events, w.secret, now(), now()
         FROM webhooks w, generate_series(1, $3)
         WHERE w.workspace_id = $1 AND w.user_id <> $2
         LIMIT $3",
    )
    .bind(workspace)
    .bind(uuid::Uuid::try_parse(&dee).expect("an id"))
    .bind(MAX_PER_WORKSPACE - held)
    .execute(&pool)
    .await
    .expect("filled in");
    let full = register(&t.bob, ws, &bob, url, events.clone()).await;
    assert_eq!(
        (full.status, full.code()),
        (StatusCode::CONFLICT, "webhook_limit"),
        "{}",
        full.body
    );
    assert!(
        full.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("this workspace has 200 webhooks")),
        "{}",
        full.body
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn test_events_are_held_to_a_rate(pool: PgPool) {
    let state = delivering(pool).await;
    let app = router(state.clone(), false);
    let t = team(&state, &app).await;
    let ws = &t.workspace;
    let receiver = Receiver::start().await;
    let dee = me(&t.dee).await;
    let (hook, _) = webhook(&t.dee, ws, &dee, &receiver.url("/hook")).await;
    let test = format!("/api/v1/workspaces/{ws}/webhooks/{hook}/test");
    for _ in 0..10 {
        let sent = t.dee.post(&test, json!({})).await;
        assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    }

    let limited = t.dee.post(&test, json!({})).await;

    assert_eq!(
        (limited.status, limited.code()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        "{}",
        limited.body
    );
    assert!(limited.headers.contains_key("retry-after"));
    assert_eq!(receiver.received().len(), 10);
    // Each person is counted on their own.
    let by_admin = t.ada.post(&test, json!({})).await;
    assert_eq!(by_admin.status, StatusCode::OK, "{}", by_admin.body);
}
