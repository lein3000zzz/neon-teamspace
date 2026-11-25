//! Real-time sync over real WebSockets: clients with documents of their own converging, a
//! viewer's change refused, the upgrade's checks, content kept across restarts and
//! compaction, access changes, an agent's owner's among them, ended sessions and tokens and
//! the trash's purge closing or limiting connections, and the limits.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::http::header::{COOKIE, ORIGIN};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use futures_util::{SinkExt as _, StreamExt as _};
use neon_app::crypto::Envelope;
use neon_app::sync::{Limits as SyncLimits, READ_ONLY};
use neon_blocks::{Block, SequentialIds};
use serde_json::json;
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{self, Message as Frame};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use yrs::sync::{Awareness, Message, SyncMessage};
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{Doc, ReadTxn as _, StateVector, Transact as _, Update, XmlFragmentRef};

use super::agents_tests::{Bearer, agent, agent_token};
use super::auth_tests::{admin, user};
use super::pages_tests::{create, general, guest};
use super::tests::state_for;
use super::workspaces_tests::{Browser, Team, id, team};
use super::*;

/// Long enough for any step here; reaching it means something never came.
const PATIENCE: Duration = Duration::from_secs(10);

/// Limits that log at once and never unload or ping while a test runs.
fn quick() -> SyncLimits {
    SyncLimits {
        persist_delay: Duration::from_millis(5),
        idle_unload: Duration::from_secs(3600),
        ..SyncLimits::default()
    }
}

async fn state_of(pool: PgPool, limits: SyncLimits) -> AppState {
    state_for(
        pool,
        Envelope::default(),
        WebConfig::new(None, Vec::new()),
        limits,
    )
    .await
}

/// The router of a state, listening on a free port of loopback.
pub(super) struct Server {
    pub(super) addr: SocketAddr,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Server {
    pub(super) async fn start(app: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("bound");
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let stop = stop.clone();
            async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(stop.cancelled_owned())
                    .await
                    .expect("serves");
            }
        });
        Self { addr, stop, task }
    }

    /// Stops listening, and waits until the server let go of its router.
    pub(super) async fn stop(mut self) {
        self.stop.cancel();
        timeout(PATIENCE, &mut self.task)
            .await
            .expect("the server stops")
            .expect("the server's task ends");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A test that failed: its runtime ends with it.
        self.task.abort();
    }
}

/// Ends what a test started: its clients' sockets, the live documents, which are saved,
/// and the servers. Nothing then holds the test's database pool, which its end closes:
/// a live document left running would keep the pool's connections from the other tests.
async fn finish(state: &AppState, servers: impl IntoIterator<Item = Server>, peers: Vec<Peer>) {
    drop(peers);
    state.sync.shutdown().await;
    for server in servers {
        server.stop().await;
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens a page's WebSocket with `headers`, or answers the status the upgrade got instead.
async fn open(
    addr: SocketAddr,
    page: &str,
    headers: &[(&str, &str)],
) -> Result<Socket, StatusCode> {
    let mut request = format!("ws://{addr}/ws/pages/{page}")
        .into_client_request()
        .expect("a request");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
            HeaderValue::from_str(value).expect("a header value"),
        );
    }
    match timeout(PATIENCE, tokio_tungstenite::connect_async(request))
        .await
        .expect("the handshake ends")
    {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response)) => Err(response.status()),
        Err(error) => panic!("the handshake failed: {error}"),
    }
}

/// What a peer saw of one message from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    /// The server's sync step 2: the peer has everything the server had.
    Synced,
    Updated,
    Awareness,
    /// An auth message: permission denied for this reason, or granted.
    Told(Option<String>),
    /// A close frame with this code.
    Closed(Option<u16>),
    /// The connection ended without a close frame.
    Ended,
    Other,
}

/// A client with a document of its own, as the browser's y-websocket provider is: it
/// answers the server's sync step 1, applies the server's updates and sends its own.
struct Peer {
    socket: Socket,
    doc: Doc,
    fragment: XmlFragmentRef,
    ids: SequentialIds,
}

impl Peer {
    /// Opens the page with a session cookie and syncs.
    async fn join(addr: SocketAddr, page: &str, cookie: &str, name: &str) -> Self {
        let socket = open(addr, page, &[(COOKIE.as_str(), cookie)])
            .await
            .expect("the upgrade works");
        let mut peer = Self::over(socket, name);
        peer.sync().await;
        peer
    }

    fn over(socket: Socket, name: &str) -> Self {
        let doc = Doc::new();
        let fragment = neon_blocks::fragment(&doc);
        Self {
            socket,
            doc,
            fragment,
            ids: SequentialIds::new(name),
        }
    }

    async fn send(&mut self, message: &Message) {
        self.socket
            .send(Frame::Binary(message.encode_v1().into()))
            .await
            .expect("sent");
    }

    /// Sends sync step 1 and reads until the server answers it.
    async fn sync(&mut self) {
        let state_vector = self.doc.transact().state_vector();
        self.send(&Message::Sync(SyncMessage::SyncStep1(state_vector)))
            .await;
        self.until("the server's sync step 2", |_, event| {
            *event == Event::Synced
        })
        .await;
    }

    /// Reads and handles what the server sends until `done` holds.
    async fn until(&mut self, what: &str, mut done: impl FnMut(&Doc, &Event) -> bool) -> Event {
        let waited = timeout(PATIENCE, async {
            loop {
                let event = self.read().await;
                if done(&self.doc, &event) {
                    return event;
                }
                assert!(
                    !matches!(event, Event::Closed(_) | Event::Ended),
                    "closed while waiting for {what}: {event:?}"
                );
            }
        })
        .await;
        waited.unwrap_or_else(|_| {
            panic!(
                "waited too long for {what}; the text is {:?}",
                text_of(&self.doc)
            )
        })
    }

    async fn read(&mut self) -> Event {
        match self.socket.next().await {
            Some(Ok(Frame::Binary(data))) => self.handle(&data).await,
            Some(Ok(Frame::Close(frame))) => Event::Closed(frame.map(|frame| frame.code.into())),
            Some(Ok(_)) => Event::Other,
            Some(Err(_)) | None => Event::Ended,
        }
    }

    async fn handle(&mut self, data: &[u8]) -> Event {
        match Message::decode_v1(data).expect("a y-sync message") {
            Message::Sync(SyncMessage::SyncStep1(state_vector)) => {
                let update = self.doc.transact().encode_state_as_update_v1(&state_vector);
                self.send(&Message::Sync(SyncMessage::SyncStep2(update)))
                    .await;
                Event::Other
            }
            Message::Sync(SyncMessage::SyncStep2(update)) => {
                self.apply(&update);
                Event::Synced
            }
            Message::Sync(SyncMessage::Update(update)) => {
                self.apply(&update);
                Event::Updated
            }
            Message::Auth(reason) => Event::Told(reason),
            Message::Awareness(_) => Event::Awareness,
            _ => Event::Other,
        }
    }

    fn apply(&self, update: &[u8]) {
        let update = Update::decode_v1(update).expect("an update");
        self.doc
            .transact_mut()
            .apply_update(update)
            .expect("applies");
    }

    /// Adds a paragraph, as typing in the editor does, and sends the update.
    async fn write(&mut self, text: &str) {
        let update = {
            let mut txn = self.doc.transact_mut();
            neon_blocks::append(
                &mut txn,
                &self.fragment,
                &[Block::paragraph(text)],
                &mut self.ids,
            )
            .expect("a document");
            txn.encode_update_v1()
        };
        self.send(&Message::Sync(SyncMessage::Update(update))).await;
    }

    /// Reads until the document's text is `text`, if it is not yet.
    async fn until_text(&mut self, text: &str) {
        if text_of(&self.doc) != text {
            self.until(&format!("the text {text:?}"), |doc, _| text_of(doc) == text)
                .await;
        }
    }

    /// Reads until the server closes the connection, and returns its close code.
    async fn closed(&mut self) -> Option<u16> {
        match self
            .until("the connection to close", |_, event| {
                matches!(event, Event::Closed(_) | Event::Ended)
            })
            .await
        {
            Event::Closed(code) => code,
            _ => None,
        }
    }
}

fn text_of(doc: &Doc) -> String {
    neon_blocks::read_doc(doc)
        .expect("the document reads as blocks")
        .plain_text()
}

/// The page's document as the database has it: its snapshot with the updates after it.
async fn saved(pool: &PgPool, page: Uuid) -> Doc {
    let doc = Doc::new();
    let snapshot = neon_store::documents::snapshot(pool, page)
        .await
        .expect("read");
    let after = snapshot.as_ref().map_or(0, |snapshot| snapshot.last_seq);
    let updates = neon_store::documents::updates_after(pool, page, after)
        .await
        .expect("read");
    {
        let mut txn = doc.transact_mut();
        for update in snapshot
            .map(|snapshot| snapshot.state)
            .into_iter()
            .chain(updates.into_iter().map(|logged| logged.update))
        {
            txn.apply_update(Update::decode_v1(&update).expect("decodes"))
                .expect("applies");
        }
    }
    doc
}

async fn logged_updates(pool: &PgPool, page: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM page_updates WHERE page_id = $1")
        .bind(page)
        .fetch_one(pool)
        .await
        .expect("counted")
}

async fn has_snapshot(pool: &PgPool, page: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM page_snapshots WHERE page_id = $1)")
        .bind(page)
        .fetch_one(pool)
        .await
        .expect("read")
}

/// Waits until `check` holds, polling.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    timeout(PATIENCE, async {
        while !check().await {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("waited too long for {what}"));
}

fn uuid(text: &str) -> Uuid {
    text.parse().expect("an id")
}

/// Ada's team, a page in General, and a server.
struct Room<'a> {
    team: Team<'a>,
    page: String,
    server: Server,
}

async fn room<'a>(state: &AppState, app: &'a Router) -> Room<'a> {
    let team = team(state, app).await;
    let general = general(&team.ada, &team.workspace).await;
    let page = create(
        &team.ada,
        &team.workspace,
        json!({"teamspace_id": general}),
        "Notes",
    )
    .await;
    let server = Server::start(app.clone()).await;
    Room { team, page, server }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn two_clients_converge_on_concurrent_edits(pool: PgPool) {
    let state = state_of(pool.clone(), quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    ada.write("Agenda").await;
    // Ada's update and Dee's join travel over separate sockets, so Dee's first sync may be
    // answered before the server applies the update; the update then follows.
    let mut dee = Peer::join(addr, &room.page, room.team.dee.cookie(), "dee").await;
    dee.until_text("Agenda").await;

    // Both type at once, each before seeing the other's change.
    ada.write("from Ada").await;
    dee.write("from Dee").await;
    ada.until("Dee's paragraph", |doc, _| {
        text_of(doc).contains("from Dee")
    })
    .await;
    dee.until("Ada's paragraph", |doc, _| {
        text_of(doc).contains("from Ada")
    })
    .await;

    let text = text_of(&ada.doc);
    assert_eq!(text, text_of(&dee.doc));
    let mut lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.remove(0), "Agenda");
    lines.sort_unstable();
    assert_eq!(lines, ["from Ada", "from Dee"]);
    assert_eq!(
        ada.doc.transact().state_vector(),
        dee.doc.transact().state_vector()
    );

    // The log has every change, each with its author.
    let page = uuid(&room.page);
    eventually("the updates to be logged", || async {
        text_of(&saved(&pool, page).await) == text
    })
    .await;
    let authors: Vec<Option<Uuid>> =
        sqlx::query_scalar("SELECT DISTINCT author_id FROM page_updates WHERE page_id = $1")
            .bind(page)
            .fetch_all(&pool)
            .await
            .expect("read");
    assert_eq!(authors.len(), 2, "{authors:?}");
    finish(&state, [room.server], vec![ada, dee]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_viewer_s_changes_are_refused(pool: PgPool) {
    let state = state_of(pool.clone(), quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let ws = &room.team.workspace;
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &room.team.ada, ws, "gus@example.com").await;
    let gus_id = gus.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let shared = room
        .team
        .ada
        .post(
            &format!("/api/v1/pages/{}/shares", room.page),
            json!({"user_id": gus_id, "access": "can_view"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let addr = room.server.addr;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    ada.write("Plan").await;

    // The viewer syncs, its answer to the server's sync step 1 changing nothing, and gets
    // the editor's changes as they come.
    let mut viewer = Peer::join(addr, &room.page, gus.cookie(), "gus").await;
    viewer.until_text("Plan").await;
    // Nor is all it holds, deletions included, as a client sends after reconnecting.
    let everything = viewer
        .doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    viewer
        .send(&Message::Sync(SyncMessage::SyncStep2(everything)))
        .await;
    ada.write("Budget").await;
    viewer.until_text("Plan\nBudget").await;

    viewer.write("Mine now").await;
    assert_eq!(viewer.closed().await, Some(4403));
    // A read-only token is refused alike, and nothing it sent reached the others.
    let token = room.team.ada.token(&["read"]).await;
    let socket = open(addr, &room.page, &[("authorization", &token)])
        .await
        .expect("a read token opens the page");
    let mut reader = Peer::over(socket, "token");
    reader.sync().await;
    reader.write("By token").await;
    assert_eq!(reader.closed().await, Some(4403));

    ada.write("Done").await;
    let mut late = Peer::join(addr, &room.page, room.team.dee.cookie(), "dee").await;
    late.until_text("Plan\nBudget\nDone").await;
    let page = uuid(&room.page);
    eventually("the updates to be logged", || async {
        text_of(&saved(&pool, page).await) == "Plan\nBudget\nDone"
    })
    .await;
    finish(&state, [room.server], vec![ada, viewer, reader, late]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn upgrades_need_the_origin_a_session_and_a_page_to_see(pool: PgPool) {
    let state = state_of(pool, quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let cookie = room.team.ada.cookie();
    let page = room.page.as_str();
    let own = format!("http://{addr}");

    // Cross-site WebSocket hijacking: a page elsewhere opening a socket with the cookie.
    for origin in ["https://evil.example", "null", "http://127.0.0.1:1"] {
        assert_eq!(
            open(
                addr,
                page,
                &[(COOKIE.as_str(), cookie), (ORIGIN.as_str(), origin)]
            )
            .await
            .err(),
            Some(StatusCode::FORBIDDEN),
            "{origin}"
        );
    }
    for headers in [
        vec![(COOKIE.as_str(), cookie), (ORIGIN.as_str(), own.as_str())],
        vec![(COOKIE.as_str(), cookie)],
    ] {
        let socket = open(addr, page, &headers).await.expect("opens");
        Peer::over(socket, "ada").sync().await;
    }
    assert_eq!(
        open(addr, page, &[]).await.err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        open(
            addr,
            &Uuid::now_v7().to_string(),
            &[(COOKIE.as_str(), cookie)]
        )
        .await
        .err(),
        Some(StatusCode::NOT_FOUND)
    );
    assert_eq!(
        open(addr, "not-an-id", &[(COOKIE.as_str(), cookie)])
            .await
            .err(),
        Some(StatusCode::BAD_REQUEST)
    );
    // Another member's private page is hidden, as over the API.
    let private = create(
        &room.team.ada,
        &room.team.workspace,
        json!({"private": true}),
        "Diary",
    )
    .await;
    assert_eq!(
        open(addr, &private, &[(COOKIE.as_str(), room.team.dee.cookie())])
            .await
            .err(),
        Some(StatusCode::NOT_FOUND)
    );
    // A plain GET is told to upgrade, in the error shape.
    let plain = room.team.ada.get(&format!("/ws/pages/{page}")).await;
    assert_eq!(plain.status, StatusCode::UPGRADE_REQUIRED);
    assert_eq!(plain.code(), "upgrade_required");
    finish(&state, [room.server], Vec::new()).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn content_survives_a_restart(pool: PgPool) {
    // Every second logged update compacts, so the log holds updates after a snapshot.
    let limits = SyncLimits {
        compact_updates: 2,
        ..quick()
    };
    let first = state_of(pool.clone(), limits).await;
    let app = router(first.clone(), false);
    let room = room(&first, &app).await;
    let page = uuid(&room.page);
    let cookie = room.team.ada.cookie().to_owned();
    let mut ada = Peer::join(room.server.addr, &room.page, &cookie, "ada").await;
    for (n, line) in ["One", "Two", "Three"].into_iter().enumerate() {
        ada.write(line).await;
        // One update logged at a time.
        let expected = n + 1;
        eventually("the update to be logged", || async {
            text_of(&saved(&pool, page).await).lines().count() == expected
        })
        .await;
    }
    assert!(has_snapshot(&pool, page).await);
    assert_eq!(logged_updates(&pool, page).await, 1);

    // A server that never saw the page loads it from the snapshot and the update after
    // it, as one would after the first died.
    let second = state_of(pool.clone(), limits).await;
    let second_server = Server::start(router(second.clone(), false)).await;
    let mut again = Peer::join(second_server.addr, &room.page, &cookie, "ada-2").await;
    assert_eq!(text_of(&again.doc), "One\nTwo\nThree");

    // Stopping saves the document, and closes its connections to reconnect.
    again.write("Four").await;
    eventually("the update to be logged", || async {
        text_of(&saved(&pool, page).await).ends_with("Four")
    })
    .await;
    second.sync.shutdown().await;
    assert_eq!(again.closed().await, Some(1012));
    assert_eq!(logged_updates(&pool, page).await, 0);
    let third = state_of(pool.clone(), limits).await;
    let third_server = Server::start(router(third.clone(), false)).await;
    let reloaded = Peer::join(third_server.addr, &room.page, &cookie, "ada-3").await;
    assert_eq!(text_of(&reloaded.doc), "One\nTwo\nThree\nFour");
    finish(&first, [room.server], vec![ada]).await;
    finish(&second, [second_server], vec![again]).await;
    finish(&third, [third_server], vec![reloaded]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn compaction_keeps_the_content_identical(pool: PgPool) {
    // Every logged update compacts.
    let limits = SyncLimits {
        compact_updates: 1,
        ..quick()
    };
    let state = state_of(pool.clone(), limits).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let page = uuid(&room.page);
    let mut ada = Peer::join(room.server.addr, &room.page, room.team.ada.cookie(), "ada").await;
    let mut dee = Peer::join(room.server.addr, &room.page, room.team.dee.cookie(), "dee").await;
    ada.write("Kickoff").await;
    dee.until_text("Kickoff").await;
    let deleted = {
        // Dee deletes the first block: the snapshot keeps deletions too.
        let mut txn = dee.doc.transact_mut();
        let document = neon_blocks::read(&txn, &dee.fragment).expect("blocks");
        let mut rest = document.clone();
        rest.blocks.clear();
        rest.blocks.push(Block::paragraph("Replaced"));
        neon_blocks::replace(&mut txn, &dee.fragment, &rest, &mut dee.ids);
        txn.encode_update_v1()
    };
    dee.send(&Message::Sync(SyncMessage::Update(deleted))).await;
    ada.until_text("Replaced").await;
    for line in ["Second", "Third"] {
        ada.write(line).await;
        dee.until(line, |doc, _| text_of(doc).ends_with(line)).await;
    }
    // Updates are logged in short batches, so the log can be empty while the last change is
    // still on its way: wait until the snapshot holds everything Ada wrote.
    let written = ada.doc.transact().state_vector();
    eventually("a snapshot to take every update in", || async {
        has_snapshot(&pool, page).await
            && logged_updates(&pool, page).await == 0
            && saved(&pool, page).await.transact().state_vector() == written
    })
    .await;

    // The snapshot is the document, to the last deletion: nothing is missing either way.
    let snapshot = saved(&pool, page).await;
    assert_eq!(text_of(&snapshot), "Replaced\nSecond\nThird");
    assert_eq!(
        neon_blocks::read_doc(&snapshot).expect("blocks"),
        neon_blocks::read_doc(&ada.doc).expect("blocks")
    );
    assert_eq!(
        snapshot.transact().state_vector(),
        ada.doc.transact().state_vector()
    );
    let missing = ada
        .doc
        .transact()
        .encode_state_as_update_v1(&snapshot.transact().state_vector());
    let missing = Update::decode_v1(&missing).expect("an update");
    assert!(missing.insertions(true).is_empty());
    assert_eq!(
        missing
            .delete_set()
            .diff(&snapshot.transact().snapshot().delete_set),
        yrs::IdSet::new()
    );
    // It records the page's text and its last editor.
    let (text, editor): (Option<String>, Option<Uuid>) =
        sqlx::query_as("SELECT search_text, last_edited_by FROM pages WHERE id = $1")
            .bind(page)
            .fetch_one(&pool)
            .await
            .expect("read");
    assert_eq!(text.as_deref(), Some("Replaced\nSecond\nThird"));
    let ada_id = room.team.ada.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .map(uuid);
    assert_eq!(editor, ada_id);
    finish(&state, [room.server], vec![ada, dee]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn access_changes_reach_open_connections_at_once(pool: PgPool) {
    let state = state_of(pool.clone(), quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let ws = &room.team.workspace;
    user(&state, "gus@example.com").await;
    let gus = guest(&app, &room.team.ada, ws, "gus@example.com").await;
    let gus_id = gus.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let share = format!("/api/v1/pages/{}/shares", room.page);
    let shared = room
        .team
        .ada
        .post(&share, json!({"user_id": gus_id, "access": "can_edit"}))
        .await;
    assert_eq!(shared.status, StatusCode::CREATED, "{}", shared.body);
    let addr = room.server.addr;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    let mut editor = Peer::join(addr, &room.page, gus.cookie(), "gus").await;
    editor.write("By Gus").await;
    ada.until_text("By Gus").await;

    // Down to viewing: the connection stays, read-only, and is told so.
    let changed = room
        .team
        .ada
        .send(
            Method::PATCH,
            &format!("{share}/{gus_id}"),
            Some(json!({"access": "can_view"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    editor
        .until("read-only", |_, event| {
            *event == Event::Told(Some(READ_ONLY.to_owned()))
        })
        .await;
    ada.write("By Ada").await;
    editor.until_text("By Gus\nBy Ada").await;
    // Back to editing, told as permission granted.
    let changed = room
        .team
        .ada
        .send(
            Method::PATCH,
            &format!("{share}/{gus_id}"),
            Some(json!({"access": "can_edit"})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    editor
        .until("edit access", |_, event| *event == Event::Told(None))
        .await;
    editor.write("Gus again").await;
    ada.until_text("By Gus\nBy Ada\nGus again").await;

    // The share removed, the guest may not see the page: the socket closes.
    let removed = room
        .team
        .ada
        .send(Method::DELETE, &format!("{share}/{gus_id}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert_eq!(editor.closed().await, Some(4404));
    assert_eq!(
        open(addr, &room.page, &[(COOKIE.as_str(), gus.cookie())])
            .await
            .err(),
        Some(StatusCode::NOT_FOUND)
    );

    // Trashing the page leaves its editors viewing it, and closes it to the rest.
    let mut dee = Peer::join(addr, &room.page, room.team.dee.cookie(), "dee").await;
    let trashed = room
        .team
        .ada
        .post(&format!("/api/v1/pages/{}/trash", room.page), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    for peer in [&mut ada, &mut dee] {
        peer.until("read-only", |_, event| {
            *event == Event::Told(Some(READ_ONLY.to_owned()))
        })
        .await;
    }
    // Dee leaving the workspace loses the page.
    let left = room
        .team
        .dee
        .post(&format!("/api/v1/workspaces/{ws}/leave"), json!({}))
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    assert_eq!(dee.closed().await, Some(4404));
    finish(&state, [room.server], vec![ada, editor, dee]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn connections_keep_to_their_limits(pool: PgPool) {
    let limits = SyncLimits {
        max_message: 2048,
        messages_per_second: 1,
        message_burst: 5,
        ..quick()
    };
    let state = state_of(pool, limits).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let cookie = room.team.ada.cookie().to_owned();
    let connect = || async {
        let socket = open(addr, &room.page, &[(COOKIE.as_str(), cookie.as_str())])
            .await
            .expect("opens");
        let mut peer = Peer::over(socket, "ada");
        peer.sync().await;
        peer
    };

    let mut big = connect().await;
    big.socket
        .send(Frame::Binary(vec![0; 4096].into()))
        .await
        .expect("sent");
    assert_eq!(big.closed().await, Some(1009));

    let mut text = connect().await;
    text.socket
        .send(Frame::Text("hello".into()))
        .await
        .expect("sent");
    assert_eq!(text.closed().await, Some(1003));

    let mut garbage = connect().await;
    garbage
        .socket
        .send(Frame::Binary(vec![0, 9, 9].into()))
        .await
        .expect("sent");
    assert_eq!(garbage.closed().await, Some(1007));

    // The burst is five messages, sync step 1 one of them.
    let mut chatty = connect().await;
    for _ in 0..8 {
        let awareness = Awareness::new(Doc::new()).update().expect("an update");
        if chatty
            .socket
            .send(Frame::Binary(
                Message::Awareness(awareness).encode_v1().into(),
            ))
            .await
            .is_err()
        {
            break;
        }
    }
    assert_eq!(chatty.closed().await, Some(4429));
    finish(&state, [room.server], vec![big, text, garbage, chatty]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn clients_that_stop_answering_are_dropped(pool: PgPool) {
    let limits = SyncLimits {
        ping_interval: Duration::from_millis(100),
        ..quick()
    };
    let state = state_of(pool, limits).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let mut peer = Peer::join(room.server.addr, &room.page, room.team.ada.cookie(), "ada").await;

    // Reading answers pings; a client that reads nothing answers none.
    sleep(Duration::from_millis(500)).await;
    let mut pings = 0;
    let ending = loop {
        match peer.socket.next().await {
            Some(Ok(Frame::Ping(_))) => pings += 1,
            Some(Ok(Frame::Close(frame))) => break Some(frame),
            Some(Ok(_)) => {}
            Some(Err(_)) | None => break None,
        }
    };
    assert!(pings >= 1, "the server pinged");
    assert_eq!(ending, None, "dropped without a close");
    finish(&state, [room.server], vec![peer]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_opened_by_a_bearer_token_follows_its_scope(pool: PgPool) {
    let state = state_of(pool.clone(), quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let token = room.team.ada.token(&["write"]).await;
    let socket = open(addr, &room.page, &[("authorization", &token)])
        .await
        .expect("a write token opens the page");
    let mut agent = Peer::over(socket, "agent");
    agent.sync().await;
    agent.write("From an agent").await;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    ada.until_text("From an agent").await;
    assert_eq!(
        open(addr, &room.page, &[("authorization", "Bearer nw_unknown")])
            .await
            .err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    finish(&state, [room.server], vec![agent, ada]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_s_connections_follow_its_owner_s_access(pool: PgPool) {
    let state = state_of(pool, quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let ws = &room.team.workspace;
    let addr = room.server.addr;
    let scout = agent(&room.team.dee, ws, "Scout").await;
    let worker = Bearer::new(
        &app,
        agent_token(&room.team.dee, ws, &scout, &["write"]).await,
    );

    // In General, where both are, it edits.
    let socket = open(addr, &room.page, &[("authorization", worker.header())])
        .await
        .expect("its token opens the page");
    let mut scout_here = Peer::over(socket, "scout");
    scout_here.sync().await;
    scout_here.write("By Scout").await;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    ada.until_text("By Scout").await;

    // In Ops, which both joined, it edits until Dee leaves Ops: then it views.
    let ops = room
        .team
        .cy
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces"),
            json!({"name": "Ops", "access": "open"}),
        )
        .await;
    assert_eq!(ops.status, StatusCode::CREATED, "{}", ops.body);
    let ops = id(&ops);
    let plan = create(&room.team.cy, ws, json!({"teamspace_id": ops}), "Plan").await;
    for joined in [
        room.team
            .dee
            .post(
                &format!("/api/v1/workspaces/{ws}/teamspaces/{ops}/join"),
                json!({}),
            )
            .await,
        worker
            .post(
                &format!("/api/v1/workspaces/{ws}/teamspaces/{ops}/join"),
                json!({}),
            )
            .await,
    ] {
        assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    }
    let socket = open(addr, &plan, &[("authorization", worker.header())])
        .await
        .expect("its token opens the plan");
    let mut scout_ops = Peer::over(socket, "scout-ops");
    scout_ops.sync().await;
    let left = room
        .team
        .dee
        .post(
            &format!("/api/v1/workspaces/{ws}/teamspaces/{ops}/leave"),
            json!({}),
        )
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    scout_ops
        .until("read-only", |_, event| {
            *event == Event::Told(Some(READ_ONLY.to_owned()))
        })
        .await;

    // Dee leaves the workspace: the agent is suspended, and its connections close.
    let left = room
        .team
        .dee
        .post(&format!("/api/v1/workspaces/{ws}/leave"), json!({}))
        .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    assert_eq!(scout_here.closed().await, Some(4401));
    assert_eq!(scout_ops.closed().await, Some(4401));
    assert_eq!(
        open(addr, &room.page, &[("authorization", worker.header())])
            .await
            .err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    finish(&state, [room.server], vec![ada, scout_here, scout_ops]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn turning_agents_off_closes_their_connections(pool: PgPool) {
    let state = state_of(pool, quick()).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let ws = &room.team.workspace;
    let addr = room.server.addr;
    let scout = agent(&room.team.cy, ws, "Scout").await;
    let worker = Bearer::new(
        &app,
        agent_token(&room.team.cy, ws, &scout, &["write"]).await,
    );
    let socket = open(addr, &room.page, &[("authorization", worker.header())])
        .await
        .expect("its token opens the page");
    let mut scout = Peer::over(socket, "scout");
    scout.sync().await;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;

    let root = Browser::sign_in(&app, "admin@example.com").await;
    let off = root
        .send(
            Method::PATCH,
            "/api/v1/admin/settings",
            Some(json!({"changes": {"mcp.identities": ["user"]}})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(scout.closed().await, Some(4401));
    // People's connections stay.
    ada.write("Still here").await;
    let mut cy = Peer::join(addr, &room.page, room.team.cy.cookie(), "cy").await;
    cy.until_text("Still here").await;
    finish(&state, [room.server], vec![scout, ada, cy]).await;
}

/// The id of the account a browser is signed in to.
async fn me(browser: &Browser<'_>) -> String {
    browser.get("/api/v1/account/me").await.body["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn ended_sessions_and_tokens_close_their_connections(pool: PgPool) {
    let state = state_of(pool, quick()).await;
    admin(&state).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;

    // Signing out closes the session's connections, and only them.
    let bob = Browser::sign_in(&app, "bob@example.com").await;
    let mut signed_out = Peer::join(addr, &room.page, bob.cookie(), "bob").await;
    let mut bob_elsewhere = Peer::join(addr, &room.page, room.team.bob.cookie(), "bob-2").await;
    let out = bob.post("/api/v1/auth/logout", json!({})).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT, "{}", out.body);
    assert_eq!(signed_out.closed().await, Some(4401));

    // Revoking a token closes what it opened.
    let token = room.team.ada.token(&["write"]).await;
    let socket = open(addr, &room.page, &[("authorization", &token)])
        .await
        .expect("a write token opens the page");
    let mut agent = Peer::over(socket, "agent");
    agent.sync().await;
    let tokens = room.team.ada.get("/api/v1/account/tokens").await;
    let token_id = tokens.body["tokens"][0]["id"].as_str().expect("an id");
    let revoked = room
        .team
        .ada
        .send(
            Method::DELETE,
            &format!("/api/v1/account/tokens/{token_id}"),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT, "{}", revoked.body);
    assert_eq!(agent.closed().await, Some(4401));

    // An admin disabling an account closes every connection of it.
    let mut dee = Peer::join(addr, &room.page, room.team.dee.cookie(), "dee").await;
    let dee_id = me(&room.team.dee).await;
    let admin = Browser::sign_in(&app, "admin@example.com").await;
    let disabled = admin
        .send(
            Method::PATCH,
            &format!("/api/v1/admin/users/{dee_id}"),
            Some(json!({"disabled": true})),
        )
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.body);
    assert_eq!(dee.closed().await, Some(4401));

    // The others' connections stay open, and work.
    bob_elsewhere.write("Still here").await;
    ada.until_text("Still here").await;
    finish(
        &state,
        [room.server],
        vec![ada, signed_out, bob_elsewhere, agent, dee],
    )
    .await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn connections_are_looked_at_again_now_and_then(pool: PgPool) {
    let limits = SyncLimits {
        revalidate_interval: Duration::from_millis(100),
        ..quick()
    };
    let state = state_of(pool.clone(), limits).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let mut dee = Peer::join(room.server.addr, &room.page, room.team.dee.cookie(), "dee").await;

    // Ended without telling anyone, as when it expires, the session's connection closes
    // at the next look.
    let dee_id = uuid(&me(&room.team.dee).await);
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(dee_id)
        .execute(&pool)
        .await
        .expect("deleted");
    assert_eq!(dee.closed().await, Some(4401));
    finish(&state, [room.server], vec![dee]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn purging_the_trash_closes_its_pages(pool: PgPool) {
    let state = state_of(pool.clone(), quick()).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let mut ada = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;
    let trashed = room
        .team
        .ada
        .post(&format!("/api/v1/pages/{}/trash", room.page), json!({}))
        .await;
    assert_eq!(trashed.status, StatusCode::OK, "{}", trashed.body);
    ada.until("read-only", |_, event| {
        *event == Event::Told(Some(READ_ONLY.to_owned()))
    })
    .await;

    // Long enough in the trash, the page goes for good, and so does its connection.
    sqlx::query("UPDATE pages SET trashed_at = trashed_at - interval '400 days' WHERE id = $1")
        .bind(uuid(&room.page))
        .execute(&pool)
        .await
        .expect("aged");
    assert_eq!(state.pages.purge_trash().await.expect("purged"), 1);
    assert_eq!(ada.closed().await, Some(4404));
    finish(&state, [room.server], vec![ada]).await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn one_connection_too_many_is_refused_before_its_upgrade(pool: PgPool) {
    let limits = SyncLimits {
        connections_per_user: 1,
        ..quick()
    };
    let state = state_of(pool, limits).await;
    let app = router(state.clone(), false);
    let room = room(&state, &app).await;
    let addr = room.server.addr;
    let cookie = [(COOKIE.as_str(), room.team.ada.cookie())];
    let first = Peer::join(addr, &room.page, room.team.ada.cookie(), "ada").await;

    assert_eq!(
        open(addr, &room.page, &cookie).await.err(),
        Some(StatusCode::TOO_MANY_REQUESTS)
    );
    // Another person still opens the page, and Ada again once her first is closed.
    let dee = Peer::join(addr, &room.page, room.team.dee.cookie(), "dee").await;
    drop(first);
    let mut again = None;
    for _ in 0..200 {
        if let Ok(socket) = open(addr, &room.page, &cookie).await {
            again = Some(Peer::over(socket, "ada-2"));
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let mut again = again.expect("room for Ada's next connection");
    again.sync().await;
    finish(&state, [room.server], vec![dee, again]).await;
}
