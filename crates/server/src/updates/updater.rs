//! The [`Updater`] port over the updater's HTTP protocol, JSON in snake case:
//!
//! - `GET /v1/status` answers its latest update;
//! - `POST /v1/update` with `{"version": "0.2.1"}` starts one, and answers `202` with the
//!   status, `409` while an update is under way, and `400` for a version it does not take.
//!
//! Both take the token as `Authorization: Bearer`, and answer `401` without the right one.
//! Errors come in the API's shape, `{"error": {"code", "message"}}`. The paths come after
//! the updater's URL, path included, so `http://updater:8080` is asked at
//! `http://updater:8080/v1/status`.

use std::time::Duration;

use async_trait::async_trait;
use axum::http::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use axum::http::{HeaderValue, Method, StatusCode};
use bytes::Bytes;
use neon_app::updates::{Updater, UpdaterEndpoint, UpdaterError};
use neon_domain::updates::{UpdaterState, UpdaterStatus};
use semver::Version;
use serde::Deserialize;
use time::OffsetDateTime;
use url::Url;

use super::client::{Client, Outgoing};

/// The updater's paths.
pub const STATUS_PATH: &str = "/v1/status";
pub const UPDATE_PATH: &str = "/v1/update";

/// How long a request to the updater may take. The status waits less, as the use case
/// bounds it.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The most of an answer that is read: a status, with the end of its log, 16 KiB at most.
const MAX_ANSWER: usize = 64 * 1024;

/// Asks the updater; see the module docs.
#[derive(Default)]
pub struct UpdaterClient {
    client: Client,
}

/// A status as the updater sends it.
#[derive(Deserialize)]
struct Status {
    state: UpdaterState,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    log: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    started_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    finished_at: Option<OffsetDateTime>,
    updater_version: String,
}

/// An error as the updater sends it.
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

impl UpdaterClient {
    async fn call(
        &self,
        at: &UpdaterEndpoint,
        method: Method,
        path: &str,
        json: Option<Bytes>,
    ) -> Result<UpdaterStatus, UpdaterError> {
        let url = join(&at.url, path);
        let mut bearer = HeaderValue::try_from(format!("Bearer {}", at.token.expose()))
            .map_err(|_| UpdaterError::Refused)?;
        bearer.set_sensitive(true);
        let answer = self
            .client
            .send(Outgoing {
                method,
                url: &url,
                headers: vec![
                    (AUTHORIZATION, bearer),
                    (ACCEPT, HeaderValue::from_static("application/json")),
                    (
                        USER_AGENT,
                        HeaderValue::from_static(concat!(
                            "NeonTeamspace/",
                            env!("CARGO_PKG_VERSION")
                        )),
                    ),
                ],
                json,
                timeout: TIMEOUT,
                max_answer: MAX_ANSWER,
            })
            .await
            .map_err(|error| {
                UpdaterError::Unreachable(format!(
                    "the updater at {} did not answer: {error}",
                    at.url
                ))
            })?;
        if answer.status.is_success() {
            return serde_json::from_slice::<Status>(&answer.body)
                .map(status)
                .map_err(|_| {
                    UpdaterError::Unreachable(format!(
                        "the updater at {} answered something other than its status",
                        at.url
                    ))
                });
        }
        let message = serde_json::from_slice::<ErrorBody>(&answer.body)
            .map(|body| body.error.message)
            .ok();
        Err(match answer.status {
            StatusCode::UNAUTHORIZED => UpdaterError::Refused,
            StatusCode::CONFLICT => UpdaterError::Busy,
            StatusCode::BAD_REQUEST => UpdaterError::Rejected(
                message.unwrap_or_else(|| "it refused the request".to_owned()),
            ),
            other => UpdaterError::Unreachable(match message {
                Some(message) => format!("the updater answered {}: {message}", other.as_u16()),
                None => format!("the updater answered {}", other.as_u16()),
            }),
        })
    }
}

#[async_trait]
impl Updater for UpdaterClient {
    async fn status(&self, at: &UpdaterEndpoint) -> Result<UpdaterStatus, UpdaterError> {
        self.call(at, Method::GET, STATUS_PATH, None).await
    }

    async fn update(
        &self,
        at: &UpdaterEndpoint,
        version: &Version,
    ) -> Result<UpdaterStatus, UpdaterError> {
        let body = serde_json::json!({ "version": version.to_string() }).to_string();
        self.call(at, Method::POST, UPDATE_PATH, Some(Bytes::from(body)))
            .await
    }
}

/// `path` after the updater's URL, and its path.
fn join(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    url.set_path(&format!("{}{path}", base.path().trim_end_matches('/')));
    url
}

fn status(sent: Status) -> UpdaterStatus {
    UpdaterStatus {
        state: sent.state,
        version: sent.version,
        error: sent.error,
        log: sent.log,
        started_at: sent.started_at,
        finished_at: sent.finished_at,
        updater_version: sent.updater_version,
    }
}
