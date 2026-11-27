use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::{get, post};
use http_body_util::BodyExt as _;
use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::attachments::{AttachmentService, BlobStore, ObjectBlobs};
use neon_app::auth::AuthService;
use neon_app::clock::Clock;
use neon_app::comments::CommentService;
use neon_app::crypto::{Argon2Params, Envelope};
use neon_app::databases::{DatabaseService, LiveDatabases, LiveLimits};
use neon_app::history::HistoryService;
use neon_app::mail::MailQueue;
use neon_app::notifications::{FeedLimits, InboxFeeds, InboxService};
use neon_app::oauth::OAuthService;
use neon_app::pages::{AccessChanges, Fanout, PageService};
use neon_app::ratelimit::{MemoryLimiter, RateLimiter};
use neon_app::search::SearchService;
use neon_app::settings::{Change, SettingsService, registry};
use neon_app::sync::{
    Broadcaster, Limits as SyncLimits, LocalBroadcaster, SyncService, access_feed,
};
use neon_app::transfer::TransferService;
use neon_app::webhooks::{Services, WebhookService, WebhookTransport};
use neon_app::workspaces::{AgentService, WorkspaceService};
use neon_domain::settings::Pinned;
use neon_observability::http::X_REQUEST_ID;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt as _;

use super::*;

/// Where mail links point in the tests. Mail still needs its settings and a secret key.
pub(super) const PUBLIC_URL: &str = "https://neon.example.com";

/// Hashes cheap enough for tests.
pub(super) const CHEAP: Argon2Params = Argon2Params {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
    output_len: 32,
    salt_len: 16,
};

pub(super) async fn state(pool: PgPool) -> AppState {
    state_with(pool, Envelope::default(), WebConfig::new(None, Vec::new())).await
}

pub(super) async fn state_with(pool: PgPool, envelope: Envelope, web: WebConfig) -> AppState {
    state_for(pool, envelope, web, SyncLimits::default()).await
}

/// The state with a secret key, whose webhooks go through `transport`.
pub(super) async fn state_for_webhooks(
    pool: PgPool,
    transport: Arc<dyn WebhookTransport>,
) -> AppState {
    build_state(
        pool,
        super::auth_tests::keyed(),
        WebConfig::new(None, Vec::new()),
        Setup {
            transport,
            ..Setup::default()
        },
    )
    .await
}

/// The state, with live documents that keep to `limits`.
pub(super) async fn state_for(
    pool: PgPool,
    envelope: Envelope,
    web: WebConfig,
    limits: SyncLimits,
) -> AppState {
    let setup = Setup {
        sync: limits,
        ..Setup::default()
    };
    build_state(pool, envelope, web, setup).await
}

/// The state with `web`, its sessions, tokens and OAuth reading the time from `clock`.
pub(super) async fn state_at(pool: PgPool, web: WebConfig, clock: Clock) -> AppState {
    let setup = Setup {
        clock,
        ..Setup::default()
    };
    build_state(pool, Envelope::default(), web, setup).await
}

/// The state, with database feeds that keep to `limits`.
pub(super) async fn state_with_feeds(pool: PgPool, limits: LiveLimits) -> AppState {
    let setup = Setup {
        feeds: limits,
        ..Setup::default()
    };
    build_state(
        pool,
        Envelope::default(),
        WebConfig::new(None, Vec::new()),
        setup,
    )
    .await
}

/// The state, with the MCP server counting requests, and the content routes edits, in
/// `limiter`, on `web`.
pub(crate) async fn state_with_mcp(
    pool: PgPool,
    web: WebConfig,
    limits: SyncLimits,
    limiter: Arc<dyn RateLimiter>,
) -> AppState {
    let setup = Setup {
        sync: limits,
        limiter,
        ..Setup::default()
    };
    build_state(pool, Envelope::default(), web, setup).await
}

/// The state, its files kept in `blobs` and its attachments' time read from `clock`.
pub(super) async fn state_with_blobs(
    pool: PgPool,
    blobs: Arc<dyn BlobStore>,
    limiter: Arc<dyn RateLimiter>,
    clock: Clock,
) -> AppState {
    let setup = Setup {
        blobs,
        limiter,
        clock,
        ..Setup::default()
    };
    build_state(
        pool,
        Envelope::default(),
        WebConfig::new(None, Vec::new()),
        setup,
    )
    .await
}

/// What a test's state is built with, beside its database, key and web configuration.
struct Setup {
    sync: SyncLimits,
    feeds: LiveLimits,
    inboxes: FeedLimits,
    transport: Arc<dyn WebhookTransport>,
    /// Where every use case's attempts are counted.
    limiter: Arc<dyn RateLimiter>,
    /// The time sessions, tokens, OAuth, webhooks and attachments read.
    clock: Clock,
    /// Where files are kept: in memory unless a test says.
    blobs: Arc<dyn BlobStore>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            sync: SyncLimits::default(),
            feeds: LiveLimits::default(),
            inboxes: FeedLimits::default(),
            transport: Arc::new(crate::webhooks::HttpTransport::default()),
            limiter: Arc::new(MemoryLimiter::new()),
            clock: Clock::system(),
            blobs: Arc::new(ObjectBlobs::new(Arc::new(
                object_store::memory::InMemory::new(),
            ))),
        }
    }
}

async fn build_state(pool: PgPool, envelope: Envelope, web: WebConfig, setup: Setup) -> AppState {
    let Setup {
        sync: limits,
        feeds,
        inboxes: inbox_limits,
        transport,
        limiter,
        clock,
        blobs,
    } = setup;
    let (settings, warnings) = SettingsService::load(
        pool.clone(),
        registry(),
        Pinned::default(),
        envelope.clone(),
    )
    .await
    .expect("settings load");
    assert!(warnings.is_empty(), "{warnings:?}");
    let settings = Arc::new(settings);
    let mail = MailQueue::new(
        Arc::clone(&settings),
        envelope.clone(),
        Some(PUBLIC_URL),
        Clock::system(),
    );
    let (documents, feed) = access_feed();
    let (followers, databases_feed) = access_feed();
    let (inboxes, inbox_feed) = access_feed();
    let (sessions, mcp_feed) = crate::mcp::access_feed();
    let listener: Arc<dyn AccessChanges> =
        Arc::new(Fanout::new(vec![documents, followers, inboxes, sessions]));
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(LocalBroadcaster::new());
    let auth = AuthService::builder(pool.clone(), Arc::clone(&settings))
        .password_params(CHEAP)
        .envelope(envelope.clone())
        .clock(clock.clone())
        .mail(mail.clone())
        .access_changes(Arc::clone(&listener))
        .build()
        .await
        .expect("the auth service starts");
    let workspaces = WorkspaceService::builder(pool.clone(), Arc::clone(&settings))
        .envelope(envelope.clone())
        .access_changes(Arc::clone(&listener))
        .broadcaster(Arc::clone(&broadcaster))
        .build();
    let agents = AgentService::new(workspaces.clone());
    let oauth = OAuthService::builder(pool.clone(), Arc::clone(&settings))
        .clock(clock.clone())
        .access_changes(Arc::clone(&listener))
        .build();
    let pages = PageService::builder(pool.clone(), Arc::clone(&settings))
        .access_changes(listener)
        .broadcaster(Arc::clone(&broadcaster))
        .build();
    let sync = SyncService::builder(pool.clone(), pages.clone(), auth.clone(), feed)
        .limits(limits)
        .broadcaster(Arc::clone(&broadcaster))
        .limiter(Arc::clone(&limiter))
        .build();
    let history = HistoryService::builder(
        pool.clone(),
        Arc::clone(&settings),
        pages.clone(),
        sync.clone(),
    )
    .build();
    let search = SearchService::builder(pool.clone())
        .clock(clock.clone())
        .limiter(Arc::clone(&limiter))
        .build();
    let databases = DatabaseService::builder(pages.clone()).build();
    let live = LiveDatabases::builder(databases.clone(), auth.clone(), databases_feed)
        .limits(feeds)
        .build();
    let attachments = AttachmentService::builder_with_store(
        pool.clone(),
        Arc::clone(&settings),
        pages.clone(),
        sync.clone(),
        blobs,
    )
    .limiter(Arc::clone(&limiter))
    .clock(clock.clone())
    .build();
    let transfer = TransferService::builder(
        pages.clone(),
        sync.clone(),
        databases.clone(),
        attachments.clone(),
    )
    .build();
    let comments = CommentService::new(pages.clone(), sync.clone());
    let inbox = InboxService::new(pages.clone(), mail, envelope.clone());
    let inbox_feeds = InboxFeeds::new(
        auth.clone(),
        Arc::clone(&broadcaster),
        inbox_feed,
        inbox_limits,
    );
    let webhooks = WebhookService::builder(
        pool.clone(),
        Arc::clone(&settings),
        Services {
            workspaces: workspaces.clone(),
            agents: agents.clone(),
            pages: pages.clone(),
            databases: databases.clone(),
            sync: sync.clone(),
            comments: comments.clone(),
        },
        transport,
    )
    .envelope(envelope)
    .limiter(Arc::clone(&limiter))
    .clock(clock)
    .build();
    let mcp = crate::mcp::Mcp::new(
        crate::mcp::Services {
            settings: Arc::clone(&settings),
            auth: auth.clone(),
            workspaces: workspaces.clone(),
            pages: pages.clone(),
            sync: sync.clone(),
            history: history.clone(),
            search: search.clone(),
            databases: databases.clone(),
            comments: comments.clone(),
            inbox: inbox.clone(),
            broadcaster,
        },
        limiter,
        mcp_feed,
    );
    AppState {
        settings,
        auth,
        workspaces,
        agents,
        oauth,
        pages,
        sync,
        history,
        search,
        databases,
        live,
        comments,
        inbox,
        inbox_feeds,
        transfer,
        attachments,
        webhooks,
        web: Arc::new(web),
        mcp,
    }
}

#[derive(Deserialize)]
struct Item {
    #[expect(dead_code, reason = "only parsed, to make malformed bodies fail")]
    name: String,
}

async fn echo(Json(item): Json<Value>) -> Json<Value> {
    Json(item)
}

async fn panics() -> StatusCode {
    panic!("handler bug")
}

/// Which of the request's credential headers arrive marked sensitive, answered with a
/// cookie for the response side.
async fn sensitive_headers(headers: header::HeaderMap) -> impl IntoResponse {
    let sensitive = |name| headers.get(name).map(header::HeaderValue::is_sensitive);
    (
        [(header::SET_COOKIE, "session=hunter2")],
        Json(json!({
            "authorization": sensitive(header::AUTHORIZATION),
            "cookie": sensitive(header::COOKIE),
            "accept": sensitive(header::ACCEPT),
        })),
    )
}

/// The API plus routes that exercise the extractors and the middleware.
pub(super) fn test_app(state: AppState, limits: Limits) -> Router {
    let routes = OpenApiRouter::new()
        .route("/api/v1/test/echo", post(echo))
        .route(
            "/api/v1/test/item",
            post(|Json(_): Json<Item>| async { StatusCode::NO_CONTENT }),
        )
        .route(
            "/api/v1/test/items/{id}",
            get(|Path(id): Path<u32>| async move { id.to_string() }),
        )
        .route(
            "/api/v1/test/search",
            get(
                |Query(q): Query<std::collections::HashMap<String, u32>>| async move {
                    q.len().to_string()
                },
            ),
        )
        .route("/api/v1/test/panic", get(panics))
        .route("/api/v1/test/headers", get(sensitive_headers))
        .route(
            "/api/v1/test/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                "late"
            }),
        );
    let web = Arc::clone(&state.web);
    app(api().merge(routes), limits, HttpLayer::new(), web).with_state(state)
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.expect("a response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or_else(|_| {
        panic!("not JSON: {}", String::from_utf8_lossy(&body));
    });
    (status, json)
}

fn get_request(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).expect("a request")
}

fn json_request(uri: &str, content_type: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut request = Request::builder().method(Method::POST).uri(uri);
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    request.body(body.into()).expect("a request")
}

fn error_code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_else(|| {
        panic!("not the error shape: {body}");
    })
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn meta_names_the_instance_and_the_version(pool: PgPool) {
    let state = state(pool).await;
    let app = router(state.clone(), false);

    let (status, body) = send(app.clone(), get_request("/api/v1/meta")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "product": "neon-teamspace",
            "name": "Neon Teamspace",
            "version": env!("CARGO_PKG_VERSION"),
        })
    );

    state
        .settings
        .update(
            [("instance.name".to_owned(), Change::Set(json!("Acme")))],
            None,
        )
        .await
        .expect("saved");
    let (_, body) = send(app, get_request("/api/v1/meta")).await;
    assert_eq!(body["name"], "Acme");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_description_is_served(pool: PgPool) {
    let app = router(state(pool).await, false);

    let response = app
        .oneshot(get_request("/api/v1/openapi.json"))
        .await
        .expect("a response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    assert_eq!(body, spec_json().as_bytes());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn unknown_routes_and_methods_answer_in_the_error_shape(pool: PgPool) {
    let app = router(state(pool).await, false);

    let (status, body) = send(app.clone(), get_request("/api/v1/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json!({"error": {"code": "not_found", "message": "no such route"}})
    );

    let (status, body) = send(app.clone(), get_request("/elsewhere")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "not_found");

    let (status, body) = send(app, json_request("/api/v1/meta", None, "")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(error_code(&body), "method_not_allowed");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn bad_bodies_answer_in_the_error_shape(pool: PgPool) {
    let limits = Limits {
        body: 64,
        ..Limits::default()
    };
    let app = test_app(state(pool).await, limits);
    let json = Some("application/json");

    let (status, body) = send(app.clone(), json_request("/api/v1/test/echo", json, "{")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_json");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("EOF")),
        "{body}"
    );

    let (status, body) = send(
        app.clone(),
        json_request("/api/v1/test/item", json, r#"{"name": 1}"#),
    )
    .await;
    // JSON that is not what the route takes is input it refuses, naming the field.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_input");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("the body does not fit the route: name")),
        "{body}"
    );

    let (status, body) = send(app.clone(), json_request("/api/v1/test/echo", None, "{}")).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(error_code(&body), "unsupported_media_type");

    let large = format!(r#"{{"name": "{}"}}"#, "x".repeat(100));
    let (status, body) = send(app.clone(), json_request("/api/v1/test/echo", json, large)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code(&body), "payload_too_large");

    let (status, body) = send(app, json_request("/api/v1/test/echo", json, r#"{"a": 1}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"a": 1}));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn bad_paths_and_queries_answer_in_the_error_shape(pool: PgPool) {
    let app = test_app(state(pool).await, Limits::default());

    let (status, body) = send(app.clone(), get_request("/api/v1/test/items/abc")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_path");

    let (status, body) = send(app.clone(), get_request("/api/v1/test/search?limit=many")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&body), "invalid_query");

    let (status, body) = send(app, get_request("/api/v1/test/items/7")).await;
    assert_eq!((status, body), (StatusCode::OK, json!(7)));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_panicking_handler_answers_500_in_the_error_shape(pool: PgPool) {
    let app = test_app(state(pool).await, Limits::default());

    let (status, body) = send(app, get_request("/api/v1/test/panic")).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error": {"code": "internal", "message": "internal error"}})
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn credential_headers_are_marked_sensitive(pool: PgPool) {
    let app = test_app(state(pool).await, Limits::default());
    let request = Request::get("/api/v1/test/headers")
        .header(header::AUTHORIZATION, "Bearer hunter2")
        .header(header::COOKIE, "session=hunter2")
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .expect("a request");

    let response = app.oneshot(request).await.expect("a response");

    assert!(response.headers()[header::SET_COOKIE].is_sensitive());
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    let seen: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(
        seen,
        json!({"authorization": true, "cookie": true, "accept": false})
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_slow_handler_times_out_in_the_error_shape(pool: PgPool) {
    let limits = Limits {
        timeout: Duration::from_millis(50),
        ..Limits::default()
    };
    let app = test_app(state(pool).await, limits);

    let (status, body) = send(app, get_request("/api/v1/test/slow")).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body,
        json!({"error": {
            "code": "timeout",
            "message": "the server did not answer within 50ms"
        }})
    );
}

/// `neon_http_operations_total` by operation and status. A snapshot resets the counters.
fn request_counts(snapshotter: &Snapshotter) -> HashMap<(String, String), u64> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let (MetricKind::Counter, DebugValue::Counter(count)) = (kind, value) else {
                return None;
            };
            if key.name() != "neon_http_operations_total" {
                return None;
            }
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            Some(((label("operation")?, label("status")?), count))
        })
        .collect()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn red_metrics_count_the_status_the_client_gets(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    // Tests run on a single-threaded runtime, so the recorder sees every request.
    let _recording = metrics::set_default_local_recorder(&recorder);
    let limits = Limits {
        timeout: Duration::from_millis(50),
        ..Limits::default()
    };
    let app = test_app(state(pool).await, limits);

    for uri in [
        "/api/v1/meta",
        "/api/v1/test/items/7",
        "/api/v1/test/items/abc",
        "/api/v1/test/panic",
        "/api/v1/test/slow",
        "/api/v1/nope",
    ] {
        let response = app
            .clone()
            .oneshot(get_request(uri))
            .await
            .expect("a response");
        assert!(response.headers().contains_key(&X_REQUEST_ID), "{uri}");
    }

    let counts = request_counts(&snapshotter);
    let expected = [
        ("GET /api/v1/meta", "ok", 1),
        ("GET /api/v1/test/items/{id}", "ok", 2),
        ("GET /api/v1/test/panic", "error", 1),
        ("GET /api/v1/test/slow", "error", 1),
        ("GET unmatched", "ok", 1),
    ]
    .map(|(operation, status, count)| ((operation.to_owned(), status.to_owned()), count));
    assert_eq!(counts, HashMap::from(expected));
}
