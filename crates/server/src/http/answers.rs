//! Large answers on their way: an answer built whole in memory, such as a large page's
//! markdown or a version of it, stays there until its client has taken the last of it, and a
//! client that reads slowly keeps it so as long as it likes. Each client has at most
//! [`PER_CLIENT`] answers of [`LARGE`] or more on their way at once; one more is answered
//! with `503 busy` instead, and its answer dropped. A client is the person a request
//! authenticated as, their agents with them, or else the source it came from, an IPv4
//! address or an IPv6 /64. Answers streamed as they are made, such as files and exports,
//! hold little of themselves at once, and are not counted.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use axum::body::{Body, HttpBody as _};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt as _;
use neon_domain::network::client_source;
use tracing::warn;
use uuid::Uuid;

use super::error::ApiError;
use super::web::WebConfig;

/// The size from which an answer counts as large.
pub(crate) const LARGE: u64 = 256 * 1024;

/// How many large answers one client has on their way at once.
pub(crate) const PER_CLIENT: usize = 8;

/// When a client refused one more should try again.
const BUSY_RETRY: Duration = Duration::from_secs(1);

/// The large answers on their way, by client.
#[derive(Debug)]
pub struct LargeAnswers {
    per_client: usize,
    running: Arc<Mutex<HashMap<String, usize>>>,
}

impl LargeAnswers {
    pub fn new(per_client: usize) -> Self {
        Self {
            per_client: per_client.max(1),
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A turn of `client`'s, while they have fewer answers on their way than they may.
    fn turn(&self, client: String) -> Option<Turn> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let count = running.entry(client.clone()).or_default();
        if *count >= self.per_client {
            return None;
        }
        *count += 1;
        Some(Turn {
            running: Arc::clone(&self.running),
            client,
        })
    }
}

impl Default for LargeAnswers {
    fn default() -> Self {
        Self::new(PER_CLIENT)
    }
}

/// A large answer's place among its client's, given back as the answer is dropped: once
/// sent, or once its connection is gone.
struct Turn {
    running: Arc<Mutex<HashMap<String, usize>>>,
    client: String,
}

impl Turn {
    /// Passes an error of the answer's body on, so the body's adapter holds the turn.
    fn pass<E>(&self, error: E) -> E {
        error
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = running.get_mut(&self.client) {
            *count -= 1;
            if *count == 0 {
                running.remove(&self.client);
            }
        }
    }
}

/// The person a request authenticated as, which authentication notes for the answer.
#[derive(Debug, Clone, Default)]
pub(crate) struct AnsweredFor(Arc<OnceLock<Uuid>>);

impl AnsweredFor {
    /// Notes the person an authenticated request is theirs: an agent's owner, or the
    /// person themselves.
    pub(crate) fn note(&self, person: Uuid) {
        let _ = self.0.set(person);
    }
}

/// Counts each large answer against its client's, as the module docs describe.
pub(super) async fn hold_large_answers(
    State((answers, web)): State<(Arc<LargeAnswers>, Arc<WebConfig>)>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let source = client_source(&web.client_ip(peer, request.headers()));
    let answered_for = AnsweredFor::default();
    request.extensions_mut().insert(answered_for.clone());
    let response = next.run(request).await;
    if response.body().size_hint().lower() < LARGE {
        return response;
    }
    let client = match answered_for.0.get() {
        Some(person) => format!("person:{person}"),
        None => format!("source:{source}"),
    };
    let Some(turn) = answers.turn(client) else {
        warn!("too many large answers on their way to one client");
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "too many large answers are on their way to you: read them, then ask again",
        )
        .with_retry_after(BUSY_RETRY)
        .into_response();
    };
    response.map(|body| Body::new(body.map_err(move |error| turn.pass(error))))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt as _;

    use super::*;

    fn app(answers: Arc<LargeAnswers>) -> Router {
        let web = Arc::new(WebConfig::new(None, Vec::new()));
        Router::new()
            .route("/large", get(|| async { vec![b'x'; LARGE as usize] }))
            .route("/small", get(|| async { "small" }))
            .layer(middleware::from_fn_with_state(
                (answers, web),
                hold_large_answers,
            ))
    }

    async fn ask(app: &Router, path: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("answered")
    }

    #[tokio::test]
    async fn a_client_has_a_few_large_answers_on_their_way_at_once() {
        let app = app(Arc::new(LargeAnswers::new(2)));
        let first = ask(&app, "/large").await;
        let second = ask(&app, "/large").await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        // Its size is still told.
        assert_eq!(first.body().size_hint().exact(), Some(LARGE));

        let refused = ask(&app, "/large").await;
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(refused.headers().contains_key("retry-after"));
        // Small answers are not counted.
        assert_eq!(ask(&app, "/small").await.status(), StatusCode::OK);

        // An answer read to its end gives its turn back.
        let read = first.into_body().collect().await.expect("read");
        assert_eq!(read.to_bytes().len(), LARGE as usize);
        let third = ask(&app, "/large").await;
        assert_eq!(third.status(), StatusCode::OK);
        assert_eq!(
            ask(&app, "/large").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        // One dropped unread, as when its connection is gone, does too.
        drop(second);
        assert_eq!(ask(&app, "/large").await.status(), StatusCode::OK);
        drop(third);
    }
}
