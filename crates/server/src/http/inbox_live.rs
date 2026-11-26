//! `GET /ws/inbox`: that the signed-in user's inbox changed, over a WebSocket, as JSON text
//! messages, for the app to read it again.
//!
//! The upgrade must come from the server's own origin when it names one, and authenticates
//! by the session cookie or an API token with the read scope. One more feed than the caller
//! may have answers 429, and a request that is no upgrade 426. Once upgraded, the feed sends
//! `{"type":"ready"}`, then `{"type":"inbox"}` each time the inbox changes. The client has
//! nothing to send: what it sends is dropped, within a small size and rate. The server pings
//! it, and closes the socket with the code its feed gives, as page sync does.

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade, rejection};
use axum::http::StatusCode;
use axum::response::Response;
use neon_app::notifications::{FeedLimits, InboxFeed, InboxFeeds};
use neon_app::sync::{CloseReason, Ending, Next};
use neon_domain::auth::Principal;
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tracing::{Instrument as _, Span, info, info_span, warn};

use super::AppState;
use super::error::ApiError;
use super::principal::Authenticated;
use super::sync::{close, finish, read_failed, send};
use super::web::SameOriginSocket;

/// Follows the caller's inbox.
pub(super) async fn follow_inbox(
    _: SameOriginSocket,
    Authenticated(principal): Authenticated,
    State(state): State<AppState>,
    upgrade: Result<WebSocketUpgrade, rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    if !state.inbox_feeds.has_room(&principal) {
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
    let feeds = state.inbox_feeds.clone();
    let limits = *feeds.limits();
    // Its own trace: a feed may stay open for hours, past its request's.
    let span = info_span!(parent: None, "inbox feed", user_id = %principal.user.id);
    span.follows_from(Span::current());
    Ok(upgrade
        .max_message_size(limits.max_message)
        .max_frame_size(limits.max_message)
        .on_failed_upgrade(|error| warn!(%error, "a WebSocket upgrade failed"))
        .on_upgrade(move |socket| serve(socket, feeds, principal, limits).instrument(span)))
}

async fn serve(mut socket: WebSocket, feeds: InboxFeeds, principal: Principal, limits: FeedLimits) {
    let mut feed = match feeds.connect(principal) {
        Ok(feed) => feed,
        Err(reason) => {
            close(&mut socket, reason).await;
            return;
        }
    };
    info!("inbox feed opened");
    let ending = carry(&mut socket, &mut feed, &limits).await;
    feed.leave(ending);
    match ending {
        Ending::Closed(reason) => close(&mut socket, reason).await,
        Ending::ByClient => finish(&mut socket).await,
        Ending::Unresponsive => {}
    }
    info!(cause = ending.cause(), "inbox feed closed");
}

/// Sends the feed's messages, and drops the client's, until the feed ends, and how.
async fn carry(socket: &mut WebSocket, feed: &mut InboxFeed, limits: &FeedLimits) -> Ending {
    let mut pings = interval_at(Instant::now() + limits.ping_interval, limits.ping_interval);
    pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut heard = true;
    loop {
        tokio::select! {
            next = feed.next() => match next {
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
                        if let Err(reason) = feed.receive(text.len()) {
                            return Ending::Closed(reason);
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if let Err(reason) = feed.receive(data.len()) {
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
