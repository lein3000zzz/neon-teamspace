//! Log output: JSON lines for machines or pretty text for people.
//!
//! JSON records carry `timestamp` (RFC 3339, UTC), `level`, `target`, `message`, the event's
//! `fields`, the current `span` with its fields and, inside a span, top-level `trace_id` and
//! `span_id`. Those two come from the active OpenTelemetry context, so they match the
//! exported trace and are present even when export is off.

use std::fmt;

use opentelemetry::trace::TraceContextExt as _;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer as _};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_log::NormalizeEvent as _;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{EnvFilter, ParseError};
use tracing_subscriber::fmt::format::{JsonFields, Writer};
use tracing_subscriber::fmt::time::{FormatTime as _, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormattedFields, MakeWriter};
use tracing_subscriber::registry::{LookupSpan, SpanRef};

use crate::config::{LogFormat, LoggingConfig};

/// The fmt layer for `config`, filtered by its level. The filter applies to this layer only,
/// so it never hides spans from the OpenTelemetry layer.
pub(crate) fn layer<S, W>(
    config: &LoggingConfig,
    writer: W,
    ansi: bool,
) -> Result<impl Layer<S> + Send + Sync + 'static + use<S, W>, ParseError>
where
    S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let filter = EnvFilter::builder().parse(&config.level)?;
    let layer = match config.format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .fmt_fields(JsonFields::new())
            .event_format(JsonFormat)
            .with_writer(writer)
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer()
            .pretty()
            .with_ansi(ansi)
            .with_writer(writer)
            .boxed(),
    };
    Ok(layer.with_filter(filter))
}

/// One JSON object per event, keys in a fixed order.
struct JsonFormat;

impl<S> FormatEvent<S, JsonFields> for JsonFormat
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, JsonFields>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        // Records bridged from the `log` crate keep their real target and level this way.
        let normalized = event.normalized_metadata();
        let metadata = normalized.as_ref().unwrap_or_else(|| event.metadata());

        let mut timestamp = String::new();
        SystemTime.format_time(&mut Writer::new(&mut timestamp))?;

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let mut line = Vec::with_capacity(256);
        let mut serializer = serde_json::Serializer::new(&mut line);
        let mut record = serializer.serialize_map(None).map_err(|_| fmt::Error)?;
        entry(&mut record, "timestamp", &timestamp)?;
        entry(&mut record, "level", metadata.level().as_str())?;
        entry(&mut record, "target", metadata.target())?;
        if let Some(message) = &visitor.message {
            entry(&mut record, "message", message)?;
        }
        if let Some((trace_id, span_id)) = correlation() {
            entry(&mut record, "trace_id", &trace_id)?;
            entry(&mut record, "span_id", &span_id)?;
        }
        if !visitor.fields.is_empty() {
            entry(&mut record, "fields", &visitor.fields)?;
        }
        if let Some(span) = ctx.parent_span() {
            entry(&mut record, "span", &span_object(&span))?;
        }
        record.end().map_err(|_| fmt::Error)?;

        writer.write_str(std::str::from_utf8(&line).map_err(|_| fmt::Error)?)?;
        writer.write_char('\n')
    }
}

fn entry<M, V>(record: &mut M, key: &str, value: &V) -> fmt::Result
where
    M: SerializeMap,
    V: Serialize + ?Sized,
{
    record.serialize_entry(key, value).map_err(|_| fmt::Error)
}

/// The ids of the active OpenTelemetry span, which the OpenTelemetry layer attaches whenever
/// a span is entered.
fn correlation() -> Option<(String, String)> {
    let cx = opentelemetry::Context::current();
    let span = cx.span();
    let span_context = span.span_context();
    span_context.is_valid().then(|| {
        (
            span_context.trace_id().to_string(),
            span_context.span_id().to_string(),
        )
    })
}

/// The span's name followed by the fields the fmt layer stored for it as JSON.
fn span_object<S>(span: &SpanRef<'_, S>) -> Map<String, Value>
where
    S: for<'span> LookupSpan<'span>,
{
    let mut object = Map::new();
    object.insert("name".to_owned(), Value::from(span.name()));
    let extensions = span.extensions();
    if let Some(fields) = extensions.get::<FormattedFields<JsonFields>>()
        && let Ok(Value::Object(fields)) = serde_json::from_str(fields)
    {
        object.extend(fields);
    }
    object
}

/// Collects event fields as JSON values, the message apart.
#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &Field, value: Value) {
        let name = field.name();
        if name == "message" {
            self.message = Some(match value {
                Value::String(message) => message,
                other => other.to_string(),
            });
        } else if !name.starts_with("log.") {
            // `log.*` fields are the bridged record's metadata, already normalized above.
            self.fields.insert(name.to_owned(), value);
        }
    }
}

impl Visit for FieldVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, Value::from(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, Value::from(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.insert(field, Value::from(crate::errors::chain(value)));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field, Value::from(format!("{value:?}")));
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;
    use crate::testing::{Capture, Traces};

    fn config(level: &str, format: LogFormat) -> LoggingConfig {
        LoggingConfig {
            level: level.to_owned(),
            format,
        }
    }

    fn capture_json(level: &str, f: impl FnOnce()) -> Vec<Value> {
        let capture = Capture::default();
        let traces = Traces::new();
        let subscriber = tracing_subscriber::registry()
            .with(traces.layer())
            .with(layer(&config(level, LogFormat::Json), capture.clone(), false).unwrap());
        tracing::subscriber::with_default(subscriber, f);
        capture.json_lines()
    }

    fn is_hex(value: &Value, len: usize) -> bool {
        value
            .as_str()
            .is_some_and(|s| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit()))
    }

    #[test]
    fn lines_inside_a_span_carry_trace_and_span_ids() {
        let lines = capture_json("info", || {
            let span = tracing::info_span!("request", route = "/api/v1/meta");
            let _entered = span.enter();
            tracing::info!(status = 200_u64, "handled");
        });

        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(is_hex(&line["trace_id"], 32), "{line}");
        assert!(is_hex(&line["span_id"], 16), "{line}");
        assert_eq!(line["message"], "handled");
        assert_eq!(line["level"], "INFO");
        assert_eq!(line["fields"]["status"], 200);
        assert_eq!(line["span"]["name"], "request");
        assert_eq!(line["span"]["route"], "/api/v1/meta");
    }

    #[test]
    fn ids_match_the_exported_span() {
        let capture = Capture::default();
        let traces = Traces::new();
        let subscriber = tracing_subscriber::registry()
            .with(traces.layer())
            .with(layer(&config("info", LogFormat::Json), capture.clone(), false).unwrap());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info_span!("job").in_scope(|| tracing::info!("running"));
        });

        let line = &capture.json_lines()[0];
        let span = traces.only_span();
        assert_eq!(line["trace_id"], span.span_context.trace_id().to_string());
        assert_eq!(line["span_id"], span.span_context.span_id().to_string());
    }

    #[test]
    fn lines_outside_a_span_have_no_ids() {
        let lines = capture_json("info", || tracing::warn!(attempt = 2_i64, "retrying"));

        let line = &lines[0];
        assert!(line.get("trace_id").is_none());
        assert!(line.get("span_id").is_none());
        assert!(line.get("span").is_none());
        assert_eq!(line["level"], "WARN");
        assert_eq!(line["fields"]["attempt"], 2);
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        let lines = capture_json("info", || tracing::info!("tick"));

        let timestamp = lines[0]["timestamp"].as_str().unwrap();
        assert_eq!(timestamp.len(), "2026-01-01T00:00:00.000000Z".len());
        assert_eq!(&timestamp[4..5], "-");
        assert_eq!(&timestamp[10..11], "T");
        assert!(timestamp.ends_with('Z'));
    }

    #[test]
    fn level_filters_logs_but_not_spans() {
        let capture = Capture::default();
        let traces = Traces::new();
        let subscriber = tracing_subscriber::registry()
            .with(traces.layer())
            .with(layer(&config("warn", LogFormat::Json), capture.clone(), false).unwrap());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info_span!("request").in_scope(|| {
                tracing::info!("hidden");
                tracing::warn!("shown");
            });
        });

        let lines = capture.json_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["message"], "shown");
        assert!(is_hex(&lines[0]["trace_id"], 32));
        assert_eq!(traces.only_span().name, "request");
    }

    #[test]
    fn errors_are_written_with_their_sources() {
        #[derive(Debug, thiserror::Error)]
        #[error("query failed")]
        struct QueryFailed(#[source] std::io::Error);

        let error = QueryFailed(std::io::Error::other("connection reset"));
        let lines = capture_json("info", || {
            tracing::error!(error = &error as &dyn std::error::Error, "cannot load");
        });

        assert_eq!(
            lines[0]["fields"]["error"],
            "query failed: connection reset"
        );
    }

    #[test]
    fn pretty_format_writes_text() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry()
            .with(layer(&config("info", LogFormat::Pretty), capture.clone(), false).unwrap());
        tracing::subscriber::with_default(subscriber, || tracing::info!("readable"));

        assert!(capture.text().contains("readable"));
    }

    #[test]
    fn rejects_an_invalid_level() {
        let invalid = layer::<tracing_subscriber::Registry, _>(
            &config("info,=nonsense=", LogFormat::Json),
            Capture::default(),
            false,
        );
        assert!(invalid.is_err());
    }
}
