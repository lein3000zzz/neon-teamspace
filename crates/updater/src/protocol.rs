//! The updater's HTTP protocol, which the server speaks to it: JSON in snake_case, and errors
//! in the server API's one shape, `{"error": {"code": string, "message": string}}`.
//!
//! - `GET /healthz` answers `200`, without a token.
//! - `GET /v1/status` answers `200` with the latest update's [`Status`].
//! - `POST /v1/update` with an [`UpdateRequest`] starts an update and answers `202` with its
//!   [`Status`]; `409` while another is under way, `400` for a version that is none.
//!
//! The two under `/v1` take the token as `Authorization: Bearer <token>`, and answer `401`
//! without it.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Whether the updater runs, for anyone: no token.
pub const PATH_HEALTH: &str = "/healthz";
/// The latest update.
pub const PATH_STATUS: &str = "/v1/status";
/// Asks for an update.
pub const PATH_UPDATE: &str = "/v1/update";

/// The fewest characters a token may have: `openssl rand -hex 32` makes 64.
pub const MIN_TOKEN_CHARS: usize = 32;

/// The most of an update's output a [`Status`] holds: its last 16 KiB.
pub const MAX_LOG_BYTES: usize = 16 * 1024;

/// Where the updater is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Asked for nothing since it started.
    Idle,
    /// An update is under way.
    Updating,
    /// The latest update finished.
    Done,
    /// The latest update failed; [`Status::error`] says why.
    Failed,
}

/// The latest update, or none since the updater started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    /// The release the latest update is to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The end of what it printed, at most [`MAX_LOG_BYTES`], as it runs and once it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<OffsetDateTime>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<OffsetDateTime>,
    /// The updater's own version.
    pub updater_version: String,
}

impl Status {
    /// The status of an updater asked for nothing yet.
    #[must_use]
    pub fn idle() -> Self {
        Self {
            state: State::Idle,
            version: None,
            error: None,
            log: None,
            started_at: None,
            finished_at: None,
            updater_version: crate::VERSION.to_owned(),
        }
    }
}

/// Asks for an update to `version`, such as `0.2.1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    pub version: String,
}

/// Every error answer's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// Stable and meant for programs, such as `unauthenticated` or `update_in_progress`.
    pub code: String,
    /// What went wrong, for people.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    #[test]
    fn an_idle_status_says_only_its_state_and_version() {
        assert_eq!(
            serde_json::to_value(Status::idle()).unwrap(),
            json!({"state": "idle", "updater_version": crate::VERSION})
        );
    }

    #[test]
    fn times_are_rfc_3339() {
        let status = Status {
            state: State::Failed,
            version: Some("0.2.1".to_owned()),
            error: Some("pulling the images: exit status 1".to_owned()),
            log: Some("Pulling server, web at 0.2.1\n".to_owned()),
            started_at: Some(datetime!(2026-11-28 09:00:00 UTC)),
            finished_at: Some(datetime!(2026-11-28 09:01:30 UTC)),
            updater_version: "0.2.0".to_owned(),
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value,
            json!({
                "state": "failed",
                "version": "0.2.1",
                "error": "pulling the images: exit status 1",
                "log": "Pulling server, web at 0.2.1\n",
                "started_at": "2026-11-28T09:00:00Z",
                "finished_at": "2026-11-28T09:01:30Z",
                "updater_version": "0.2.0",
            })
        );
        assert_eq!(serde_json::from_value::<Status>(value).unwrap(), status);
    }

    #[test]
    fn an_update_request_names_a_version_and_nothing_else() {
        assert!(serde_json::from_str::<UpdateRequest>(r#"{"version": "0.2.1"}"#).is_ok());
        assert!(
            serde_json::from_str::<UpdateRequest>(r#"{"version": "0.2.1", "then": "x"}"#).is_err()
        );
    }
}
