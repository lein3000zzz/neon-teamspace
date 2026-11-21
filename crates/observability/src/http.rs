//! A tower layer that traces and measures every request of an axum router.
//!
//! Apply it with `Router::layer`, so that axum's `MatchedPath` is set when the layer runs:
//!
//! ```no_run
//! use axum::{Router, routing::get};
//! use neon_observability::Config;
//! use neon_observability::http::HttpLayer;
//!
//! fn app(config: &Config) -> Router {
//!     Router::new()
//!         .route("/api/v1/meta", get(|| async { "ok" }))
//!         .layer(HttpLayer::new().trust_incoming(config.tracing.trust_incoming))
//! }
//! ```
//!
//! For each request it:
//!
//! - starts a new trace linked to the caller's `traceparent`, or continues the caller's trace
//!   and baggage when incoming context is trusted (see below);
//! - opens a server span named `<METHOD> <route>` with the HTTP semantic-convention fields;
//! - marks 5xx responses as unexpected errors and 4xx as expected ones;
//! - marks a request dropped before its response, because the client left, a timeout fired
//!   or the server shut down, as an unexpected error;
//! - records RED metrics under subsystem `http`, operation `<METHOD> <route>`, where only
//!   5xx and dropped requests count as `error`;
//! - keeps a valid incoming `x-request-id` or generates one, and returns it on the response.
//!
//! The duration runs until the response head is ready, not until a streamed body ends.
//!
//! # Routes instead of paths
//!
//! The route is the template, such as `/api/v1/pages/{id}`, never the raw path. It names the
//! span, labels the metrics, and is recorded as both `http.route` and `url.path`; the query
//! string is not recorded at all. Paths and queries carry secrets, such as incoming webhook
//! tokens and invite codes, and every log line written inside a request repeats the span's
//! fields. Requests no route matched use the fixed route `unmatched` and unknown methods use
//! `_OTHER`, so a scanner cannot create unbounded label values either.
//!
//! axum sets the matched route for `Router::route`, `Router::route_service` and nested
//! routers, but not for the paths below a `Router::nest_service` prefix, which therefore count
//! as `unmatched`. Mount a tower service under a prefix with
//! `route_service("/prefix/{*rest}", service)` instead, which hands it the full path,
//! prefix included.
//!
//! # Incoming trace context
//!
//! By default the layer distrusts `traceparent` and `baggage` from callers, because the app
//! port is public. A caller's sampled flag would otherwise override `sample_ratio`, anyone
//! could add spans to an existing trace, and forged baggage would be copied into job rows and
//! outgoing webhooks. Each request starts a new trace instead, sampled by `sample_ratio`, with
//! a span link to the caller's context when it is valid, and incoming baggage is dropped.
//! [`HttpLayer::trust_incoming`] continues the caller's trace and keeps its baggage, for
//! deployments where every caller is trusted.

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
use crate::propagation::{link_from_headers, set_parent_from_headers};
use crate::red::{Red, RedGuard};

/// RED subsystem of the HTTP metrics.
pub const SUBSYSTEM: &str = "http";

/// Route label, and `url.path`, of requests that matched no route. Paths below a
/// `Router::nest_service` prefix get it too; see the [module docs](self#routes-instead-of-paths).
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// Header carrying the request id.
pub static X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

const MAX_REQUEST_ID_LEN: usize = 128;

/// Span status description of a request dropped before its response.
const CANCELLED: &str = "the request was dropped before a response";

/// Traces, measures and tags requests with a request id. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct HttpLayer {
    red: Red,
    trust_incoming: bool,
}

impl HttpLayer {
    /// A layer recording under subsystem [`SUBSYSTEM`] that does not trust incoming trace
    /// context. Create it after [`init`](crate::init), like any [`Red`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            red: Red::new(SUBSYSTEM),
            trust_incoming: false,
        }
    }

    /// Whether requests continue the caller's trace and keep its baggage, instead of starting
    /// a new trace linked to it. Pass
    /// [`TracingConfig::trust_incoming`](crate::config::TracingConfig::trust_incoming); see
    /// the [module docs](self#incoming-trace-context).
    #[must_use]
    pub fn trust_incoming(mut self, trust: bool) -> Self {
        self.trust_incoming = trust;
        self
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
            trust_incoming: self.trust_incoming,
        }
    }
}

/// The service produced by [`HttpLayer`].
#[derive(Debug, Clone)]
pub struct HttpService<S> {
    inner: S,
    red: Red,
    trust_incoming: bool,
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
        let path = route.as_deref().unwrap_or(UNMATCHED_ROUTE);
        let operation = format!("{method} {path}");
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
            url.path = path,
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
        if self.trust_incoming {
            set_parent_from_headers(&span, request.headers());
        } else {
            link_from_headers(&span, request.headers());
        }

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
    /// The response future of [`HttpService`]. Dropping it before it completes fails the span
    /// and records the request as a RED error.
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        span: Span,
        guard: Option<RedGuard>,
        request_id: HeaderValue,
    }

    impl<F> PinnedDrop for ResponseFuture<F> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            // The guard is still here only when no response came; dropping it after this
            // records the RED error.
            if this.guard.is_some() {
                mark_unexpected(this.span, CANCELLED);
            }
        }
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
    use std::convert::Infallible;
    use std::task::Waker;

    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use http::{HeaderMap, StatusCode};
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use opentelemetry::Value;
    use opentelemetry::trace::{SpanId, SpanKind, Status};
    use opentelemetry_sdk::trace::SpanData;
    use tower::ServiceExt as _;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;
    use crate::config::LogFormat;
    use crate::errors::{EXPECTED_ATTRIBUTE, REASON_ATTRIBUTE};
    use crate::logging;
    use crate::propagation::{TRACEPARENT, inject_headers, install_propagator};
    use crate::testing::{Capture, Traces};

    const REMOTE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    const REMOTE_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const REMOTE_SPAN: &str = "00f067aa0ba902b7";
    const BAGGAGE: &str = "baggage";

    fn app(trust_incoming: bool) -> Router {
        let file = tower::service_fn(|_: Request<Body>| async {
            Ok::<_, Infallible>(Response::new(Body::from("file")))
        });
        Router::new()
            .route("/pages/{id}", get(|| async { "page" }))
            .route(
                "/broken",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            )
            .route("/forbidden", get(|| async { StatusCode::FORBIDDEN }))
            .route(
                "/hooks/in/{token}",
                get(|| async {
                    tracing::info!("webhook received");
                    "ok"
                }),
            )
            // Answers with the headers an outgoing call made inside the request would carry.
            .route(
                "/outgoing",
                get(|| async {
                    let mut headers = HeaderMap::new();
                    inject_headers(&mut headers);
                    headers
                }),
            )
            .route("/slow", get(std::future::pending::<&'static str>))
            .route_service("/files/{*rest}", file)
            .layer(HttpLayer::new().trust_incoming(trust_incoming))
    }

    fn get_request(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    fn with_trace_context(uri: &str) -> Request<Body> {
        Request::get(uri)
            .header(TRACEPARENT, REMOTE)
            .header(BAGGAGE, "role=admin")
            .body(Body::empty())
            .unwrap()
    }

    /// What one test run recorded.
    struct Observed {
        counts: Counts,
        logs: String,
    }

    /// Runs `f` on this thread with a local metrics recorder, the OpenTelemetry layer of
    /// `traces` and JSON logs at `info`, so everything the layer does is seen.
    fn observe<T>(traces: &Traces, f: impl FnOnce() -> T) -> (T, Observed) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let capture = Capture::default();
        let filter = EnvFilter::new("info");
        let subscriber = tracing_subscriber::registry()
            .with(traces.layer_following(&filter))
            .with(logging::layer(
                LogFormat::Json,
                filter,
                capture.clone(),
                false,
            ));

        let value = metrics::with_local_recorder(&recorder, || {
            tracing::subscriber::with_default(subscriber, f)
        });
        let observed = Observed {
            counts: Counts::take(&snapshotter),
            logs: capture.text(),
        };
        (value, observed)
    }

    /// Sends the requests one after the other through [`app`].
    fn send(
        traces: &Traces,
        trust_incoming: bool,
        requests: Vec<Request<Body>>,
    ) -> (Vec<Response<Body>>, Observed) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        observe(traces, || {
            let app = app(trust_incoming);
            runtime.block_on(async {
                let mut responses = Vec::new();
                for request in requests {
                    responses.push(app.clone().oneshot(request).await.unwrap());
                }
                responses
            })
        })
    }

    struct Counts(Vec<(String, String, String, u64)>);

    impl Counts {
        fn take(snapshotter: &Snapshotter) -> Self {
            let counts = snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .filter_map(|(key, _, _, value)| {
                    let (kind, key) = key.into_parts();
                    let (MetricKind::Counter, DebugValue::Counter(count)) = (kind, value) else {
                        return None;
                    };
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
                })
                .collect();
            Self(counts)
        }

        fn get(&self, operation: &str, status: &str) -> Option<u64> {
            self.0.iter().find_map(|(name, op, st, count)| {
                (name == "neon_http_operations_total" && op == operation && st == status)
                    .then_some(*count)
            })
        }
    }

    fn span<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
        spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("no span {name} in {spans:?}"))
    }

    fn traceparent_trace_id(headers: &HeaderMap) -> &str {
        let traceparent = headers[TRACEPARENT].to_str().unwrap();
        traceparent.split('-').nth(1).unwrap()
    }

    #[test]
    fn records_red_metrics_by_route_template() {
        let traces = Traces::new();
        let (responses, observed) = send(
            &traces,
            false,
            vec![
                get_request("/pages/1"),
                get_request("/pages/2"),
                get_request("/broken"),
                get_request("/forbidden"),
            ],
        );

        assert_eq!(responses[0].status(), StatusCode::OK);
        let counts = observed.counts;
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
        let (responses, observed) = send(
            &traces,
            false,
            vec![get_request("/wp-login.php"), get_request("/.env"), custom],
        );

        assert_eq!(responses[0].status(), StatusCode::NOT_FOUND);
        assert_eq!(observed.counts.get("GET unmatched", "ok"), Some(2));
        assert_eq!(observed.counts.get("_OTHER /pages/{id}", "ok"), Some(1));
        let spans = traces.spans();
        let unmatched = span(&spans, "GET unmatched");
        assert_eq!(Traces::attribute(unmatched, "http.route"), None);
        assert_eq!(
            Traces::attribute(unmatched, "url.path"),
            Some(Value::from(UNMATCHED_ROUTE))
        );
    }

    #[test]
    fn route_service_wildcards_label_the_whole_subtree() {
        let traces = Traces::new();
        let (responses, observed) =
            send(&traces, false, vec![get_request("/files/guide/intro.html")]);

        assert_eq!(responses[0].status(), StatusCode::OK);
        assert_eq!(observed.counts.get("GET /files/{*rest}", "ok"), Some(1));
        assert_eq!(traces.only_span().name, "GET /files/{*rest}");
    }

    #[test]
    fn spans_follow_http_semantic_conventions() {
        let traces = Traces::new();
        let request = Request::get("/pages/7")
            .header(USER_AGENT, "neon-test/1.0")
            .body(Body::empty())
            .unwrap();
        send(&traces, false, vec![request]);

        let span = traces.only_span();
        assert_eq!(span.name, "GET /pages/{id}");
        assert_eq!(span.span_kind, SpanKind::Server);
        assert_eq!(span.status, Status::Unset);
        let attribute = |key| Traces::attribute(&span, key);
        assert_eq!(attribute("http.request.method"), Some(Value::from("GET")));
        assert_eq!(attribute("http.route"), Some(Value::from("/pages/{id}")));
        assert_eq!(attribute("url.path"), Some(Value::from("/pages/{id}")));
        assert_eq!(attribute("url.query"), None);
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
    fn secrets_in_paths_and_queries_never_reach_logs_or_spans() {
        let traces = Traces::new();
        let (responses, observed) = send(
            &traces,
            false,
            vec![
                get_request("/hooks/in/whsec_s3cr3t-path?token=s3cr3t-query"),
                get_request("/hooks/out/whsec_s3cr3t-unmatched?code=s3cr3t-query"),
            ],
        );
        assert_eq!(responses[0].status(), StatusCode::OK);

        let logs = observed.logs;
        assert!(!logs.contains("s3cr3t"), "{logs}");
        let lines: Vec<serde_json::Value> = logs
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let received = lines
            .iter()
            .find(|line| line["message"] == "webhook received")
            .expect("the handler's line was logged");
        assert_eq!(received["span"]["url.path"], "/hooks/in/{token}");

        let spans = traces.spans();
        assert_eq!(spans.len(), 2);
        for span in &spans {
            assert!(!span.name.contains("s3cr3t"), "{span:?}");
            for attribute in &span.attributes {
                assert!(!attribute.value.as_str().contains("s3cr3t"), "{span:?}");
            }
        }
        assert_eq!(
            Traces::attribute(span(&spans, "GET /hooks/in/{token}"), "url.path"),
            Some(Value::from("/hooks/in/{token}"))
        );
    }

    #[test]
    fn server_errors_fail_the_span_and_client_errors_do_not() {
        let traces = Traces::new();
        send(
            &traces,
            false,
            vec![get_request("/broken"), get_request("/forbidden")],
        );

        let spans = traces.spans();
        let broken = span(&spans, "GET /broken");
        assert_eq!(broken.status, Status::error("500 Internal Server Error"));
        let forbidden = span(&spans, "GET /forbidden");
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
    fn cancelled_requests_fail_the_span_and_count_as_errors() {
        let traces = Traces::new();
        let ((), observed) = observe(&traces, || {
            let mut response = Box::pin(app(false).oneshot(get_request("/slow")));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(response.as_mut().poll(&mut cx).is_pending());
            drop(response);
        });

        let span = traces.only_span();
        assert_eq!(span.name, "GET /slow");
        assert_eq!(span.status, Status::error(CANCELLED));
        assert_eq!(Traces::attribute(&span, "http.response.status_code"), None);
        assert_eq!(observed.counts.get("GET /slow", "error"), Some(1));
        assert_eq!(observed.counts.get("GET /slow", "ok"), None);
    }

    #[test]
    fn untrusted_callers_get_a_new_trace_that_links_to_theirs() {
        install_propagator();
        let traces = Traces::new();
        let (responses, _) = send(&traces, false, vec![with_trace_context("/outgoing")]);

        let outgoing = responses[0].headers();
        assert!(outgoing.get(BAGGAGE).is_none(), "{outgoing:?}");
        assert_ne!(traceparent_trace_id(outgoing), REMOTE_TRACE);

        let span = traces.only_span();
        assert_ne!(span.span_context.trace_id().to_string(), REMOTE_TRACE);
        assert_eq!(span.parent_span_id, SpanId::INVALID);
        assert_eq!(span.links.len(), 1);
        let linked = &span.links[0].span_context;
        assert_eq!(linked.trace_id().to_string(), REMOTE_TRACE);
        assert_eq!(linked.span_id().to_string(), REMOTE_SPAN);
    }

    #[test]
    fn trusted_callers_continue_their_trace_and_baggage() {
        install_propagator();
        let traces = Traces::new();
        let (responses, _) = send(&traces, true, vec![with_trace_context("/outgoing")]);

        let outgoing = responses[0].headers();
        assert_eq!(outgoing[BAGGAGE], "role=admin");
        assert_eq!(traceparent_trace_id(outgoing), REMOTE_TRACE);

        let span = traces.only_span();
        assert_eq!(span.span_context.trace_id().to_string(), REMOTE_TRACE);
        assert_eq!(span.parent_span_id.to_string(), REMOTE_SPAN);
        assert!(span.links.is_empty());
    }

    #[test]
    fn a_callers_sampled_flag_overrides_the_ratio_only_when_trusted() {
        install_propagator();
        let untrusted = Traces::with_sample_ratio(0.0);
        send(&untrusted, false, vec![with_trace_context("/pages/1")]);
        assert!(untrusted.spans().is_empty());

        let trusted = Traces::with_sample_ratio(0.0);
        send(&trusted, true, vec![with_trace_context("/pages/1")]);
        assert_eq!(trusted.only_span().name, "GET /pages/{id}");
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
        let (responses, _) = send(
            &traces,
            false,
            vec![with_id, get_request("/pages/1"), hostile],
        );

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
