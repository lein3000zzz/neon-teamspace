//! Attachments: uploads streamed to the blob store as their bytes come, the files each page's
//! document and cover show, and the cleanup that keeps those, deletes the rest once past
//! their grace, and deletes the blobs of every file gone, with its page, its workspace or an
//! upload that never finished.

mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use neon_app::attachments::{
    AttachmentError, AttachmentService, BlobError, BlobStore, CleanupReport, ObjectBlobs,
};
use neon_app::pages::{Location, NewPage, PageUpdate};
use neon_app::sync::{Limits as SyncLimits, SyncService, access_feed, revision};
use neon_blocks::edit::Edit;
use neon_blocks::{Block, BlockKind, Document, ImageProps};
use neon_domain::attachments::{File, FileKind, MediaType, path};
use neon_domain::auth::Principal;
use neon_domain::pages::Placement;
use neon_domain::workspaces::WorkspaceRole;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use support::Harness;
use tempfile::TempDir;
use uuid::Uuid;

/// Long enough for what is waited on to happen, on the slowest runner.
const PATIENCE: Duration = Duration::from_secs(20);

/// The smallest PNG header that says its size.
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(32u32.to_be_bytes());
    bytes.extend(16u32.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

/// Ada owns Design, whose General teamspace Dee is in too; files go to a directory of local
/// disk.
struct Files {
    h: Harness,
    sync: SyncService,
    attachments: AttachmentService,
    store: Arc<ObjectBlobs>,
    _dir: TempDir,
    workspace: Uuid,
    general: Uuid,
    ada: Principal,
    dee: Principal,
}

async fn files(pool: PgPool) -> Files {
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
    let (_, feed) = access_feed();
    // Each logged update is compacted at once, so a snapshot follows every edit.
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed)
        .limits(SyncLimits {
            persist_delay: Duration::from_millis(10),
            compact_updates: 1,
            ..SyncLimits::default()
        })
        .clock(h.time.clock())
        .build();
    let dir = tempfile::tempdir().expect("a directory");
    let store = Arc::new(ObjectBlobs::local(dir.path()).expect("a store"));
    let attachments = AttachmentService::builder_with_store(
        h.pool.clone(),
        Arc::clone(&h.settings),
        h.pages.clone(),
        sync.clone(),
        Arc::clone(&store) as Arc<dyn BlobStore>,
    )
    .limiter(Arc::clone(&h.limiter) as Arc<dyn neon_app::ratelimit::RateLimiter>)
    .clock(h.time.clock())
    .build();
    Files {
        h,
        sync,
        attachments,
        store,
        _dir: dir,
        workspace,
        general,
        ada,
        dee,
    }
}

impl Files {
    async fn page(&self, title: &str, content: Option<Document>) -> Uuid {
        let content = content.map(|document| {
            self.sync
                .first_snapshot(&document)
                .expect("fits")
                .expect("has blocks")
        });
        self.h
            .pages
            .create_page(
                &self.ada,
                self.workspace,
                Location::Teamspace(self.general),
                Placement::Last,
                NewPage {
                    title: title.to_owned(),
                    content,
                    ..NewPage::default()
                },
            )
            .await
            .expect("made")
            .page
            .id
    }

    /// Uploads `content` to a page, in chunks of `chunk` bytes, as Ada.
    async fn upload(
        &self,
        page: Uuid,
        kind: FileKind,
        name: &str,
        content: &[u8],
        chunk: usize,
    ) -> Result<File, AttachmentError> {
        let ticket = self
            .attachments
            .admit_upload(&self.ada, page, kind, None, 0)
            .await?;
        let chunks: Vec<Result<Bytes, String>> = content
            .chunks(chunk.max(1))
            .map(|part| Ok(Bytes::copy_from_slice(part)))
            .collect();
        self.attachments
            .upload(ticket, name, futures_util::stream::iter(chunks))
            .await
    }

    async fn cleanup(&self) -> CleanupReport {
        self.attachments.cleanup().await.expect("cleaned up")
    }

    /// Whether the store still holds a file's blob.
    async fn kept(&self, file: &File) -> bool {
        match self.store.read(&file.key, None).await {
            Ok(_) => true,
            Err(BlobError::NotFound) => false,
            Err(error) => panic!("the store failed: {error}"),
        }
    }

    async fn exists(&self, file: &File) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM files WHERE id = $1)")
            .bind(file.id)
            .fetch_one(&self.h.pool)
            .await
            .expect("asked")
    }

    /// Sets a page's blocks to `blocks`, through its live document, as Ada.
    async fn set_blocks(&self, page: Uuid, blocks: Vec<Block>) {
        let (_, document) = self.sync.read(&self.ada, page).await.expect("read");
        self.sync
            .edit(
                &self.ada,
                page,
                vec![Edit::ReplaceBody { blocks }],
                Some(revision(&document)),
            )
            .await
            .expect("edited");
    }

    /// Waits until a page's last snapshot names `files`.
    async fn snapshot_shows(&self, page: Uuid, files: &[Uuid]) {
        let mut wanted = files.to_vec();
        wanted.sort();
        tokio::time::timeout(PATIENCE, async {
            loop {
                let refs = page_refs(&self.h.pool, page).await.expect("read");
                if refs == wanted {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the snapshot names the files");
    }
}

fn image(file: Uuid) -> Block {
    Block::new(BlockKind::Image {
        props: ImageProps {
            url: path(file),
            ..ImageProps::default()
        },
    })
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_upload_streams_to_the_store_and_is_kept_with_what_it_is(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    // In pieces smaller than what tells the type, the first of them alone too short.
    let content = png();
    let file = t
        .upload(page, FileKind::Image, "shots/plan.png", &content, 3)
        .await
        .expect("uploaded");
    assert_eq!(file.page_id, page);
    assert_eq!(file.workspace_id, t.workspace);
    assert_eq!(file.uploaded_by, Some(t.ada.user.id));
    assert_eq!(file.media, MediaType::Png);
    assert_eq!(file.name, "plan.png");
    assert_eq!(file.size, u64::try_from(content.len()).expect("small"));
    assert_eq!(file.dimensions, Some((32, 16)));
    assert_eq!(file.sha256, <[u8; 32]>::from(Sha256::digest(&content)));
    assert!(file.key.ends_with(&file.id.simple().to_string()));
    let readable = t
        .attachments
        .file(&t.dee, file.id)
        .await
        .expect("Dee sees it");
    let mut read = t
        .attachments
        .read(&readable, None)
        .await
        .expect("read")
        .bytes;
    let mut back = Vec::new();
    while let Some(chunk) = read.next().await {
        back.extend_from_slice(&chunk.expect("a chunk"));
    }
    assert_eq!(back, content);
    assert!(
        matches!(
            t.attachments.read(&readable, Some(0..100)).await,
            Err(AttachmentError::InvalidInput(_))
        ),
        "a range past the file is refused before the store is asked"
    );
    // The upload is no longer left to the cleanup.
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM blob_deletions")
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(left, 0);

    // A refused upload leaves nothing.
    let refused = t
        .upload(page, FileKind::Video, "x.png", &content, 1024)
        .await
        .expect_err("not a video");
    assert!(matches!(
        refused,
        AttachmentError::UnsupportedType {
            kind: FileKind::Video,
            found: MediaType::Png
        }
    ));
    t.h.set("attachments.max_file_bytes", json!(1024)).await;
    let large = t
        .upload(page, FileKind::File, "big.txt", &vec![b'a'; 5000], 1000)
        .await
        .expect_err("too large");
    assert!(matches!(large, AttachmentError::TooLarge { max: 1024 }));
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM files")
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(made, 1);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM blob_deletions")
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(left, 0, "what the large one wrote went with it");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn files_no_page_shows_go_once_past_their_grace(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    let file = t
        .upload(page, FileKind::Image, "a.png", &png(), 1024)
        .await
        .expect("uploaded");

    // Shown nowhere, it is marked, and kept within its grace.
    let first = t.cleanup().await;
    assert_eq!((first.marked, first.deleted), (1, 0));
    t.h.time.advance(Duration::from_secs(29 * 24 * 3600));
    assert_eq!(t.cleanup().await.deleted, 0);
    assert!(t.exists(&file).await);

    // Shown again, as when its block comes back from an undo or an older version, it is
    // unmarked and starts over.
    t.set_blocks(page, vec![image(file.id)]).await;
    t.snapshot_shows(page, &[file.id]).await;
    assert_eq!(t.cleanup().await.unmarked, 1);
    t.h.time.advance(Duration::from_secs(2 * 24 * 3600));
    assert_eq!(t.cleanup().await, CleanupReport::default());

    // Taken out of the page, it waits its whole grace again from then.
    t.set_blocks(page, vec![Block::paragraph("no image")]).await;
    t.snapshot_shows(page, &[]).await;
    assert_eq!(t.cleanup().await.marked, 1);
    t.h.time.advance(Duration::from_secs(29 * 24 * 3600));
    assert_eq!(t.cleanup().await.deleted, 0);
    t.h.time.advance(Duration::from_secs(2 * 24 * 3600));
    let swept = t.cleanup().await;
    assert_eq!((swept.deleted, swept.blobs), (1, 1));
    assert!(!t.exists(&file).await);
    assert!(!t.kept(&file).await);
    assert!(matches!(
        t.attachments.file(&t.ada, file.id).await,
        Err(AttachmentError::NotFound)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn files_shown_by_any_page_or_a_cover_are_kept(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    let copied = t
        .upload(page, FileKind::Image, "copied.png", &png(), 1024)
        .await
        .expect("uploaded");
    let cover = t
        .upload(page, FileKind::Image, "cover.png", &png(), 1024)
        .await
        .expect("uploaded");
    let unsaved = t
        .upload(page, FileKind::Image, "unsaved.png", &png(), 1024)
        .await
        .expect("uploaded");
    // Another page shows the first, copied into it.
    let other = t
        .page("Other", Some(Document::new(vec![image(copied.id)])))
        .await;
    assert_eq!(
        page_refs(&t.h.pool, other).await.expect("read"),
        [copied.id]
    );
    t.h.pages
        .update_page(
            &t.ada,
            page,
            PageUpdate {
                cover_image: Some(Some(cover.id)),
                ..PageUpdate::default()
            },
        )
        .await
        .expect("covered");
    // The page shows the third in its live document, which no snapshot holds yet.
    let (_, document) = t.sync.read(&t.ada, page).await.expect("read");
    let stalled = SyncService::builder(
        t.h.pool.clone(),
        t.h.pages.clone(),
        t.h.auth.clone(),
        access_feed().1,
    )
    .limits(SyncLimits {
        persist_delay: Duration::from_secs(3600),
        ..SyncLimits::default()
    })
    .build();
    stalled
        .edit(
            &t.ada,
            page,
            vec![Edit::ReplaceBody {
                blocks: vec![image(unsaved.id)],
            }],
            Some(revision(&document)),
        )
        .await
        .expect("edited");
    let open = AttachmentService::builder_with_store(
        t.h.pool.clone(),
        Arc::clone(&t.h.settings),
        t.h.pages.clone(),
        stalled.clone(),
        Arc::clone(&t.store) as Arc<dyn BlobStore>,
    )
    .clock(t.h.time.clock())
    .build();

    t.h.time.advance(Duration::from_secs(365 * 24 * 3600));
    let report = open.cleanup().await.expect("cleaned up");
    assert_eq!(report, CleanupReport::default());
    for file in [&copied, &cover, &unsaved] {
        assert!(t.exists(file).await);
        assert!(t.kept(file).await);
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_file_the_text_links_to_is_kept(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    let file = t
        .upload(page, FileKind::File, "report.pdf", b"%PDF-1.7", 1024)
        .await
        .expect("uploaded");
    let linked = neon_blocks::markdown::from_markdown(&format!(
        "A link to [the report]({}) in a paragraph.\n",
        path(file.id)
    ));
    t.set_blocks(page, linked.blocks).await;
    t.snapshot_shows(page, &[file.id]).await;

    assert_eq!(t.cleanup().await, CleanupReport::default());
    t.h.time.advance(Duration::from_secs(365 * 24 * 3600));
    assert_eq!(t.cleanup().await, CleanupReport::default());
    assert!(t.exists(&file).await);
    assert!(t.kept(&file).await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn files_follow_their_page_into_the_trash_and_out_of_it(pool: PgPool) {
    let t = files(pool).await;
    let dee_sees = t.page("Plan", None).await;
    let file = t
        .upload(dee_sees, FileKind::File, "notes.txt", b"words", 1024)
        .await
        .expect("uploaded");
    t.set_blocks(dee_sees, vec![image(file.id)]).await;
    t.snapshot_shows(dee_sees, &[file.id]).await;

    t.h.pages
        .trash_page(&t.ada, dee_sees)
        .await
        .expect("trashed");
    assert!(
        t.attachments.file(&t.dee, file.id).await.is_ok(),
        "an editor of it sees it"
    );
    let refused = t
        .attachments
        .admit_upload(&t.ada, dee_sees, FileKind::File, None, 0)
        .await
        .expect_err("nothing goes into the trash");
    assert_eq!(refused.code(), "forbidden");
    t.h.pages
        .restore_page(&t.ada, dee_sees)
        .await
        .expect("restored");
    assert!(t.attachments.file(&t.dee, file.id).await.is_ok());

    // Purged from the trash, the page's files go with it, their blobs at the next cleanup.
    t.h.pages
        .trash_page(&t.ada, dee_sees)
        .await
        .expect("trashed");
    t.h.time.advance(Duration::from_secs(31 * 24 * 3600));
    assert_eq!(t.h.pages.purge_trash().await.expect("purged"), 1);
    assert!(!t.exists(&file).await);
    assert!(t.kept(&file).await, "until the cleanup runs");
    assert_eq!(t.cleanup().await.blobs, 1);
    assert!(!t.kept(&file).await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn deleting_a_workspace_deletes_its_files(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    let mut uploaded = Vec::new();
    for name in ["a.png", "b.png"] {
        uploaded.push(
            t.upload(page, FileKind::Image, name, &png(), 1024)
                .await
                .expect("uploaded"),
        );
    }
    let admin = t.h.admin().await;
    t.h.workspaces
        .admin_delete_workspace(&admin, t.workspace)
        .await
        .expect("deleted");
    assert_eq!(t.cleanup().await.blobs, 2);
    for file in &uploaded {
        assert!(!t.exists(file).await);
        assert!(!t.kept(file).await);
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_blob_whose_upload_never_finished_goes_once_none_could_be_running(pool: PgPool) {
    let t = files(pool).await;
    let key = "0193a5b20000700080000000000000a1/0193a5b20000700080000000000000f9";
    let mut writer = t.store.create(key).await.expect("created");
    writer
        .write(Bytes::from_static(b"half"))
        .await
        .expect("written");
    writer.finish().await.expect("finished");
    let now = t.h.time.clock().now();
    neon_store::files::delete_blob_later(&t.h.pool, key, now + Duration::from_secs(1800))
        .await
        .expect("left to the cleanup");

    assert_eq!(t.cleanup().await.blobs, 0, "it may still be writing");
    t.h.time.advance(Duration::from_secs(1801));
    assert_eq!(t.cleanup().await.blobs, 1);
    assert!(matches!(
        t.store.read(key, None).await,
        Err(BlobError::NotFound)
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn one_run_deletes_every_blob_due_a_batch_after_another(pool: PgPool) {
    let t = files(pool).await;
    // More than a batch, as a workspace's deletion leaves them.
    sqlx::query(
        "INSERT INTO blob_deletions (storage_key, due_at)
         SELECT '0193a5b20000700080000000000000a1/' || lpad(n::text, 32, '0'), '-infinity'
         FROM generate_series(1, 2345) AS n",
    )
    .execute(&t.h.pool)
    .await
    .expect("queued");
    assert_eq!(t.cleanup().await.blobs, 2345);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM blob_deletions")
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn uploads_need_an_editor_and_room(pool: PgPool) {
    let t = files(pool).await;
    let page = t.page("Plan", None).await;
    let gus = t.h.person("gus@example.com").await;
    t.h.bring_in(&t.ada, t.workspace, &gus, WorkspaceRole::Guest)
        .await;
    let hidden = t
        .attachments
        .admit_upload(&gus, page, FileKind::Image, None, 0)
        .await
        .expect_err("a guest sees no page unshared");
    assert_eq!(hidden.code(), "not_found");
    let declared = t
        .attachments
        .admit_upload(&t.ada, page, FileKind::Image, Some(60 * 1024 * 1024), 1024)
        .await
        .expect_err("declared too large");
    assert!(matches!(declared, AttachmentError::TooLarge { .. }));

    t.h.set("workspaces.limits.storage_bytes", json!(40)).await;
    t.upload(page, FileKind::Image, "a.png", &png(), 1024)
        .await
        .expect("fits");
    let full = t
        .upload(page, FileKind::Image, "b.png", &png(), 1024)
        .await
        .expect_err("no room");
    assert!(matches!(full, AttachmentError::StorageLimit { limit: 40 }));
    // Past the limit once it is lowered, nothing more is let in at all.
    t.h.set("workspaces.limits.storage_bytes", json!(10)).await;
    let full = t
        .attachments
        .admit_upload(&t.ada, page, FileKind::Image, None, 0)
        .await
        .expect_err("full");
    assert!(matches!(full, AttachmentError::StorageLimit { limit: 10 }));
    let details =
        t.h.workspaces
            .workspace(&t.ada, t.workspace)
            .await
            .expect("read");
    assert_eq!(
        details.storage_bytes,
        Some(i64::try_from(png().len()).expect("small"))
    );
    assert_eq!(details.limits.storage_bytes, Some(10));
    t.h.limiter.refuse(neon_app::ratelimit::Bucket::FileUpload);
    t.h.set("workspaces.limits.storage_bytes", json!(0)).await;
    let limited = t
        .attachments
        .admit_upload(&t.ada, page, FileKind::Image, None, 0)
        .await
        .expect_err("rate limited");
    assert!(matches!(limited, AttachmentError::RateLimited { .. }));
    assert!(t.h.limiter.seen().contains(&(
        neon_app::ratelimit::Bucket::FileUpload,
        t.ada.user.id.to_string()
    )));
}

/// The files a page's last snapshot names, by id.
async fn page_refs(pool: &PgPool, page: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar("SELECT file_id FROM page_file_refs WHERE page_id = $1 ORDER BY file_id")
        .bind(page)
        .fetch_all(pool)
        .await
}
