//! The OAuth use cases' RED metrics and spans, and the rule that no code, token or verifier
//! reaches a log line, a span or an error message.

mod support;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::crypto::pkce_challenge;
use neon_app::oauth::{Approval, OAuthService, TokenRequest};
use neon_domain::auth::Principal;
use neon_domain::oauth::{AuthorizationRequest, ClientMetadata, Endpoints, Identity};
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use sqlx::PgPool;
use support::{Harness, IP};
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "http://127.0.0.1:43110/callback";

fn endpoints() -> Endpoints {
    Endpoints::new("https://neon.example.com")
}

/// The OAuth service of a harness, on its pool, settings, clock, limiter and listener.
fn oauth(h: &Harness) -> OAuthService {
    OAuthService::builder(h.pool.clone(), Arc::clone(&h.settings))
        .clock(h.time.clock())
        .limiter(h.limiter.clone())
        .access_changes(h.changes.clone())
        .build()
}

/// A PKCE verifier, low in entropy: tests never hold secrets that look real.
fn verifier() -> String {
    "q".repeat(50)
}

/// A client registered, and a request of it, with the challenge of [`verifier`].
async fn client(oauth: &OAuthService) -> (Uuid, AuthorizationRequest) {
    let client = oauth
        .register(
            &ClientMetadata {
                client_name: Some("Claude Code".into()),
                redirect_uris: Some(vec![REDIRECT.into()]),
                ..ClientMetadata::default()
            },
            IP,
        )
        .await
        .expect("registers");
    let request = AuthorizationRequest {
        response_type: Some("code".into()),
        client_id: Some(client.id.to_string()),
        redirect_uri: Some(REDIRECT.into()),
        code_challenge: Some(pkce_challenge(&verifier())),
        code_challenge_method: Some("S256".into()),
        scope: None,
        state: Some("opaque".into()),
        resource: None,
    };
    (client.id, request)
}

/// The code an approval sends the browser back with.
async fn approve(
    oauth: &OAuthService,
    person: &Principal,
    request: &AuthorizationRequest,
    workspace_id: Uuid,
    identity: Identity,
) -> String {
    let back = oauth
        .approve(
            person,
            request,
            &Approval {
                workspace_id,
                identity,
                scopes: vec!["read".into(), "write".into()],
            },
            &endpoints(),
        )
        .await
        .expect("approved");
    let url = Url::parse(&back).expect("an address");
    url.query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, code)| code.into_owned())
        .expect("a code")
}

fn exchange(client: Uuid, code: &str) -> TokenRequest {
    TokenRequest {
        grant_type: Some("authorization_code".into()),
        client_id: Some(client.to_string()),
        code: Some(code.to_owned()),
        redirect_uri: Some(REDIRECT.into()),
        code_verifier: Some(verifier()),
        ..TokenRequest::default()
    }
}

fn refresh(client: Uuid, token: &str) -> TokenRequest {
    TokenRequest {
        grant_type: Some("refresh_token".into()),
        client_id: Some(client.to_string()),
        refresh_token: Some(token.to_owned()),
        ..TokenRequest::default()
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

    fn spans(&self) -> Vec<SpanData> {
        self.provider.force_flush().expect("flushes");
        self.exporter.get_finished_spans().expect("spans")
    }

    /// Every log line, and every span's name, attributes and events.
    fn everything(&self) -> String {
        let mut text =
            String::from_utf8_lossy(&self.logs.0.lock().expect("capture lock")).into_owned();
        for span in self.spans() {
            text.push_str(&format!(
                "{:?} {:?} {:?}\n",
                span.name, span.attributes, span.events
            ));
        }
        text
    }
}

/// The OAuth operations counted since the last snapshot, by operation and status. A
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
            && key.name() == "neon_oauth_operations_total"
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

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refusals_count_as_ok_and_no_code_token_or_verifier_is_written(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let recorded = Recorded::install();
    let h = Harness::new(pool).await;
    let oauth = oauth(&h);
    let ada = h.person("ada@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let (client, request) = client(&oauth).await;
    let code = approve(&oauth, &ada, &request, design, Identity::Person).await;
    let tokens = oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect("issued");
    h.auth
        .authenticate_token(tokens.access_token.expose())
        .await
        .expect("works");
    let rotated = oauth
        .token(
            &refresh(client, tokens.refresh_token.expose()),
            &endpoints(),
        )
        .await
        .expect("rotated");
    let reused = oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect_err("used already");
    let mut wrong = exchange(client, &code);
    wrong.code_verifier = Some("w".repeat(50));
    let wrong = oauth
        .token(&wrong, &endpoints())
        .await
        .expect_err("refused");

    let span = recorded
        .spans()
        .into_iter()
        .rev()
        .find(|span| span.name == "oauth token")
        .expect("a token span");
    assert_eq!(span.status, Status::Unset, "a refusal fails no span");
    let counted = counts(&snapshotter);
    assert_eq!(of(&counted, "token", "ok"), 4);
    assert_eq!(of(&counted, "token", "error"), 0);
    assert_eq!(of(&counted, "approve", "ok"), 1);
    assert_eq!(of(&counted, "register", "ok"), 1);

    let mut seen = recorded.everything();
    for error in [&reused, &wrong] {
        seen.push_str(&format!("{error} {error:?}\n"));
    }
    seen.push_str(&format!("{tokens:?} {rotated:?}\n"));
    assert!(seen.contains("oauth approve"), "the capture has the spans");
    assert!(
        seen.contains("OAuth tokens issued"),
        "the capture has the logs"
    );
    for secret in [
        code.as_str(),
        verifier().as_str(),
        tokens.access_token.expose(),
        tokens.refresh_token.expose(),
        rotated.access_token.expose(),
        rotated.refresh_token.expose(),
    ] {
        assert!(!seen.contains(secret), "{secret:?} was written somewhere");
    }

    // A failure of the server counts as one.
    h.pool.close().await;
    oauth
        .token(&exchange(client, &code), &endpoints())
        .await
        .expect_err("the pool is closed");
    assert_eq!(of(&counts(&snapshotter), "token", "error"), 1);
}
