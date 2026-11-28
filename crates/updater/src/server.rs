//! The updater's HTTP server: the protocol of [`crate::protocol`], one update at a time, with
//! the latest update's status and the end of its output held in memory.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use crate::docker::Job;
use crate::output::Recorder;
use crate::protocol::{
    ErrorBody, ErrorDetail, MAX_LOG_BYTES, MIN_TOKEN_CHARS, PATH_HEALTH, PATH_STATUS, PATH_UPDATE,
    State as UpdateState, Status, UpdateRequest,
};
use crate::version::Version;

/// The largest request body taken: an update request is a version.
const MAX_REQUEST_BYTES: usize = 1024;

/// How long a request may take to arrive and be answered. Every request is small, and an
/// update runs apart from the request that started it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The token the server sends, kept as its SHA-256 only.
#[derive(Clone)]
pub struct Token([u8; 32]);

/// A token shorter than [`MIN_TOKEN_CHARS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenTooShort;

impl fmt::Display for TokenTooShort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the token must be at least {MIN_TOKEN_CHARS} characters, such as the output of openssl rand -hex 32"
        )
    }
}

impl std::error::Error for TokenTooShort {}

impl Token {
    /// Takes `token`, refusing one shorter than [`MIN_TOKEN_CHARS`] characters.
    ///
    /// # Errors
    ///
    /// When the token is too short.
    pub fn new(token: &str) -> Result<Self, TokenTooShort> {
        if token.chars().count() < MIN_TOKEN_CHARS {
            return Err(TokenTooShort);
        }
        Ok(Self(Sha256::digest(token.as_bytes()).into()))
    }

    /// Whether `given` is the token. Both are hashed first, so the comparison, in constant
    /// time, takes as long whatever `given`'s length and whichever byte differs first.
    #[must_use]
    pub fn matches(&self, given: &str) -> bool {
        let given: [u8; 32] = Sha256::digest(given.as_bytes()).into();
        bool::from(self.0.ct_eq(&given))
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// The updater's server: one update at a time, each run by its job.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

struct Inner {
    token: Token,
    job: Arc<dyn Job>,
    now: fn() -> OffsetDateTime,
    latest: Mutex<Latest>,
}

/// The latest update.
struct Latest {
    status: Status,
    /// Its output while it runs.
    output: Option<Arc<Recorder>>,
    task: Option<JoinHandle<()>>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

/// Now, to the second.
fn now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(0).unwrap_or(now)
}

impl Server {
    /// Serves the holder of `token`, running `job` for each update.
    pub fn new(token: Token, job: Arc<dyn Job>) -> Self {
        Self {
            inner: Arc::new(Inner {
                token,
                job,
                now,
                latest: Mutex::new(Latest {
                    status: Status::idle(),
                    output: None,
                    task: None,
                }),
            }),
        }
    }

    /// The routes of the protocol, `/v1` behind the token, every other path `404`.
    pub fn router(&self) -> Router {
        let v1 = Router::new()
            .route(PATH_STATUS, get(status))
            .route(PATH_UPDATE, post(update))
            .route_layer(middleware::from_fn_with_state(self.clone(), authorize));
        Router::new()
            .route(PATH_HEALTH, get(|| async { "ok\n" }))
            .merge(v1)
            .fallback(|| async { refusal(StatusCode::NOT_FOUND, "not_found", "no such route") })
            .method_not_allowed_fallback(|| async {
                refusal(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "method_not_allowed",
                    "the route takes another method",
                )
            })
            .with_state(self.clone())
            .layer(middleware::from_fn(time_limit))
    }

    /// The latest update, with the end of its output so far while it runs.
    #[must_use]
    pub fn status(&self) -> Status {
        let latest = self.latest();
        let mut status = latest.status.clone();
        if let Some(output) = &latest.output {
            status.log = Some(output.text()).filter(|log| !log.is_empty());
        }
        status
    }

    /// Waits for the latest update to end, for tests. Stopping the updater does not wait for
    /// it: the update runs in a container of its own and carries on.
    pub async fn wait(&self) {
        let task = self.latest().task.take();
        if let Some(task) = task {
            // A task that panicked has nothing more to wait for.
            let _ = task.await;
        }
    }

    /// Starts an update to `version` and returns its status, unless one is under way, whose
    /// version it then returns.
    fn start(&self, version: Version) -> Result<Status, Option<String>> {
        let mut latest = self.latest();
        if latest.status.state == UpdateState::Updating {
            return Err(latest.status.version.clone());
        }
        let output = Arc::new(Recorder::new(MAX_LOG_BYTES));
        latest.status = Status {
            state: UpdateState::Updating,
            version: Some(version.to_string()),
            started_at: Some((self.inner.now)()),
            ..Status::idle()
        };
        latest.output = Some(Arc::clone(&output));
        let status = latest.status.clone();
        let server = self.clone();
        latest.task = Some(tokio::spawn(async move {
            server.run(version, output).await;
        }));
        Ok(status)
    }

    async fn run(&self, version: Version, output: Arc<Recorder>) {
        tracing::info!(%version, "updating");
        // On a task of its own, so that a job that panics fails the update rather than leave
        // it under way for good.
        let job = Arc::clone(&self.inner.job);
        let out = Arc::clone(&output);
        let asked = version.clone();
        let result = tokio::spawn(async move { job.update(&asked, out.as_ref()).await })
            .await
            .unwrap_or_else(|error| Err(anyhow::anyhow!("the update stopped: {error}")));
        output.finish();
        let log = output.text();

        let mut latest = self.latest();
        latest.output = None;
        latest.status.finished_at = Some((self.inner.now)());
        latest.status.log = Some(log).filter(|log| !log.is_empty());
        match result {
            Ok(()) => {
                latest.status.state = UpdateState::Done;
                tracing::info!(%version, "updated");
            }
            Err(error) => {
                let error = format!("{error:#}");
                tracing::error!(%version, %error, "the update failed");
                latest.status.state = UpdateState::Failed;
                latest.status.error = Some(error);
            }
        }
    }

    fn latest(&self) -> MutexGuard<'_, Latest> {
        self.inner
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Lets only the token's holder through.
async fn authorize(State(server): State<Server>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match given {
        Some(given) if server.inner.token.matches(given) => next.run(request).await,
        _ => {
            let mut response = refusal(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "the token is not the updater's",
            );
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
            response
        }
    }
}

/// Answers `408` to a request that takes longer than [`REQUEST_TIMEOUT`], such as one whose
/// body trickles in.
async fn time_limit(request: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => refusal(
            StatusCode::REQUEST_TIMEOUT,
            "request_timeout",
            "the request took too long",
        ),
    }
}

async fn status(State(server): State<Server>) -> Response {
    axum::Json(server.status()).into_response()
}

async fn update(State(server): State<Server>, body: Body) -> Response {
    let Ok(bytes) = axum::body::to_bytes(body, MAX_REQUEST_BYTES).await else {
        return refusal(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            format!("an update request is at most {MAX_REQUEST_BYTES} bytes"),
        );
    };
    let Ok(request) = serde_json::from_slice::<UpdateRequest>(&bytes) else {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            r#"the body must be {"version": "0.2.1"}"#,
        );
    };
    let version = match request.version.parse::<Version>() {
        Ok(version) => version,
        Err(error) => {
            return refusal(
                StatusCode::BAD_REQUEST,
                "invalid_version",
                error.to_string(),
            );
        }
    };
    // Never back: an older release, signed as it may be, can have flaws fixed since, and the
    // token would otherwise move the stack, the updater included, back to one. The server asks
    // for newer releases alone, and going back is a backup's restore, by hand. The updater's
    // own version passes, so that a failed update can be tried again.
    if version.precedes(crate::VERSION) {
        return refusal(
            StatusCode::BAD_REQUEST,
            "older_release",
            format!(
                "{version} is older than the updater's {}: the updater never goes back",
                crate::VERSION
            ),
        );
    }
    match server.start(version) {
        Ok(status) => (StatusCode::ACCEPTED, axum::Json(status)).into_response(),
        Err(under_way) => refusal(
            StatusCode::CONFLICT,
            "update_in_progress",
            format!(
                "an update is under way already, to {}",
                under_way.as_deref().unwrap_or("a release")
            ),
        ),
    }
}

/// An error answer, in the server API's one shape.
fn refusal(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = ErrorBody {
        error: ErrorDetail {
            code: code.to_owned(),
            message: message.into(),
        },
    };
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use serde_json::{Value, json};
    use tokio::sync::{Semaphore, mpsc};
    use tower::ServiceExt as _;

    use super::*;
    use crate::output::Output;

    const TOKEN: &str = "test-token-not-a-secret-0123456789";

    /// A release `minor` minor versions after the updater's own, so that the tests hold
    /// whatever the workspace's version is.
    fn newer(minor: u64) -> String {
        let own = semver::Version::parse(crate::VERSION).unwrap();
        semver::Version::new(own.major, own.minor + minor, 0).to_string()
    }

    /// An update request's body for `version`.
    fn asking(version: &str) -> String {
        format!(r#"{{"version": "{version}"}}"#)
    }

    /// A job that says it was asked, waits to be let through, prints, then fails as told.
    struct Gate {
        through: Semaphore,
        asked: mpsc::UnboundedSender<String>,
        fail: Mutex<Option<String>>,
        print: usize,
    }

    impl Job for Gate {
        fn update<'a>(
            &'a self,
            version: &'a Version,
            out: &'a dyn Output,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.asked.send(version.to_string()).unwrap();
                self.through.acquire().await.unwrap().forget();
                for _ in 0..self.print {
                    out.line(&format!("pulled {version}"));
                }
                match self.fail.lock().unwrap().take() {
                    Some(error) => Err(anyhow::anyhow!(error)),
                    None => Ok(()),
                }
            })
        }
    }

    fn server(print: usize) -> (Server, Arc<Gate>, mpsc::UnboundedReceiver<String>) {
        let (asked, receiver) = mpsc::unbounded_channel();
        let gate = Arc::new(Gate {
            through: Semaphore::new(0),
            asked,
            fail: Mutex::new(None),
            print,
        });
        let server = Server::new(
            Token::new(TOKEN).unwrap(),
            Arc::clone(&gate) as Arc<dyn Job>,
        );
        (server, gate, receiver)
    }

    async fn call(
        server: &Server,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = server
            .router()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    #[test]
    fn a_token_must_be_long_enough() {
        assert_eq!(Token::new(&"x".repeat(31)).unwrap_err(), TokenTooShort);
        assert!(Token::new(&"x".repeat(32)).is_ok());
        // Characters, not bytes.
        assert!(Token::new(&"é".repeat(31)).is_err());
        assert!(TokenTooShort.to_string().contains("at least 32 characters"));
    }

    #[test]
    fn a_token_matches_itself_only() {
        let token = Token::new(TOKEN).unwrap();
        assert!(token.matches(TOKEN));
        for other in [
            "",
            &TOKEN[..TOKEN.len() - 1],
            &format!("{TOKEN}x"),
            &format!("x{}", &TOKEN[1..]),
            &format!("{}x", &TOKEN[..TOKEN.len() - 1]),
            &TOKEN.to_uppercase(),
        ] {
            assert!(!token.matches(other), "{other}");
        }
        assert_eq!(format!("{token:?}"), "Token(..)", "never printed");
    }

    /// The comparison is of two SHA-256 digests, 32 bytes each, through `subtle`, whatever the
    /// lengths of the tokens: so its time says nothing of how much of a guess was right.
    #[test]
    fn the_comparison_is_of_fixed_length_digests() {
        let token = Token::new(TOKEN).unwrap();
        let expected: [u8; 32] = Sha256::digest(TOKEN.as_bytes()).into();
        assert_eq!(token.0, expected);
        assert!(bool::from(token.0.ct_eq(&expected)));
        let short: [u8; 32] = Sha256::digest(b"x").into();
        assert!(!bool::from(token.0.ct_eq(&short)));
    }

    #[tokio::test]
    async fn health_is_open_and_the_rest_needs_the_token() {
        let (server, _, _) = server(1);
        let (code, body) = call(&server, "GET", "/healthz", None, "").await;
        assert_eq!((code, body), (StatusCode::OK, json!("ok\n")));

        let other = "y".repeat(40);
        let prefix = &TOKEN[..TOKEN.len() - 1];
        for token in [None, Some(other.as_str()), Some(prefix), Some("")] {
            let (code, body) = call(&server, "GET", "/v1/status", token, "").await;
            assert_eq!(code, StatusCode::UNAUTHORIZED, "{token:?}");
            assert_eq!(
                body,
                json!({"error": {"code": "unauthenticated", "message": "the token is not the updater's"}})
            );
            let (code, _) = call(
                &server,
                "POST",
                "/v1/update",
                token,
                r#"{"version": "0.2.1"}"#,
            )
            .await;
            assert_eq!(code, StatusCode::UNAUTHORIZED, "{token:?}");
        }
        assert_eq!(server.status().state, UpdateState::Idle, "nothing updated");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header("Authorization", format!("Basic {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }

    #[tokio::test]
    async fn bad_requests_start_nothing() {
        let (server, _, _) = server(1);
        for (body, code) in [
            (r#"{"version": "v0.2.1"}"#, "invalid_version"),
            (r#"{"version": "0.2.1; reboot"}"#, "invalid_version"),
            (r#"{"version": "latest"}"#, "invalid_version"),
            (r#"{"version": "0.2.1", "then": "x"}"#, "invalid_json"),
            ("0.2.1", "invalid_json"),
            ("", "invalid_json"),
        ] {
            let (status, answer) = call(&server, "POST", "/v1/update", Some(TOKEN), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(answer["error"]["code"], code, "{body}");
        }
        let long = format!(r#"{{"version": "0.2.1", "x": "{}"}}"#, "x".repeat(2000));
        let (status, answer) = call(&server, "POST", "/v1/update", Some(TOKEN), &long).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(answer["error"]["code"], "payload_too_large");

        let (status, answer) = call(&server, "GET", "/v1/update", Some(TOKEN), "").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(answer["error"]["code"], "method_not_allowed");
        let (status, answer) = call(&server, "GET", "/v2/status", Some(TOKEN), "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(answer["error"]["code"], "not_found");
        assert_eq!(server.status().state, UpdateState::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_never_arrives_whole_times_out() {
        let (server, _, _) = server(1);
        let body = Body::from_stream(futures_util::stream::pending::<Result<Vec<u8>, String>>());
        let request = Request::builder()
            .method("POST")
            .uri("/v1/update")
            .header("Authorization", format!("Bearer {TOKEN}"))
            .body(body)
            .unwrap();
        let response = server.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "request_timeout");
        assert_eq!(server.status().state, UpdateState::Idle, "nothing started");
    }

    #[tokio::test]
    async fn one_update_at_a_time_from_idle_to_done_or_failed() {
        let (server, gate, mut asked) = server(1);
        let (first, second) = (newer(1), newer(2));

        let (code, body) = call(&server, "GET", "/v1/status", Some(TOKEN), "").await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            body,
            json!({"state": "idle", "updater_version": crate::VERSION})
        );

        let (code, body) = call(&server, "POST", "/v1/update", Some(TOKEN), &asking(&first)).await;
        assert_eq!(code, StatusCode::ACCEPTED);
        assert_eq!(body["state"], "updating");
        assert_eq!(body["version"], first.as_str());
        assert!(body["started_at"].is_string());
        assert!(body.get("finished_at").is_none());
        assert_eq!(asked.recv().await.unwrap(), first);

        let (code, body) = call(&server, "POST", "/v1/update", Some(TOKEN), &asking(&second)).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(
            body,
            json!({"error": {
                "code": "update_in_progress",
                "message": format!("an update is under way already, to {first}"),
            }})
        );

        gate.through.add_permits(1);
        server.wait().await;
        let (_, body) = call(&server, "GET", "/v1/status", Some(TOKEN), "").await;
        assert_eq!(body["state"], "done");
        assert_eq!(body["version"], first.as_str());
        assert_eq!(body["log"], format!("pulled {first}\n"));
        assert!(body.get("error").is_none());
        let started = body["started_at"].as_str().unwrap();
        let finished = body["finished_at"].as_str().unwrap();
        assert!(started <= finished, "{started} {finished}");

        // The last one done, another may start, and fail.
        *gate.fail.lock().unwrap() =
            Some("pulling the images: docker compose: exit status 1".to_owned());
        let (code, _) = call(&server, "POST", "/v1/update", Some(TOKEN), &asking(&second)).await;
        assert_eq!(code, StatusCode::ACCEPTED);
        assert_eq!(asked.recv().await.unwrap(), second);
        let (_, body) = call(&server, "GET", "/v1/status", Some(TOKEN), "").await;
        assert_eq!(body["state"], "updating");
        assert!(body.get("error").is_none() && body.get("finished_at").is_none());
        gate.through.add_permits(1);
        server.wait().await;
        let (_, body) = call(&server, "GET", "/v1/status", Some(TOKEN), "").await;
        assert_eq!(body["state"], "failed");
        assert_eq!(body["version"], second.as_str());
        assert_eq!(
            body["error"],
            "pulling the images: docker compose: exit status 1"
        );
    }

    /// `0.0.1` comes before every version the workspace has had, from 0.1.0 on.
    #[tokio::test]
    async fn an_older_release_is_refused_and_the_updater_s_own_taken() {
        let (server, gate, mut asked) = server(1);

        let (code, body) = call(&server, "POST", "/v1/update", Some(TOKEN), &asking("0.0.1")).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({"error": {
                "code": "older_release",
                "message": format!(
                    "0.0.1 is older than the updater's {}: the updater never goes back",
                    crate::VERSION
                ),
            }})
        );
        assert!(asked.try_recv().is_err(), "the job was never asked");
        assert_eq!(server.status(), Status::idle());

        // The same release again, as when a failed update is tried again.
        let (code, body) = call(
            &server,
            "POST",
            "/v1/update",
            Some(TOKEN),
            &asking(crate::VERSION),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED, "{body}");
        assert_eq!(asked.recv().await.unwrap(), crate::VERSION);
        gate.through.add_permits(1);
        server.wait().await;
        assert_eq!(server.status().state, UpdateState::Done);
    }

    /// A job that panics.
    struct Panics;

    impl Job for Panics {
        fn update<'a>(
            &'a self,
            _: &'a Version,
            out: &'a dyn Output,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move {
                out.line("about to fail");
                panic!("a bug");
            })
        }
    }

    #[tokio::test]
    async fn a_job_that_panics_fails_the_update_and_frees_the_updater() {
        let server = Server::new(Token::new(TOKEN).unwrap(), Arc::new(Panics));
        let (code, _) = call(
            &server,
            "POST",
            "/v1/update",
            Some(TOKEN),
            &asking(&newer(1)),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED);
        server.wait().await;

        let status = server.status();
        assert_eq!(status.state, UpdateState::Failed);
        let error = status.error.unwrap();
        assert!(error.starts_with("the update stopped: "), "{error}");
        assert_eq!(status.log.as_deref(), Some("about to fail\n"));
        let (code, _) = call(
            &server,
            "POST",
            "/v1/update",
            Some(TOKEN),
            &asking(&newer(1)),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED, "another may start");
        server.wait().await;
    }

    #[tokio::test]
    async fn the_status_holds_the_end_of_the_log_while_it_runs_and_after() {
        let (server, gate, mut asked) = server(2000);
        let version = newer(1);
        call(
            &server,
            "POST",
            "/v1/update",
            Some(TOKEN),
            &asking(&version),
        )
        .await;
        asked.recv().await.unwrap();
        assert!(server.status().log.is_none(), "nothing printed yet");

        gate.through.add_permits(1);
        server.wait().await;
        let log = server.status().log.unwrap();
        let line = format!("pulled {version}\n");
        assert!(log.len() <= MAX_LOG_BYTES);
        assert!(log.len() > MAX_LOG_BYTES - line.len());
        assert!(log.ends_with(&line));
    }
}
