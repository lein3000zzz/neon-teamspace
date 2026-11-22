//! The auth use cases' RED metrics and spans, and the rule that no password, token or code
//! reaches a log line, a span or an error message.

mod support;

use std::io;
use std::sync::{Arc, Mutex};

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::auth::{AuthError, InviteSpec, LoginInput, RegisterInput};
use neon_domain::auth::Role;
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, IP, Options, PASSWORD, secret};
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;

/// Keeps everything written to it.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("capture lock")).into_owned()
    }
}

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
/// Tests run on a single-threaded runtime, so this sees the background work too.
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

    fn span(&self, name: &str) -> SpanData {
        self.spans()
            .into_iter()
            .rev()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("no span {name}"))
    }

    /// Every log line, and every span's name, attributes and events.
    fn everything(&self) -> String {
        let mut text = self.logs.text();
        for span in self.spans() {
            text.push_str(&format!(
                "{:?} {:?} {:?}\n",
                span.name, span.attributes, span.events
            ));
        }
        text
    }
}

fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

fn count(snapshotter: &Snapshotter, operation: &str, status: &str) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            match (kind, value) {
                (MetricKind::Counter, DebugValue::Counter(n))
                    if key.name() == "neon_auth_operations_total"
                        && label("operation").as_deref() == Some(operation)
                        && label("status").as_deref() == Some(status) =>
                {
                    Some(n)
                }
                _ => None,
            }
        })
        .sum()
}

fn login(email: &str, password: &str) -> LoginInput {
    LoginInput {
        email: email.to_owned(),
        password: secret(password),
        ip: IP.to_owned(),
        user_agent: "tests".to_owned(),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn use_cases_count_expected_errors_as_ok_and_failures_as_errors(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let recorded = Recorded::install();
    let h = Harness::new(pool).await;
    h.account("ada@example.com", Role::User).await;

    h.auth
        .login(login("ada@example.com", PASSWORD))
        .await
        .expect("signs in");
    let wrong = h
        .auth
        .login(login("ada@example.com", "a wrong password"))
        .await;
    assert!(matches!(wrong, Err(AuthError::InvalidCredentials)));

    let span = recorded.span("auth login");
    assert_eq!(span.status, Status::Unset);
    assert_eq!(
        attribute(&span, "neon.error.expected").as_deref(),
        Some("true")
    );
    assert_eq!(
        attribute(&span, "neon.error.reason").as_deref(),
        Some("invalid_credentials")
    );

    h.pool.close().await;
    let failed = h.auth.authenticate_session("any-token").await;
    assert!(matches!(failed, Err(AuthError::Database(_))), "{failed:?}");
    let span = recorded.span("auth authenticate_session");
    assert!(
        matches!(span.status, Status::Error { .. }),
        "{:?}",
        span.status
    );

    let snapshot_ok = count(&snapshotter, "login", "ok");
    assert_eq!(
        snapshot_ok, 2,
        "a wrong password is part of normal operation"
    );
    // A snapshot resets the counters, so read the failure on a fresh one.
    h.auth
        .authenticate_session("any-token")
        .await
        .expect_err("the pool is closed");
    assert_eq!(count(&snapshotter, "authenticate_session", "error"), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn no_password_token_or_code_reaches_logs_spans_or_errors(pool: PgPool) {
    let recorded = Recorded::install();
    let h = Harness::with(
        pool,
        Options {
            settings: vec![
                ("auth.verify_email", json!(true)),
                ("auth.requests", json!(true)),
            ],
            ..Options::default()
        },
    )
    .await;
    let mut secrets = vec![PASSWORD.to_owned()];
    let admin = h.admin().await;

    // An invite, typed and generated, copied again.
    let typed = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                code: Some(secret("Hunter2-Team-Code")),
                ..InviteSpec::default()
            },
        )
        .await
        .expect("an invite");
    secrets.push(typed.code.expose().to_owned());
    let generated = h
        .auth
        .create_invite(&admin, InviteSpec::default())
        .await
        .expect("an invite");
    secrets.push(generated.code.expose().to_owned());
    h.auth
        .invite_code_of(&admin, generated.invite.id)
        .await
        .expect("copies");
    let taken = h
        .auth
        .create_invite(
            &admin,
            InviteSpec {
                code: Some(secret("hunter2-team-code")),
                ..InviteSpec::default()
            },
        )
        .await
        .expect_err("taken");

    // A sign-up with the invite, its verification link, a weak password.
    let weak_password = "hunter2 x";
    let weak = h
        .auth
        .register(RegisterInput {
            email: "ada@example.com".into(),
            password: secret(weak_password),
            display_name: None,
            invite_code: Some(typed.code.clone()),
            ip: IP.into(),
        })
        .await;
    assert!(matches!(weak, Err(AuthError::WeakPassword(_))), "{weak:?}");
    secrets.push(weak_password.to_owned());
    let registered = h
        .auth
        .register(RegisterInput {
            email: "grace@example.com".into(),
            password: secret(PASSWORD),
            display_name: None,
            invite_code: Some(generated.code.clone()),
            ip: IP.into(),
        })
        .await
        .expect("registers");
    assert!(registered.must_verify);
    let link = h.token_of("verification").await;
    secrets.push(link.expose().to_owned());
    h.auth.verify_email(&link, IP).await.expect("verifies");

    // Sessions, wrong passwords, unknown addresses, tokens.
    let wrong_password = "hunter2 is not it";
    secrets.push(wrong_password.to_owned());
    let wrong = h
        .auth
        .login(login("grace@example.com", wrong_password))
        .await
        .expect_err("wrong");
    let unknown = h
        .auth
        .login(login("nobody@example.com", wrong_password))
        .await
        .expect_err("unknown");
    let (grace, session) = h.sign_in("grace@example.com").await;
    secrets.push(session.expose().to_owned());
    let (api_token, _) = h
        .auth
        .create_api_token(&grace, "ci", None, &["read"])
        .await
        .expect("a token");
    secrets.push(api_token.expose().to_owned());
    h.auth
        .authenticate_token(api_token.expose())
        .await
        .expect("works");
    let forged = h
        .auth
        .authenticate_token("nw_hunter2forgedtoken")
        .await
        .expect_err("forged");

    // A changed password, and a reset.
    let changed = "hunter2 changed passphrase";
    secrets.push(changed.to_owned());
    h.auth
        .change_password(&grace, &secret(PASSWORD), &secret(changed))
        .await
        .expect("changes");
    h.auth
        .request_password_reset("grace@example.com", IP)
        .await
        .expect("answers");
    h.auth.settle().await;
    let reset = h.token_of("password_reset").await;
    secrets.push(reset.expose().to_owned());
    let reset_password = "hunter2 reset passphrase";
    secrets.push(reset_password.to_owned());
    h.auth
        .reset_password(&reset, &secret(reset_password), IP)
        .await
        .expect("resets");
    let used = h
        .auth
        .reset_password(&reset, &secret(reset_password), IP)
        .await
        .expect_err("used");

    // A request to join and its confirmation.
    h.auth
        .request_to_join("linus@example.com", "hello", IP)
        .await
        .expect("answers");
    h.auth.settle().await;
    let join = h.token_of("join_confirmation").await;
    secrets.push(join.expose().to_owned());
    h.auth.confirm_join(&join, IP).await.expect("confirms");
    h.auth.settle().await;

    let mut seen = recorded.everything();
    for error in [&taken, &wrong, &unknown, &forged, &used] {
        seen.push_str(&format!("{error} {error:?}\n"));
    }
    seen.push_str(&format!("{:?}\n", login("grace@example.com", PASSWORD)));
    assert!(seen.contains("auth login"), "the capture works");
    for secret in &secrets {
        assert!(
            !seen.contains(secret.as_str()),
            "{secret:?} was written somewhere"
        );
    }
    assert!(
        !seen.contains("hunter2"),
        "a secret's part was written somewhere"
    );
}
