//! One live document: the task that owns a page's Yjs document and its awareness while
//! anyone has the page open, and for a while after.
//!
//! Its connections send it what their clients send, through one queue, and it answers
//! each in turn, so the document needs no lock. What it has to tell everyone goes through
//! the [`Broadcaster`](super::Broadcaster); answers to one client go straight to that
//! client's outbox. Updates it applies wait a moment to be logged together, and once enough
//! are logged, or the document unloads or the server stops, the whole document is saved as
//! a snapshot that takes them in.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use neon_blocks::edit::{self, Applied, Edit, EditError};
use neon_domain::auth::UserKind;
use neon_domain::pages::PageAccess;
use neon_store::documents::{self, Edited, LoggedUpdate, NewSnapshot, NewUpdate, Snapshot};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until};
use tracing::{Instrument as _, debug, error, info, info_span, warn};
use uuid::Uuid;
use yrs::block::ClientID;
use yrs::sync::{Awareness, AwarenessUpdate, Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Doc, Options, ReadTxn, StateVector, Transact, TransactionMut, Update, WriteTxn as _,
    XmlFragmentRef,
};

use super::broadcast::{ClientId, Outbox, Queued, Room};
use super::limits::Bucket;
use super::protocol::{self, encode};
use super::{CloseReason, Inner, LIVE_DOCUMENTS, MESSAGES};
use crate::history::record;

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
    /// A connection must close, for this reason.
    Close {
        client: ClientId,
        reason: CloseReason,
    },
    /// What of the document reads as blocks, as it is now.
    Read {
        done: oneshot::Sender<neon_blocks::Lenient>,
    },
    /// Blocks to add after the last one, as `author`, told to every connection and logged
    /// at once, or kept to log later when the database fails. The answer is dropped, rather
    /// than sent, when the document is unloading, as for a join.
    Append {
        author: Uuid,
        blocks: Vec<neon_blocks::Block>,
        done: oneshot::Sender<Result<(), AppendError>>,
    },
    /// Edits of the document's blocks, made by `author` from the server, all or none, on
    /// the document as it was at the revision `base`, when one is given; see
    /// [`revision`](super::revision).
    Edit {
        author: Uuid,
        edits: Vec<Edit>,
        base: Option<String>,
        done: oneshot::Sender<Result<Made, Unedited>>,
    },
    /// A version of the page to hold again, restored by `author` on the revision `base`:
    /// the document as it is is kept as a version first, then the restore is made as one
    /// change, and kept as a version naming `from`, the version restored and when it was
    /// made.
    Restore {
        author: Uuid,
        blocks: Vec<neon_blocks::Block>,
        from: (Uuid, time::OffsetDateTime),
        base: String,
        done: oneshot::Sender<Result<Made, Unedited>>,
    },
}

/// Why blocks were not added to a live document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AppendError {
    #[error("the page is gone")]
    Gone,
    #[error("the page's content would be larger than it may be")]
    TooLarge,
    #[error("the page's content is not a document of blocks")]
    NotADocument,
}

/// What a live document made of an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Made {
    /// What each part did, in order.
    pub(crate) applied: Vec<Applied>,
    /// The document's revision once they were made, when they were made on the revision
    /// the edit was based on: then it is that one and these parts alone.
    pub(crate) revision: Option<String>,
}

/// Why a live document made none of an edit's parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unedited {
    /// The document changed since the revision the edit was based on, and the edit may
    /// remove blocks: it would remove what was added or changed since.
    Conflict,
    /// The part at this index, counted from 0, cannot be made.
    Refused(usize, EditError),
    /// The document would grow larger than the server keeps.
    TooLarge,
    /// The page was deleted.
    Gone,
    /// The document as it was could not be saved first, as a restore needs.
    Unsaved,
}

/// A connection opening the document.
pub(crate) struct Joining {
    pub(crate) id: ClientId,
    pub(crate) user_id: Uuid,
    /// Who the connection is, as its awareness states show it: see [`Presence`].
    pub(crate) presence: Presence,
    pub(crate) access: PageAccess,
    pub(crate) outbox: Outbox,
}

/// Who a connection is, as the server writes it into the `user` field of every awareness
/// state it sends: the account's id, display name, hue and kind and, for an agent or
/// integration, the name of the person it acts for. Whatever the client put there is
/// replaced, so nobody shows under another person's name, nor an agent as a person, and
/// nothing but these reaches the others' editors, which draw cursors from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Presence {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) hue: u16,
    pub(crate) kind: UserKind,
    /// The owner's display name, for an agent or integration whose owner's account is
    /// still there.
    pub(crate) owner: Option<String>,
}

impl Presence {
    /// An awareness state with its `user` field set to this. A state that is not a JSON
    /// object stays as it is: `null` is how a client says its cursor left.
    fn stamp(&self, json: &str) -> Result<Arc<str>, CloseReason> {
        let mut state: serde_json::Value =
            serde_json::from_str(json).map_err(|_| CloseReason::InvalidMessage)?;
        let Some(fields) = state.as_object_mut() else {
            return Ok(json.into());
        };
        let mut user = serde_json::json!({
            "id": self.id.to_string(),
            "name": self.name,
            "hue": self.hue,
            "kind": self.kind.as_str(),
        });
        if let Some(owner) = &self.owner {
            user["owner"] = serde_json::Value::from(owner.as_str());
        }
        fields.insert("user".to_owned(), user);
        Ok(state.to_string().into())
    }
}

/// A connection that has the document open.
struct Client {
    user_id: Uuid,
    presence: Presence,
    access: PageAccess,
    outbox: Outbox,
    /// The awareness client ids its client holds: the cursors and presence it shows.
    awareness_ids: HashSet<ClientID>,
    /// The rates of its sync step 1 messages and of its awareness messages.
    sync_requests: Bucket,
    awareness: Bucket,
    /// The last answer to its sync step 1, until it is sent.
    answer: Option<Queued>,
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
    /// Who changed the document since the last snapshot, whom its version names.
    authors: BTreeSet<Uuid>,
    /// The version a restore made the document from, which the next version names.
    restored: Option<(Uuid, time::OffsetDateTime)>,
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
        self.authors.clear();
        self.restored = None;
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
            let saved = documents::saved(&inner.pool, page_id).await?;
            let after = saved
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.last_seq);
            Ok::<_, LoadError>((saved.snapshot, after, saved.updates))
        }
        .await
        .and_then(|(snapshot, after, updates)| {
            let doc = assemble(snapshot.as_ref(), &updates)?;
            let fragment = neon_blocks::fragment(&doc);
            let storage = Storage {
                last_seq: updates.last().map_or(after, |last| last.seq.max(after)),
                snapshot_bytes: snapshot.as_ref().map_or(0, |snapshot| snapshot.state.len()),
                logged: updates.len(),
                logged_bytes: updates.iter().map(|logged| logged.update.len()).sum(),
                edited: updates.last().map(|last| Edited {
                    at: last.created_at,
                    by: last.author_id,
                }),
                authors: updates
                    .iter()
                    .filter_map(|logged| logged.author_id)
                    .collect(),
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
            Command::Close { client, reason } => self.close(client, reason),
            Command::Read { done } => {
                let txn = self.awareness.doc().transact();
                let _ = done.send(neon_blocks::read_lenient(&txn, &self.fragment));
            }
            Command::Append {
                author,
                blocks,
                done,
            } => {
                let appended = self.append(author, &blocks).await;
                let _ = done.send(appended);
            }
            Command::Edit {
                author,
                edits,
                base,
                done,
            } => {
                let edited = self.edit(author, &edits, base.as_deref()).await;
                let _ = done.send(edited);
            }
            Command::Restore {
                author,
                blocks,
                from,
                base,
                done,
            } => {
                let restored = self.restore(author, blocks, from, &base).await;
                let _ = done.send(restored);
            }
        }
    }

    /// Adds blocks after the document's last one, as `author`: every connection gets the
    /// update, and it is logged at once rather than after the usual pause, so it is kept
    /// once the caller hears it was added; when the database fails then, it stays in memory
    /// and is logged again later, as every edit is. Refused when it would take the document
    /// past its size limit, which is told before anything changes, and when the page turns
    /// out to be gone as it is logged.
    async fn append(
        &mut self,
        author: Uuid,
        blocks: &[neon_blocks::Block],
    ) -> Result<(), AppendError> {
        if self.gone {
            return Err(AppendError::Gone);
        }
        let max = self.inner.limits.max_document;
        let estimate = {
            let scratch = Doc::new();
            let fragment = neon_blocks::fragment(&scratch);
            let mut txn = scratch.transact_mut();
            let mut ids = || Uuid::now_v7().to_string();
            neon_blocks::append(&mut txn, &fragment, blocks, &mut ids)
                .map_err(|_| AppendError::NotADocument)?;
            txn.encode_update_v1().len()
        };
        if self.storage.size() + estimate > max {
            if self.storage.needs_snapshot() {
                self.compact().await;
            }
            if self.gone {
                return Err(AppendError::Gone);
            }
            if self.storage.size() + estimate > max {
                return Err(AppendError::TooLarge);
            }
        }
        let changed = {
            let mut txn = self.awareness.doc().transact_mut();
            let mut ids = || Uuid::now_v7().to_string();
            neon_blocks::append(&mut txn, &self.fragment, blocks, &mut ids)
                .map_err(|_| AppendError::NotADocument)?;
            txn.encode_update_v1()
        };
        if changed.as_slice() != Update::EMPTY_V1 {
            let message = encode(&Message::Sync(SyncMessage::Update(changed.clone())));
            self.inner
                .broadcaster
                .broadcast(Room::Page(self.page_id), message, None);
            self.queue(changed, author);
            self.flush().await;
            if self.gone {
                return Err(AppendError::Gone);
            }
        }
        debug!(%author, blocks = blocks.len(), "blocks appended");
        Ok(())
    }

    /// Makes edits of the server's, as one change everyone connected gets at once and the
    /// log keeps as `author`'s. They are tried on a copy first, so that either every part
    /// is made or none, and the document's growth is known before it grows. With `base`,
    /// the revision the edit's author read, an edit that may remove blocks is refused once
    /// the document is at another revision, and one made on that revision tells the one it
    /// leaves.
    async fn edit(
        &mut self,
        author: Uuid,
        edits: &[Edit],
        base: Option<&str>,
    ) -> Result<Made, Unedited> {
        if self.gone {
            return Err(Unedited::Gone);
        }
        let max = self.inner.limits.max_document;
        let mut tried = self.trial(edits);
        let too_large = |tried: &Result<usize, Unedited>, size: usize| match tried {
            Ok(growth) => size + growth > max,
            Err(refused) => *refused == Unedited::TooLarge,
        };
        if too_large(&tried, self.storage.size()) && self.storage.needs_snapshot() {
            // The estimate errs large; a snapshot tells the size for sure.
            self.compact().await;
            if self.gone {
                return Err(Unedited::Gone);
            }
            tried = self.trial(edits);
        }
        if too_large(&tried, self.storage.size()) {
            return Err(Unedited::TooLarge);
        }
        tried?;
        let on_base = base.is_some_and(|base| base == self.revision());
        if base.is_some() && !on_base && edits.iter().any(Edit::removes) {
            return Err(Unedited::Conflict);
        }
        let room = self.room();
        let (made, changed) = {
            let mut txn = self.awareness.doc().transact_mut();
            let made = make(&mut txn, &self.fragment, edits, room);
            (made, txn.encode_update_v1())
        };
        // A part the trial made and this did not still leaves the parts before it, which
        // the others and the log must have too.
        if changed.as_slice() != Update::EMPTY_V1 {
            let message = encode(&Message::Sync(SyncMessage::Update(changed.clone())));
            self.inner
                .broadcaster
                .broadcast(Room::Page(self.page_id), message, None);
            self.queue(changed, author);
        }
        if let Err(refused) = &made {
            error!(
                ?refused,
                "an edit refused on the document after its trial passed"
            );
        }
        if self.clients.is_empty() {
            // Opened for the edit alone: it stays a while for the next.
            self.idle_since = Some(Instant::now());
        }
        let applied = made?;
        Ok(Made {
            revision: on_base.then(|| self.revision()),
            applied,
        })
    }

    /// Makes the document hold a version's blocks again, as `author`, when it is still at
    /// the revision `base` the restorer compared against: refused otherwise, so nothing
    /// written since is replaced unseen. The document as it is is saved first, and kept as
    /// a version unless the latest one holds it already, so the restore can be undone; then
    /// the restore is made as one edit, which merges as edits do with what is typed
    /// meanwhile in the blocks it keeps, and saved again, its version naming the one
    /// restored. Saving that last snapshot may fail, and is tried again as every snapshot
    /// is; the restore stands.
    async fn restore(
        &mut self,
        author: Uuid,
        blocks: Vec<neon_blocks::Block>,
        from: (Uuid, time::OffsetDateTime),
        base: &str,
    ) -> Result<Made, Unedited> {
        if self.gone {
            return Err(Unedited::Gone);
        }
        if base != self.revision() {
            return Err(Unedited::Conflict);
        }
        if !self.compact().await {
            return Err(if self.gone {
                Unedited::Gone
            } else {
                Unedited::Unsaved
            });
        }
        let made = self
            .edit(author, &[Edit::Restore { blocks }], Some(base))
            .await?;
        self.storage.restored = Some(from);
        if self.storage.needs_snapshot() {
            self.compact().await;
        }
        Ok(made)
    }

    /// The document's revision now, as reading it gives it.
    fn revision(&self) -> String {
        let txn = self.awareness.doc().transact();
        super::revision(&neon_blocks::read_lenient(&txn, &self.fragment).document)
    }

    /// How many bytes the document may grow by before it is larger than the server keeps.
    fn room(&self) -> usize {
        self.inner
            .limits
            .max_document
            .saturating_sub(self.storage.size())
    }

    /// Makes the edits on a copy of the document, and says how large their update is.
    fn trial(&self, edits: &[Edit]) -> Result<usize, Unedited> {
        let doc = self.awareness.doc();
        let (state, before) = {
            let txn = doc.transact();
            (
                txn.encode_state_as_update_v1(&StateVector::default()),
                txn.state_vector(),
            )
        };
        let copy = Doc::with_options(Options {
            offset_kind: doc.offset_kind(),
            ..Options::default()
        });
        let fragment = neon_blocks::fragment(&copy);
        let mut txn = copy.transact_mut();
        let loaded = Update::decode_v1(&state)
            .ok()
            .and_then(|update| txn.apply_update(update).ok());
        if loaded.is_none() {
            // The document's own state always loads; should it not, the edit is made on the
            // document itself, whose size check then counts the rest.
            return Ok(0);
        }
        make(&mut txn, &fragment, edits, self.room())?;
        Ok(txn.encode_state_as_update_v1(&before).len())
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
            .join(Room::Page(self.page_id), joining.id, joining.outbox.clone());
        debug!(
            client = %joining.id,
            user_id = %joining.user_id,
            access = %joining.access,
            "client joined"
        );
        let limits = &self.inner.limits;
        self.clients.insert(
            joining.id,
            Client {
                user_id: joining.user_id,
                presence: joining.presence,
                access: joining.access,
                outbox: joining.outbox,
                awareness_ids: HashSet::new(),
                sync_requests: Bucket::new(
                    limits.sync_requests_per_second,
                    limits.sync_request_burst,
                ),
                awareness: Bucket::new(limits.awareness_per_second, limits.awareness_burst),
                answer: None,
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
                    self.send_missing(id, &state_vector)
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

    /// Answers a client's sync step 1 with what its state vector lacks, unless the answer
    /// to its last one still waits to be sent: what that answer lacks, the client gets as
    /// the updates sent after it. Each answer may be the whole document, so they are few.
    fn send_missing(
        &mut self,
        id: ClientId,
        state_vector: &StateVector,
    ) -> Result<(), CloseReason> {
        let Some(client) = self.clients.get_mut(&id) else {
            return Ok(());
        };
        if !client.sync_requests.take() {
            return Err(CloseReason::TooManyMessages);
        }
        if client
            .answer
            .is_some_and(|answer| !client.outbox.was_taken(answer))
        {
            return Ok(());
        }
        let update = self
            .awareness
            .doc()
            .transact()
            .encode_state_as_update_v1(state_vector);
        client.answer = client
            .outbox
            .send_tracked(encode(&Message::Sync(SyncMessage::SyncStep2(update))));
        Ok(())
    }

    /// Applies a client's update, then tells the others what changed and logs it. From a
    /// client that only views the page, only an update that changes nothing is taken.
    ///
    /// What of an update needs changes the document does not have is dropped rather than
    /// kept waiting for them, and closes the connection as out of order: kept, it would grow
    /// the document unseen, past its size limit, as it changes nothing yet.
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
        let (applied, changed, stranded) = {
            let mut txn = self.awareness.doc().transact_mut();
            let applied = txn.apply_update(update);
            let stranded = txn.prune_pending().is_some();
            (applied, txn.encode_update_v1(), stranded)
        };
        // Even an update that failed may have changed some of the document, which the
        // others and the log must have too.
        if changed.as_slice() != Update::EMPTY_V1 {
            let message = encode(&Message::Sync(SyncMessage::Update(changed.clone())));
            self.inner
                .broadcaster
                .broadcast(Room::Page(self.page_id), message, Some(id));
            self.queue(changed, author);
        }
        applied.map_err(|error| {
            warn!(client = %id, %error, "a client's update did not apply");
            CloseReason::InvalidMessage
        })?;
        if stranded {
            warn!(client = %id, "a client's update needs changes the document lacks");
            return Err(CloseReason::OutOfOrder);
        }
        Ok(())
    }

    /// Queues an update to be logged soon, or at once when enough wait.
    fn queue(&mut self, update: Vec<u8>, author: Uuid) {
        let at = self.inner.clock.now();
        let limits = &self.inner.limits;
        self.storage.pending_bytes += update.len();
        self.storage.pending.push(Pending { update, author, at });
        self.storage.authors.insert(author);
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
    /// Past its rate, an update is dropped; a state past its size closes the connection.
    fn update_awareness(
        &mut self,
        id: ClientId,
        mut update: AwarenessUpdate,
    ) -> Result<(), CloseReason> {
        let limits = &self.inner.limits;
        let limit = limits.awareness_clients;
        if update
            .clients
            .values()
            .any(|entry| entry.json.len() > limits.max_awareness_state)
        {
            return Err(CloseReason::MessageTooBig);
        }
        let Some(client) = self.clients.get_mut(&id) else {
            return Ok(());
        };
        if !client.awareness.take() {
            return Ok(());
        }
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
        for entry in update.clients.values_mut() {
            entry.json = client.presence.stamp(&entry.json)?;
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
                Room::Page(self.page_id),
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
        self.inner.broadcaster.leave(Room::Page(self.page_id), id);
        if !client.awareness_ids.is_empty() {
            for awareness_id in &client.awareness_ids {
                self.holders.remove(awareness_id);
                self.awareness.remove_state(*awareness_id);
            }
            if let Ok(removed) = self.awareness.update_with_clients(client.awareness_ids) {
                self.inner.broadcaster.broadcast(
                    Room::Page(self.page_id),
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
    /// the updates it takes in, recording its text and latest change on the page, and
    /// keeping its blocks as a version when they changed. Returns whether it was saved. A
    /// failure is logged, and the updates stay.
    async fn compact(&mut self) -> bool {
        let guard = self.inner.red.start("compact");
        let (state, state_vector, text, document) = {
            let txn = self.awareness.doc().transact();
            let (text, document) = match neon_blocks::read(&txn, &self.fragment) {
                Ok(document) => (Some(document.plain_text()), document),
                Err(error) => {
                    // Its structure, never its content.
                    warn!(%error, "the document does not read as blocks; its search text stays");
                    (
                        None,
                        neon_blocks::read_lenient(&txn, &self.fragment).document,
                    )
                }
            };
            (
                txn.encode_state_as_update_v1(&StateVector::default()),
                txn.state_vector().encode_v1(),
                text,
                document,
            )
        };
        let page_id = self.page_id;
        let last_seq = self.storage.last_seq;
        let edited = self.storage.edited;
        let at = self.inner.clock.now();
        let authors: Vec<Uuid> = self.storage.authors.iter().copied().collect();
        let restored = self.storage.restored;
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
            let versioned = record::record(
                &mut tx,
                record::Snapshot {
                    page_id,
                    document: &document,
                    authors: &authors,
                    at: edited.map_or(at, |edited| edited.at),
                    restored_from: restored,
                },
            )
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>((deleted, versioned))
        }
        .instrument(info_span!("sync compact", bytes = state.len()))
        .await;
        match saved {
            Ok((deleted, versioned)) => {
                guard.ok();
                info!(
                    bytes = state.len(),
                    updates = deleted,
                    versioned,
                    "document compacted"
                );
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
            let mut registry = self.inner.registry();
            if !commands.is_empty() {
                return false;
            }
            registry.remove(self.page_id, self.generation);
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

/// Makes `edits` in `txn`, in order, until one is refused. What find and replace would add
/// is weighed, before it is added, against the `room` the document has, less what the parts
/// before added by replacing: so no part grows the document far past what it keeps, nor
/// takes the time that would, before its size is checked.
fn make(
    txn: &mut TransactionMut,
    fragment: &XmlFragmentRef,
    edits: &[Edit],
    mut room: usize,
) -> Result<Vec<Applied>, Unedited> {
    let mut ids = new_block_id;
    let mut made = Vec::with_capacity(edits.len());
    for (index, part) in edits.iter().enumerate() {
        match edit::apply(txn, fragment, part, &mut ids, room) {
            Ok(applied) => {
                room = room.saturating_sub(part.replaced_bytes(&applied));
                made.push(applied);
            }
            Err(EditError::TooLarge) => return Err(Unedited::TooLarge),
            Err(error) => return Err(Unedited::Refused(index, error)),
        }
    }
    Ok(made)
}

/// The id of a block the server adds.
fn new_block_id() -> String {
    Uuid::now_v7().to_string()
}

/// A page's document from its snapshot and the updates logged after it. What of them needs
/// changes they do not hold is left out, as a live document leaves it out, so a document
/// never holds what does not show.
pub(crate) fn assemble(
    snapshot: Option<&Snapshot>,
    updates: &[LoggedUpdate],
) -> Result<Doc, LoadError> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        if let Some(snapshot) = snapshot {
            txn.apply_update(Update::decode_v1(&snapshot.state)?)?;
        }
        for logged in updates {
            txn.apply_update(Update::decode_v1(&logged.update)?)?;
        }
        if txn.prune_pending().is_some() {
            warn!("a saved document holds changes that need others it lacks; left out");
        }
    }
    Ok(doc)
}

/// Why a page's saved document did not load.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("a saved update does not decode: {0}")]
    Decode(#[from] yrs::encoding::read::Error),
    #[error("a saved update does not apply: {0}")]
    Apply(#[from] yrs::error::UpdateError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_of(text: &str) -> (Doc, XmlFragmentRef) {
        let doc = Doc::new();
        let fragment = neon_blocks::fragment(&doc);
        {
            let mut txn = doc.transact_mut();
            neon_blocks::append(
                &mut txn,
                &fragment,
                &[neon_blocks::Block::paragraph(text)],
                &mut neon_blocks::SequentialIds::new("b"),
            )
            .expect("a document");
        }
        (doc, fragment)
    }

    fn find(find: &str, replace: &str) -> Edit {
        Edit::FindReplace {
            find: find.to_owned(),
            replace: replace.to_owned(),
            all: true,
        }
    }

    #[test]
    fn what_each_part_adds_by_replacing_leaves_less_room_for_the_next() {
        let (doc, fragment) = doc_of("a b");
        let edits = [find("a", "1234"), find("b", "5678")];
        let mut txn = doc.transact_mut();
        assert_eq!(
            make(&mut txn, &fragment, &edits, 7),
            Err(Unedited::TooLarge)
        );
        drop(txn);
        let (doc, fragment) = doc_of("a b");
        let mut txn = doc.transact_mut();
        let made = make(&mut txn, &fragment, &edits, 8).expect("room for both");
        assert_eq!(made.len(), 2);
        // A refusal of another kind names its part.
        assert_eq!(
            make(
                &mut txn,
                &fragment,
                &[find("1", "one"), find("absent", "")],
                8
            ),
            Err(Unedited::Refused(1, EditError::NotFound))
        );
    }
}
