//! `POST /hooks/in/{token}`: what senders post to an incoming webhook.
//!
//! The token in the path is all that says which webhook, and who: no cookie, no
//! `Authorization` header, nothing else a request carries is taken for who sends it. So the
//! browser's same-origin check, which guards what a visitor's cookie could do, leaves this
//! path alone; every other guard holds, the time limit and the rate limits among them. The
//! body is JSON of at most 256 KiB, signed when the webhook asks for it, in the format the
//! webhooks guide describes. Its answer is `{"ok": true, "action", "row"}`, or the error
//! shape.

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use http_body_util::LengthLimitError;
use neon_app::webhooks::signature::{SIGNATURE_HEADER, TIMESTAMP_HEADER};
use neon_app::webhooks::{BodyError, IncomingRequest, MAX_INCOMING_BODY};
use serde::Serialize;
use uuid::Uuid;

use super::AppState;
use super::error::ApiError;
use super::extract::{Json, Path};
use super::web::ClientIp;

/// Where incoming webhooks are, as the same-origin check knows them.
pub(super) const PREFIX: &str = "/hooks/in/";

/// What a request did.
#[derive(Debug, Serialize)]
struct Answer {
    ok: bool,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    row: Option<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    id: Uuid,
    unique_id: String,
}

/// Whether a `Content-Type` is JSON: `application/json`, or a type ending in `+json`, with
/// any parameters.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|kind| kind.trim().to_ascii_lowercase())
        .is_some_and(|kind| kind == "application/json" || kind.ends_with("+json"))
}

pub(super) async fn receive_hook(
    State(state): State<AppState>,
    Path(token): Path<String>,
    ClientIp(client): ClientIp,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let request = IncomingRequest {
        token: &token,
        client_ip: &client,
        json: is_json(&headers),
        signature: header(SIGNATURE_HEADER),
        timestamp: header(TIMESTAMP_HEADER),
    };
    let read = async move {
        to_bytes(body, MAX_INCOMING_BODY).await.map_err(|error| {
            if error.into_inner().is::<LengthLimitError>() {
                BodyError::TooLarge
            } else {
                BodyError::Unreadable
            }
        })
    };
    match state.webhooks.receive_incoming(request, read).await {
        Ok(accepted) => Json(Answer {
            ok: true,
            action: accepted.action,
            row: accepted.row.map(|(id, unique_id)| Row { id, unique_id }),
        })
        .into_response(),
        Err(error) => ApiError::from(error).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn json_is_told_by_its_content_type() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).expect("a value"));
            is_json(&headers)
        };
        assert!(with("application/json"));
        assert!(with("Application/JSON; charset=utf-8"));
        assert!(with("application/vnd.github+json"));
        assert!(!with("text/plain"));
        assert!(!with("application/x-www-form-urlencoded"));
        assert!(!is_json(&HeaderMap::new()));
    }
}
