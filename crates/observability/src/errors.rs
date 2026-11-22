//! Expected and unexpected errors on spans.
//!
//! An expected error, such as not found, forbidden or a failed validation, is part of normal
//! operation: the span keeps an `exception` event and the attributes `neon.error.expected` and
//! `neon.error.reason`, but its status stays unset so error-rate views and tail sampling only
//! see real failures. An unexpected error also sets the span status to `ERROR`.
//!
//! These functions go through the span's OpenTelemetry extension, so they work on any span
//! without declaring `otel.status_code` up front. Logging inside a span has its own rule:
//! an `ERROR`-level event marks the span as failed, lower levels never do, even with an
//! `error` field.

use std::error::Error;
use std::fmt::Write;

use opentelemetry::KeyValue;
use opentelemetry::trace::Status;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Attribute set to `true` on spans that ended in an expected error.
pub const EXPECTED_ATTRIBUTE: &str = "neon.error.expected";
/// Attribute holding why the error is expected, such as `not found`.
pub const REASON_ATTRIBUTE: &str = "neon.error.reason";

const EXCEPTION_EVENT: &str = "exception";
const EXCEPTION_MESSAGE: &str = "exception.message";

/// Records `error` on `span` as part of normal operation. The span status stays unset.
pub fn record_expected(span: &Span, error: &(dyn Error + 'static), reason: &str) {
    add_exception(span, error);
    mark_expected(span, reason);
}

/// Records `error` on `span` as a failure and sets the span status to `ERROR`.
pub fn record_unexpected(span: &Span, error: &(dyn Error + 'static)) {
    let message = add_exception(span, error);
    span.set_status(Status::error(message));
}

/// Marks `span` as ended in an expected error without a Rust error value, such as an HTTP
/// 404. The span status stays unset.
pub fn mark_expected(span: &Span, reason: impl Into<String>) {
    span.set_attribute(EXPECTED_ATTRIBUTE, true);
    span.set_attribute(REASON_ATTRIBUTE, reason.into());
}

/// Sets the status of `span` to `ERROR` with `description`, without an exception event.
pub fn mark_unexpected(span: &Span, description: impl Into<String>) {
    span.set_status(Status::error(description.into()));
}

fn add_exception(span: &Span, error: &(dyn Error + 'static)) -> String {
    let message = chain(error);
    span.add_event(
        EXCEPTION_EVENT,
        vec![KeyValue::new(EXCEPTION_MESSAGE, message.clone())],
    );
    message
}

/// The error and its sources, outermost first, since most `Display` impls omit the source.
pub(crate) fn chain(error: &(dyn Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(message, ": {cause}");
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use opentelemetry::Value;

    use super::*;
    use crate::testing::Traces;

    #[derive(Debug, thiserror::Error)]
    #[error("page not found")]
    struct NotFound;

    #[derive(Debug, thiserror::Error)]
    #[error("loading the page failed")]
    struct LoadFailed(#[source] std::io::Error);

    #[test]
    fn expected_errors_leave_the_status_unset() {
        let traces = Traces::new();
        traces.run(|| {
            let span = tracing::info_span!("load");
            record_expected(&span, &NotFound, "not found");
        });

        let span = traces.only_span();
        assert_eq!(span.status, Status::Unset);
        assert_eq!(
            Traces::attribute(&span, EXPECTED_ATTRIBUTE),
            Some(Value::Bool(true))
        );
        assert_eq!(
            Traces::attribute(&span, REASON_ATTRIBUTE),
            Some(Value::from("not found"))
        );
        let event = span.events.iter().next().unwrap();
        assert_eq!(event.name, EXCEPTION_EVENT);
        assert!(
            event
                .attributes
                .contains(&KeyValue::new(EXCEPTION_MESSAGE, "page not found"))
        );
    }

    #[test]
    fn unexpected_errors_set_status_error_with_the_source_chain() {
        let traces = Traces::new();
        traces.run(|| {
            let span = tracing::info_span!("load");
            record_unexpected(&span, &LoadFailed(std::io::Error::other("disk on fire")));
        });

        let span = traces.only_span();
        assert_eq!(
            span.status,
            Status::error("loading the page failed: disk on fire")
        );
        assert_eq!(Traces::attribute(&span, EXPECTED_ATTRIBUTE), None);
        assert_eq!(span.events.len(), 1);
    }

    #[test]
    fn warning_logs_with_an_error_field_do_not_fail_the_span() {
        let traces = Traces::new();
        traces.run(|| {
            let span = tracing::info_span!("load");
            let _entered = span.enter();
            tracing::warn!(error = %NotFound, "page missing");
        });

        assert_eq!(traces.only_span().status, Status::Unset);
    }

    #[test]
    fn error_logs_fail_the_span() {
        let traces = Traces::new();
        traces.run(|| {
            let span = tracing::info_span!("load");
            let _entered = span.enter();
            tracing::error!("database unreachable");
        });

        assert!(matches!(traces.only_span().status, Status::Error { .. }));
    }

    #[test]
    fn marks_work_without_an_error_value() {
        let traces = Traces::new();
        traces.run(|| {
            let expected = tracing::info_span!("expected");
            mark_expected(&expected, "404 Not Found");
            let unexpected = tracing::info_span!("unexpected");
            mark_unexpected(&unexpected, "502 Bad Gateway");
        });

        let spans = traces.spans();
        let expected = spans.iter().find(|s| s.name == "expected").unwrap();
        assert_eq!(expected.status, Status::Unset);
        assert!(expected.events.is_empty());
        let unexpected = spans.iter().find(|s| s.name == "unexpected").unwrap();
        assert_eq!(unexpected.status, Status::error("502 Bad Gateway"));
    }
}
