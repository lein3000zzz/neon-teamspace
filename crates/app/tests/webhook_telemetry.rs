//! The webhook use cases' RED metrics, and the rule that no secret, URL token, receiver URL
//! or payload reaches a log line or a span: registering, rotating a secret, delivering to a
//! receiver that takes the delivery and one that fails it, and an incoming webhook taking a
//! signed request, all traced and logged at every level.

mod support;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::comments::CommentService;
use neon_app::databases::{NewDatabase, NewProperty, NewRow, PropertySpec};
use neon_app::jobs::{Worker, WorkerConfig};
use neon_app::pages::Location;
use neon_app::sync::{SyncService, access_feed};
use neon_app::webhooks::signature;
use neon_app::webhooks::transport::{Answer, Outgoing, SendError, WebhookTransport};
use neon_app::webhooks::{
    BodyError, DeliveryAttempt, DeliveryJob, IncomingRequest, NewIncomingWebhook, NewWebhook,
    Services, WebhookService,
};
use neon_domain::auth::Principal;
use neon_domain::databases::PropertyKind;
use neon_domain::jobs::Backoff;
use neon_domain::pages::Placement;
use neon_domain::webhooks::EventType;
use neon_domain::workspaces::WorkspaceRole;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, envelope};
use time::OffsetDateTime;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use url::Url;
use uuid::Uuid;

/// The receiver that takes its deliveries.
const TAKES: &str = "https://hooks.example.com/quiet-receiver-that-takes";
/// The receiver that answers each delivery with an error.
const FAILS: &str = "https://hooks.example.com/quiet-receiver-that-fails";
/// The title of the row the deliveries tell of, and what an incoming webhook is sent.
const TITLE: &str = "A row nobody logs";
const POSTED: &str = "A value posted quietly";

/// Answers each delivery to [`FAILS`] with 500 and the rest with 204.
struct Receivers;

#[async_trait]
impl WebhookTransport for Receivers {
    async fn check(&self, _: &Url, _: bool) -> Result<(), SendError> {
        Ok(())
    }

    async fn send(&self, request: Outgoing, _: bool) -> Result<Answer, SendError> {
        let fails = request.url.as_str() == FAILS;
        Ok(Answer {
            status: if fails { 500 } else { 204 },
            excerpt: fails.then(|| "the receiver is down".to_owned()),
        })
    }
}

/// Keeps everything written to it.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("capture lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Logs at every level with span fields, and spans exported to memory, for this thread.
struct Recorded {
    logs: Capture,
    exporter: InMemorySpanExporter,
    provider: SdkTracerProvider,
    _guard: tracing::subscriber::DefaultGuard,
}

impl Recorded {
    fn install() -> Self {
        let logs = Capture::default();
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(logs.clone())
                    .with_ansi(false)
                    .with_span_events(FmtSpan::FULL)
                    .with_filter(tracing_subscriber::filter::LevelFilter::TRACE),
            );
        let guard = tracing::subscriber::set_default(subscriber);
        Self {
            logs,
            exporter,
            provider,
            _guard: guard,
        }
    }

    /// Every log line, and every span's name, attributes and events.
    fn everything(&self) -> String {
        self.provider.force_flush().expect("flushes");
        let mut text =
            String::from_utf8_lossy(&self.logs.0.lock().expect("capture lock")).into_owned();
        for span in self.exporter.get_finished_spans().expect("spans") {
            text.push_str(&format!(
                "{:?} {:?} {:?}\n",
                span.name, span.attributes, span.events
            ));
        }
        text
    }
}

/// The webhook operations counted since the last snapshot, by operation and status. A
/// snapshot resets the counters.
fn counts(snapshotter: &Snapshotter) -> HashMap<(String, String), u64> {
    let mut counts = HashMap::new();
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        let (kind, key) = key.into_parts();
        let label = |name: &str| {
            key.labels()
                .find(|label| label.key() == name)
                .map(|label| label.value().to_owned())
                .unwrap_or_default()
        };
        if let (MetricKind::Counter, DebugValue::Counter(n)) = (kind, value)
            && key.name() == "neon_webhooks_operations_total"
        {
            *counts
                .entry((label("operation"), label("status")))
                .or_default() += n;
        }
    }
    counts
}

fn of(counts: &HashMap<(String, String), u64>, operation: &str, status: &str) -> u64 {
    counts
        .get(&(operation.to_owned(), status.to_owned()))
        .copied()
        .unwrap_or_default()
}

/// A database with an Assignee in Design, which Ada owns with Dee as a member.
async fn tasks(h: &Harness, ada: &Principal, workspace: Uuid) -> Uuid {
    let general = h
        .workspaces
        .list_teamspaces(ada, workspace)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let db = h
        .databases
        .create_database(
            ada,
            workspace,
            Location::Teamspace(general),
            Placement::Last,
            NewDatabase {
                title: "Tasks".to_owned(),
                icon: None,
                inline: false,
            },
        )
        .await
        .expect("created")
        .page
        .page
        .id;
    h.databases
        .add_property(
            ada,
            db,
            NewProperty {
                name: "Assignee".to_owned(),
                spec: PropertySpec {
                    kind: PropertyKind::Person,
                    number_format: None,
                    currency: None,
                    prefix: None,
                },
                options: None,
                placement: Placement::Last,
            },
        )
        .await
        .expect("added");
    db
}

/// The webhook's log once its first attempt is in it.
async fn first_attempt(
    webhooks: &WebhookService,
    by: &Principal,
    workspace: Uuid,
    hook: Uuid,
) -> DeliveryAttempt {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let log = webhooks
            .webhook_attempts(by, workspace, hook)
            .await
            .expect("the log");
        if let Some(attempt) = log.into_iter().find(|attempt| attempt.attempt == 1) {
            return attempt;
        }
        assert!(Instant::now() < deadline, "no attempt was logged");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn body(bytes: Vec<u8>) -> Result<Bytes, BodyError> {
    Ok(Bytes::from(bytes))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn webhooks_count_refusals_as_ok_and_write_no_secret_url_or_payload(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let recorded = Recorded::install();
    let h = Harness::new(pool.clone()).await;
    let ada = h.person("ada@example.com").await;
    let dee = h.person("dee@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, workspace, &dee, WorkspaceRole::Member)
        .await;
    let db = tasks(&h, &ada, workspace).await;
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed).build();
    let webhooks = WebhookService::builder(
        h.pool.clone(),
        Arc::clone(&h.settings),
        Services {
            workspaces: h.workspaces.clone(),
            agents: h.agents.clone(),
            pages: h.pages.clone(),
            databases: h.databases.clone(),
            comments: CommentService::new(h.pages.clone(), sync.clone()),
            sync,
        },
        Arc::new(Receivers),
    )
    .envelope(envelope())
    .build();

    // Dee registers a webhook to each receiver, and rotates the first one's secret.
    let mut secrets = Vec::new();
    let mut hooks = Vec::new();
    for url in [TAKES, FAILS] {
        let (secret, made) = webhooks
            .create_webhook(
                &dee,
                workspace,
                NewWebhook {
                    user_id: dee.user.id,
                    url: url.to_owned(),
                    events: vec![EventType::Assigned],
                    include_own: false,
                },
            )
            .await
            .expect("registered");
        secrets.push(secret.expose().to_owned());
        hooks.push(made.id);
    }
    let (rotated, _) = webhooks
        .rotate_webhook_secret(&dee, workspace, hooks[0])
        .await
        .expect("rotated");
    secrets.push(rotated.expose().to_owned());

    // Ada assigns her a row: both are told, one takes it and the other fails it.
    h.databases
        .create_row(
            &ada,
            db,
            NewRow {
                values: json!({"Name": TITLE, "Assignee": [dee.user.id.to_string()]})
                    .as_object()
                    .cloned()
                    .expect("an object"),
                ..NewRow::default()
            },
        )
        .await
        .expect("a row");
    let worker = Worker::new(
        pool.clone(),
        WorkerConfig {
            poll_interval: Duration::from_millis(20),
            // One attempt each: the retry waits far longer than the test.
            backoff: Backoff {
                base: Duration::from_secs(3600),
                max: Duration::from_secs(3600),
            },
            ..WorkerConfig::default()
        },
    )
    .handler(DeliveryJob::new(webhooks.clone()))
    .start()
    .await
    .expect("starts");
    let taken = first_attempt(&webhooks, &dee, workspace, hooks[0]).await;
    let failed = first_attempt(&webhooks, &dee, workspace, hooks[1]).await;
    worker.stop().await;
    assert_eq!(taken.outcome, "delivered");
    assert_eq!(failed.outcome, "retrying");
    assert_eq!(failed.status_code, Some(500));

    // An incoming webhook takes a signed request.
    let incoming = webhooks
        .create_incoming(
            &ada,
            workspace,
            NewIncomingWebhook {
                name: "Forms".to_owned(),
                target_id: db,
                signed: true,
            },
        )
        .await
        .expect("made");
    let token = incoming.token.expose().to_owned();
    let incoming_secret = incoming
        .signing_secret
        .as_ref()
        .expect("a secret")
        .expose()
        .to_owned();
    let posted = json!({"version": 1, "action": "create_row", "values": {"Name": POSTED}})
        .to_string()
        .into_bytes();
    let at = OffsetDateTime::now_utc().unix_timestamp();
    let signed = signature::header(&[incoming_secret.as_bytes()], at, &posted);
    let at = at.to_string();
    let accepted = webhooks
        .receive_incoming(
            IncomingRequest {
                token: &token,
                client_ip: "203.0.113.7",
                json: true,
                signature: Some(&signed),
                timestamp: Some(&at),
            },
            body(posted),
        )
        .await
        .expect("taken");
    assert_eq!(accepted.action, "create_row");
    // A request with another token is refused, and counts as ok too.
    let unknown = webhooks
        .receive_incoming(
            IncomingRequest {
                token: "nwh_unknown",
                client_ip: "203.0.113.7",
                json: true,
                signature: None,
                timestamp: None,
            },
            body(b"{}".to_vec()),
        )
        .await
        .expect_err("no such webhook");
    assert_eq!(unknown.code(), "not_found");

    let counted = counts(&snapshotter);
    assert_eq!(of(&counted, "create_webhook", "ok"), 2);
    assert_eq!(of(&counted, "rotate_webhook_secret", "ok"), 1);
    assert!(of(&counted, "deliver", "ok") >= 2, "{counted:?}");
    assert_eq!(of(&counted, "create_incoming", "ok"), 1);
    assert_eq!(of(&counted, "receive", "ok"), 2);
    assert!(
        counted.iter().all(|((_, status), _)| status == "ok"),
        "{counted:?}"
    );

    let seen = recorded.everything();
    assert!(
        seen.contains("webhooks deliver") && seen.contains("webhooks receive"),
        "the capture has the spans"
    );
    assert!(
        seen.contains("incoming webhook request"),
        "the capture has the logs"
    );
    for secret in secrets
        .iter()
        .map(String::as_str)
        .chain([token.as_str(), incoming_secret.as_str(), signed.as_str()])
        .chain([TAKES, FAILS, TITLE, POSTED, "the receiver is down"])
    {
        assert!(!seen.contains(secret), "{secret:?} was written somewhere");
    }

    // A failure of the server counts as one.
    h.pool.close().await;
    webhooks
        .rotate_webhook_secret(&dee, workspace, hooks[0])
        .await
        .expect_err("the pool is closed");
    assert_eq!(
        of(&counts(&snapshotter), "rotate_webhook_secret", "error"),
        1
    );
}
