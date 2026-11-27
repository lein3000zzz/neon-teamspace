//! Import and export: pages in from a markdown file or a zip of them, and out as markdown, a
//! page alone, a page with the pages under it, or a teamspace's pages, zipped.
//!
//! - **Importing** is let in first, by [`TransferService::admit_import`]: the rules' word
//!   that the caller may add pages where the import goes, their rate limit, and a turn
//!   among the transfers they run. Then the upload is read, and once it is, the import
//!   takes a place among those the server runs: a markdown file is one page, and in a
//!   zip each markdown file is one and each folder holding some is one too, a folder and a
//!   markdown file of its name beside it being the same page. A file's first heading, or
//!   else its name, is the title. The pages are made in one transaction, each with its
//!   markdown, as blocks, for the first snapshot of its document. Other files of a zip that
//!   a page's blocks or the links of its text lead to by a relative path become files of
//!   the first page that shows them, as uploads do, their links the files' paths; the rest
//!   are left out, and the answer lists them.
//! - **Exporting** writes a page's markdown file, its title as its first heading; a zip of a
//!   page's file and those of the pages under it, each page's in the folder of the page it
//!   is under; or a zip of a teamspace's pages. Content comes from the live document when
//!   the page is open, else from what is saved. A database's file ends with a table of its
//!   rows, their titles and the properties its first view shows; in a zip, each row is a
//!   file of its own in the database's folder, its properties listed under its title, the
//!   rows read whole a batch at a time. In a zip, the files a page's blocks show, or its
//!   text links to, that the caller may read go in the page's folder, its links to them
//!   made relative.
//! - **Limits** bound everything: the upload, what a zip unpacks to, its entries and how
//!   deep they go, the pages one import makes and the pages and rows one export holds, the
//!   time each takes, and how many run at once, on the server and for each person; see
//!   [`Limits`]. The rules answer before any limit, so someone who may not see a page or
//!   place is told so. A zip is read and written in memory, never on disk, and no entry's
//!   path may lead outside it.
//! - **Content** that does not read as blocks, which the editor would not show either, is
//!   left out of an export rather than failing it.
//!
//! Every use case runs in a span named `transfer <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`], counted as the page use cases are: a refusal is `ok`.

mod archive;
mod convert;
mod error;
mod limits;
mod write;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use neon_domain::auth::Principal;
use neon_domain::pages::PageKind;
use neon_domain::pages::files::{EXTENSION, file_name};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{Instrument as _, info, info_span, warn};
use uuid::Uuid;

use crate::attachments::{AttachmentError, AttachmentService, Prepared, shown_files};
use crate::databases::{DatabaseService, ExportRow};
use crate::pages::{Location, OutlinePage, PageService, PageView};
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::sync::SyncService;
use write::ZipExport;

pub use error::TransferError;
pub use limits::Limits;

/// RED subsystem of imports and exports.
pub const SUBSYSTEM: &str = "transfer";

/// A file uploaded to be imported.
#[derive(Debug, Clone)]
pub struct Upload {
    /// Its name as the client sent it, whose extension says what it is: `.md` or
    /// `.markdown` for a markdown file, `.zip` for a zip of them.
    pub name: String,
    pub bytes: Bytes,
}

/// Why a file of a zip was not imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Not a markdown file, such as an image, and no page links to it.
    Attachment,
    /// A file a page links to whose type its block does not take.
    UnsupportedType,
    /// A file a page links to that is larger than an upload may be.
    TooLarge,
    /// Hidden: its name, or a folder's it is in, starts with a dot.
    Hidden,
    /// A symbolic link.
    Link,
    /// Encrypted.
    Encrypted,
    /// Compressed otherwise than deflated or stored.
    Compression,
    /// Not text in UTF-8.
    NotText,
    /// A second markdown file for a page that has one, such as `Notes.markdown` beside
    /// `Notes.md`.
    Duplicate,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attachment => "attachment",
            Self::UnsupportedType => "unsupported_type",
            Self::TooLarge => "too_large",
            Self::Hidden => "hidden",
            Self::Link => "link",
            Self::Encrypted => "encrypted",
            Self::Compression => "unsupported_compression",
            Self::NotText => "not_text",
            Self::Duplicate => "duplicate",
        }
    }
}

/// A file of a zip left out of an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Its path in the zip.
    pub path: String,
    pub reason: SkipReason,
}

/// What an import made, and what it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// The pages made where the import went, in order; the pages under them came with them.
    pub pages: Vec<PageView>,
    /// How many pages were made, those under them included.
    pub count: usize,
    pub skipped: Vec<Skipped>,
}

/// What an export is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One page's markdown file.
    Markdown,
    /// A zip of markdown files in folders.
    Zip,
}

impl Format {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Zip => "application/zip",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => EXTENSION,
            Self::Zip => "zip",
        }
    }
}

/// A file exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// Its name, with its extension: the page's or the teamspace's, as a file system takes
    /// it.
    pub name: String,
    pub format: Format,
    pub bytes: Vec<u8>,
}

/// An export made: its file, and its place and turn, which whoever sends the file keeps
/// until its last byte is sent, so a client that reads slowly holds them as long as the
/// file; the place for at most [`Limits::place_while_sent`].
#[derive(Debug)]
pub struct Exported {
    pub export: Export,
    pub hold: Hold,
}

/// An import let in: the rules let the caller add pages where it goes, and it holds one of
/// the caller's turns of [`Limits::per_user`] until it ends.
pub struct ImportTicket {
    principal: Principal,
    workspace_id: Uuid,
    location: Location,
    turn: Arc<Turn>,
}

/// What an import or export holds while any of its work runs: one of the places of
/// [`Limits::parallel`] and one of its person's turns of [`Limits::per_user`]. Work off the
/// async threads keeps a clone until it ends, so a deadline, which drops the rest of the
/// work, lets go of neither while it still runs; an export's file keeps one until it is
/// sent, the place for [`Limits::place_while_sent`] at most.
#[derive(Clone)]
pub struct Hold {
    held: Arc<Held>,
}

/// A place and a turn, held together; the place may go first. When both go at the end,
/// the turn goes before the place (fields drop in order), so whoever takes the place next
/// finds the turn already given back.
struct Held {
    _turn: Arc<Turn>,
    place: Mutex<Option<OwnedSemaphorePermit>>,
}

impl Hold {
    /// Gives the place back once `after` has passed, if the hold lasts that long; its
    /// person's turn stays until the hold ends.
    fn place_for(&self, after: Duration) {
        let held = Arc::downgrade(&self.held);
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if let Some(held) = held.upgrade() {
                let place = held
                    .place
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                drop(place);
            }
        });
    }
}

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hold")
    }
}

/// One of a person's turns of [`Limits::per_user`], held while their import or export runs.
struct Turn {
    running: Arc<Mutex<HashMap<Uuid, usize>>>,
    user_id: Uuid,
}

impl Drop for Turn {
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

/// The import and export use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct TransferService {
    inner: Arc<Inner>,
}

struct Inner {
    pages: PageService,
    sync: SyncService,
    databases: DatabaseService,
    attachments: AttachmentService,
    limiter: Arc<dyn RateLimiter>,
    limits: Limits,
    places: Arc<Semaphore>,
    /// How many transfers each person runs.
    running: Arc<Mutex<HashMap<Uuid, usize>>>,
    red: Red,
}

/// Puts a [`TransferService`] together. Without being told otherwise it limits rates in
/// memory, with the default limits.
pub struct TransferServiceBuilder {
    pages: PageService,
    sync: SyncService,
    databases: DatabaseService,
    attachments: AttachmentService,
    limiter: Option<Arc<dyn RateLimiter>>,
    limits: Limits,
}

impl TransferServiceBuilder {
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> TransferService {
        TransferService {
            inner: Arc::new(Inner {
                pages: self.pages,
                sync: self.sync,
                databases: self.databases,
                attachments: self.attachments,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                places: Arc::new(Semaphore::new(self.limits.parallel)),
                running: Arc::default(),
                limits: self.limits,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl TransferService {
    /// Imports and exports of `pages`, reading the content of pages through `sync`, the
    /// rows of databases through `databases`, and the files pages show through
    /// `attachments`.
    pub fn builder(
        pages: PageService,
        sync: SyncService,
        databases: DatabaseService,
        attachments: AttachmentService,
    ) -> TransferServiceBuilder {
        TransferServiceBuilder {
            pages,
            sync,
            databases,
            attachments,
            limiter: None,
            limits: Limits::default(),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Lets an import in, before its file is read: the rules' word that the caller may add
    /// pages at `location` of the workspace, their rate limit, and one of their turns.
    pub async fn admit_import(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        location: Location,
    ) -> Result<ImportTicket, TransferError> {
        self.observe("admit_import", async {
            self.inner
                .pages
                .check_place(principal, workspace_id, location)
                .await?;
            self.limit(Bucket::PageImport, principal).await?;
            let turn = self.turn(principal)?;
            Ok(ImportTicket {
                principal: principal.clone(),
                workspace_id,
                location,
                turn: Arc::new(turn),
            })
        })
        .await
    }

    /// Imports an upload where its ticket says, as the module docs describe: the pages made
    /// go last there. Nothing is made unless all of it can be.
    pub async fn import(
        &self,
        ticket: ImportTicket,
        upload: Upload,
    ) -> Result<ImportReport, TransferError> {
        self.observe("import", async {
            let hold = self.hold(Arc::clone(&ticket.turn))?;
            let limits = self.inner.limits.clone();
            let deadline = Deadline::after(limits.time_limit);
            let sync_limits = self.inner.sync.limits();
            let sizes = convert::Sizes {
                document: sync_limits.max_document,
                nodes: sync_limits.max_nodes,
            };
            let work = async {
                let bytes = upload.bytes.clone();
                let read_limits = limits.clone();
                let (mut contents, parsed) = blocking(&hold, move || {
                    let mut contents = archive::read(&upload, &read_limits, deadline)?;
                    let parsed = convert::parse(std::mem::take(&mut contents.entries), deadline)?;
                    if convert::nodes(&parsed) > read_limits.max_nodes {
                        return Err(TransferError::too_large(format!(
                            "the pages make more than {} blocks, table rows and cells, runs \
                             of text, line breaks and mentions, the most one import writes: \
                             import fewer at once",
                            read_limits.max_nodes
                        )));
                    }
                    Ok((contents, parsed))
                })
                .await?;
                let links = convert::links(&parsed, &contents.files);
                let prepared = self
                    .import_files(&ticket, &bytes, &mut contents, links, deadline, &hold)
                    .await?;
                let made: HashMap<String, (usize, neon_domain::attachments::File)> = prepared
                    .iter()
                    .map(|(link, file)| (link.path.clone(), (link.owner, file.file.clone())))
                    .collect();
                let imported = async {
                    let pages = blocking(&hold, move || {
                        convert::finish(parsed, &made, sizes, deadline)
                    })
                    .await?;
                    Ok::<_, TransferError>(
                        self.inner
                            .pages
                            .import_pages(
                                &ticket.principal,
                                ticket.workspace_id,
                                ticket.location,
                                pages,
                            )
                            .await?,
                    )
                }
                .await;
                let imported = match imported {
                    Ok(imported) => imported,
                    Err(error) => {
                        let files: Vec<Prepared> =
                            prepared.into_iter().map(|(_, file)| file).collect();
                        self.inner.attachments.discard(&files).await;
                        return Err(error);
                    }
                };
                Ok(ImportReport {
                    pages: imported.pages,
                    count: imported.count,
                    skipped: contents.skipped,
                })
            };
            let report = deadline.bound(work).await?;
            info!(
                user_id = %ticket.principal.user.id,
                workspace_id = %ticket.workspace_id,
                pages = report.count,
                skipped = report.skipped.len(),
                "import done"
            );
            Ok(report)
        })
        .await
    }

    /// Writes the files of an upload its pages link to to the blob store, each as a file of
    /// the kind its first block shows, within the limits of an upload and of what an import
    /// unpacks. A file refused for its type or size is listed as left out for that; one
    /// imported leaves the list. Any other failure discards what was written.
    async fn import_files(
        &self,
        ticket: &ImportTicket,
        upload: &Bytes,
        contents: &mut archive::Contents,
        links: Vec<convert::Link>,
        deadline: Deadline,
        hold: &Hold,
    ) -> Result<Vec<(convert::Link, Prepared)>, TransferError> {
        let attachments = &self.inner.attachments;
        let max_file = attachments.max_file_bytes();
        let most = u64::try_from(self.inner.limits.max_files_unpacked).unwrap_or(u64::MAX);
        let mut made: Vec<(convert::Link, Prepared)> = Vec::new();
        let mut unpacked: u64 = 0;
        let written = async {
            for link in links {
                deadline.check()?;
                let Some(&index) = contents.files.get(&link.path) else {
                    continue;
                };
                let bytes = upload.clone();
                let budget = most.saturating_sub(unpacked);
                let data = blocking(hold, move || {
                    archive::unpack(bytes, index, max_file, budget)
                })
                .await?;
                let refused = match data {
                    archive::Unpacked::TooLarge => Some(SkipReason::TooLarge),
                    archive::Unpacked::OverBudget => {
                        return Err(TransferError::too_large(format!(
                            "the files the pages show unpack to more than {}, the most one \
                             import takes",
                            size(self.inner.limits.max_files_unpacked)
                        )));
                    }
                    archive::Unpacked::Data(data) => {
                        unpacked =
                            unpacked.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
                        let prepared = attachments
                            .prepare(
                                &ticket.principal,
                                ticket.workspace_id,
                                link.kind,
                                &link.name,
                                data,
                            )
                            .await;
                        match prepared {
                            Ok(prepared) => {
                                made.push((link.clone(), prepared));
                                None
                            }
                            Err(AttachmentError::UnsupportedType { .. }) => {
                                Some(SkipReason::UnsupportedType)
                            }
                            Err(AttachmentError::TooLarge { .. }) => Some(SkipReason::TooLarge),
                            Err(error) => return Err(error.into()),
                        }
                    }
                };
                let path = archive::shown(&link.path);
                match refused {
                    None => contents.skipped.retain(|skipped| skipped.path != path),
                    Some(reason) => {
                        for skipped in &mut contents.skipped {
                            if skipped.path == path {
                                skipped.reason = reason;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = written {
            let files: Vec<Prepared> = made.into_iter().map(|(_, file)| file).collect();
            attachments.discard(&files).await;
            return Err(error);
        }
        Ok(made)
    }

    /// A page the caller sees as a markdown file, or with `children`, as a zip holding its
    /// file and those of the pages under it.
    pub async fn export_page(
        &self,
        principal: &Principal,
        page_id: Uuid,
        children: bool,
    ) -> Result<Exported, TransferError> {
        self.observe("export_page", async {
            self.inner.pages.page(principal, page_id).await?;
            self.limit(Bucket::PageExport, principal).await?;
            let hold = self.hold(Arc::new(self.turn(principal)?))?;
            let deadline = Deadline::after(self.inner.limits.time_limit);
            let export = deadline
                .bound(async {
                    let pages = self
                        .inner
                        .pages
                        .page_outline(principal, page_id, children, self.outline_limit())
                        .await?;
                    self.check_count(&pages)?;
                    let first = pages.first().cloned().ok_or_else(|| {
                        TransferError::Internal(format!("page {page_id} has no outline"))
                    })?;
                    if children {
                        self.zipped(principal, &first.title, pages, deadline, &hold)
                            .await
                    } else {
                        let document = self.content(first.id).await?;
                        let extra = self.extra(principal, &first).await?;
                        let name = named(&first.title, Format::Markdown);
                        let bytes = blocking(&hold, move || {
                            Ok(convert::markdown(&first.title, document, &extra))
                        })
                        .await?
                        .into_bytes();
                        Ok(Export {
                            name,
                            format: Format::Markdown,
                            bytes,
                        })
                    }
                })
                .await?;
            info!(
                user_id = %principal.user.id,
                page_id = %page_id,
                bytes = export.bytes.len(),
                "page exported"
            );
            // A client that reads the file slowly keeps the person's turn while it does,
            // but no place past a while, so slow readers cannot take every place.
            hold.place_for(self.inner.limits.place_while_sent);
            Ok(Exported { export, hold })
        })
        .await
    }

    /// A teamspace's pages out of the trash as a zip, for everyone who may list them: the
    /// pages at its top at the top of the zip.
    pub async fn export_teamspace(
        &self,
        principal: &Principal,
        teamspace_id: Uuid,
    ) -> Result<Exported, TransferError> {
        self.observe("export_teamspace", async {
            self.inner
                .pages
                .check_teamspace_pages(principal, teamspace_id)
                .await?;
            self.limit(Bucket::PageExport, principal).await?;
            let hold = self.hold(Arc::new(self.turn(principal)?))?;
            let deadline = Deadline::after(self.inner.limits.time_limit);
            let export = deadline
                .bound(async {
                    let outline = self
                        .inner
                        .pages
                        .teamspace_outline(principal, teamspace_id, self.outline_limit())
                        .await?;
                    self.check_count(&outline.pages)?;
                    self.zipped(principal, &outline.name, outline.pages, deadline, &hold)
                        .await
                })
                .await?;
            info!(
                user_id = %principal.user.id,
                teamspace_id = %teamspace_id,
                bytes = export.bytes.len(),
                "teamspace exported"
            );
            // A client that reads the file slowly keeps the person's turn while it does,
            // but no place past a while, so slow readers cannot take every place.
            hold.place_for(self.inner.limits.place_while_sent);
            Ok(Exported { export, hold })
        })
        .await
    }

    /// A zip named after `name` of the pages' files, parents first.
    async fn zipped(
        &self,
        principal: &Principal,
        name: &str,
        pages: Vec<OutlinePage>,
        deadline: Deadline,
        hold: &Hold,
    ) -> Result<Export, TransferError> {
        // Boxed, as it moves to a thread and back for each page.
        let mut zip = Box::new(ZipExport::new(self.inner.limits.max_export_bytes));
        // The path in the zip of each file added, and how many bytes they take.
        let mut paths: HashMap<Uuid, String> = HashMap::new();
        let mut file_bytes: u64 = 0;
        // The properties of the batch of rows read last, by row.
        let mut rows: HashMap<Uuid, ExportRow> = HashMap::new();
        for (index, page) in pages.iter().enumerate() {
            deadline.check()?;
            let document = self.content(page.id).await?;
            let databases = &self.inner.databases;
            let extra = match page.kind {
                PageKind::Page => convert::Extra::None,
                PageKind::Database => {
                    convert::Extra::Table(databases.export_table(principal, page.id).await?)
                }
                PageKind::Row => convert::Extra::Properties(match rows.remove(&page.id) {
                    Some(row) => row,
                    None => match page.parent_id {
                        // Its database is in the zip, and its rows follow it: this one and
                        // those after it are read together, a batch at a time.
                        Some(database_id) => {
                            let batch: Vec<Uuid> = pages[index..]
                                .iter()
                                .take_while(|next| {
                                    next.kind == PageKind::Row && next.parent_id == page.parent_id
                                })
                                .take(self.inner.limits.export_rows_at_once.max(1))
                                .map(|next| next.id)
                                .collect();
                            rows = databases
                                .export_rows(principal, database_id, &batch)
                                .await?;
                            // A row gone since the outline was read has no properties.
                            rows.remove(&page.id).unwrap_or_default()
                        }
                        // The zip starts at the row: its database is not in it.
                        None => databases.export_row(principal, page.id).await?,
                    },
                }),
            };
            let files = self
                .files_to_add(principal, &document, &paths, &mut file_bytes)
                .await?;
            let page = page.clone();
            (zip, paths) = blocking(hold, move || {
                let placed = zip.place(&page);
                for (file, bytes) in files {
                    let path = zip.attach(page.id, &file, &bytes)?;
                    paths.insert(file.id, path);
                }
                let mut document = document;
                let folder: Vec<&str> = placed.folder.iter().map(String::as_str).collect();
                let in_zip = |url: &str| {
                    neon_domain::attachments::file_id(url)
                        .and_then(|id| paths.get(&id))
                        .map(|path| convert::relative(&folder, path))
                };
                document.map_files(|_, url| in_zip(url));
                document.map_links(in_zip);
                let markdown = convert::markdown(&page.title, document, &extra);
                zip.write(&placed, &markdown)?;
                Ok((zip, paths))
            })
            .await?;
        }
        Ok(Export {
            name: named(name, Format::Zip),
            format: Format::Zip,
            bytes: blocking(hold, move || zip.finish()).await?,
        })
    }

    /// The files a page's document shows that the caller may read and the zip lacks yet,
    /// with their bytes, read whole: within [`Limits::max_export_files`] for the whole
    /// export, which `file_bytes` counts.
    async fn files_to_add(
        &self,
        principal: &Principal,
        document: &neon_blocks::Document,
        added: &HashMap<Uuid, String>,
        file_bytes: &mut u64,
    ) -> Result<Vec<(neon_domain::attachments::File, Vec<u8>)>, TransferError> {
        let ids: Vec<Uuid> = shown_files(document)
            .into_iter()
            .filter(|id| !added.contains_key(id))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let attachments = &self.inner.attachments;
        let readable = attachments.readable(principal, &ids).await?;
        let most = u64::try_from(self.inner.limits.max_export_files).unwrap_or(u64::MAX);
        let mut files = Vec::with_capacity(readable.len());
        for id in ids {
            let Some(readable) = readable.get(&id) else {
                continue;
            };
            *file_bytes = file_bytes.saturating_add(readable.file().size);
            if *file_bytes > most {
                return Err(TransferError::too_large(format!(
                    "the pages show more than {} of files, the most one export holds: export \
                     fewer at once",
                    size(self.inner.limits.max_export_files)
                )));
            }
            let mut bytes = Vec::with_capacity(usize::try_from(readable.file().size).unwrap_or(0));
            let mut read = attachments.read(readable, None).await?.bytes;
            while let Some(chunk) = futures_util::StreamExt::next(&mut read).await {
                bytes.extend_from_slice(&chunk.map_err(AttachmentError::from)?);
            }
            files.push((readable.file().clone(), bytes));
        }
        Ok(files)
    }

    /// What a page's file holds besides its title and blocks, exported alone: a database's
    /// table of rows, or a row's properties.
    async fn extra(
        &self,
        principal: &Principal,
        page: &OutlinePage,
    ) -> Result<convert::Extra, TransferError> {
        let databases = &self.inner.databases;
        Ok(match page.kind {
            PageKind::Page => convert::Extra::None,
            PageKind::Database => {
                convert::Extra::Table(databases.export_table(principal, page.id).await?)
            }
            PageKind::Row => {
                convert::Extra::Properties(databases.export_row(principal, page.id).await?)
            }
        })
    }

    async fn content(&self, page_id: Uuid) -> Result<neon_blocks::Document, TransferError> {
        self.inner
            .sync
            .content(page_id)
            .await
            .map_err(|error| TransferError::Content { page_id, error })
    }

    /// How many pages, rows included, an outline reads at most: one more than an export
    /// holds tells there are too many.
    fn outline_limit(&self) -> usize {
        let limits = &self.inner.limits;
        limits
            .max_export_pages
            .saturating_add(limits.max_export_rows)
    }

    /// Refuses an outline with more pages, or rows of databases, than one export holds.
    /// One cut at [`outline_limit`](Self::outline_limit) and one more has too many of one.
    fn check_count(&self, pages: &[OutlinePage]) -> Result<(), TransferError> {
        let limits = &self.inner.limits;
        let rows = pages
            .iter()
            .filter(|page| page.kind == PageKind::Row)
            .count();
        let (max_pages, max_rows) = (limits.max_export_pages, limits.max_export_rows);
        if pages.len() - rows > max_pages {
            return Err(TransferError::too_large(format!(
                "more than {max_pages} pages, the most one export holds: export fewer at once"
            )));
        }
        if rows > max_rows {
            return Err(TransferError::too_large(format!(
                "more than {max_rows} rows of databases, the most one export holds besides its \
                 pages: export fewer at once"
            )));
        }
        Ok(())
    }

    /// One of the caller's turns of [`Limits::per_user`], held while their transfer runs.
    fn turn(&self, principal: &Principal) -> Result<Turn, TransferError> {
        // A person's agents take turns of theirs, so agents do not multiply them.
        let user_id = principal.user.person_id();
        let mut running = self
            .inner
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let count = running.entry(user_id).or_default();
        if *count >= self.inner.limits.per_user {
            return Err(TransferError::Busy);
        }
        *count += 1;
        Ok(Turn {
            running: Arc::clone(&self.inner.running),
            user_id,
        })
    }

    /// One of the places of [`Limits::parallel`], with a person's turn, held while the
    /// work runs.
    fn hold(&self, turn: Arc<Turn>) -> Result<Hold, TransferError> {
        let place = Arc::clone(&self.inner.places)
            .try_acquire_owned()
            .map_err(|_| TransferError::Busy)?;
        Ok(Hold {
            held: Arc::new(Held {
                place: Mutex::new(Some(place)),
                _turn: turn,
            }),
        })
    }

    async fn limit(&self, bucket: Bucket, principal: &Principal) -> Result<(), TransferError> {
        self.inner
            .limiter
            .check(bucket, &principal.user.id.to_string())
            .await
            .map_err(|limited| {
                warn!(bucket = bucket.prefix(), "rate limited");
                TransferError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, TransferError>>,
    ) -> Result<T, TransferError> {
        let span = info_span!(
            "transfer",
            otel.name = %format!("transfer {operation}"),
            transfer.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        // On the heap: an export's work, with the zip it writes, is large to keep on the
        // caller's stack.
        let result = Box::pin(work.instrument(span.clone())).await;
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

/// When work must be done by, and the limit that set it, for the error that says so.
#[derive(Debug, Clone, Copy)]
struct Deadline {
    at: Instant,
    limit: Duration,
}

impl Deadline {
    fn after(limit: Duration) -> Self {
        Self {
            at: Instant::now() + limit,
            limit,
        }
    }

    /// Fails once the deadline passed: work off the async threads asks between its steps.
    fn check(self) -> Result<(), TransferError> {
        if Instant::now() > self.at {
            Err(TransferError::TimedOut(self.limit))
        } else {
            Ok(())
        }
    }

    /// Runs `work` until the deadline, and drops it there.
    async fn bound<T>(
        self,
        work: impl Future<Output = Result<T, TransferError>>,
    ) -> Result<T, TransferError> {
        let left = self.at.saturating_duration_since(Instant::now());
        tokio::time::timeout(left, work)
            .await
            .unwrap_or(Err(TransferError::TimedOut(self.limit)))
    }
}

/// Runs work that keeps a thread busy, such as reading a zip, off the async threads, the
/// transfer's place and turn held until it ends: dropping what waits for it, as a deadline
/// does, does not stop it.
async fn blocking<T: Send + 'static>(
    hold: &Hold,
    work: impl FnOnce() -> Result<T, TransferError> + Send + 'static,
) -> Result<T, TransferError> {
    let hold = hold.clone();
    tokio::task::spawn_blocking(move || {
        let done = work();
        drop(hold);
        done
    })
    .await
    .unwrap_or_else(|error| {
        Err(TransferError::Internal(format!(
            "the work stopped short: {error}"
        )))
    })
}

/// An export's file name: a page's or teamspace's name as a file system takes it, and the
/// format's extension.
fn named(name: &str, format: Format) -> String {
    format!("{}.{}", file_name(name), format.extension())
}

/// A number of bytes as people read it: `64 B`, `512 KiB`, `4 MiB`.
fn size(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * KIB;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hold of the one place of `places` and a turn of `user_id`'s, counted in `running`.
    fn hold_of(
        places: &Arc<Semaphore>,
        running: &Arc<Mutex<HashMap<Uuid, usize>>>,
        user_id: Uuid,
    ) -> Hold {
        running.lock().expect("unpoisoned").insert(user_id, 1);
        let turn = Arc::new(Turn {
            running: Arc::clone(running),
            user_id,
        });
        let place = Arc::clone(places).try_acquire_owned().expect("a place");
        Hold {
            held: Arc::new(Held {
                place: Mutex::new(Some(place)),
                _turn: turn,
            }),
        }
    }

    #[tokio::test]
    async fn work_off_the_async_threads_keeps_its_place_and_turn_past_a_deadline() {
        let places = Arc::new(Semaphore::new(1));
        let user_id = Uuid::now_v7();
        let running = Arc::new(Mutex::new(HashMap::new()));
        let hold = hold_of(&places, &running, user_id);
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let deadline = Deadline::after(Duration::from_millis(10));
        let work = blocking(&hold, move || {
            wait.recv()
                .map_err(|error| TransferError::Internal(error.to_string()))
        });
        assert!(matches!(
            deadline.bound(work).await,
            Err(TransferError::TimedOut(_))
        ));
        drop(hold);
        assert_eq!(places.available_permits(), 0, "the work still runs");
        assert_eq!(running.lock().expect("unpoisoned").get(&user_id), Some(&1));
        release.send(()).expect("the work waits");
        let _again = places.acquire().await.expect("the place comes back");
        assert!(running.lock().expect("unpoisoned").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_export_sent_slowly_gives_its_place_back_after_a_while_and_keeps_its_turn() {
        let places = Arc::new(Semaphore::new(1));
        let user_id = Uuid::now_v7();
        let running = Arc::new(Mutex::new(HashMap::new()));
        let hold = hold_of(&places, &running, user_id);
        hold.place_for(Duration::from_secs(60));
        assert_eq!(
            places.available_permits(),
            0,
            "the place is held while it is sent"
        );

        // The place comes back once the while has passed, the file still on its way.
        let place = places.acquire().await.expect("the place comes back");
        assert_eq!(running.lock().expect("unpoisoned").get(&user_id), Some(&1));
        drop(hold);
        assert!(running.lock().expect("unpoisoned").is_empty());

        // One sent before then gives both back as it ends.
        let other = Uuid::now_v7();
        drop(place);
        let hold = hold_of(&places, &running, other);
        hold.place_for(Duration::from_secs(60));
        drop(hold);
        assert_eq!(places.available_permits(), 1);
        assert!(running.lock().expect("unpoisoned").is_empty());
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(size(64), "64 B");
        assert_eq!(size(1024), "1 KiB");
        assert_eq!(size(1536), "1536 B");
        assert_eq!(size(4 * 1024 * 1024), "4 MiB");
        assert_eq!(size(1024 * 1024 + 1024), "1025 KiB");
    }

    #[test]
    fn exports_are_named_after_what_they_hold() {
        assert_eq!(named("Q1: Goals", Format::Markdown), "Q1 Goals.md");
        assert_eq!(named("", Format::Zip), "Untitled.zip");
        assert_eq!(Format::Zip.content_type(), "application/zip");
    }

    #[tokio::test]
    async fn work_past_its_deadline_is_dropped() {
        let deadline = Deadline::after(Duration::from_millis(10));
        let late = deadline
            .bound(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await;
        assert_eq!(late.map_err(|error| error.code()), Err("timeout"));
        let done = Deadline::after(Duration::from_secs(5))
            .bound(async { Ok(1) })
            .await;
        assert_eq!(done.ok(), Some(1));
    }
}
