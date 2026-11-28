//! `/api/v1/admin/updates`: the server's version, whether a newer release is out, and having
//! the updater update the stack to it. Reading and checking take an instance admin's
//! session, or their API token with the admin scope; updating takes the session.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::updates::{UpdateStatus, UpdaterReport};
use neon_domain::updates::{self as domain, UpdaterState};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::Json;
use super::principal::Admin;

/// What is known of updates.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Updates {
    /// The server's version, as `GET /meta` says it.
    #[schema(example = json!(env!("CARGO_PKG_VERSION")))]
    pub current: String,
    /// A development build, compiled with debug assertions, which is never behind a
    /// release.
    pub development: bool,
    /// `updates.check`: off, the server asks GitHub nothing, and no release is known.
    pub check: bool,
    /// `updates.prereleases`: pre-releases count as newer releases too.
    pub prereleases: bool,
    /// The newest release the last check that worked saw, as the settings count releases.
    pub latest: Option<Release>,
    /// `latest` is newer than the server's version.
    pub newer: bool,
    /// When the last check ran.
    #[serde(with = "time::serde::rfc3339::option")]
    pub checked_at: Option<OffsetDateTime>,
    /// Why the last check failed, when it did. `latest` is then what the check before it
    /// saw.
    #[schema(
        example = "GitHub answered 403: it limits how often one address asks, so the next check tries again"
    )]
    pub error: Option<String>,
    pub updater: Updater,
}

/// A release on GitHub.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Release {
    /// Its tag without the `v`.
    #[schema(example = "0.2.1")]
    pub version: String,
    pub prerelease: bool,
    /// Its page on GitHub, with its notes.
    #[schema(example = "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.1")]
    pub url: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
}

/// The updater beside the stack.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Updater {
    /// One is set up: `updates.updater_token` is set.
    pub set_up: bool,
    /// Its latest update, when it answered.
    pub status: Option<UpdaterStatus>,
    /// Why it could not be asked, when it did not answer.
    #[schema(example = "the updater did not answer within 3s")]
    pub error: Option<String>,
}

/// Where the updater is with updating the stack.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    /// Asked nothing since it started.
    Idle,
    /// Pulling the release's images and restarting the services with them.
    Updating,
    /// The last update finished.
    Done,
    /// The last update failed; its error and log say how far it got.
    Failed,
}

/// The updater's latest update.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdaterStatus {
    pub state: UpdateState,
    /// The release it updates or updated to.
    #[schema(example = "0.2.1")]
    pub version: Option<String>,
    /// Why the update failed.
    pub error: Option<String>,
    /// The end of what it did, 16 KiB at most.
    pub log: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// The updater's own version.
    #[schema(example = json!(env!("CARGO_PKG_VERSION")))]
    pub updater_version: String,
}

/// The release to update to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyUpdate {
    /// A release newer than the server's that the last check saw, without the `v`.
    #[schema(example = "0.2.1")]
    pub version: String,
}

impl From<UpdateStatus> for Updates {
    fn from(status: UpdateStatus) -> Self {
        Self {
            current: status.running.version.to_string(),
            development: status.running.development,
            check: status.check,
            prereleases: status.prereleases,
            latest: status.latest.map(|release| Release {
                version: release.version.to_string(),
                prerelease: release.is_prerelease(),
                url: release.url.to_string(),
                published_at: release.published_at,
            }),
            newer: status.newer,
            checked_at: status.checked_at,
            error: status.error,
            updater: status.updater.into(),
        }
    }
}

impl From<UpdaterReport> for Updater {
    fn from(report: UpdaterReport) -> Self {
        Self {
            set_up: report.set_up,
            status: report.status.map(Into::into),
            error: report.error,
        }
    }
}

impl From<domain::UpdaterStatus> for UpdaterStatus {
    fn from(status: domain::UpdaterStatus) -> Self {
        Self {
            state: match status.state {
                UpdaterState::Idle => UpdateState::Idle,
                UpdaterState::Updating => UpdateState::Updating,
                UpdaterState::Done => UpdateState::Done,
                UpdaterState::Failed => UpdateState::Failed,
            },
            version: status.version,
            error: status.error,
            log: status.log,
            started_at: status.started_at,
            finished_at: status.finished_at,
            updater_version: status.updater_version,
        }
    }
}

#[utoipa::path(
    get,
    path = "/admin/updates",
    tag = "admin",
    summary = "The server's version and updates",
    description = "The server's version; the newest release of Neon Teamspace the last check \
                   saw on GitHub, as `updates.prereleases` counts releases, and whether it is \
                   newer; and, where an updater is set up, its latest update, asked for 3 \
                   seconds at most. The server checks GitHub once a day while `updates.check` \
                   is on.",
    responses(
        (status = OK, description = "What is known of updates.", body = Updates),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_updates(
    State(state): State<AppState>,
    Admin(admin): Admin,
) -> Result<Json<Updates>, ApiError> {
    Ok(Json(state.updates.status(&admin).await?.into()))
}

#[utoipa::path(
    post,
    path = "/admin/updates/check",
    tag = "admin",
    summary = "Check for a newer release now",
    description = "Asks GitHub for the releases now, while `updates.check` is on, then answers \
                   as `GET /admin/updates` does. Off, it asks nothing. A check that fails \
                   answers 200 too, saying why in `error`.",
    responses(
        (status = OK, description = "What is known of updates.", body = Updates),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn check_updates(
    State(state): State<AppState>,
    Admin(admin): Admin,
) -> Result<Json<Updates>, ApiError> {
    Ok(Json(state.updates.check_now(&admin).await?.into()))
}

#[utoipa::path(
    post,
    path = "/admin/updates/apply",
    tag = "admin",
    summary = "Update the server to a release",
    description = "Has the updater update the stack to a release newer than the server's that \
                   the last check saw on GitHub: it pulls the release's images and restarts \
                   the services with them, and the new server migrates the database as it \
                   starts. Answers once the updater has started, with its status; `GET \
                   /admin/updates` follows it, and `GET /meta` says the new version once the \
                   server is back. Only from a browser session.",
    request_body = ApplyUpdate,
    responses(
        (status = ACCEPTED, description = "The updater started the update.", body = Updates),
        (status = BAD_REQUEST, description = "update_not_allowed: not a release newer than the server's that the last check saw, or one the updater refused; or invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for an API token, or insufficient_scope.", body = ErrorBody),
        (status = CONFLICT, description = "no_updater, as `updates.updater_token` is empty; or update_in_progress.", body = ErrorBody),
        (status = BAD_GATEWAY, description = "updater_unreachable: the updater did not answer, or refused the server's token.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn apply_update(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(body): Json<ApplyUpdate>,
) -> Result<(StatusCode, Json<Updates>), ApiError> {
    let status = state.updates.apply(&admin, &body.version).await?;
    Ok((StatusCode::ACCEPTED, Json(status.into())))
}
