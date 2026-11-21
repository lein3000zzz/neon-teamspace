//! W3C trace context across process boundaries: HTTP headers, job rows and webhooks.
//!
//! Injection and extraction use the global propagator, which [`init`](crate::init) sets to
//! W3C TraceContext plus Baggage. OpenTelemetry's default propagator is a no-op, so without
//! that step `traceparent` is silently never written and every hop starts a new trace.
//!
//! Setting a parent only works on a span that has not been entered yet; do it right after
//! creating the span.

use std::collections::HashMap;

use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use opentelemetry::propagation::{
    Extractor, Injector, TextMapCompositePropagator, TextMapPropagator as _,
};
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry::{Context, global};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use tracing::Span;
use tracing_opentelemetry::{OpenTelemetrySpanExt as _, SetParentError};

/// The W3C header and carrier key holding the trace and span ids.
pub const TRACEPARENT: &str = "traceparent";

/// Sets the global propagator to W3C TraceContext plus Baggage. Idempotent.
pub fn install_propagator() {
    global::set_text_map_propagator(TextMapCompositePropagator::new(vec![
        Box::new(TraceContextPropagator::new()),
        Box::new(BaggagePropagator::new()),
    ]));
}

/// Writes the current span's context into `carrier`, for queues and other non-HTTP hops.
pub fn inject_map(carrier: &mut HashMap<String, String>) {
    let cx = Span::current().context();
    global::get_text_map_propagator(|propagator| propagator.inject_context(&cx, carrier));
}

/// Writes the current span's context into outgoing HTTP headers.
pub fn inject_headers(headers: &mut HeaderMap) {
    let cx = Span::current().context();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(headers));
    });
}

/// The current span's `traceparent`, to store in a job row or send with a webhook.
///
/// `None` outside a span or when no OpenTelemetry layer is installed. It does not depend on
/// the global propagator, because the stored value is always the W3C format.
#[must_use]
pub fn current_traceparent() -> Option<String> {
    let cx = Span::current().context();
    if !cx.span().span_context().is_valid() {
        return None;
    }
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(&cx, &mut carrier);
    carrier.remove(TRACEPARENT)
}

/// Continues the trace found in `carrier` with `span`.
pub fn set_parent_from_map(span: &Span, carrier: &HashMap<String, String>) {
    let cx = global::get_text_map_propagator(|propagator| propagator.extract(carrier));
    set_parent(span, cx);
}

/// Continues the trace of a stored `traceparent` value with `span`. An invalid value
/// leaves `span` as the root of a new trace.
pub fn set_parent_from_traceparent(span: &Span, traceparent: &str) {
    let carrier = HashMap::from([(TRACEPARENT.to_owned(), traceparent.to_owned())]);
    set_parent(span, TraceContextPropagator::new().extract(&carrier));
}

/// Continues the trace of an incoming HTTP request with `span`.
pub fn set_parent_from_headers(span: &Span, headers: &HeaderMap) {
    let cx =
        global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)));
    set_parent(span, cx);
}

/// A carrier without a valid context leaves the span's own parent, so a missing header never
/// detaches a span from the trace it already belongs to.
fn set_parent(span: &Span, cx: Context) {
    if !cx.span().span_context().is_valid() {
        return;
    }
    match span.set_parent(cx) {
        // Disabled spans and subscribers without the OpenTelemetry layer have nothing to link.
        Ok(()) | Err(SetParentError::SpanDisabled | SetParentError::LayerNotFound) => {}
        Err(SetParentError::AlreadyStarted) => tracing::warn!(
            span = span.metadata().map(|meta| meta.name()),
            "cannot set the parent of a span that was already entered"
        ),
    }
}

struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TraceId;

    use super::*;
    use crate::testing::Traces;

    const REMOTE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    fn trace_id(span: &Span) -> TraceId {
        span.context().span().span_context().trace_id()
    }

    #[test]
    fn map_round_trip_keeps_the_trace_id() {
        install_propagator();
        let traces = Traces::new();
        traces.run(|| {
            let producer = tracing::info_span!("enqueue");
            let mut carrier = HashMap::new();
            producer.in_scope(|| inject_map(&mut carrier));
            assert!(carrier.contains_key(TRACEPARENT));

            let consumer = tracing::info_span!("run job");
            set_parent_from_map(&consumer, &carrier);
            assert_eq!(trace_id(&consumer), trace_id(&producer));
        });
    }

    #[test]
    fn header_round_trip_keeps_the_trace_id() {
        install_propagator();
        let traces = Traces::new();
        traces.run(|| {
            let client = tracing::info_span!("send webhook");
            let mut headers = HeaderMap::new();
            client.in_scope(|| inject_headers(&mut headers));

            let server = tracing::info_span!("receive webhook");
            set_parent_from_headers(&server, &headers);
            assert_eq!(trace_id(&server), trace_id(&client));
        });
    }

    #[test]
    fn traceparent_round_trip_keeps_the_trace_id() {
        let traces = Traces::new();
        traces.run(|| {
            let request = tracing::info_span!("request");
            let stored = request.in_scope(current_traceparent).unwrap();
            assert!(stored.starts_with("00-"));
            assert!(stored.contains(&trace_id(&request).to_string()));

            let job = tracing::info_span!("job");
            set_parent_from_traceparent(&job, &stored);
            assert_eq!(trace_id(&job), trace_id(&request));
        });
    }

    #[test]
    fn continues_a_remote_trace() {
        let traces = Traces::new();
        traces.run(|| {
            let span = tracing::info_span!("job");
            set_parent_from_traceparent(&span, REMOTE);
            assert_eq!(
                trace_id(&span).to_string(),
                "4bf92f3577b34da6a3ce929d0e0e4736"
            );
        });
    }

    #[test]
    fn garbage_keeps_the_existing_parent() {
        let traces = Traces::new();
        traces.run(|| {
            let parent = tracing::info_span!("parent");
            let _entered = parent.enter();
            let child = tracing::info_span!("child");
            set_parent_from_traceparent(&child, "not a traceparent");
            assert_eq!(trace_id(&child), trace_id(&parent));
        });
    }

    #[test]
    fn no_traceparent_outside_a_span() {
        let traces = Traces::new();
        traces.run(|| assert_eq!(current_traceparent(), None));
        assert_eq!(current_traceparent(), None);
    }
}
