//! The updates use cases' RED metrics, spans and log lines: an update asked of the updater is
//! logged with the admin who asked and the release, whether or not its answer arrived, a
//! refusal counts as part of normal operation, and the updater's token never reaches a log
//! line, a span or an error message.

mod support;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::updates::{
    Releases, ReleasesError, UpdateService, Updater, UpdaterEndpoint, UpdaterError, UpdatesError,
};
use neon_domain::updates::{Release, Running, UpdaterState, UpdaterStatus};
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use semver::Version;
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, Options};
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use url::Url;

/// A token the setting takes, obviously fake.
const TOKEN: &str = "fake-updater-token-for-the-telemetry-tests";

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

/// The operations counted since the last snapshot, by operation and status. A snapshot
/// resets the counters, so each test reads them once.
fn counts(snapshotter: &Snapshotter) -> HashMap<(String, String), u64> {
    let mut counts = HashMap::new();
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        let (kind, key) = key.into_parts();
        let label = |name: &str| {
            key.labels()
                .find(|label| label.key() == name)
                .map(|label| label.value().to_owned())
        };
        if let (MetricKind::Counter, DebugValue::Counter(n)) = (kind, value)
            && key.name() == "neon_updates_operations_total"
            && let (Some(operation), Some(status)) = (label("operation"), label("status"))
        {
            *counts.entry((operation, status)).or_default() += n;
        }
    }
    counts
}

/// GitHub, listing 0.2.0 and 0.1.0.
struct TwoReleases;

#[async_trait]
impl Releases for TwoReleases {
    async fn list(&self) -> Result<Vec<Release>, ReleasesError> {
        Ok(["0.2.0", "0.1.0"]
            .into_iter()
            .map(|version| Release {
                version: Version::parse(version).expect("a version"),
                prerelease: false,
                draft: false,
                url: Url::parse(&format!(
                    "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v{version}"
                ))
                .expect("a URL"),
                published_at: None,
            })
            .collect())
    }
}

/// The updater, answering the status and each update as the test says; while the update's
/// answer is unset, it never answers one.
struct ScriptedUpdater {
    status: Mutex<Result<UpdaterStatus, UpdaterError>>,
    update: Mutex<Option<Result<UpdaterStatus, UpdaterError>>>,
}

impl ScriptedUpdater {
    fn answering(update: Result<UpdaterStatus, UpdaterError>) -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(Ok(status(UpdaterState::Idle))),
            update: Mutex::new(Some(update)),
        })
    }
}

#[async_trait]
impl Updater for ScriptedUpdater {
    async fn status(&self, _at: &UpdaterEndpoint) -> Result<UpdaterStatus, UpdaterError> {
        self.status.lock().expect("unpoisoned").clone()
    }

    async fn update(
        &self,
        _at: &UpdaterEndpoint,
        _version: &Version,
    ) -> Result<UpdaterStatus, UpdaterError> {
        let answer = self.update.lock().expect("unpoisoned").clone();
        match answer {
            Some(answer) => answer,
            None => std::future::pending().await,
        }
    }
}

fn status(state: UpdaterState) -> UpdaterStatus {
    UpdaterStatus {
        state,
        version: (state != UpdaterState::Idle).then(|| "0.2.0".to_owned()),
        error: None,
        log: None,
        started_at: None,
        finished_at: None,
        updater_version: "0.2.0".to_owned(),
    }
}

/// A harness whose settings hold the updater's token.
async fn harness(pool: PgPool) -> Harness {
    Harness::with(
        pool,
        Options {
            settings: vec![("updates.updater_token", json!(TOKEN))],
            ..Options::default()
        },
    )
    .await
}

/// The updates of a release build of 0.1.0 on the harness's settings, whose release check
/// has run.
async fn updates(h: &Harness, updater: Arc<ScriptedUpdater>) -> UpdateService {
    let updates = UpdateService::builder(
        Arc::clone(&h.settings),
        Arc::new(TwoReleases),
        updater,
        Running {
            version: Version::parse("0.1.0").expect("a version"),
            development: false,
        },
    )
    .clock(h.time.clock())
    .build();
    updates.look().await;
    updates
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_update_is_logged_with_the_admin_and_the_release_and_refusals_count_as_ok(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let recorded = Recorded::install();
    let h = harness(pool).await;
    let admin = h.admin().await;
    let updates = updates(
        &h,
        ScriptedUpdater::answering(Ok(status(UpdaterState::Updating))),
    )
    .await;

    let refused = updates.apply(&admin, "0.1.0").await;
    assert!(
        matches!(refused, Err(UpdatesError::NotAllowed(_))),
        "{refused:?}"
    );
    let span = recorded.span("updates apply");
    assert_eq!(span.status, Status::Unset);
    assert_eq!(
        attribute(&span, "neon.error.expected").as_deref(),
        Some("true")
    );
    assert_eq!(
        attribute(&span, "neon.error.reason").as_deref(),
        Some("update_not_allowed")
    );
    assert!(
        !recorded
            .logs
            .text()
            .contains("the updater was asked to update the stack"),
        "a refused update is not logged as asked"
    );

    updates.apply(&admin, "0.2.0").await.expect("asked");
    let logs = recorded.logs.text();
    let line = logs
        .lines()
        .find(|line| line.contains("the updater was asked to update the stack to a release"))
        .unwrap_or_else(|| panic!("no line of the update in {logs}"));
    assert!(line.contains(" INFO "), "{line}");
    assert!(
        line.contains(&format!("actor_id={}", admin.user.id)),
        "{line}"
    );
    assert!(line.contains("version=0.2.0"), "{line}");
    assert!(line.contains("running=0.1.0"), "{line}");

    let counts = counts(&snapshotter);
    let ok = |operation: &str| {
        counts
            .get(&(operation.to_owned(), "ok".to_owned()))
            .copied()
    };
    assert_eq!(
        ok("apply"),
        Some(2),
        "the refusal is part of normal operation"
    );
    assert_eq!(ok("check"), Some(1), "the periodic check counts as check");
    assert!(
        counts.keys().all(|(_, status)| status == "ok"),
        "{counts:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_update_whose_answer_is_lost_is_logged_with_the_admin_as_maybe_under_way(pool: PgPool) {
    let recorded = Recorded::install();
    let h = harness(pool).await;
    let admin = h.admin().await;
    let garbled = "the updater at http://updater:8080/ answered something other than its status";
    let updater = ScriptedUpdater::answering(Err(UpdaterError::Unreachable(garbled.to_owned())));
    let updates = updates(&h, Arc::clone(&updater)).await;

    let answer = updates.apply(&admin, "0.2.0").await;
    assert_eq!(
        answer,
        Err(UpdatesError::UpdaterUnreachable(garbled.to_owned()))
    );

    // The updater never answers, and the paused clock skips the wait for it.
    *updater.update.lock().expect("unpoisoned") = None;
    tokio::time::pause();
    let answer = updates.apply(&admin, "0.2.0").await;
    tokio::time::resume();
    let silent = "the updater did not answer within 10s";
    assert_eq!(
        answer,
        Err(UpdatesError::UpdaterUnreachable(silent.to_owned()))
    );

    // Refusals mean nothing started, so they log nothing of the kind.
    *updater.update.lock().expect("unpoisoned") = Some(Err(UpdaterError::Busy));
    let busy = updates.apply(&admin, "0.2.0").await;
    assert_eq!(busy, Err(UpdatesError::UpdateInProgress));

    let logs = recorded.logs.text();
    let lines: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("the update may be under way"))
        .collect();
    assert_eq!(lines.len(), 2, "one line for each lost answer in {logs}");
    for (line, why) in lines.into_iter().zip([garbled, silent]) {
        assert!(line.contains(" WARN "), "{line}");
        assert!(
            line.contains(&format!("actor_id={}", admin.user.id)),
            "{line}"
        );
        assert!(line.contains("version=0.2.0"), "{line}");
        assert!(line.contains("running=0.1.0"), "{line}");
        assert!(line.contains(why), "{line}");
    }
    assert!(!logs.contains(TOKEN), "the token was written in {logs}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_updater_s_token_never_reaches_logs_spans_or_errors(pool: PgPool) {
    let recorded = Recorded::install();
    let h = harness(pool).await;
    let admin = h.admin().await;
    let updater = ScriptedUpdater::answering(Err(UpdaterError::Refused));
    *updater.status.lock().expect("unpoisoned") = Err(UpdaterError::Refused);
    let updates = updates(&h, Arc::clone(&updater)).await;

    let mut errors = Vec::new();
    let status = updates.status(&admin).await.expect("the status");
    errors.push(status.updater.error.clone().expect("refused"));
    errors.push(format!("{status:?}"));
    let refused = updates
        .apply(&admin, "0.2.0")
        .await
        .expect_err("the updater refused the token");
    errors.push(format!("{refused} {refused:?}"));
    *updater.update.lock().expect("unpoisoned") = Some(Err(UpdaterError::Busy));
    let busy = updates.apply(&admin, "0.2.0").await.expect_err("busy");
    errors.push(format!("{busy} {busy:?}"));
    updates.check_now(&admin).await.expect("checks");

    let mut seen = recorded.everything();
    seen.push_str(&errors.join("\n"));
    assert!(seen.contains("updates apply"), "the capture works");
    assert!(seen.contains("updates check_now"), "and has every use case");
    assert!(!seen.contains(TOKEN), "the token was written somewhere");
    assert!(
        !seen.contains("fake-updater-token"),
        "a part of the token was written somewhere"
    );
}
