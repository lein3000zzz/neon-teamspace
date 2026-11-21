//! Test helpers: an in-memory span exporter and a log capture writer.

use std::io;
use std::sync::{Arc, Mutex};

use opentelemetry::Value;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer};

use crate::{ServiceInfo, traces};

/// A tracer provider that keeps finished spans in memory.
pub(crate) struct Traces {
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
}

impl Traces {
    /// Records every span.
    pub(crate) fn new() -> Self {
        Self::build(SdkTracerProvider::builder())
    }

    fn build(builder: opentelemetry_sdk::trace::TracerProviderBuilder) -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = builder.with_simple_exporter(exporter.clone()).build();
        Self { provider, exporter }
    }

    /// The crate's own OpenTelemetry layer, wired to this provider, next to an `info` log
    /// filter.
    pub(crate) fn layer<S>(&self) -> impl Layer<S> + Send + Sync + 'static + use<S>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        self.layer_following(&EnvFilter::new("info"))
    }

    /// The crate's own OpenTelemetry layer next to the log filter `log_filter`.
    pub(crate) fn layer_following<S>(
        &self,
        log_filter: &EnvFilter,
    ) -> impl Layer<S> + Send + Sync + 'static + use<S>
    where
        S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        traces::layer(
            &self.provider,
            &ServiceInfo::new("test", "0.0.0"),
            log_filter,
        )
    }

    /// Runs `f` with a thread-local subscriber that only has the OpenTelemetry layer.
    pub(crate) fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        let subscriber = tracing_subscriber::registry().with(self.layer());
        tracing::subscriber::with_default(subscriber, f)
    }

    pub(crate) fn spans(&self) -> Vec<SpanData> {
        self.provider.force_flush().unwrap();
        self.exporter.get_finished_spans().unwrap()
    }

    pub(crate) fn only_span(&self) -> SpanData {
        let mut spans = self.spans();
        assert_eq!(spans.len(), 1, "expected one span, got {spans:?}");
        spans.remove(0)
    }

    pub(crate) fn attribute(span: &SpanData, key: &str) -> Option<Value> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.clone())
    }
}

/// A `MakeWriter` that keeps everything written to it.
#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    /// Every line written so far, parsed as JSON.
    pub(crate) fn json_lines(&self) -> Vec<serde_json::Value> {
        self.text()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
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
