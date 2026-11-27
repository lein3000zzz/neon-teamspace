//! Live documents, through their connections without a socket: clients that fall behind,
//! awareness, unloading, the size limit, batched logging, a page deleted under its
//! document, the limits on sync requests, awareness and connections, updates out of
//! order, and the metrics.

mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::pages::{Location, NewPage, PageError};
use neon_app::ratelimit::{Bucket, Limited, MemoryLimiter, RateLimiter};
use neon_app::sync::{
    CloseReason, Connection, ContentError, Ending, Limits, Next, SyncService, access_feed,
};
use neon_blocks::edit::{Edit, EditError};
use neon_blocks::markdown::from_markdown;
use neon_blocks::{Block, SequentialIds};
use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, Placement};
use neon_domain::workspaces::WorkspaceRole;
use sqlx::PgPool;
use support::Harness;
use tokio::time::{sleep, timeout};
use uuid::Uuid;
use yrs::block::ClientID;
use yrs::sync::{Awareness, AwarenessUpdate, Message, SyncMessage};
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{
    Doc, ReadTxn as _, StateVector, Transact as _, Update, XmlFragment as _, XmlFragmentRef, XmlOut,
};

/// Long enough for any step here; reaching it means something never came.
const PATIENCE: Duration = Duration::from_secs(10);

/// Ada owns Design, with Dee as a member; Notes is a page in General.
struct Team {
    h: Harness,
    ada: Principal,
    dee: Principal,
    page: Uuid,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let dee = h.person("dee@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, workspace, &dee, WorkspaceRole::Member)
        .await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, workspace)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let page = h
        .pages
        .create_page(
            &ada,
            workspace,
            Location::Teamspace(general),
            Placement::Last,
            NewPage::default(),
        )
        .await
        .expect("created")
        .page
        .id;
    Team { h, ada, dee, page }
}

impl Team {
    fn sync(&self, limits: Limits) -> SyncService {
        self.sync_counting(limits, Arc::new(MemoryLimiter::new()))
    }

    /// The service, counting edits in `limiter`.
    fn sync_counting(&self, limits: Limits, limiter: Arc<dyn RateLimiter>) -> SyncService {
        let (_, feed) = access_feed();
        SyncService::builder(
            self.h.pool.clone(),
            self.h.pages.clone(),
            self.h.auth.clone(),
            feed,
        )
        .limits(limits)
        .limiter(limiter)
        .build()
    }
}

/// Logs at once and keeps documents loaded.
fn quick() -> Limits {
    Limits {
        persist_delay: Duration::from_millis(5),
        idle_unload: Duration::from_secs(3600),
        ..Limits::default()
    }
}

/// A client with a document of its own, talking to a connection directly.
struct Client {
    connection: Connection,
    doc: Doc,
    fragment: XmlFragmentRef,
    ids: SequentialIds,
}

impl Client {
    async fn open(sync: &SyncService, who: &Principal, page: Uuid, name: &str) -> Self {
        let admission = sync.admit(who.clone(), page).await.expect("admitted");
        let connection = sync.connect(admission).await.expect("connected");
        let doc = Doc::new();
        let fragment = neon_blocks::fragment(&doc);
        let mut client = Self {
            connection,
            doc,
            fragment,
            ids: SequentialIds::new(name),
        };
        let state_vector = client.doc.transact().state_vector();
        client
            .send(&Message::Sync(SyncMessage::SyncStep1(state_vector)))
            .await
            .expect("sent");
        client
            .until("the server's sync step 2", |message| {
                matches!(message, Message::Sync(SyncMessage::SyncStep2(_)))
            })
            .await;
        client
    }

    async fn send(&mut self, message: &Message) -> Result<(), CloseReason> {
        self.connection
            .receive(Bytes::from(message.encode_v1()))
            .await
    }

    /// The next message from the server, applied, or why the connection closed.
    async fn next(&mut self) -> Result<Message, CloseReason> {
        let next = timeout(PATIENCE, self.connection.next())
            .await
            .expect("something comes");
        let data = match next {
            Next::Send(data) => data,
            Next::Close(reason) => return Err(reason),
        };
        let message = Message::decode_v1(&data).expect("a y-sync message");
        match &message {
            Message::Sync(SyncMessage::SyncStep1(state_vector)) => {
                let update = self.doc.transact().encode_state_as_update_v1(state_vector);
                self.send(&Message::Sync(SyncMessage::SyncStep2(update)))
                    .await
                    .expect("sent");
            }
            Message::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) => {
                self.doc
                    .transact_mut()
                    .apply_update(Update::decode_v1(update).expect("an update"))
                    .expect("applies");
            }
            _ => {}
        }
        Ok(message)
    }

    async fn until(&mut self, what: &str, mut done: impl FnMut(&Message) -> bool) -> Message {
        loop {
            let message = self
                .next()
                .await
                .unwrap_or_else(|reason| panic!("closed waiting for {what}: {reason}"));
            if done(&message) {
                return message;
            }
        }
    }

    /// The messages waiting for the client, until none comes for a moment. Sync step 2
    /// messages are applied.
    async fn waiting(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();
        while let Ok(Next::Send(data)) =
            timeout(Duration::from_millis(200), self.connection.next()).await
        {
            let message = Message::decode_v1(&data).expect("a y-sync message");
            if let Message::Sync(SyncMessage::SyncStep2(update)) = &message {
                self.doc
                    .transact_mut()
                    .apply_update(Update::decode_v1(update).expect("an update"))
                    .expect("applies");
            }
            messages.push(message);
        }
        messages
    }

    async fn closed(&mut self) -> CloseReason {
        loop {
            if let Err(reason) = self.next().await {
                return reason;
            }
        }
    }

    /// A paragraph added, as the update that adds it.
    fn paragraph(&mut self, text: &str) -> Message {
        let mut txn = self.doc.transact_mut();
        neon_blocks::append(
            &mut txn,
            &self.fragment,
            &[Block::paragraph(text)],
            &mut self.ids,
        )
        .expect("a document");
        Message::Sync(SyncMessage::Update(txn.encode_update_v1()))
    }

    /// Text typed at the end of the last block, as the update that types it.
    fn type_last(&mut self, text: &str) -> Message {
        use yrs::{Text as _, XmlFragment as _, XmlOut, XmlTextPrelim};
        let mut txn = self.doc.transact_mut();
        let Some(XmlOut::Element(group)) = self.fragment.first_child() else {
            panic!("a document");
        };
        let Some(XmlOut::Element(last)) = group.get(&txn, group.len(&txn) - 1) else {
            panic!("a block");
        };
        let Some(XmlOut::Element(content)) = last.first_child() else {
            panic!("its content");
        };
        match content.first_child() {
            Some(XmlOut::Text(run)) => {
                let end = run.len(&txn);
                run.insert(&mut txn, end, text);
            }
            _ => {
                content.push_back(&mut txn, XmlTextPrelim::new(text));
            }
        }
        Message::Sync(SyncMessage::Update(txn.encode_update_v1()))
    }

    async fn write(&mut self, text: &str) -> Result<(), CloseReason> {
        let update = self.paragraph(text);
        self.send(&update).await
    }

    fn text(&self) -> String {
        neon_blocks::read_doc(&self.doc)
            .expect("blocks")
            .plain_text()
    }
}

async fn rows(pool: &PgPool, page: Uuid) -> Vec<(Option<Uuid>, i64)> {
    sqlx::query_as(
        "SELECT author_id, length(\"update\")::bigint FROM page_updates WHERE page_id = $1
         ORDER BY seq",
    )
    .bind(page)
    .fetch_all(pool)
    .await
    .expect("read")
}

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

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_client_that_falls_behind_is_disconnected(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        client_queue: 4,
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    // Dee opens the page and reads nothing more.
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;

    for n in 0..10 {
        ada.write(&format!("line {n}")).await.expect("taken");
    }
    // Ada is not held up, and goes on.
    let mut late = Client::open(&sync, &t.ada, t.page, "late").await;
    assert_eq!(late.text().lines().count(), 10);
    loop {
        match dee.next().await {
            Ok(_) => {}
            Err(reason) => {
                assert_eq!(reason, CloseReason::TooSlow);
                break;
            }
        }
    }
    dee.connection
        .leave(Ending::Closed(CloseReason::TooSlow))
        .await;
    ada.write("after").await.expect("taken");
    late.until("Ada's last line", |_| true).await;
    assert!(late.text().ends_with("after"));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn awareness_reaches_everyone_and_leaves_with_its_client(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    let mut ada_awareness = Awareness::new(Doc::with_client_id(11));
    ada_awareness
        .set_local_state(serde_json::json!({"user": {"name": "Ada"}, "cursor": null}))
        .expect("set");
    let presence = Message::Awareness(ada_awareness.update().expect("an update"));
    ada.send(&presence).await.expect("taken");

    // Everyone hears it, Ada too, which keeps her client's connection alive.
    for client in [&mut ada, &mut dee] {
        let heard = client
            .until("Ada's presence", |message| {
                matches!(message, Message::Awareness(_))
            })
            .await;
        let Message::Awareness(update) = heard else {
            unreachable!()
        };
        assert_eq!(update.clients.len(), 1);
        assert_eq!(states_of(&update), [presence_of(&t.ada)]);
    }
    // A newcomer gets everyone's state with its sync.
    let admission = sync.admit(t.dee.clone(), t.page).await.expect("admitted");
    let mut newcomer = sync.connect(admission).await.expect("connected");
    let _step1 = newcomer.next().await;
    let Next::Send(states) = newcomer.next().await else {
        panic!("no awareness")
    };
    let Message::Awareness(states) = Message::decode_v1(&states).expect("decodes") else {
        panic!("not awareness")
    };
    assert_eq!(states.clients.len(), 1);
    newcomer.leave(Ending::ByClient).await;

    // Dee cannot speak for Ada's cursor.
    let mut forged = Awareness::new(Doc::with_client_id(11));
    for _ in 0..3 {
        forged
            .set_local_state(serde_json::json!({"user": {"name": "Not Ada"}}))
            .expect("set");
    }
    dee.send(&Message::Awareness(forged.update().expect("an update")))
        .await
        .expect("taken");
    dee.send(&Message::AwarenessQuery).await.expect("taken");
    let answer = dee
        .until("the states", |message| {
            matches!(message, Message::Awareness(_))
        })
        .await;
    let Message::Awareness(answer) = answer else {
        unreachable!()
    };
    assert_eq!(states_of(&answer), [presence_of(&t.ada)]);

    // Ada leaving takes her presence with her.
    ada.connection.leave(Ending::ByClient).await;
    let gone = dee
        .until("Ada's presence to go", |message| {
            matches!(message, Message::Awareness(_))
        })
        .await;
    let Message::Awareness(update) = gone else {
        unreachable!()
    };
    assert!(update.clients.values().all(|entry| &*entry.json == "null"));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn awareness_says_who_each_connection_is(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    // Dee claims to be Ada, in a colour that would style others' editors.
    let mut claimed = Awareness::new(Doc::with_client_id(21));
    claimed
        .set_local_state(serde_json::json!({
            "user": {
                "id": t.ada.user.id.to_string(),
                "name": "Ada",
                "color": "#000000; position: fixed; inset: 0",
            },
            "cursor": null,
        }))
        .expect("set");
    dee.send(&Message::Awareness(claimed.update().expect("an update")))
        .await
        .expect("taken");

    let heard = ada
        .until("Dee's presence", |message| {
            matches!(message, Message::Awareness(_))
        })
        .await;
    let Message::Awareness(update) = heard else {
        unreachable!()
    };
    assert_eq!(states_of(&update), [presence_of(&t.dee)]);
    // A cursor leaving, a state of null, stays null.
    claimed.clean_local_state();
    let leaving = claimed
        .update_with_clients([ClientID::new(21)])
        .expect("an update");
    dee.send(&Message::Awareness(leaving)).await.expect("taken");
    let left = ada
        .until("Dee's cursor to go", |message| {
            matches!(message, Message::Awareness(_))
        })
        .await;
    let Message::Awareness(update) = left else {
        unreachable!()
    };
    assert!(update.clients.values().all(|entry| &*entry.json == "null"));
    sync.shutdown().await;
}

/// The states an awareness update holds, read as JSON.
fn states_of(update: &AwarenessUpdate) -> Vec<serde_json::Value> {
    update
        .clients
        .values()
        .map(|entry| serde_json::from_str(&entry.json).expect("JSON"))
        .collect()
}

/// An awareness state as the server passes one on for `who`, a person, with no cursor.
fn presence_of(who: &Principal) -> serde_json::Value {
    serde_json::json!({
        "user": {
            "id": who.user.id.to_string(),
            "name": who.user.display_name,
            "hue": who.user.hue(),
            "kind": "human",
        },
        "cursor": null,
    })
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_s_presence_says_what_it_is_and_whom_it_acts_for(pool: PgPool) {
    let t = team(pool).await;
    let workspace =
        t.h.workspaces
            .list_workspaces(&t.ada)
            .await
            .expect("listed")[0]
            .0
            .id;
    let scout = t.h.agent(&t.ada, workspace, "Scout").await;
    let (_, worker) = t.h.agent_token(&t.ada, &scout, &["write"]).await;
    let sync = t.sync(quick());
    let mut agent = Client::open(&sync, &worker, t.page, "scout").await;
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    // It claims to be a person.
    let mut claimed = Awareness::new(Doc::with_client_id(31));
    claimed
        .set_local_state(serde_json::json!({
            "user": {"name": "Ada", "kind": "human"},
            "cursor": null,
        }))
        .expect("set");
    agent
        .send(&Message::Awareness(claimed.update().expect("an update")))
        .await
        .expect("taken");

    let heard = dee
        .until("Scout's presence", |message| {
            matches!(message, Message::Awareness(_))
        })
        .await;
    let Message::Awareness(update) = heard else {
        unreachable!()
    };
    assert_eq!(
        states_of(&update),
        [serde_json::json!({
            "user": {
                "id": scout.id.to_string(),
                "name": "Scout",
                "hue": scout.hue(),
                "kind": "agent",
                "owner": t.ada.user.display_name,
            },
            "cursor": null,
        })]
    );
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn idle_documents_are_saved_and_unload(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let mut metrics = Metrics::new(recorder.snapshotter());
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let t = team(pool.clone()).await;
    let sync = t.sync(Limits {
        idle_unload: Duration::from_millis(50),
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    assert_eq!(metrics.gauge("neon_sync_open_connections"), 1.0);
    assert_eq!(metrics.gauge("neon_sync_live_documents"), 1.0);
    ada.write("Kept").await.expect("taken");
    eventually("the update to be logged", || async {
        !rows(&pool, t.page).await.is_empty()
    })
    .await;
    ada.connection.leave(Ending::ByClient).await;

    timeout(PATIENCE, async {
        while metrics.gauge("neon_sync_live_documents") != 0.0 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the document unloads");
    assert!(
        rows(&pool, t.page).await.is_empty(),
        "the snapshot took them in"
    );
    assert_eq!(metrics.gauge("neon_sync_open_connections"), 0.0);
    assert_eq!(metrics.red("load", "ok"), 1);
    assert_eq!(metrics.red("persist", "ok"), 1);
    assert_eq!(metrics.red("compact", "ok"), 1);
    assert_eq!(
        metrics.counter("neon_sync_disconnects_total", "cause", "client"),
        1
    );

    let again = Client::open(&sync, &t.dee, t.page, "dee").await;
    assert_eq!(again.text(), "Kept");
    assert_eq!(metrics.red("load", "ok"), 2);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn documents_keep_to_their_size_limit(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        max_document: 4096,
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    let line = "x".repeat(200);
    let mut refused = None;
    for _ in 0..100 {
        if let Err(reason) = ada.write(&line).await {
            refused = Some(reason);
            break;
        }
        // Let the document close the connection before the next write, once it is full.
        sleep(Duration::from_millis(5)).await;
    }
    let reason = match refused {
        Some(reason) => reason,
        None => ada.closed().await,
    };
    assert_eq!(reason, CloseReason::DocumentTooLarge);

    // What was taken is kept, and only deleting is still allowed.
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    let lines = dee.text().lines().count();
    assert!(lines > 1, "{lines}");
    let deletion = {
        let mut txn = dee.doc.transact_mut();
        let Some(XmlOut::Element(group)) = dee.fragment.first_child() else {
            panic!("no block group");
        };
        let blocks = group.len(&txn);
        group.remove_range(&mut txn, 1, blocks - 1);
        txn.encode_update_v1()
    };
    let only_deletes = Update::decode_v1(&deletion).expect("an update");
    assert!(only_deletes.insertions(true).is_empty());
    dee.send(&Message::Sync(SyncMessage::Update(deletion)))
        .await
        .expect("taken");
    let mut check = Client::open(&sync, &t.ada, t.page, "check").await;
    assert_eq!(check.text().lines().count(), 1);
    // Saved again, the document is small, and takes more.
    check.write(&line).await.expect("taken");
    dee.until("the new line", |message| {
        matches!(message, Message::Sync(SyncMessage::Update(_)))
    })
    .await;
    assert_eq!(dee.text().lines().count(), 2);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn quick_updates_are_logged_together_with_their_author(pool: PgPool) {
    let t = team(pool.clone()).await;
    let sync = t.sync(Limits {
        persist_delay: Duration::from_millis(300),
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    for word in ["one", "two", "three"] {
        ada.write(word).await.expect("taken");
    }
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    dee.write("four").await.expect("taken");
    eventually("both authors' updates to be logged", || async {
        rows(&pool, t.page).await.len() == 2
    })
    .await;

    let logged = rows(&pool, t.page).await;
    let authors: Vec<Option<Uuid>> = logged.iter().map(|(author, _)| *author).collect();
    assert_eq!(authors, [Some(t.ada.user.id), Some(t.dee.user.id)]);
    sync.shutdown().await;
    assert!(rows(&pool, t.page).await.is_empty());
    let text: Option<String> = sqlx::query_scalar("SELECT search_text FROM pages WHERE id = $1")
        .bind(t.page)
        .fetch_one(&pool)
        .await
        .expect("read");
    assert_eq!(text.as_deref(), Some("one\ntwo\nthree\nfour"));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_deleted_under_its_document_closes_it(pool: PgPool) {
    let t = team(pool.clone()).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    ada.write("Soon gone").await.expect("taken");
    t.h.pages.trash_page(&t.ada, t.page).await.expect("trashed");
    t.h.pages
        .delete_page(&t.ada, t.page)
        .await
        .expect("deleted");

    // The delete's access change may close the document before the next change arrives;
    // if it has not yet, logging that change finds the page gone. Either way the socket
    // ends with AccessLost.
    let reason = match ada.write("Too late").await {
        Ok(()) => ada.closed().await,
        Err(reason) => reason,
    };
    assert_eq!(reason, CloseReason::AccessLost);
    let admission = sync.admit(t.ada.clone(), t.page).await;
    assert!(admission.is_err());
    assert!(rows(&pool, t.page).await.is_empty());
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_client_that_falls_behind_by_bytes_is_disconnected(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        client_queue_bytes: 8 * 1024,
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;

    // Few messages, many bytes: Dee reads none of them.
    for n in 0..8 {
        ada.write(&format!("{n}{}", "y".repeat(2000)))
            .await
            .expect("taken");
    }
    assert_eq!(dee.closed().await, CloseReason::TooSlow);
    let late = Client::open(&sync, &t.ada, t.page, "late").await;
    assert_eq!(late.text().lines().count(), 8);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sync_requests_get_one_answer_at_a_time(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        sync_request_burst: 100,
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    ada.write(&"x".repeat(10_000)).await.expect("taken");
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;

    // Dee asks for the whole document again and again, and reads nothing.
    let whole = Message::Sync(SyncMessage::SyncStep1(StateVector::default()));
    for _ in 0..50 {
        dee.send(&whole).await.expect("taken");
    }
    // What the document does in turn, it did by the time it answers a read.
    sync.content(t.page).await.expect("read");
    let answers = |messages: &[Message]| {
        messages
            .iter()
            .filter(|message| matches!(message, Message::Sync(SyncMessage::SyncStep2(_))))
            .count()
    };
    assert_eq!(answers(&dee.waiting().await), 1, "one answer waits, not 50");
    // Once that answer is sent, the next request has one of its own.
    dee.send(&whole).await.expect("taken");
    sync.content(t.page).await.expect("read");
    assert_eq!(answers(&dee.waiting().await), 1);
    assert!(dee.text().starts_with('x'));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sync_requests_past_their_rate_close_the_connection(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        sync_requests_per_second: 1,
        sync_request_burst: 3,
        ..quick()
    });
    // Opening sent the first.
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    let whole = Message::Sync(SyncMessage::SyncStep1(StateVector::default()));
    for _ in 0..3 {
        dee.send(&whole).await.expect("taken");
    }
    assert_eq!(dee.closed().await, CloseReason::TooManyMessages);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_update_needing_changes_the_document_lacks_is_refused(pool: PgPool) {
    let t = team(pool.clone()).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    // The second paragraph's update needs the first's, which never comes.
    let _withheld = ada.paragraph("first");
    let second = ada.paragraph(&"z".repeat(5000));
    ada.send(&second).await.expect("taken");
    assert_eq!(ada.closed().await, CloseReason::OutOfOrder);

    // The document kept none of it, nor does the one saved.
    let dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    assert_eq!(dee.text(), "");
    let everything = dee
        .doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    assert!(everything.len() < 64, "{} bytes", everything.len());
    sync.shutdown().await;
    assert!(rows(&pool, t.page).await.is_empty());
    let snapshot = neon_store::documents::snapshot(&pool, t.page)
        .await
        .expect("read");
    assert!(snapshot.is_none_or(|snapshot| snapshot.state.len() < 64));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn awareness_states_keep_to_their_size_and_rate(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        max_awareness_state: 256,
        awareness_per_second: 1,
        awareness_burst: 2,
        ..quick()
    });
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    let mut cursor = Awareness::new(Doc::with_client_id(11));

    // Past the rate, presence is dropped, not passed on, and the connection stays.
    for n in 0..5 {
        cursor
            .set_local_state(serde_json::json!({"user": {"name": format!("Ada {n}")}}))
            .expect("set");
        ada.send(&Message::Awareness(cursor.update().expect("an update")))
            .await
            .expect("taken");
    }
    sync.content(t.page).await.expect("read");
    let heard = dee
        .waiting()
        .await
        .into_iter()
        .filter(|message| matches!(message, Message::Awareness(_)))
        .count();
    assert_eq!(heard, 2);
    // A state past its size closes the connection, as too big.
    cursor
        .set_local_state(serde_json::json!({"user": {"name": "A".repeat(300)}}))
        .expect("set");
    ada.send(&Message::Awareness(cursor.update().expect("an update")))
        .await
        .expect("taken");
    assert_eq!(ada.closed().await, CloseReason::MessageTooBig);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn connections_are_limited_per_person_and_per_page(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        connections_per_user: 2,
        connections_per_page: 3,
        ..quick()
    });
    let first = Client::open(&sync, &t.ada, t.page, "ada-1").await;
    let _second = Client::open(&sync, &t.ada, t.page, "ada-2").await;

    let ada = sync.admit(t.ada.clone(), t.page).await.expect("admitted");
    assert!(!sync.has_room(&ada), "Ada has two open");
    assert_eq!(
        sync.connect(ada).await.err(),
        Some(CloseReason::TooManyConnections)
    );
    let _dee = Client::open(&sync, &t.dee, t.page, "dee-1").await;
    let dee = sync.admit(t.dee.clone(), t.page).await.expect("admitted");
    assert!(!sync.has_room(&dee), "the page has three open");

    // One leaving makes room again.
    first.connection.leave(Ending::ByClient).await;
    assert!(sync.has_room(&dee));
    let again = sync.connect(dee).await;
    assert!(again.is_ok());
    sync.shutdown().await;
}

/// What the recorder saw, added up: each snapshot swaps counters and gauges back to zero.
struct Metrics {
    snapshotter: Snapshotter,
    gauges: HashMap<String, f64>,
    counters: HashMap<(String, Vec<(String, String)>), u64>,
}

impl Metrics {
    fn new(snapshotter: Snapshotter) -> Self {
        Self {
            snapshotter,
            gauges: HashMap::new(),
            counters: HashMap::new(),
        }
    }

    fn read(&mut self) {
        for (key, _, _, value) in self.snapshotter.snapshot().into_vec() {
            let (kind, key) = key.into_parts();
            match (kind, value) {
                (MetricKind::Gauge, DebugValue::Gauge(value)) => {
                    *self.gauges.entry(key.name().to_owned()).or_default() += value.into_inner();
                }
                (MetricKind::Counter, DebugValue::Counter(n)) => {
                    let labels = key
                        .labels()
                        .map(|label| (label.key().to_owned(), label.value().to_owned()))
                        .collect();
                    *self
                        .counters
                        .entry((key.name().to_owned(), labels))
                        .or_default() += n;
                }
                _ => {}
            }
        }
    }

    fn gauge(&mut self, name: &str) -> f64 {
        self.read();
        self.gauges.get(name).copied().unwrap_or(0.0)
    }

    /// The total of a counter over the series whose labels include these.
    fn counted(&mut self, name: &str, labels: &[(&str, &str)]) -> u64 {
        self.read();
        self.counters
            .iter()
            .filter(|((counted, have), _)| {
                counted == name
                    && labels
                        .iter()
                        .all(|(key, value)| have.iter().any(|(k, v)| k == key && v == value))
            })
            .map(|(_, n)| n)
            .sum()
    }

    fn counter(&mut self, name: &str, label: &str, value: &str) -> u64 {
        self.counted(name, &[(label, value)])
    }

    fn red(&mut self, operation: &str, status: &str) -> u64 {
        self.counted(
            "neon_sync_operations_total",
            &[("operation", operation), ("status", status)],
        )
    }
}

/// An edit appending the blocks of `markdown`.
fn append(markdown: &str) -> Vec<Edit> {
    vec![Edit::Append {
        blocks: from_markdown(markdown).blocks,
    }]
}

fn replace(find: &str, replace: &str) -> Edit {
    Edit::FindReplace {
        find: find.to_owned(),
        replace: replace.to_owned(),
        all: false,
    }
}

/// The code of an error, or `ok`.
fn refused<T>(result: Result<T, ContentError>) -> &'static str {
    match result {
        Err(error) => error.code(),
        Ok(_) => "ok",
    }
}

async fn text_of(sync: &SyncService, who: &Principal, page: Uuid) -> String {
    sync.read(who, page).await.expect("read").1.plain_text()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_reaches_whoever_has_the_page_open_at_once(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    ada.write("Typed").await.expect("sent");

    let edited = sync
        .edit(&t.dee, t.page, append("Added by Dee"), None)
        .await
        .expect("edited");
    assert_eq!(edited.applied.len(), 1);
    assert_eq!(edited.applied[0].added.len(), 1);
    assert_eq!(edited.page.page.id, t.page);
    ada.until("the edit", |message| {
        matches!(message, Message::Sync(SyncMessage::Update(_)))
    })
    .await;
    assert_eq!(ada.text(), "Typed\nAdded by Dee");
    assert_eq!(text_of(&sync, &t.dee, t.page).await, "Typed\nAdded by Dee");
    // The log keeps it as Dee's change.
    eventually("the edit logged as Dee's", || async {
        rows(&t.h.pool, t.page)
            .await
            .iter()
            .any(|(author, _)| *author == Some(t.dee.user.id))
    })
    .await;
    drop(ada);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_merges_with_typing_the_server_had_not_heard_of(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    ada.write("Mine").await.expect("sent");
    // Typed before the edit, sent after it.
    let typed = ada.paragraph("Typed meanwhile");

    sync.edit(&t.dee, t.page, vec![replace("Mine", "Ours")], None)
        .await
        .expect("edited");
    ada.send(&typed).await.expect("sent");
    ada.until("the edit", |message| {
        matches!(message, Message::Sync(SyncMessage::Update(_)))
    })
    .await;
    assert_eq!(ada.text(), "Ours\nTyped meanwhile");
    eventually("the server merged both", || async {
        text_of(&sync, &t.ada, t.page).await == "Ours\nTyped meanwhile"
    })
    .await;
    drop(ada);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_opens_the_document_nobody_has_open_and_it_is_saved(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        idle_unload: Duration::from_millis(50),
        ..quick()
    });
    sync.edit(&t.ada, t.page, append("# Hello\n\nWorld"), None)
        .await
        .expect("edited");
    eventually("the edit saved", || async {
        neon_store::documents::saved(&t.h.pool, t.page)
            .await
            .is_ok_and(|saved| saved.snapshot.is_some() || !saved.updates.is_empty())
    })
    .await;
    // Another server reads it from what was saved.
    let other = t.sync(quick());
    assert_eq!(text_of(&other, &t.dee, t.page).await, "Hello\nWorld");
    sync.shutdown().await;
    other.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_of_several_parts_makes_all_or_none(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    sync.edit(&t.ada, t.page, append("First"), None)
        .await
        .expect("edited");

    let refused = sync
        .edit(
            &t.ada,
            t.page,
            vec![
                Edit::Append {
                    blocks: from_markdown("Second").blocks,
                },
                replace("absent", "x"),
            ],
            None,
        )
        .await;
    match refused {
        Err(ContentError::Refused { index, error }) => {
            assert_eq!(index, 1);
            assert_eq!(error, EditError::NotFound);
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(text_of(&sync, &t.ada, t.page).await, "First");

    let made = sync
        .edit(
            &t.ada,
            t.page,
            vec![
                Edit::Append {
                    blocks: from_markdown("Second").blocks,
                },
                replace("First", "1st"),
            ],
            None,
        )
        .await
        .expect("edited");
    assert_eq!(made.applied[1].replaced, 1);
    assert_eq!(text_of(&sync, &t.ada, t.page).await, "1st\nSecond");
    assert!(matches!(
        sync.edit(&t.ada, t.page, Vec::new(), None).await,
        Err(ContentError::InvalidInput(_))
    ));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn reading_needs_seeing_the_page_and_editing_needs_editing_it(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let workspace =
        t.h.pages
            .page(&t.ada, t.page)
            .await
            .expect("read")
            .page
            .workspace_id;
    let gus = t.h.person("gus@example.com").await;
    t.h.bring_in(&t.ada, workspace, &gus, WorkspaceRole::Guest)
        .await;
    let stranger = t.h.person("zed@example.com").await;

    assert_eq!(refused(sync.read(&gus, t.page).await), "not_found");
    assert_eq!(
        refused(sync.edit(&gus, t.page, append("x"), None).await),
        "not_found"
    );
    assert_eq!(refused(sync.read(&stranger, t.page).await), "not_found");
    t.h.pages
        .share_page(&t.ada, t.page, gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    assert_eq!(refused(sync.read(&gus, t.page).await), "ok");
    assert_eq!(
        refused(sync.edit(&gus, t.page, append("x"), None).await),
        "forbidden"
    );

    // A token without the write scope reads only.
    let agent = t.h.agent(&t.ada, workspace, "Reader").await;
    let (_, reader) = t.h.agent_token(&t.ada, &agent, &["read"]).await;
    assert_eq!(refused(sync.read(&reader, t.page).await), "ok");
    assert_eq!(
        refused(sync.edit(&reader, t.page, append("x"), None).await),
        "forbidden"
    );

    // A database has no body; a page in the trash is not edited.
    let database =
        t.h.databases
            .create_database(
                &t.ada,
                workspace,
                Location::Under(t.page),
                Placement::Last,
                neon_app::databases::NewDatabase {
                    title: "Tasks".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("created")
            .page
            .page
            .id;
    assert!(matches!(
        sync.read(&t.ada, database).await,
        Err(ContentError::Database)
    ));
    assert!(matches!(
        sync.edit(&t.ada, database, append("x"), None).await,
        Err(ContentError::Database)
    ));
    t.h.pages.trash_page(&t.ada, t.page).await.expect("trashed");
    // It says it is in the trash, rather than that it may only be viewed.
    let in_trash = sync.edit(&t.ada, t.page, append("x"), None).await;
    assert!(
        matches!(
            &in_trash,
            Err(ContentError::Page(PageError::Forbidden(message)))
                if *message == "this page is in the trash: restore it first"
        ),
        "{in_trash:?}"
    );
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_past_the_size_a_document_keeps_is_refused(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        max_document: 512,
        ..quick()
    });
    let refused = sync
        .edit(&t.ada, t.page, append(&"word ".repeat(200)), None)
        .await;
    assert!(
        matches!(refused, Err(ContentError::TooLarge(512))),
        "{refused:?}"
    );
    assert_eq!(text_of(&sync, &t.ada, t.page).await, "");
    let document = from_markdown(&"word ".repeat(200));
    assert!(matches!(
        sync.first_snapshot(&document).await,
        Err(ContentError::TooLarge(512))
    ));
    assert_eq!(
        sync.first_snapshot(&neon_blocks::Document::default())
            .await
            .expect("nothing to save"),
        None
    );
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn content_making_more_nodes_than_are_written_at_once_is_refused_before_writing(
    pool: PgPool,
) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        max_nodes: 10,
        ..quick()
    });
    // Six paragraphs of one run each: twelve nodes.
    let markdown = "a\n\nb\n\nc\n\nd\n\ne\n\nf\n";
    assert!(matches!(
        sync.first_snapshot(&from_markdown(markdown)).await,
        Err(ContentError::TooManyNodes(10))
    ));
    let refused = sync.edit(&t.ada, t.page, append(markdown), None).await;
    assert!(
        matches!(refused, Err(ContentError::TooManyNodes(10))),
        "{refused:?}"
    );
    let appended = sync
        .append(&t.ada, t.page, from_markdown(markdown).blocks)
        .await;
    assert!(
        matches!(&appended, Err(PageError::InvalidInput(message)) if message.contains("more than 10")),
        "{appended:?}"
    );
    assert_eq!(text_of(&sync, &t.ada, t.page).await, "");
    // Five nodes fit.
    sync.first_snapshot(&from_markdown("a\n\nb\n"))
        .await
        .expect("few enough");
    sync.edit(&t.ada, t.page, append("a\n\nb\n"), None)
        .await
        .expect("few enough");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_new_page_starts_with_its_content(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let workspace =
        t.h.pages
            .page(&t.ada, t.page)
            .await
            .expect("read")
            .page
            .workspace_id;
    let content = sync
        .first_snapshot(&from_markdown("# Plan\n\n- one\n- two"))
        .await
        .expect("small enough");
    let made =
        t.h.pages
            .create_page(
                &t.ada,
                workspace,
                Location::Under(t.page),
                Placement::Last,
                NewPage {
                    title: "Plan".to_owned(),
                    content,
                    ..NewPage::default()
                },
            )
            .await
            .expect("created");
    assert_eq!(text_of(&sync, &t.dee, made.page.id).await, "Plan\none\ntwo");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn replacing_text_that_multiplies_it_stops_at_the_limit_at_once(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    sync.edit(&t.ada, t.page, append("x"), None)
        .await
        .expect("edited");
    let tenfold = Edit::FindReplace {
        find: "x".to_owned(),
        replace: "x".repeat(10),
        all: true,
    };

    // Each part makes ten of every x: the sixth would replace a hundred thousand.
    let started = std::time::Instant::now();
    let refused = sync.edit(&t.ada, t.page, vec![tenfold; 7], None).await;
    match refused {
        Err(ContentError::Refused { index, error }) => {
            assert_eq!(index, 5);
            assert_eq!(error, EditError::TooManyMatches(100_000));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(text_of(&sync, &t.ada, t.page).await, "x");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn replacements_past_the_size_a_document_keeps_are_refused_before_they_are_made(
    pool: PgPool,
) {
    let t = team(pool).await;
    let sync = t.sync(Limits {
        max_document: 64 * 1024,
        ..quick()
    });
    sync.edit(&t.ada, t.page, append(&"x ".repeat(100)), None)
        .await
        .expect("edited");
    let grown = Edit::FindReplace {
        find: "x".to_owned(),
        replace: "y".repeat(1024),
        all: true,
    };

    let refused = sync.edit(&t.ada, t.page, vec![grown], None).await;
    assert!(
        matches!(refused, Err(ContentError::TooLarge(65_536))),
        "{refused:?}"
    );
    assert_eq!(
        text_of(&sync, &t.ada, t.page).await,
        "x ".repeat(100).trim_end()
    );
    sync.shutdown().await;
}

/// A limiter that refuses one user's edits of pages' content, and counts the rest.
struct RefusingEdits(Uuid);

#[async_trait::async_trait]
impl RateLimiter for RefusingEdits {
    async fn check(&self, bucket: Bucket, key: &str) -> Result<(), Limited> {
        if bucket == Bucket::PageEdit && key == self.0.to_string() {
            return Err(Limited {
                retry_after: Duration::from_secs(7),
            });
        }
        Ok(())
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn edits_past_their_rate_are_refused_per_user(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync_counting(quick(), Arc::new(RefusingEdits(t.dee.user.id)));

    let refused = sync.edit(&t.dee, t.page, append("Dee's"), None).await;
    match refused {
        Err(ContentError::RateLimited { retry_after }) => {
            assert_eq!(retry_after, Duration::from_secs(7));
        }
        other => panic!("expected a rate limit, got {other:?}"),
    }
    sync.edit(&t.ada, t.page, append("Ada's"), None)
        .await
        .expect("another user's edit");
    // Reading is not counted.
    assert_eq!(text_of(&sync, &t.dee, t.page).await, "Ada's");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_body_sent_back_as_read_keeps_what_is_typed_in_its_last_paragraph(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    // A page as the editor leaves it: an empty paragraph last.
    ada.write("Intro").await.expect("sent");
    ada.write("").await.expect("sent");
    eventually("the server to hold both blocks", || async {
        sync.read(&t.dee, t.page)
            .await
            .expect("read")
            .1
            .blocks
            .len()
            == 2
    })
    .await;
    let (_, read) = sync.read(&t.dee, t.page).await.expect("read");
    let sent = from_markdown(&neon_blocks::markdown::to_markdown_with_ids(&read)).blocks;

    // Ada types in the last paragraph while the body goes back as it was read.
    let typed = ada.type_last("Typed meanwhile");
    let edited = sync
        .edit(
            &t.dee,
            t.page,
            vec![Edit::ReplaceBody { blocks: sent }],
            Some(neon_app::sync::revision(&read)),
        )
        .await
        .expect("edited");
    assert_eq!(edited.applied[0].added, Vec::<String>::new());
    assert_eq!(edited.applied[0].removed, 0);
    ada.send(&typed).await.expect("sent");
    eventually("the server to hold the typing", || async {
        text_of(&sync, &t.dee, t.page).await == "Intro\nTyped meanwhile"
    })
    .await;
    assert_eq!(ada.text(), "Intro\nTyped meanwhile");
    drop(ada);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn replacing_blocks_from_a_stale_read_is_refused_rather_than_remove_what_came_since(
    pool: PgPool,
) {
    let t = team(pool).await;
    let sync = t.sync(quick());
    sync.edit(
        &t.ada,
        t.page,
        append("Fix the redirect\n\nAdd a regression test"),
        None,
    )
    .await
    .expect("edited");
    let (_, read) = sync.read(&t.dee, t.page).await.expect("read");
    let base = neon_app::sync::revision(&read);
    let done = |read: &neon_blocks::Document| Edit::ReplaceBody {
        blocks: from_markdown(
            &neon_blocks::markdown::to_markdown_with_ids(read)
                .replace("Fix the redirect", "Fix the redirect (done)"),
        )
        .blocks,
    };

    // Replacing names the revision it was made from.
    assert!(matches!(
        sync.edit(&t.dee, t.page, vec![done(&read)], None).await,
        Err(ContentError::NoBase)
    ));

    // Ada adds a block after Dee read the page: Dee's body, made from the read, is refused,
    // and Ada's block stays.
    sync.edit(&t.ada, t.page, append("Added after the read"), None)
        .await
        .expect("edited");
    assert!(matches!(
        sync.edit(&t.dee, t.page, vec![done(&read)], Some(base.clone()))
            .await,
        Err(ContentError::Conflict)
    ));
    assert_eq!(
        text_of(&sync, &t.dee, t.page).await,
        "Fix the redirect\nAdd a regression test\nAdded after the read"
    );
    // Adding needs no base; made off it, it tells no revision.
    let appended = sync
        .edit(&t.dee, t.page, append("Dee's note"), Some(base))
        .await
        .expect("edited");
    assert_eq!(appended.revision, None);

    // Read again, it goes in, and its answer is the base of the next edit.
    let (_, read) = sync.read(&t.dee, t.page).await.expect("read");
    let edited = sync
        .edit(
            &t.dee,
            t.page,
            vec![done(&read)],
            Some(neon_app::sync::revision(&read)),
        )
        .await
        .expect("edited");
    let next = edited.revision.expect("a revision");
    let (_, now) = sync.read(&t.dee, t.page).await.expect("read");
    assert_eq!(next, neon_app::sync::revision(&now));
    let note = now
        .blocks
        .iter()
        .find(|block| {
            neon_blocks::Document::new(vec![(*block).clone()]).plain_text() == "Dee's note"
        })
        .and_then(|block| block.id.clone())
        .expect("the note's id");
    let removed = sync
        .edit(
            &t.dee,
            t.page,
            vec![Edit::ReplaceRange {
                from: note.clone(),
                to: note.clone(),
                blocks: Vec::new(),
            }],
            Some(next),
        )
        .await
        .expect("edited");
    assert_eq!(removed.applied[0].removed_ids, [note]);
    assert_eq!(
        text_of(&sync, &t.dee, t.page).await,
        "Fix the redirect (done)\nAdd a regression test\nAdded after the read"
    );
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn blocks_the_server_appends_reach_every_editor_and_are_logged_at_once(pool: PgPool) {
    let t = team(pool.clone()).await;
    // A long pause before logging: the append logs at once all the same.
    let sync = t.sync(Limits {
        persist_delay: Duration::from_secs(3600),
        ..quick()
    });
    let mut dee = Client::open(&sync, &t.dee, t.page, "dee").await;
    dee.write("typed by Dee").await.expect("taken");

    sync.append(
        &t.ada,
        t.page,
        vec![
            Block::paragraph("appended"),
            Block::paragraph("by the server"),
        ],
    )
    .await
    .expect("appended");

    dee.until("the server's blocks", |message| {
        matches!(message, Message::Sync(SyncMessage::Update(_)))
    })
    .await;
    assert_eq!(dee.text(), "typed by Dee\nappended\nby the server");
    // Logged before the call answered, as Ada's, with Dee's typing before it.
    let logged = rows(&pool, t.page).await;
    assert_eq!(
        logged.iter().map(|(author, _)| *author).collect::<Vec<_>>(),
        [Some(t.dee.user.id), Some(t.ada.user.id)]
    );
    let read = sync.content(t.page).await.expect("its content");
    assert_eq!(read.plain_text(), "typed by Dee\nappended\nby the server");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_server_appends_to_a_page_nobody_has_open_for_those_who_edit_it(pool: PgPool) {
    let t = team(pool.clone()).await;
    let sync = t.sync(quick());
    let gus = t.h.person("gus@example.com").await;
    let workspace =
        t.h.pages
            .page(&t.ada, t.page)
            .await
            .expect("a page")
            .page
            .workspace_id;
    t.h.bring_in(&t.ada, workspace, &gus, WorkspaceRole::Guest)
        .await;

    sync.append(&t.dee, t.page, vec![Block::paragraph("while closed")])
        .await
        .expect("appended");
    assert_eq!(
        sync.content(t.page)
            .await
            .expect("its content")
            .plain_text(),
        "while closed"
    );
    let refused = sync
        .append(&gus, t.page, vec![Block::paragraph("not mine")])
        .await;
    assert!(
        matches!(refused, Err(neon_app::pages::PageError::NotFound(_))),
        "{refused:?}"
    );
    let mut ada = Client::open(&sync, &t.ada, t.page, "ada").await;
    assert_eq!(ada.waiting().await.len(), 0);
    assert_eq!(ada.text(), "while closed");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_append_to_a_page_deleted_as_it_is_logged_is_not_found(pool: PgPool) {
    let t = team(pool.clone()).await;
    let sync = t.sync(quick());
    // The page goes just as the append is logged: the log finds no page to belong to.
    sqlx::raw_sql(
        "CREATE FUNCTION delete_page_first() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN DELETE FROM pages WHERE id = NEW.page_id; RETURN NEW; END $$;
         CREATE TRIGGER delete_page_first BEFORE INSERT ON page_updates
             FOR EACH ROW EXECUTE FUNCTION delete_page_first();",
    )
    .execute(&pool)
    .await
    .expect("the trigger");

    let appended = sync
        .append(&t.ada, t.page, vec![Block::paragraph("too late")])
        .await;

    assert!(
        matches!(appended, Err(neon_app::pages::PageError::NotFound(_))),
        "{appended:?}"
    );
    sync.shutdown().await;
}
