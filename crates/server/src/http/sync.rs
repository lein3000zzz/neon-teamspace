//! `GET /ws/pages/{page_id}`: a page's live document over a WebSocket, in y-websocket's
//! protocol, so the browser's stock `WebsocketProvider` connects to it with the page's id
//! as its room.
//!
//! Before the upgrade, the request must come from the server's own origin when it names
//! one, as browsers always do, so a page elsewhere cannot open a connection with the
//! visitor's cookie. It authenticates as any API route does, by the session cookie or an
//! API token with the read scope, and the rules must let the caller see the page: one they
//! may not see answers 404, before any upgrade. One more connection than the caller or the
//! page may have open answers 429. A request that is no WebSocket upgrade answers 426.
//!
//! With `?comments=true`, the connection also hears, in a message of type 100 whose
//! payload is a JSON text, `{"type":"comments","thread_id":"..."}`, whenever a thread of the
//! page's comments changes; stock clients ask for none.
//!
//! Once upgraded, the sync service's [`Connection`] decides what happens; this module
//! carries the messages both ways, pings the client and closes the socket with the code
//! the connection gives. A client that sent nothing, not even a pong, between two pings is
//! taken for gone and dropped without a close.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, rejection};
use axum::http::StatusCode;
use axum::response::Response;
use neon_app::sync::{Admission, CloseReason, Connection, Ending, Limits, Next, SyncService};
use serde::Deserialize;
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};
use tracing::{Instrument as _, Span, info, info_span, warn};
use uuid::Uuid;

use super::AppState;
use super::error::ApiError;
use super::extract::{Path, Query};
use super::principal::Authenticated;
use super::web::SameOriginSocket;

/// How long closing waits for the client's close in return.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// What a client may ask of a page's connection beside the document.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Follow {
    /// `true` to hear, with a message of type 100, whenever a thread of the page's comments
    /// changes.
    #[serde(default)]
    comments: bool,
}

/// Opens a page's live document.
pub(super) async fn open_page(
    _: SameOriginSocket,
    Authenticated(principal): Authenticated,
    State(state): State<AppState>,
    Path(page_id): Path<Uuid>,
    Query(follow): Query<Follow>,
    upgrade: Result<WebSocketUpgrade, rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let admission = state
        .sync
        .admit(principal, page_id)
        .await?
        .with_comments(follow.comments);
    if !state.sync.has_room(&admission) {
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
    let sync = state.sync.clone();
    let limits = *sync.limits();
    // Its own trace: a connection may stay open for hours, past its request's.
    let span = info_span!(
        parent: None,
        "sync connection",
        page_id = %page_id,
        access = %admission.access(),
    );
    span.follows_from(Span::current());
    Ok(upgrade
        .max_message_size(limits.max_message)
        .max_frame_size(limits.max_message)
        .on_failed_upgrade(|error| warn!(%error, "a WebSocket upgrade failed"))
        .on_upgrade(move |socket| serve(socket, sync, admission, limits).instrument(span)))
}

/// Serves one connection until it ends.
async fn serve(mut socket: WebSocket, sync: SyncService, admission: Admission, limits: Limits) {
    let mut connection = match sync.connect(admission).await {
        Ok(connection) => connection,
        Err(reason) => {
            close(&mut socket, reason).await;
            return;
        }
    };
    let client = connection.id();
    info!(%client, "connection opened");
    let ending = carry(&mut socket, &mut connection, &limits).await;
    // The document hears first, so it sends nothing more while the socket closes.
    connection.leave(ending).await;
    match ending {
        Ending::Closed(reason) => close(&mut socket, reason).await,
        Ending::ByClient => finish(&mut socket).await,
        Ending::Unresponsive => {}
    }
    info!(%client, cause = ending.cause(), "connection closed");
}

/// Carries messages both ways until the connection ends, and how.
async fn carry(socket: &mut WebSocket, connection: &mut Connection, limits: &Limits) -> Ending {
    let mut pings = interval_at(Instant::now() + limits.ping_interval, limits.ping_interval);
    pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Whether the client sent anything since the last ping.
    let mut heard = true;
    loop {
        tokio::select! {
            next = connection.next() => match next {
                Next::Send(message) => {
                    let sent = send(socket, Message::Binary(message), limits.write_timeout).await;
                    if let Err(ending) = sent {
                        return ending;
                    }
                }
                Next::Close(reason) => return Ending::Closed(reason),
            },
            received = socket.recv() => {
                heard = true;
                match received {
                    Some(Ok(Message::Binary(data))) => {
                        if let Err(reason) = connection.receive(data).await {
                            return Ending::Closed(reason);
                        }
                    }
                    Some(Ok(Message::Text(_))) => {
                        return Ending::Closed(CloseReason::UnsupportedData);
                    }
                    // The WebSocket answers pings itself.
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
                let sent = send(socket, Message::Ping(Bytes::new()), limits.write_timeout).await;
                if let Err(ending) = sent {
                    return ending;
                }
            }
        }
    }
}

/// Sends a message within the write timeout.
pub(super) async fn send(
    socket: &mut WebSocket,
    message: Message,
    write_timeout: Duration,
) -> Result<(), Ending> {
    match timeout(write_timeout, socket.send(message)).await {
        Ok(Ok(())) => Ok(()),
        // The socket is gone.
        Ok(Err(_)) => Err(Ending::ByClient),
        Err(_) => Err(Ending::Closed(CloseReason::TooSlow)),
    }
}

/// How a connection whose read failed ends: a message over the size limit is refused as
/// too big, which the WebSocket reports as a capacity error; anything else, such as a
/// connection reset or a broken frame, is the client's going.
pub(super) fn read_failed(error: &axum::Error) -> Ending {
    if error.to_string().contains("Message too long") {
        Ending::Closed(CloseReason::MessageTooBig)
    } else {
        Ending::ByClient
    }
}

/// Sends a close frame for `reason`, then waits a moment for the client's.
pub(super) async fn close(socket: &mut WebSocket, reason: CloseReason) {
    let frame = CloseFrame {
        code: reason.code(),
        reason: reason.message().into(),
    };
    if let Ok(Ok(())) = timeout(CLOSE_TIMEOUT, socket.send(Message::Close(Some(frame)))).await {
        finish(socket).await;
    }
}

/// Reads what the client still sends until it closes, for a moment, so the WebSocket
/// answers its close and the connection ends cleanly.
pub(super) async fn finish(socket: &mut WebSocket) {
    let _ = timeout(CLOSE_TIMEOUT, async {
        while let Some(Ok(_)) = socket.recv().await {}
    })
    .await;
}
