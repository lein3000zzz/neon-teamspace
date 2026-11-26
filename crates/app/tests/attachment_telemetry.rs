//! The attachment use cases' RED metrics and spans, and the rule that no file's name, beyond
//! its extension, and none of its bytes reach a log line or a span: uploading, refusing an
//! upload, reading and cleaning up, all traced and logged at every level.

mod support;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::attachments::{AttachmentService, BlobStore, ObjectBlobs};
use neon_app::pages::{Location, NewPage};
use neon_app::sync::{SyncService, access_feed};
use neon_domain::attachments::FileKind;
use neon_domain::pages::Placement;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use sqlx::PgPool;
use support::Harness;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;

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

    fn make_writer(&'a self) -> Self {
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

    fn span_names(&self) -> Vec<String> {
        self.provider.force_flush().expect("flushes");
        self.exporter
            .get_finished_spans()
            .expect("spans")
            .into_iter()
            .map(|span| span.name.into_owned())
            .collect()
    }
}

/// The attachment operations counted since the last snapshot, by operation and status.
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
            && key.name() == "neon_attachments_operations_total"
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

fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(4u32.to_be_bytes());
    bytes.extend(4u32.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend(b"pixels of the confidential chart");
    bytes
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn attachments_count_refusals_as_ok_and_log_no_name_or_bytes(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let recorded = Recorded::install();
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    let general = h
        .workspaces
        .list_teamspaces(&ada, workspace)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let page = h
        .pages
        .create_page(
            &ada,
            workspace,
            Location::Teamspace(general),
            Placement::Last,
            NewPage::default(),
        )
        .await
        .expect("made")
        .page
        .id;
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed).build();
    let store: Arc<dyn BlobStore> = Arc::new(ObjectBlobs::new(Arc::new(
        object_store::memory::InMemory::new(),
    )));
    let attachments = AttachmentService::builder_with_store(
        h.pool.clone(),
        Arc::clone(&h.settings),
        h.pages.clone(),
        sync,
        store,
    )
    .build();
    snapshotter.snapshot();

    let upload = |name: &'static str, content: Vec<u8>, kind| {
        let attachments = attachments.clone();
        let ada = ada.clone();
        async move {
            let ticket = attachments.admit_upload(&ada, page, kind, None, 0).await?;
            let body = futures_util::stream::iter([Ok::<_, String>(Bytes::from(content))]);
            attachments.upload(ticket, name, body).await
        }
    };
    let file = upload("Quarterly salaries.png", png(), FileKind::Image)
        .await
        .expect("uploaded");
    let refused = upload(
        "Secret layoffs.html",
        b"<html><body>names of everyone leaving</body></html>".to_vec(),
        FileKind::Image,
    )
    .await
    .expect_err("not an image");
    assert_eq!(refused.code(), "unsupported_type");
    let readable = attachments.file(&ada, file.id).await.expect("read");
    attachments.read(&readable, None).await.expect("bytes");
    attachments.cleanup().await.expect("cleaned up");

    let counted = counts(&snapshotter);
    assert_eq!(of(&counted, "admit_upload", "ok"), 2);
    assert_eq!(of(&counted, "upload", "ok"), 2, "a refusal is ok");
    assert_eq!(of(&counted, "open", "ok"), 1);
    assert_eq!(of(&counted, "cleanup", "ok"), 1);
    assert!(
        counted.keys().all(|(_, status)| status == "ok"),
        "{counted:?}"
    );
    let names = recorded.span_names();
    for operation in ["admit_upload", "upload", "open", "cleanup"] {
        assert!(
            names.contains(&format!("attachments {operation}")),
            "{operation}: {names:?}"
        );
    }

    let everything = recorded.everything();
    assert!(everything.contains("file uploaded"), "{everything}");
    assert!(everything.contains("png"), "the extension is logged");
    for secret in [
        "Quarterly",
        "salaries",
        "Secret layoffs",
        "layoffs",
        "confidential chart",
        "names of everyone",
    ] {
        assert!(
            !everything.contains(secret),
            "{secret} reached the logs or spans"
        );
    }
}
