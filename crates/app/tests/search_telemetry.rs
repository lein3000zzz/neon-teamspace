//! The search and history use cases' RED metrics and spans, and the rule that no query, but
//! for its length, and nothing a page holds reaches a log line or a span: searching, a
//! refused search, and a version listed, compared and restored, all traced and logged at
//! every level.

mod support;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use metrics::{SharedString, Unit};
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use metrics_util::{CompositeKey, MetricKind};
use neon_app::history::HistoryService;
use neon_app::pages::{ImportedContent, Location, NewPage};
use neon_app::search::{SearchInput, SearchService};
use neon_app::sync::{SyncService, access_feed};
use neon_blocks::markdown::from_markdown;
use neon_domain::pages::Placement;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use sqlx::PgPool;
use support::Harness;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;

/// What the page holds, and what is searched for: neither may be logged.
const CONTENT: &str = "A quietly kept sentence";
const QUERY: &str = "quietly kept";

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

/// What a snapshot of the metrics holds.
type Metrics = Vec<(CompositeKey, Option<Unit>, Option<SharedString>, DebugValue)>;

/// The operations of `subsystem` counted, by operation and status.
fn counts(metrics: &Metrics, subsystem: &str) -> HashMap<(String, String), u64> {
    let name = format!("neon_{subsystem}_operations_total");
    let mut counts = HashMap::new();
    for (key, _, _, value) in metrics {
        let (kind, key) = key.clone().into_parts();
        let label = |name: &str| {
            key.labels()
                .find(|label| label.key() == name)
                .map(|label| label.value().to_owned())
                .unwrap_or_default()
        };
        if let (MetricKind::Counter, DebugValue::Counter(n)) = (kind, value)
            && key.name() == name
        {
            *counts
                .entry((label("operation"), label("status")))
                .or_default() += *n;
        }
    }
    counts
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn search_and_history_are_measured_and_traced_without_what_they_read(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
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
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let ws = h.workspace(&ada, "Design").await.workspace.id;
    let general = h
        .workspaces
        .list_teamspaces(&ada, ws)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let page = h
        .pages
        .create_page(
            &ada,
            ws,
            Location::Teamspace(general),
            Placement::Last,
            NewPage {
                title: "Plan".to_owned(),
                content: ImportedContent::of(&from_markdown(CONTENT)),
                ..NewPage::default()
            },
        )
        .await
        .expect("created")
        .page
        .id;
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed).build();
    let history = HistoryService::builder(
        h.pool.clone(),
        Arc::clone(&h.settings),
        h.pages.clone(),
        sync.clone(),
    )
    .build();
    let search = SearchService::builder(h.pool.clone()).build();

    let found = search
        .search(
            &ada,
            ws,
            SearchInput {
                query: QUERY.to_owned(),
                ..SearchInput::default()
            },
        )
        .await
        .expect("searched");
    assert_eq!(found.hits.len(), 1);
    search
        .search(
            &ada,
            ws,
            SearchInput {
                query: "-quietly".to_owned(),
                ..SearchInput::default()
            },
        )
        .await
        .expect_err("refused");
    let version = history
        .versions(&ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let compared = history
        .compare(&ada, page, version)
        .await
        .expect("compared");
    history
        .restore(&ada, page, version, compared.revision)
        .await
        .expect("restored");
    sync.shutdown().await;

    let metrics: Metrics = snapshotter.snapshot().into_vec();
    let searches = counts(&metrics, "search");
    assert_eq!(searches.get(&("search".into(), "ok".into())), Some(&2));
    let histories = counts(&metrics, "history");
    for operation in ["list", "compare", "restore"] {
        assert_eq!(
            histories.get(&(operation.into(), "ok".into())),
            Some(&1),
            "{operation}"
        );
    }

    provider.force_flush().expect("flushes");
    let mut everything =
        String::from_utf8_lossy(&logs.0.lock().expect("capture lock")).into_owned();
    let spans = exporter.get_finished_spans().expect("spans");
    for span in &spans {
        everything.push_str(&format!(
            "{:?} {:?} {:?}\n",
            span.name, span.attributes, span.events
        ));
    }
    for name in ["search search", "history compare", "history restore"] {
        assert!(spans.iter().any(|span| span.name == name), "no span {name}");
    }
    assert!(everything.contains("search.query_chars"), "{everything}");
    for secret in ["quietly", "kept sentence"] {
        assert!(!everything.contains(secret), "{secret} was logged");
    }
}
