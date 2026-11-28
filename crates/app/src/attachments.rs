//! Attachments: files uploaded to pages, for their image, video, audio and file blocks and
//! their covers; reading them back; and the hourly cleanup of those no page shows.
//!
//! - **Uploading** is let in first, by [`AttachmentService::admit_upload`]: the rules' word
//!   that the caller edits the page, their rate of uploads, the size the client declares
//!   against the largest file one takes, and a workspace not full already. Then the file
//!   streams to the [`BlobStore`] under a key made for it, never whole in memory: its first
//!   bytes say its type, which its kind of block must take, its size is counted against the
//!   largest file as it comes, and its SHA-256 and an image's width and height are read on
//!   the way. Once it is all written, one transaction locks the workspace, asks the rules
//!   again, checks the file fits the workspace's storage limit, and adds its row. A file
//!   refused on the way leaves nothing behind; one whose server stopped mid-way leaves a
//!   blob the cleanup deletes.
//! - **Reading** a file asks the rules about its page on every request: a file follows the
//!   page it was uploaded to, its shares, guests, its trash and agents' bounds, and one the
//!   caller may not see is not found, as one that does not exist.
//! - **Cleaning up** keeps every file a page's document or cover shows, in a block or as a
//!   link of its text. A file none shows
//!   is marked, and deleted once it stayed so for `attachments.orphan_grace`; being shown
//!   again unmarks it. Files go with their page and their workspace too, and the blob of
//!   every file deleted, however it went, is deleted from the store afterwards.
//!
//! Every use case runs in a span named `attachments <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]: a refusal counts as `ok`. Logs and spans carry a file's id,
//! type, size and its name's extension, never its name or bytes.

mod blobs;
mod cleanup;
mod error;
mod upload;

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use neon_domain::attachments::{File, FileKind, StorageBackend, file_id};
use neon_domain::auth::Principal;
use neon_domain::settings::Snapshot;
use neon_domain::workspaces::{Action, Limits};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::files;
use sqlx::{PgConnection, PgPool};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{Instrument as _, info_span, warn};
use uuid::Uuid;

use crate::clock::Clock;
use crate::pages::{Facts, PageError, PageService};
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::settings::{SettingsService, attachments as keys};
use crate::sync::SyncService;

pub use blobs::{
    BlobError, BlobRead, BlobStore, BlobWriter, Connector, LOCAL_PART_BYTES, ObjectBlobs,
    PART_BYTES, S3Location,
};
pub use cleanup::CleanupReport;
pub use error::AttachmentError;
pub(crate) use upload::Prepared;

/// RED subsystem of the attachment use cases.
pub const SUBSYSTEM: &str = "attachments";

/// How long one upload may take, from its first byte to its last, as the HTTP route allows.
pub const UPLOAD_TIME_LIMIT: Duration = Duration::from_secs(15 * 60);

/// How long an upload may send nothing before it is given up on, with what it held.
pub const UPLOAD_IDLE: Duration = Duration::from_secs(30);

/// How many uploads one person runs at once, their agents' with theirs: each holds a part
/// of its file in memory while it runs.
pub const UPLOADS_PER_USER: usize = 4;

/// How many uploads the server runs at once.
pub const UPLOADS_AT_ONCE: usize = 32;

/// How long the blob of an upload under way is kept without its file's row: past any
/// upload's time limit, so the cleanup never deletes one still being written.
const UPLOAD_WINDOW: Duration = Duration::from_secs(2 * 15 * 60);

/// The folder of the data directory local files are kept in.
pub const LOCAL_FOLDER: &str = "attachments";

/// The ids of the files a document shows, by their paths, in order and once each: those of
/// its image, video, audio and file blocks, and those its text links to.
pub fn shown_files(document: &neon_blocks::Document) -> Vec<Uuid> {
    document
        .files()
        .map(|(_, url)| url)
        .chain(document.links())
        .filter_map(file_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Where the blobs are: one store, as tests have it, or the one the settings name now,
/// made again whenever they change.
enum Blobs {
    Fixed(Arc<dyn BlobStore>),
    Settings {
        data_dir: PathBuf,
        connector: Connector,
        made: Mutex<Option<(Where, Arc<dyn BlobStore>)>>,
    },
}

/// What a blob store the settings name is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Where {
    Local(PathBuf),
    S3(S3Location),
}

impl Where {
    fn of(settings: &Snapshot, data_dir: &std::path::Path) -> Self {
        if StorageBackend::parse(settings.get(keys::STORAGE)) == Some(StorageBackend::S3) {
            Self::S3(S3Location {
                endpoint: settings.get(keys::S3_ENDPOINT).to_owned(),
                bucket: settings.get(keys::S3_BUCKET).to_owned(),
                region: settings.get(keys::S3_REGION).to_owned(),
                access_key_id: settings.get(keys::S3_ACCESS_KEY_ID).to_owned(),
                secret_access_key: settings.get(keys::S3_SECRET_ACCESS_KEY).expose().to_owned(),
                path_style: *settings.get(keys::S3_PATH_STYLE),
            })
        } else {
            Self::Local(data_dir.join(LOCAL_FOLDER))
        }
    }
}

/// The attachment use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct AttachmentService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    pages: PageService,
    sync: SyncService,
    blobs: Blobs,
    limiter: Arc<dyn RateLimiter>,
    clock: Clock,
    uploads: Uploads,
    red: Red,
}

/// The uploads running: how many each person runs, their agents' with theirs, and the
/// places among those the server runs at once.
struct Uploads {
    per_user: usize,
    running: Arc<Mutex<HashMap<Uuid, usize>>>,
    places: Arc<Semaphore>,
    idle: Duration,
}

/// An upload's place among those running, given back as it is dropped.
struct UploadSlot {
    running: Arc<Mutex<HashMap<Uuid, usize>>>,
    user_id: Uuid,
    _place: OwnedSemaphorePermit,
}

impl Drop for UploadSlot {
    fn drop(&mut self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = running.get_mut(&self.user_id) {
            *count -= 1;
            if *count == 0 {
                running.remove(&self.user_id);
            }
        }
    }
}

/// Puts an [`AttachmentService`] together. Without being told otherwise it limits rates in
/// memory, reads the system clock, runs [`UPLOADS_PER_USER`] uploads of one person and
/// [`UPLOADS_AT_ONCE`] in all at once, and gives up on an upload idle for [`UPLOAD_IDLE`].
pub struct AttachmentServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    pages: PageService,
    sync: SyncService,
    blobs: Blobs,
    limiter: Option<Arc<dyn RateLimiter>>,
    clock: Clock,
    uploads_per_user: usize,
    uploads_at_once: usize,
    upload_idle: Duration,
}

impl AttachmentServiceBuilder {
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// How many uploads one person runs at once, their agents' with theirs, and how many the
    /// server runs in all; at least one each.
    pub fn uploads_at_once(mut self, per_user: usize, in_all: usize) -> Self {
        self.uploads_per_user = per_user.max(1);
        self.uploads_at_once = in_all.max(1);
        self
    }

    /// How long an upload may send nothing before it is given up on.
    pub fn upload_idle(mut self, idle: Duration) -> Self {
        self.upload_idle = idle;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> AttachmentService {
        AttachmentService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                pages: self.pages,
                sync: self.sync,
                blobs: self.blobs,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                clock: self.clock,
                uploads: Uploads {
                    per_user: self.uploads_per_user,
                    running: Arc::new(Mutex::new(HashMap::new())),
                    places: Arc::new(Semaphore::new(self.uploads_at_once)),
                    idle: self.upload_idle,
                },
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

/// An upload let in: the caller edits the page, and may upload now. It says what the file
/// is for, and the most bytes it may have, and holds the upload's place among those
/// running until it is dropped.
pub struct UploadTicket {
    principal: Principal,
    workspace_id: Uuid,
    page_id: Uuid,
    kind: FileKind,
    max: u64,
    _slot: UploadSlot,
}

impl std::fmt::Debug for UploadTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadTicket")
            .field("user_id", &self.principal.user.id)
            .field("page_id", &self.page_id)
            .field("kind", &self.kind)
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

impl UploadTicket {
    /// The most bytes the file may have.
    pub fn max(&self) -> u64 {
        self.max
    }
}

/// A file the caller may read, as the rules said a moment ago.
#[derive(Debug, Clone)]
pub struct Readable {
    file: File,
}

impl Readable {
    pub fn file(&self) -> &File {
        &self.file
    }
}

/// A workspace's storage: what its files take and the most they may.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Storage {
    used: i64,
    limit: Option<u64>,
}

impl AttachmentService {
    /// Attachments of the pages `pages` keeps, read for the cleanup through `sync`, in the
    /// blob store the settings name: under `data_dir` on local disk, or in an S3 bucket
    /// reached through `connector`.
    pub fn builder(
        pool: PgPool,
        settings: Arc<SettingsService>,
        pages: PageService,
        sync: SyncService,
        data_dir: PathBuf,
        connector: Connector,
    ) -> AttachmentServiceBuilder {
        Self::with_blobs(
            pool,
            settings,
            pages,
            sync,
            Blobs::Settings {
                data_dir,
                connector,
                made: Mutex::new(None),
            },
        )
    }

    /// Attachments kept in `store` whatever the settings say, as tests keep them.
    pub fn builder_with_store(
        pool: PgPool,
        settings: Arc<SettingsService>,
        pages: PageService,
        sync: SyncService,
        store: Arc<dyn BlobStore>,
    ) -> AttachmentServiceBuilder {
        Self::with_blobs(pool, settings, pages, sync, Blobs::Fixed(store))
    }

    fn with_blobs(
        pool: PgPool,
        settings: Arc<SettingsService>,
        pages: PageService,
        sync: SyncService,
        blobs: Blobs,
    ) -> AttachmentServiceBuilder {
        AttachmentServiceBuilder {
            pool,
            settings,
            pages,
            sync,
            blobs,
            limiter: None,
            clock: Clock::system(),
            uploads_per_user: UPLOADS_PER_USER,
            uploads_at_once: UPLOADS_AT_ONCE,
            upload_idle: UPLOAD_IDLE,
        }
    }

    /// How long an upload may send nothing before it is given up on.
    pub(crate) fn upload_idle(&self) -> Duration {
        self.inner.uploads.idle
    }

    /// A place for an upload of the person `user_id` among those running, while they run
    /// fewer than they may and the server has room; [`AttachmentError::Busy`] otherwise.
    fn upload_slot(&self, user_id: Uuid) -> Result<UploadSlot, AttachmentError> {
        let uploads = &self.inner.uploads;
        let mut running = uploads
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let count = running.entry(user_id).or_default();
        if *count >= uploads.per_user {
            warn!(%user_id, "too many uploads of one person at once");
            return Err(AttachmentError::Busy);
        }
        let place = Arc::clone(&uploads.places)
            .try_acquire_owned()
            .map_err(|_| {
                warn!("too many uploads at once");
                AttachmentError::Busy
            })?;
        *count += 1;
        Ok(UploadSlot {
            running: Arc::clone(&uploads.running),
            user_id,
            _place: place,
        })
    }

    fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    fn settings(&self) -> Arc<Snapshot> {
        self.inner.settings.current()
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.clock.now()
    }

    /// The blob store files go to and come from now.
    pub fn blobs(&self) -> Result<Arc<dyn BlobStore>, AttachmentError> {
        match &self.inner.blobs {
            Blobs::Fixed(store) => Ok(Arc::clone(store)),
            Blobs::Settings {
                data_dir,
                connector,
                made,
            } => {
                let wanted = Where::of(&self.settings(), data_dir);
                let mut made = made.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((at, store)) = made.as_ref()
                    && *at == wanted
                {
                    return Ok(Arc::clone(store));
                }
                let store: Arc<dyn BlobStore> = Arc::new(match &wanted {
                    Where::Local(dir) => ObjectBlobs::local(dir)?,
                    Where::S3(location) => ObjectBlobs::s3(location, Arc::clone(connector))?,
                });
                *made = Some((wanted, Arc::clone(&store)));
                Ok(store)
            }
        }
    }

    /// The most bytes one upload takes now.
    pub fn max_file_bytes(&self) -> u64 {
        u64::try_from(*self.settings().get(keys::MAX_FILE_BYTES)).unwrap_or(0)
    }

    /// Lets an upload to a page in, before its body is read: the caller edits the page,
    /// has uploads left, declares no more than the largest file, the workspace is not full,
    /// and fewer uploads run than may, theirs and all. `declared` is the size of what the
    /// client says it sends, and `overhead` the most its form adds to the file.
    pub async fn admit_upload(
        &self,
        principal: &Principal,
        page_id: Uuid,
        kind: FileKind,
        declared: Option<u64>,
        overhead: u64,
    ) -> Result<UploadTicket, AttachmentError> {
        self.observe("admit_upload", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            context.check(Action::EditPage)?;
            let workspace_id = context.page().workspace_id;
            self.limit(principal).await?;
            let max = self.max_file_bytes();
            if declared.is_some_and(|declared| declared > max.saturating_add(overhead)) {
                return Err(AttachmentError::TooLarge { max });
            }
            let storage = self.storage_on(&mut conn, workspace_id).await?;
            if let Some(limit) = storage.limit
                && u64::try_from(storage.used).is_ok_and(|used| used >= limit)
            {
                return Err(AttachmentError::StorageLimit { limit });
            }
            Ok(UploadTicket {
                // A person's agents take places of theirs, so agents do not multiply them.
                _slot: self.upload_slot(principal.user.person_id())?,
                principal: principal.clone(),
                workspace_id,
                page_id,
                kind,
                max,
            })
        })
        .await
    }

    /// Streams an upload let in to the blob store, then keeps it as a file of its page, as
    /// the module docs describe. `name` is the name its client gave it.
    pub async fn upload<S, E>(
        &self,
        ticket: UploadTicket,
        name: &str,
        body: S,
    ) -> Result<File, AttachmentError>
    where
        S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Unpin + Send,
        E: std::fmt::Display,
    {
        self.observe("upload", async {
            let store = self.blobs()?;
            let prepared = upload::write(
                self,
                store.as_ref(),
                upload::Template {
                    workspace_id: ticket.workspace_id,
                    page_id: ticket.page_id,
                    uploaded_by: ticket.principal.user.id,
                    kind: ticket.kind,
                    name,
                    max: ticket.max,
                },
                body,
            )
            .await?;
            let kept = self.keep(&ticket.principal, &prepared).await;
            if let Err(error) = kept {
                self.discard(std::slice::from_ref(&prepared)).await;
                return Err(error);
            }
            let file = prepared.file;
            tracing::info!(
                user_id = %ticket.principal.user.id,
                page_id = %file.page_id,
                file_id = %file.id,
                kind = %file.kind,
                content_type = %file.media,
                bytes = file.size,
                extension = neon_domain::attachments::extension(&file.name).as_deref().unwrap_or(""),
                "file uploaded"
            );
            Ok(file)
        })
        .await
    }

    /// Adds a written file's row, in one transaction with locking its workspace, asking the
    /// rules again and checking its storage limit.
    async fn keep(
        &self,
        principal: &Principal,
        prepared: &Prepared,
    ) -> Result<(), AttachmentError> {
        let file = &prepared.file;
        let mut tx = self.pool().begin().await?;
        neon_store::workspaces::lock(&mut *tx, file.workspace_id)
            .await?
            .ok_or(AttachmentError::Page(PageError::NotFound("no such page")))?;
        let context = self
            .inner
            .pages
            .context(&mut tx, principal, file.page_id)
            .await?;
        context.check(Action::EditPage)?;
        if let Some(limit) =
            storage_full(&mut tx, &self.settings(), file.workspace_id, file.size).await?
        {
            return Err(AttachmentError::StorageLimit { limit });
        }
        files::insert(&mut *tx, file).await?;
        files::keep_blob(&mut *tx, &file.key).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the blobs of files written and not kept, now, and forgets them once gone. One
    /// the store does not delete stays left to the cleanup.
    pub(crate) async fn discard(&self, prepared: &[Prepared]) {
        let Ok(store) = self.blobs() else {
            return;
        };
        for file in prepared {
            match store.delete(&file.file.key).await {
                Ok(()) => {
                    if let Err(error) = files::blob_deleted(self.pool(), &file.file.key).await {
                        warn!(%error, file_id = %file.file.id, "a discarded blob stays listed for the cleanup");
                    }
                }
                Err(error) => {
                    warn!(%error, file_id = %file.file.id, "a discarded blob is left to the cleanup");
                }
            }
        }
    }

    /// Writes a file of an import to the blob store, not kept until its page is made: its
    /// kind takes its type, and it is no larger than an upload may be.
    pub(crate) async fn prepare(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        kind: FileKind,
        name: &str,
        bytes: bytes::Bytes,
    ) -> Result<Prepared, AttachmentError> {
        let store = self.blobs()?;
        let body = futures_util::stream::iter([Ok::<_, std::convert::Infallible>(bytes)]);
        upload::write(
            self,
            store.as_ref(),
            upload::Template {
                workspace_id,
                page_id: Uuid::nil(),
                uploaded_by: principal.user.id,
                kind,
                name,
                max: self.max_file_bytes(),
            },
            body,
        )
        .await
    }

    /// A file the caller may read: one of a page they see. Any other is not found, as one
    /// that does not exist.
    pub async fn file(&self, principal: &Principal, id: Uuid) -> Result<Readable, AttachmentError> {
        self.observe("open", async {
            let mut conn = self.pool().acquire().await?;
            let file = files::get(&mut *conn, id)
                .await?
                .ok_or(AttachmentError::NotFound)?;
            let seen = self
                .inner
                .pages
                .context(&mut conn, principal, file.page_id)
                .await
                .and_then(|context| context.check(Action::ViewPage));
            match seen {
                Ok(()) => Ok(Readable { file }),
                Err(PageError::NotFound(_) | PageError::Forbidden(_)) => {
                    Err(AttachmentError::NotFound)
                }
                Err(error) => Err(error.into()),
            }
        })
        .await
    }

    /// The files of `ids` the caller may read, by id: those of pages they see.
    pub async fn readable(
        &self,
        principal: &Principal,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Readable>, AttachmentError> {
        self.observe("list", async {
            if ids.is_empty() {
                return Ok(HashMap::new());
            }
            let mut conn = self.pool().acquire().await?;
            let found = files::by_ids(&mut *conn, ids).await?;
            let pages: Vec<Uuid> = found
                .iter()
                .map(|file| file.page_id)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let seen = Facts::read(&mut conn, principal, &pages)
                .await?
                .seen(principal)?;
            Ok(found
                .into_iter()
                .filter(|file| seen.get(&file.page_id).is_some_and(Option::is_some))
                .map(|file| (file.id, Readable { file }))
                .collect())
        })
        .await
    }

    /// A readable file's bytes, or `range` of them, which must be within the file.
    pub async fn read(
        &self,
        readable: &Readable,
        range: Option<Range<u64>>,
    ) -> Result<BlobRead, AttachmentError> {
        if range
            .as_ref()
            .is_some_and(|range| range.start >= range.end || range.end > readable.file.size)
        {
            return Err(AttachmentError::InvalidInput(
                "the range is not within the file".to_owned(),
            ));
        }
        Ok(self.blobs()?.read(&readable.file.key, range).await?)
    }

    async fn storage_on(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
    ) -> Result<Storage, AttachmentError> {
        Ok(Storage {
            used: files::usage(&mut *conn, workspace_id).await?,
            limit: storage_limit(conn, &self.settings(), workspace_id).await?,
        })
    }

    async fn limit(&self, principal: &Principal) -> Result<(), AttachmentError> {
        self.inner
            .limiter
            .check(Bucket::FileUpload, &principal.user.id.to_string())
            .await
            .map_err(|limited| {
                warn!(bucket = Bucket::FileUpload.prefix(), "rate limited");
                AttachmentError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, AttachmentError>>,
    ) -> Result<T, AttachmentError> {
        let span = info_span!(
            "attachments",
            otel.name = %format!("attachments {operation}"),
            attachments.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }
}

/// The storage limit that holds for a workspace now, if any.
async fn storage_limit(
    conn: &mut PgConnection,
    settings: &Snapshot,
    workspace_id: Uuid,
) -> Result<Option<u64>, sqlx::Error> {
    let overrides = neon_store::workspaces::limits(&mut *conn, workspace_id)
        .await?
        .unwrap_or_default();
    Ok(Limits::resolve(overrides, crate::workspaces::limit_defaults(settings)).storage_bytes)
}

/// The storage limit a workspace would pass with `adding` bytes more, if it would. Run it
/// after locking the workspace, so the usage is the one the change sees.
pub(crate) async fn storage_full(
    conn: &mut PgConnection,
    settings: &Snapshot,
    workspace_id: Uuid,
    adding: u64,
) -> Result<Option<u64>, sqlx::Error> {
    let Some(limit) = storage_limit(conn, settings, workspace_id).await? else {
        return Ok(None);
    };
    let used = files::usage(&mut *conn, workspace_id).await?;
    let limits = Limits {
        storage_bytes: Some(limit),
        ..Limits::default()
    };
    Ok((!limits.admits_bytes(used, adding)).then_some(limit))
}

#[cfg(test)]
mod tests {
    use neon_blocks::{Block, BlockKind, Document, FileProps, ImageProps};

    use super::*;

    #[test]
    fn documents_show_their_files_by_path() {
        let a = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00f1);
        let b = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00f2);
        let image = |url: String| {
            Block::new(BlockKind::Image {
                props: ImageProps {
                    url,
                    ..ImageProps::default()
                },
            })
        };
        let document = Document::new(vec![
            image(neon_domain::attachments::path(b)),
            image("https://example.com/elsewhere.png".to_owned()),
            Block::paragraph("text").with_children(vec![image(neon_domain::attachments::path(a))]),
            Block::new(BlockKind::File {
                props: FileProps {
                    url: neon_domain::attachments::path(b),
                    ..FileProps::default()
                },
            }),
        ]);
        assert_eq!(shown_files(&document), [a, b]);
        let c = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00f3);
        let linked = neon_blocks::markdown::from_markdown(&format!(
            "A [report]({}) and [elsewhere](https://example.com/{c}).\n",
            neon_domain::attachments::path(c)
        ));
        assert_eq!(
            shown_files(&linked),
            [c],
            "a link of the text shows its file"
        );
        assert!(shown_files(&Document::default()).is_empty());
    }
}
