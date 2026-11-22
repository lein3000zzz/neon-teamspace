//! `GET /ws/databases/{database_id}`: a database's changes over a WebSocket, as JSON text
//! messages, for every client that shows it, without Yjs.
//!
//! The upgrade is checked as a page's is: the request must come from the server's own
//! origin when it names one, it authenticates by the session cookie or an API token with
//! the read scope, and the rules must let the caller see the database: one they may not see
//! answers 404. One more connection than the caller or the database may have answers 429,
//! and a request that is no upgrade 426.
//!
//! Once upgraded, the feed sends `ready`, then each change as it commits. The client has
//! nothing to send: what it sends is dropped, within a small size and rate. The server pings
//! it, and closes the socket with the code its feed gives, as page sync does.

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade, rejection};
use axum::http::StatusCode;
use axum::response::Response;
use neon_app::databases::{Admission, Follow, LiveDatabases, LiveLimits};
use neon_app::sync::{CloseReason, Ending, Next};
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tracing::{Instrument as _, Span, info, info_span, warn};
use uuid::Uuid;

use super::AppState;
use super::error::ApiError;
use super::extract::Path;
use super::principal::Authenticated;
use super::sync::{close, finish, read_failed, send};
use super::web::SameOriginSocket;

/// Follows a database's changes.
pub(super) async fn follow_database(
    _: SameOriginSocket,
    Authenticated(principal): Authenticated,
    State(state): State<AppState>,
    Path(database_id): Path<Uuid>,
    upgrade: Result<WebSocketUpgrade, rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let admission = state.live.admit(principal, database_id).await?;
    if !state.live.has_room(&admission) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_connections",
            CloseReason::TooManyConnections.message(),
        ));
    }
    let Ok(upgrade) = upgrade else {
        return Err(ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            "upgrade_required",
            "this route opens a WebSocket connection",
        ));
    };
    let live = state.live.clone();
    let limits = *live.limits();
    // Its own trace: a feed may stay open for hours, past its request's.
    let span = info_span!(
        parent: None,
        "database feed",
        database_id = %database_id,
        access = %admission.access(),
    );
    span.follows_from(Span::current());
    Ok(upgrade
        .max_message_size(limits.max_message)
        .max_frame_size(limits.max_message)
        .on_failed_upgrade(|error| warn!(%error, "a WebSocket upgrade failed"))
        .on_upgrade(move |socket| serve(socket, live, admission, limits).instrument(span)))
}

async fn serve(
    mut socket: WebSocket,
    live: LiveDatabases,
    admission: Admission,
    limits: LiveLimits,
) {
    let mut follow = match live.connect(admission) {
        Ok(follow) => follow,
        Err(reason) => {
            close(&mut socket, reason).await;
            return;
        }
    };
    let client = follow.id();
    info!(%client, "database feed opened");
    let ending = carry(&mut socket, &mut follow, &limits).await;
    follow.leave(ending);
    match ending {
        Ending::Closed(reason) => close(&mut socket, reason).await,
        Ending::ByClient => finish(&mut socket).await,
        Ending::Unresponsive => {}
    }
    info!(%client, cause = ending.cause(), "database feed closed");
}

/// Sends the feed's messages, and drops the client's, until the feed ends, and how.
async fn carry(socket: &mut WebSocket, follow: &mut Follow, limits: &LiveLimits) -> Ending {
    let mut pings = interval_at(Instant::now() + limits.ping_interval, limits.ping_interval);
    pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut heard = true;
    loop {
        tokio::select! {
            next = follow.next() => match next {
                Next::Send(message) => {
                    let Ok(text) = Utf8Bytes::try_from(message) else {
                        return Ending::Closed(CloseReason::ServerError);
                    };
                    if let Err(ending) = send(socket, Message::Text(text), limits.write_timeout).await {
                        return ending;
                    }
                }
                Next::Close(reason) => return Ending::Closed(reason),
            },
            received = socket.recv() => {
                heard = true;
                match received {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(reason) = follow.receive(text.len()) {
                            return Ending::Closed(reason);
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if let Err(reason) = follow.receive(data.len()) {
                            return Ending::Closed(reason);
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return Ending::ByClient,
                    Some(Err(error)) => return read_failed(&error),
                }
            }
            _ = pings.tick() => {
                if !heard {
                    return Ending::Unresponsive;
                }
                heard = false;
                if let Err(ending) = send(socket, Message::Ping(Bytes::new()), limits.write_timeout).await {
                    return ending;
                }
            }
        }
    }
}
