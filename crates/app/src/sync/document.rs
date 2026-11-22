//! One live document: the task that owns a page's Yjs document and its awareness while
//! anyone has the page open, and for a while after.
//!
//! Its connections send it what their clients send, through one queue, and it answers
//! each in turn, so the document needs no lock. What it has to tell everyone goes through
//! the [`Broadcaster`](super::Broadcaster); answers to one client go straight to that
//! client's outbox. Updates it applies wait a moment to be logged together, and once enough
//! are logged, or the document unloads or the server stops, the whole document is saved as
//! a snapshot that takes them in.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use neon_domain::pages::PageAccess;
use neon_store::documents::{self, Edited, NewSnapshot, NewUpdate};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};
use tracing::{Instrument as _, debug, error, info, info_span, warn};
use uuid::Uuid;
use yrs::block::ClientID;
use yrs::sync::{Awareness, AwarenessUpdate, Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update, XmlFragmentRef};

use super::broadcast::{ClientId, Outbox};
use super::protocol::{self, encode};
use super::{CloseReason, Inner, LIVE_DOCUMENTS, MESSAGES};

/// What the server tells a connection that may no longer edit, in y-protocols' auth
/// message, as permission denied.
pub const READ_ONLY: &str = "read-only";

/// The longest a failed write waits before it is tried again.
const MAX_RETRY: Duration = Duration::from_secs(30);

/// What a live document is asked to do.
pub(crate) enum Command {
    /// A connection opens the document. The answer is dropped, rather than sent, when the
    /// document is unloading: the connection then opens it afresh.
    Join {
        client: Joining,
        done: oneshot::Sender<Result<(), CloseReason>>,
    },
    /// What a client sent.
    Message { client: ClientId, message: Bytes },
    /// A connection closed.
    Leave { client: ClientId },
    /// What a connection's person may do now; `None` when they may no longer see the page.
    Access {
        client: ClientId,
        access: Option<PageAccess>,
    },
}

/// A connection opening the document.
pub(crate) struct Joining {
    pub(crate) id: ClientId,
    pub(crate) user_id: Uuid,
    pub(crate) access: PageAccess,
    pub(crate) outbox: Outbox,
}

/// A connection that has the document open.
struct Client {
    user_id: Uuid,
    access: PageAccess,
    outbox: Outbox,
    /// The awareness client ids its client holds: the cursors and presence it shows.
    awareness_ids: HashSet<ClientID>,
}

/// Runs a page's document until it unloads or the server stops: loads it, then serves it.
pub(crate) async fn run(
    inner: Arc<Inner>,
    page_id: Uuid,
    generation: u64,
    mut commands: mpsc::Receiver<Command>,
) {
    let span = info_span!("sync document", page_id = %page_id);
    async move {
        let Some(mut document) = Document::load(Arc::clone(&inner), page_id, generation).await
        else {
            // Those waiting to join hear the document cannot open. A connection that comes
            // later finds the registry without it, and loads it afresh.
            inner.forget(page_id, generation);
            commands.close();
            while let Ok(command) = commands.try_recv() {
                if let Command::Join { done, .. } = command {
                    let _ = done.send(Err(CloseReason::ServerError));
                }
            }
            return;
        };
        metrics::gauge!(LIVE_DOCUMENTS).increment(1.0);
        document.serve(&mut commands).await;
        metrics::gauge!(LIVE_DOCUMENTS).decrement(1.0);
    }
    .instrument(span)
    .await;
}

struct Document {
    inner: Arc<Inner>,
    page_id: Uuid,
    generation: u64,
    awareness: Awareness,
    fragment: XmlFragmentRef,
    clients: HashMap<ClientId, Client>,
    /// Which connection holds each awareness client id, so no client speaks for another's.
    holders: HashMap<ClientID, ClientId>,
    storage: Storage,
    /// Since when nobody has had the document open.
    idle_since: Option<Instant>,
    /// The page was deleted: the document closes.
    gone: bool,
}

/// What the database holds of the document, and what waits to be written.
#[derive(Debug, Default)]
struct Storage {
    /// The last update logged that the document holds.
    last_seq: i64,
    /// The size of the last snapshot, and the updates logged after it.
    snapshot_bytes: usize,
    logged: usize,
    logged_bytes: usize,
    /// Updates applied but not yet logged, in order.
    pending: Vec<Pending>,
    pending_bytes: usize,
    /// When the pending updates are to be logged.
    flush_at: Option<Instant>,
    /// Writes that failed in a row.
    failures: u32,
    /// The latest change, which the next snapshot records on the page.
    edited: Option<Edited>,
}

#[derive(Debug)]
struct Pending {
    update: Vec<u8>,
    author: Uuid,
    at: time::OffsetDateTime,
}

impl Storage {
    /// The document's size as far as the database goes: its snapshot and the updates
    /// after it. Updates repeat some of what they change, so this errs large.
    fn size(&self) -> usize {
        self.snapshot_bytes + self.logged_bytes + self.pending_bytes
    }

    /// Whether the database lacks a snapshot of the document as it is.
    fn needs_snapshot(&self) -> bool {
        self.logged > 0 || !self.pending.is_empty()
    }

    /// Consecutive pending updates of one author merged into one, so typing logs a row per
    /// pause rather than per key. An update that does not merge stays on its own.
    fn batch(&self) -> Vec<Pending> {
        let mut batch: Vec<Pending> = Vec::new();
        for pending in &self.pending {
            if let Some(last) = batch.last_mut()
                && last.author == pending.author
                && let Ok(merged) = yrs::merge_updates_v1([&last.update, &pending.update])
            {
                last.update = merged;
                last.at = pending.at;
                continue;
            }
            batch.push(Pending {
                update: pending.update.clone(),
                author: pending.author,
                at: pending.at,
            });
        }
        batch
    }

    /// The pending updates were logged, the last as `seq`.
    fn logged(&mut self, seq: Option<i64>, rows: usize, bytes: usize) {
        if let Some(seq) = seq {
            self.last_seq = self.last_seq.max(seq);
        }
        self.logged += rows;
        self.logged_bytes += bytes;
        self.pending.clear();
        self.pending_bytes = 0;
        self.flush_at = None;
        self.failures = 0;
    }

    /// A snapshot of `bytes` took in everything.
    fn compacted(&mut self, bytes: usize) {
        self.snapshot_bytes = bytes;
        self.logged = 0;
        self.logged_bytes = 0;
        self.pending.clear();
        self.pending_bytes = 0;
        self.flush_at = None;
        self.failures = 0;
    }

    /// When to try a write again after one more failure: twice as long each time.
    fn retry_at(&mut self, first: Duration) -> Instant {
        self.failures = self.failures.saturating_add(1);
        let wait = first
            .saturating_mul(2u32.saturating_pow(self.failures.min(16)))
            .min(MAX_RETRY);
        Instant::now() + wait
    }
}

impl Document {
    /// Reads the page's snapshot and the updates logged after it. `None` when that fails,
    /// which is logged.
    async fn load(inner: Arc<Inner>, page_id: Uuid, generation: u64) -> Option<Self> {
        let guard = inner.red.start("load");
        let loaded = async {
            let snapshot = documents::snapshot(&inner.pool, page_id).await?;
            let after = snapshot.as_ref().map_or(0, |snapshot| snapshot.last_seq);
            let updates = documents::updates_after(&inner.pool, page_id, after).await?;
            Ok::<_, LoadError>((snapshot, after, updates))
        }
        .await
        .and_then(|(snapshot, after, updates)| {
            let doc = Doc::new();
            let fragment = neon_blocks::fragment(&doc);
            {
                let mut txn = doc.transact_mut();
                if let Some(snapshot) = &snapshot {
                    txn.apply_update(Update::decode_v1(&snapshot.state)?)?;
                }
                for logged in &updates {
                    txn.apply_update(Update::decode_v1(&logged.update)?)?;
                }
            }
            let storage = Storage {
                last_seq: updates.last().map_or(after, |last| last.seq.max(after)),
                snapshot_bytes: snapshot.as_ref().map_or(0, |snapshot| snapshot.state.len()),
                logged: updates.len(),
                logged_bytes: updates.iter().map(|logged| logged.update.len()).sum(),
                edited: updates.last().map(|last| Edited {
                    at: last.created_at,
                    by: last.author_id,
                }),
                ..Storage::default()
            };
            Ok((doc, fragment, storage))
        });
        match loaded {
            Ok((doc, fragment, storage)) => {
                guard.ok();
                info!(
                    bytes = storage.size(),
                    updates = storage.logged,
                    "document loaded"
                );
                Some(Self {
                    inner,
                    page_id,
                    generation,
                    awareness: Awareness::new(doc),
                    fragment,
                    clients: HashMap::new(),
                    holders: HashMap::new(),
                    storage,
                    idle_since: Some(Instant::now()),
                    gone: false,
                })
            }
            Err(error) => {
                guard.err();
                error!(%error, "cannot load the document");
                None
            }
        }
    }

    async fn serve(&mut self, commands: &mut mpsc::Receiver<Command>) {
        let inner = Arc::clone(&self.inner);
        loop {
            let flush_at = self.storage.flush_at;
            let unload_at = self
                .idle_since
                .map(|since| since + inner.limits.idle_unload);
            tokio::select! {
                biased;
                () = inner.stop.cancelled() => {
                    self.stop().await;
                    break;
                }
                command = commands.recv() => match command {
                    Some(command) => self.handle(command).await,
                    // The registry holds a sender until the document leaves it.
                    None => break,
                },
                () = sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() => {
                    self.flush().await;
                }
                () = sleep_until(unload_at.unwrap_or_else(Instant::now)), if unload_at.is_some() => {
                    if self.unload(commands).await {
                        break;
                    }
                }
            }
            if self.gone {
                break;
            }
        }
        // A connection that took the sender before the document left the registry gets no
        // answer to its join, and opens the document afresh.
        self.inner.forget(self.page_id, self.generation);
        commands.close();
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Join { client, done } => {
                let joined = self.join(client);
                let _ = done.send(joined);
            }
            Command::Message { client, message } => self.receive(client, &message).await,
            Command::Leave { client } => self.leave(client),
            Command::Access { client, access } => self.set_access(client, access),
        }
    }

    /// Takes a connection in, and starts the sync: the server's state vector, which the
    /// client answers with what the server lacks, and who else is there.
    fn join(&mut self, joining: Joining) -> Result<(), CloseReason> {
        if self.gone {
            return Err(CloseReason::AccessLost);
        }
        let state_vector = self.awareness.doc().transact().state_vector();
        joining
            .outbox
            .send(encode(&Message::Sync(SyncMessage::SyncStep1(state_vector))));
        if let Some(states) = self.awareness_states() {
            joining.outbox.send(encode(&Message::Awareness(states)));
        }
        self.inner
            .broadcaster
            .join(self.page_id, joining.id, joining.outbox.clone());
        debug!(
            client = %joining.id,
            user_id = %joining.user_id,
            access = %joining.access,
            "client joined"
        );
        self.clients.insert(
            joining.id,
            Client {
                user_id: joining.user_id,
                access: joining.access,
                outbox: joining.outbox,
                awareness_ids: HashSet::new(),
            },
        );
        self.idle_since = None;
        Ok(())
    }

    /// Every client's awareness state there is, if any.
    fn awareness_states(&self) -> Option<AwarenessUpdate> {
        let update = self.awareness.update().ok()?;
        (!update.clients.is_empty()).then_some(update)
    }

    async fn receive(&mut self, id: ClientId, data: &[u8]) {
        if !self.clients.contains_key(&id) {
            // Closed already: what it sent since does not count.
            return;
        }
        let Ok(messages) = protocol::decode(data) else {
            self.close(id, CloseReason::InvalidMessage);
            return;
        };
        for message in messages {
            metrics::counter!(MESSAGES, "kind" => protocol::kind(&message)).increment(1);
            let handled = match message {
                Message::Sync(SyncMessage::SyncStep1(state_vector)) => {
                    self.send_missing(id, &state_vector);
                    Ok(())
                }
                Message::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) => {
                    self.apply(id, &update).await
                }
                Message::Awareness(update) => self.update_awareness(id, update),
                Message::AwarenessQuery => {
                    if let (Some(client), Some(states)) =
                        (self.clients.get(&id), self.awareness_states())
                    {
                        client.outbox.send(encode(&Message::Awareness(states)));
                    }
                    Ok(())
                }
                // Clients have nothing to ask by auth messages, and kinds the server does not
                // know are left alone, as y-websocket's server leaves them.
                Message::Auth(_) | Message::Custom(..) => Ok(()),
            };
            if let Err(reason) = handled {
                self.close(id, reason);
                return;
            }
            if !self.clients.contains_key(&id) {
                return;
            }
        }
    }

    /// Answers a client's sync step 1 with what its state vector lacks.
    fn send_missing(&self, id: ClientId, state_vector: &StateVector) {
        let Some(client) = self.clients.get(&id) else {
            return;
        };
        let update = self
            .awareness
            .doc()
            .transact()
            .encode_state_as_update_v1(state_vector);
        client
            .outbox
            .send(encode(&Message::Sync(SyncMessage::SyncStep2(update))));
    }

    /// Applies a client's update, then tells the others what changed and logs it. From a
    /// client that only views the page, only an update that changes nothing is taken.
    async fn apply(&mut self, id: ClientId, data: &[u8]) -> Result<(), CloseReason> {
        let update = Update::decode_v1(data).map_err(|_| CloseReason::InvalidMessage)?;
        let Some(client) = self.clients.get(&id) else {
            return Ok(());
        };
        let (author, access) = (client.user_id, client.access);
        if access == PageAccess::View {
            let txn = self.awareness.doc().transact();
            return if protocol::changes(&update, &txn) {
                Err(CloseReason::ReadOnly)
            } else {
                Ok(())
            };
        }
        let max = self.inner.limits.max_document;
        let grows = !update.insertions(true).is_empty();
        if grows && self.storage.size() + data.len() > max {
            // The estimate errs large; a snapshot tells the size for sure.
            if self.storage.needs_snapshot() {
                self.compact().await;
            }
            if self.gone || self.storage.size() + data.len() > max {
                return Err(CloseReason::DocumentTooLarge);
            }
        }
        let (applied, changed) = {
            let mut txn = self.awareness.doc().transact_mut();
            let applied = txn.apply_update(update);
            (applied, txn.encode_update_v1())
        };
        // Even an update that failed may have changed some of the document, which the
        // others and the log must have too.
        if changed.as_slice() != Update::EMPTY_V1 {
            let message = encode(&Message::Sync(SyncMessage::Update(changed.clone())));
            self.inner
                .broadcaster
                .broadcast(self.page_id, message, Some(id));
            self.queue(changed, author);
        }
        applied.map_err(|error| {
            warn!(client = %id, %error, "a client's update did not apply");
            CloseReason::InvalidMessage
        })
    }

    /// Queues an update to be logged soon, or at once when enough wait.
    fn queue(&mut self, update: Vec<u8>, author: Uuid) {
        let at = self.inner.clock.now();
        let limits = &self.inner.limits;
        self.storage.pending_bytes += update.len();
        self.storage.pending.push(Pending { update, author, at });
        self.storage.edited = Some(Edited {
            at,
            by: Some(author),
        });
        let now = Instant::now();
        if self.storage.pending_bytes >= limits.persist_bytes {
            self.storage.flush_at = Some(now);
        } else if self.storage.flush_at.is_none() {
            self.storage.flush_at = Some(now + limits.persist_delay);
        }
    }

    /// Applies a client's awareness update, for the awareness ids it holds and new ones up
    /// to its limit, then tells everyone, the client too, as y-websocket's server does:
    /// browsers renew their state every 15 seconds and take the echo as a sign of life.
    fn update_awareness(
        &mut self,
        id: ClientId,
        mut update: AwarenessUpdate,
    ) -> Result<(), CloseReason> {
        let limit = self.inner.limits.awareness_clients;
        let Some(client) = self.clients.get(&id) else {
            return Ok(());
        };
        let mut held = client.awareness_ids.len();
        let holders = &self.holders;
        update
            .clients
            .retain(|awareness_id, _| match holders.get(awareness_id) {
                Some(holder) => *holder == id,
                None if held < limit => {
                    held += 1;
                    true
                }
                None => false,
            });
        if update.clients.is_empty() {
            return Ok(());
        }
        let Some(summary) = self
            .awareness
            .apply_update_summary(update)
            .map_err(|_| CloseReason::InvalidMessage)?
        else {
            return Ok(());
        };
        if let Some(client) = self.clients.get_mut(&id) {
            for awareness_id in summary.added.iter().chain(&summary.updated) {
                self.holders.insert(*awareness_id, id);
                client.awareness_ids.insert(*awareness_id);
            }
            for awareness_id in &summary.removed {
                self.holders.remove(awareness_id);
                client.awareness_ids.remove(awareness_id);
            }
        }
        if let Ok(changed) = self.awareness.update_with_clients(summary.all_changes()) {
            self.inner.broadcaster.broadcast(
                self.page_id,
                encode(&Message::Awareness(changed)),
                None,
            );
        }
        Ok(())
    }

    /// Takes a connection out, and tells the others its client's cursors are gone.
    fn leave(&mut self, id: ClientId) {
        let Some(client) = self.clients.remove(&id) else {
            return;
        };
        self.inner.broadcaster.leave(self.page_id, id);
        if !client.awareness_ids.is_empty() {
            for awareness_id in &client.awareness_ids {
                self.holders.remove(awareness_id);
                self.awareness.remove_state(*awareness_id);
            }
            if let Ok(removed) = self.awareness.update_with_clients(client.awareness_ids) {
                self.inner.broadcaster.broadcast(
                    self.page_id,
                    encode(&Message::Awareness(removed)),
                    None,
                );
            }
        }
        debug!(client = %id, "client left");
        if self.clients.is_empty() {
            self.idle_since = Some(Instant::now());
        }
    }

    /// Closes a connection for `reason`, and takes it out at once, so nothing it sends
    /// after counts.
    fn close(&mut self, id: ClientId, reason: CloseReason) {
        let Some(client) = self.clients.get(&id) else {
            return;
        };
        client.outbox.close(reason);
        info!(
            client = %id,
            user_id = %client.user_id,
            reason = reason.name(),
            "closing a connection"
        );
        self.leave(id);
    }

    /// Applies what a connection's person may do now. Losing edit access leaves the
    /// connection open to view, and the client is told so by a permission-denied auth
    /// message; gaining it is told by permission granted.
    fn set_access(&mut self, id: ClientId, access: Option<PageAccess>) {
        let Some(access) = access else {
            self.close(id, CloseReason::AccessLost);
            return;
        };
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        if client.access == access {
            return;
        }
        client.access = access;
        let told = match access {
            PageAccess::View => Message::Auth(Some(READ_ONLY.to_owned())),
            PageAccess::Edit => Message::Auth(None),
        };
        client.outbox.send(encode(&told));
        info!(client = %id, user_id = %client.user_id, %access, "connection access changed");
    }

    /// Logs the pending updates, then compacts when enough are logged. A failure is logged
    /// and the write tried again later, the updates kept meanwhile, in order.
    async fn flush(&mut self) {
        self.storage.flush_at = None;
        if self.storage.pending.is_empty() {
            return;
        }
        let batch = self.storage.batch();
        let bytes: usize = batch.iter().map(|pending| pending.update.len()).sum();
        let rows: Vec<NewUpdate<'_>> = batch
            .iter()
            .map(|pending| NewUpdate {
                update: &pending.update,
                author_id: pending.author,
                created_at: pending.at,
            })
            .collect();
        let guard = self.inner.red.start("persist");
        let logged = documents::append(&self.inner.pool, self.page_id, &rows)
            .instrument(info_span!("sync persist", updates = rows.len(), bytes))
            .await;
        match logged {
            Ok(seq) => {
                guard.ok();
                debug!(updates = rows.len(), bytes, "updates logged");
                self.storage.logged(seq, rows.len(), bytes);
                let limits = &self.inner.limits;
                if self.storage.logged >= limits.compact_updates
                    || self.storage.logged_bytes >= limits.compact_bytes
                {
                    self.compact().await;
                }
            }
            Err(error) if documents::is_page_gone(&error) => {
                guard.ok();
                self.page_gone();
            }
            Err(error) => {
                guard.err();
                let retry = self.storage.retry_at(self.inner.limits.persist_delay);
                error!(
                    %error,
                    updates = rows.len(),
                    "cannot log the document's updates; they stay in memory to try again"
                );
                self.storage.flush_at = Some(retry);
            }
        }
    }

    /// Saves the whole document as the page's snapshot, in one transaction with deleting
    /// the updates it takes in and recording its text and latest change on the page.
    /// Returns whether it was saved. A failure is logged, and the updates stay.
    async fn compact(&mut self) -> bool {
        let guard = self.inner.red.start("compact");
        let (state, state_vector, text) = {
            let txn = self.awareness.doc().transact();
            let text = match neon_blocks::read(&txn, &self.fragment) {
                Ok(document) => Some(document.plain_text()),
                Err(error) => {
                    // Its structure, never its content.
                    warn!(%error, "the document does not read as blocks; its search text stays");
                    None
                }
            };
            (
                txn.encode_state_as_update_v1(&StateVector::default()),
                txn.state_vector().encode_v1(),
                text,
            )
        };
        let page_id = self.page_id;
        let last_seq = self.storage.last_seq;
        let edited = self.storage.edited;
        let at = self.inner.clock.now();
        let pool = &self.inner.pool;
        let saved = async {
            let mut tx = pool.begin().await?;
            documents::save_snapshot(
                &mut *tx,
                page_id,
                NewSnapshot {
                    state: &state,
                    state_vector: &state_vector,
                    last_seq,
                    at,
                },
            )
            .await?;
            let deleted = documents::delete_updates_through(&mut *tx, page_id, last_seq).await?;
            documents::set_content(&mut *tx, page_id, text.as_deref(), edited).await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(deleted)
        }
        .instrument(info_span!("sync compact", bytes = state.len()))
        .await;
        match saved {
            Ok(deleted) => {
                guard.ok();
                info!(bytes = state.len(), updates = deleted, "document compacted");
                self.storage.compacted(state.len());
                true
            }
            Err(error) if documents::is_page_gone(&error) => {
                guard.ok();
                self.page_gone();
                false
            }
            Err(error) => {
                guard.err();
                error!(%error, "cannot save the document's snapshot");
                false
            }
        }
    }

    /// The page is gone: everyone's connection closes, and nothing more is saved.
    fn page_gone(&mut self) {
        info!("the page was deleted; its document closes");
        self.gone = true;
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            self.close(id, CloseReason::AccessLost);
        }
        self.storage.pending.clear();
        self.storage.pending_bytes = 0;
        self.storage.flush_at = None;
    }

    /// Whether the document may unload, once nobody had it open for a while: it is saved,
    /// and no connection came meanwhile, which the registry's lock makes sure of while the
    /// document leaves it. When saving fails it stays, to try again later.
    async fn unload(&mut self, commands: &mut mpsc::Receiver<Command>) -> bool {
        if self.storage.needs_snapshot() && !self.compact().await && !self.gone {
            self.idle_since = Some(Instant::now());
            return false;
        }
        if !self.gone {
            let mut docs = self.inner.docs();
            if !commands.is_empty() {
                return false;
            }
            Inner::remove(&mut docs, self.page_id, self.generation);
        }
        debug!("document unloaded");
        true
    }

    /// The server stops: every connection closes, to reconnect to the next server, and the
    /// document is saved.
    async fn stop(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        if !ids.is_empty() {
            info!(
                connections = ids.len(),
                "the server stops; closing the document's connections"
            );
        }
        for id in ids {
            if let Some(client) = self.clients.get(&id) {
                client.outbox.close(CloseReason::Restarting);
            }
            self.leave(id);
        }
        if self.storage.needs_snapshot() && !self.compact().await && !self.gone {
            // The log keeps what the snapshot could not.
            self.flush().await;
        }
    }
}

/// Why a document did not load.
#[derive(Debug, thiserror::Error)]
enum LoadError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("a saved update does not decode: {0}")]
    Decode(#[from] yrs::encoding::read::Error),
    #[error("a saved update does not apply: {0}")]
    Apply(#[from] yrs::error::UpdateError),
}
