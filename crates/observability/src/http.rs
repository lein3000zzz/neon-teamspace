//! A tower layer that traces and measures every request of an axum router.
//!
//! Apply it with `Router::layer`, so that axum's `MatchedPath` is set when the layer runs:
//!
//! ```no_run
//! use axum::{Router, routing::get};
//! use neon_observability::http::HttpLayer;
//!
//! let app: Router = Router::new()
//!     .route("/api/v1/meta", get(|| async { "ok" }))
//!     .layer(HttpLayer::new());
//! ```
//!
//! For each request it:
//!
//! - continues the caller's trace from `traceparent` and `baggage`;
//! - opens a server span named `<METHOD> <route>` with the HTTP semantic-convention fields;
//! - marks 5xx responses as unexpected errors and 4xx as expected ones;
//! - records RED metrics under subsystem `http`, operation `<METHOD> <route>`, where only
//!   5xx and abandoned requests count as `error`;
//! - keeps a valid incoming `x-request-id` or generates one, and returns it on the response.
//!
//! The route is the template, such as `/api/v1/pages/{id}`, never the raw path. Requests no
//! route matched use the fixed route `unmatched` and unknown methods use `_OTHER`, so a
//! scanner cannot create unbounded label values.
//!
//! The duration runs until the response head is ready, not until a streamed body ends.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use axum::extract::MatchedPath;
use http::header::{HeaderName, HeaderValue, USER_AGENT};
use http::{Method, Request, Response};
use pin_project_lite::pin_project;
use tower::{Layer, Service};
use tracing::Span;
use tracing::field::Empty;
use uuid::Uuid;

use crate::errors::{mark_expected, mark_unexpected};
use crate::propagation::set_parent_from_headers;
use crate::red::{Red, RedGuard};

/// RED subsystem of the HTTP metrics.
pub const SUBSYSTEM: &str = "http";

/// Route label of requests that matched no route.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// Header carrying the request id.
pub static X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

const MAX_REQUEST_ID_LEN: usize = 128;

/// Traces, measures and tags requests with a request id. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct HttpLayer {
    red: Red,
}

impl HttpLayer {
    /// A layer recording under subsystem [`SUBSYSTEM`]. Create it after
    /// [`init`](crate::init), like any [`Red`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            red: Red::new(SUBSYSTEM),
        }
    }
}

impl Default for HttpLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for HttpLayer {
    type Service = HttpService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HttpService {
            inner,
            red: self.red.clone(),
        }
    }
}

/// The service produced by [`HttpLayer`].
#[derive(Debug, Clone)]
pub struct HttpService<S> {
    inner: S,
    red: Red,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for HttpService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        let method = method_label(request.method());
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(|path| path.as_str().to_owned());
        let operation = format!("{method} {}", route.as_deref().unwrap_or(UNMATCHED_ROUTE));
        let request_id = request_id(request.headers().get(&X_REQUEST_ID));
        request
            .headers_mut()
            .insert(X_REQUEST_ID.clone(), request_id.clone());

        let span = tracing::info_span!(
            "http.request",
            otel.name = %operation,
            otel.kind = "server",
            http.request.method = method,
            http.route = Empty,
            url.path = request.uri().path(),
            user_agent.original = Empty,
            http.response.status_code = Empty,
            request_id = request_id.to_str().unwrap_or_default(),
        );
        if let Some(route) = &route {
            span.record("http.route", route.as_str());
        }
        if let Some(agent) = request
            .headers()
            .get(USER_AGENT)
            .and_then(|agent| agent.to_str().ok())
        {
            span.record("user_agent.original", agent);
        }
        set_parent_from_headers(&span, request.headers());

        let guard = self.red.start(operation);
        let inner = span.in_scope(|| self.inner.call(request));
        ResponseFuture {
            inner,
            span,
            guard: Some(guard),
            request_id,
        }
    }
}

pin_project! {
    /// The response future of [`HttpService`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        span: Span,
        guard: Option<RedGuard>,
        request_id: HeaderValue,
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _entered = this.span.enter();
        let mut result = ready!(this.inner.poll(cx));

        let ok = match &mut result {
            Ok(response) => {
                let status = response.status();
                // The OpenTelemetry layer turns unsigned fields into strings, signed into ints.
                this.span
                    .record("http.response.status_code", i64::from(status.as_u16()));
                if status.is_server_error() {
                    mark_unexpected(this.span, status.to_string());
                } else if status.is_client_error() {
                    mark_expected(this.span, status.to_string());
                }
                response
                    .headers_mut()
                    .entry(X_REQUEST_ID.clone())
                    .or_insert_with(|| this.request_id.clone());
                !status.is_server_error()
            }
            Err(_) => {
                mark_unexpected(this.span, "the service failed without a response");
                false
            }
        };
        if let Some(guard) = this.guard.take() {
            guard.finish(ok);
        }
        Poll::Ready(result)
    }
}

/// Standard methods keep their name; anything else would be an unbounded label value.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        _ => "_OTHER",
    }
}

/// Keeps a caller's id when it is short printable ASCII, so it can be logged and echoed
/// safely, and otherwise generates a time-ordered UUID.
fn request_id(incoming: Option<&HeaderValue>) -> HeaderValue {
    incoming
        .filter(|value| {
            let bytes = value.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= MAX_REQUEST_ID_LEN
                && bytes.iter().all(u8::is_ascii_graphic)
        })
        .cloned()
        .unwrap_or_else(|| {
            HeaderValue::try_from(Uuid::now_v7().to_string())
                .expect("a UUID is a valid header value")
        })
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use http::StatusCode;
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use opentelemetry::Value;
    use opentelemetry::trace::Status;
    use tower::ServiceExt as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;
    use crate::errors::{EXPECTED_ATTRIBUTE, REASON_ATTRIBUTE};
    use crate::propagation::install_propagator;
    use crate::testing::Traces;

    const REMOTE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    fn app() -> Router {
        Router::new()
            .route("/pages/{id}", get(|| async { "page" }))
            .route(
                "/broken",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            )
            .route("/forbidden", get(|| async { StatusCode::FORBIDDEN }))
            .layer(HttpLayer::new())
    }

    fn get_request(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    /// Sends the requests on one thread, so the thread-local recorder and subscriber see
    /// everything the layer does.
    fn send(traces: &Traces, requests: Vec<Request<Body>>) -> (Vec<Response<Body>>, Counts) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let subscriber = tracing_subscriber::registry().with(traces.layer());

        let responses = metrics::with_local_recorder(&recorder, || {
            tracing::subscriber::with_default(subscriber, || {
                let app = app();
                runtime.block_on(async {
                    let mut responses = Vec::new();
                    for request in requests {
                        responses.push(app.clone().oneshot(request).await.unwrap());
                    }
                    responses
                })
            })
        });

        let counts = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                let (kind, key) = key.into_parts();
                match (kind, value) {
                    (MetricKind::Counter, DebugValue::Counter(count)) => {
                        let label = |name: &str| {
                            key.labels()
                                .find(|label| label.key() == name)
                                .map(|label| label.value().to_owned())
                                .unwrap_or_default()
                        };
                        Some((
                            key.name().to_owned(),
                            label("operation"),
                            label("status"),
                            count,
                        ))
                    }
                    _ => None,
                }
            })
            .collect();
        (responses, Counts(counts))
    }

    struct Counts(Vec<(String, String, String, u64)>);

    impl Counts {
        fn get(&self, operation: &str, status: &str) -> Option<u64> {
            self.0.iter().find_map(|(name, op, st, count)| {
                (name == "neon_http_operations_total" && op == operation && st == status)
                    .then_some(*count)
            })
        }
    }

    #[test]
    fn records_red_metrics_by_route_template() {
        let traces = Traces::new();
        let (responses, counts) = send(
            &traces,
            vec![
                get_request("/pages/1"),
                get_request("/pages/2"),
                get_request("/broken"),
                get_request("/forbidden"),
            ],
        );

        assert_eq!(responses[0].status(), StatusCode::OK);
        assert_eq!(counts.get("GET /pages/{id}", "ok"), Some(2));
        assert_eq!(counts.get("GET /broken", "error"), Some(1));
        assert_eq!(counts.get("GET /forbidden", "ok"), Some(1));
    }

    #[test]
    fn unmatched_routes_and_unknown_methods_have_fixed_labels() {
        let traces = Traces::new();
        let custom = Request::builder()
            .method(Method::from_bytes(b"PURGE").unwrap())
            .uri("/pages/1")
            .body(Body::empty())
            .unwrap();
        let (responses, counts) = send(
            &traces,
            vec![get_request("/wp-login.php"), get_request("/.env"), custom],
        );

        assert_eq!(responses[0].status(), StatusCode::NOT_FOUND);
        assert_eq!(counts.get("GET unmatched", "ok"), Some(2));
        assert_eq!(counts.get("_OTHER /pages/{id}", "ok"), Some(1));
        let spans = traces.spans();
        assert!(spans.iter().any(|span| span.name == "GET unmatched"));
        let unmatched = spans.iter().find(|s| s.name == "GET unmatched").unwrap();
        assert_eq!(Traces::attribute(unmatched, "http.route"), None);
    }

    #[test]
    fn spans_follow_http_semantic_conventions() {
        let traces = Traces::new();
        let request = Request::get("/pages/7")
            .header(USER_AGENT, "neon-test/1.0")
            .body(Body::empty())
            .unwrap();
        send(&traces, vec![request]);

        let span = traces.only_span();
        assert_eq!(span.name, "GET /pages/{id}");
        assert_eq!(span.span_kind, opentelemetry::trace::SpanKind::Server);
        assert_eq!(span.status, Status::Unset);
        let attribute = |key| Traces::attribute(&span, key);
        assert_eq!(attribute("http.request.method"), Some(Value::from("GET")));
        assert_eq!(attribute("http.route"), Some(Value::from("/pages/{id}")));
        assert_eq!(attribute("url.path"), Some(Value::from("/pages/7")));
        assert_eq!(
            attribute("user_agent.original"),
            Some(Value::from("neon-test/1.0"))
        );
        assert_eq!(
            attribute("http.response.status_code"),
            Some(Value::I64(200))
        );
    }

    #[test]
    fn server_errors_fail_the_span_and_client_errors_do_not() {
        let traces = Traces::new();
        send(
            &traces,
            vec![get_request("/broken"), get_request("/forbidden")],
        );

        let spans = traces.spans();
        let broken = spans.iter().find(|s| s.name == "GET /broken").unwrap();
        assert_eq!(broken.status, Status::error("500 Internal Server Error"));
        let forbidden = spans.iter().find(|s| s.name == "GET /forbidden").unwrap();
        assert_eq!(forbidden.status, Status::Unset);
        assert_eq!(
            Traces::attribute(forbidden, EXPECTED_ATTRIBUTE),
            Some(Value::Bool(true))
        );
        assert_eq!(
            Traces::attribute(forbidden, REASON_ATTRIBUTE),
            Some(Value::from("403 Forbidden"))
        );
    }

    #[test]
    fn continues_the_callers_trace() {
        install_propagator();
        let traces = Traces::new();
        let request = Request::get("/pages/1")
            .header("traceparent", REMOTE)
            .body(Body::empty())
            .unwrap();
        send(&traces, vec![request]);

        let span = traces.only_span();
        assert_eq!(
            span.span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(span.parent_span_id.to_string(), "00f067aa0ba902b7");
    }

    #[test]
    fn keeps_or_generates_the_request_id() {
        let traces = Traces::new();
        let with_id = Request::get("/pages/1")
            .header(&X_REQUEST_ID, "abc-123")
            .body(Body::empty())
            .unwrap();
        let hostile = Request::get("/pages/1")
            .header(&X_REQUEST_ID, "a".repeat(MAX_REQUEST_ID_LEN + 1))
            .body(Body::empty())
            .unwrap();
        let (responses, _) = send(&traces, vec![with_id, get_request("/pages/1"), hostile]);

        assert_eq!(responses[0].headers()[&X_REQUEST_ID], "abc-123");
        for generated in &responses[1..] {
            let id = generated.headers()[&X_REQUEST_ID].to_str().unwrap();
            assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 7);
        }
    }

    #[test]
    fn request_ids_must_be_printable_ascii() {
        assert_eq!(
            request_id(Some(&HeaderValue::from_static("req-1"))),
            HeaderValue::from_static("req-1")
        );
        let spaced = request_id(Some(&HeaderValue::from_static("two words")));
        assert_ne!(spaced, HeaderValue::from_static("two words"));
        let empty = request_id(Some(&HeaderValue::from_static("")));
        assert!(!empty.is_empty());
    }
}
