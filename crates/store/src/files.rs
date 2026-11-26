//! Attachments: the rows of files uploaded to pages, the files each page's document shows,
//! and the blobs left to delete from the store.
//!
//! A file's bytes are in the blob store under its `storage_key`; this table only says what
//! they are. Deleting a file's row, on its own or with its page or workspace, leaves its key
//! in `blob_deletions`, through a trigger, for the cleanup to delete the blob. An upload
//! adds its key there too before it writes, due once it could no longer be running, and
//! takes it out in the transaction that adds the file's row: a blob whose upload never
//! finished is deleted all the same.

use neon_domain::attachments::{File, FileKind, MediaType};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;

/// A `files` row.
struct FileColumns {
    id: Uuid,
    workspace_id: Uuid,
    page_id: Uuid,
    uploaded_by: Option<Uuid>,
    kind: String,
    storage_key: String,
    name: String,
    content_type: String,
    size_bytes: i64,
    sha256: Vec<u8>,
    width: Option<i32>,
    height: Option<i32>,
    created_at: OffsetDateTime,
}

impl TryFrom<FileColumns> for File {
    type Error = sqlx::Error;

    fn try_from(row: FileColumns) -> sqlx::Result<Self> {
        let dimensions = match (row.width, row.height) {
            (Some(width), Some(height)) => Some((
                u32::try_from(width).map_err(|_| decode_error(format!("width {width}")))?,
                u32::try_from(height).map_err(|_| decode_error(format!("height {height}")))?,
            )),
            _ => None,
        };
        Ok(Self {
            id: row.id,
            workspace_id: row.workspace_id,
            page_id: row.page_id,
            uploaded_by: row.uploaded_by,
            kind: FileKind::parse(&row.kind)
                .ok_or_else(|| decode_error(format!("unknown file kind {:?}", row.kind)))?,
            key: row.storage_key,
            name: row.name,
            media: MediaType::parse(&row.content_type)
                .ok_or_else(|| decode_error(format!("unknown type {:?}", row.content_type)))?,
            size: u64::try_from(row.size_bytes)
                .map_err(|_| decode_error(format!("size {}", row.size_bytes)))?,
            sha256: row
                .sha256
                .try_into()
                .map_err(|_| decode_error("a SHA-256 is 32 bytes".to_owned()))?,
            dimensions,
            created_at: row.created_at,
        })
    }
}

fn size_column(file: &File) -> sqlx::Result<i64> {
    i64::try_from(file.size).map_err(|_| decode_error(format!("size {}", file.size)))
}

pub async fn insert(executor: impl PgExecutor<'_>, file: &File) -> sqlx::Result<()> {
    let (width, height) = file.dimensions.unzip();
    let dimension = |value: Option<u32>| value.and_then(|value| i32::try_from(value).ok());
    sqlx::query!(
        "INSERT INTO files (id, workspace_id, page_id, uploaded_by, kind, storage_key, name,
                            content_type, size_bytes, sha256, width, height, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        file.id,
        file.workspace_id,
        file.page_id,
        file.uploaded_by,
        file.kind.as_str(),
        file.key,
        file.name,
        file.media.as_str(),
        size_column(file)?,
        &file.sha256[..],
        dimension(width).filter(|_| height.is_some()),
        dimension(height).filter(|_| width.is_some()),
        file.created_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<File>> {
    sqlx::query_as!(
        FileColumns,
        "SELECT id, workspace_id, page_id, uploaded_by, kind, storage_key, name, content_type,
                size_bytes, sha256, width, height, created_at
         FROM files WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(File::try_from)
    .transpose()
}

/// The files of `ids` that exist, in no order.
pub async fn by_ids(executor: impl PgExecutor<'_>, ids: &[Uuid]) -> sqlx::Result<Vec<File>> {
    sqlx::query_as!(
        FileColumns,
        "SELECT id, workspace_id, page_id, uploaded_by, kind, storage_key, name, content_type,
                size_bytes, sha256, width, height, created_at
         FROM files WHERE id = ANY($1)",
        ids,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(File::try_from)
    .collect()
}

/// The bytes a workspace's files take together.
pub async fn usage(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(size_bytes), 0)::bigint AS "bytes!" FROM files
           WHERE workspace_id = $1"#,
        workspace_id,
    )
    .fetch_one(executor)
    .await
}

/// The bytes each workspace of `ids` that has files takes, in no order.
pub async fn usage_of(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, i64)>> {
    Ok(sqlx::query!(
        r#"SELECT workspace_id, COALESCE(SUM(size_bytes), 0)::bigint AS "bytes!" FROM files
           WHERE workspace_id = ANY($1)
           GROUP BY workspace_id"#,
        ids,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.workspace_id, row.bytes))
    .collect())
}

/// Leaves a blob to delete once `due_at` has passed, unless something takes it out before:
/// an upload's, in case it never finishes.
pub async fn delete_blob_later(
    executor: impl PgExecutor<'_>,
    key: &str,
    due_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO blob_deletions (storage_key, due_at) VALUES ($1, $2)
         ON CONFLICT (storage_key) DO UPDATE SET due_at = EXCLUDED.due_at",
        key,
        due_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Keeps a blob that was left to delete, as its file's row now holds it.
pub async fn keep_blob(executor: impl PgExecutor<'_>, key: &str) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM blob_deletions WHERE storage_key = $1", key)
        .execute(executor)
        .await?;
    Ok(())
}

/// The keys of blobs due to be deleted by `now`, oldest first, at most `limit`.
pub async fn due_blobs(
    executor: impl PgExecutor<'_>,
    now: OffsetDateTime,
    limit: i64,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "SELECT storage_key FROM blob_deletions WHERE due_at <= $1
         ORDER BY due_at, storage_key LIMIT $2",
        now,
        limit,
    )
    .fetch_all(executor)
    .await
}

/// Forgets a blob deleted from the store, unless a file's row holds its key again.
pub async fn blob_deleted(executor: impl PgExecutor<'_>, key: &str) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM blob_deletions WHERE storage_key = $1", key)
        .execute(executor)
        .await?;
    Ok(())
}

/// Records the files a page's document shows, by the ids in their paths, in place of what
/// it showed before.
pub async fn set_page_refs(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    file_ids: &[Uuid],
) -> sqlx::Result<()> {
    sqlx::query!(
        "WITH gone AS (
             DELETE FROM page_file_refs WHERE page_id = $1 AND NOT (file_id = ANY($2))
         )
         INSERT INTO page_file_refs (page_id, file_id)
         SELECT $1, id FROM UNNEST($2::uuid[]) AS ids (id)
         ON CONFLICT DO NOTHING",
        page_id,
        file_ids,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A file that no page's last snapshot shows, nor any cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreferenced {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Since when the cleanup found it so; unset the first time.
    pub since: Option<OffsetDateTime>,
}

/// Files no page's last snapshot shows and no page has as its cover: those not marked so
/// yet, or, with `marked_before`, those marked so before it. Oldest first, at most `limit`.
pub async fn unreferenced(
    executor: impl PgExecutor<'_>,
    marked_before: Option<OffsetDateTime>,
    limit: i64,
) -> sqlx::Result<Vec<Unreferenced>> {
    Ok(sqlx::query!(
        "SELECT f.id, f.workspace_id, f.unreferenced_since
         FROM files f
         WHERE ($1::timestamptz IS NULL AND f.unreferenced_since IS NULL
                OR f.unreferenced_since < $1)
           AND NOT EXISTS (SELECT 1 FROM page_file_refs r WHERE r.file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM pages p WHERE p.cover_file_id = f.id)
         ORDER BY f.unreferenced_since NULLS FIRST, f.created_at, f.id
         LIMIT $2",
        marked_before,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| Unreferenced {
        id: row.id,
        workspace_id: row.workspace_id,
        since: row.unreferenced_since,
    })
    .collect())
}

/// The pages of the workspaces of `ids`, with their workspace's, that have updates logged
/// since their last snapshot, or are among `open`: their documents may show files their
/// snapshot did not.
pub async fn pages_to_read(
    executor: impl PgExecutor<'_>,
    workspace_ids: &[Uuid],
    open: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, Uuid)>> {
    Ok(sqlx::query!(
        "SELECT p.id, p.workspace_id FROM pages p
         WHERE p.workspace_id = ANY($1)
           AND (p.id = ANY($2) OR EXISTS (SELECT 1 FROM page_updates u WHERE u.page_id = p.id))
         ORDER BY p.id",
        workspace_ids,
        open,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.id, row.workspace_id))
    .collect())
}

/// Marks files as shown by no document since `at`, those not marked already.
pub async fn mark_unreferenced(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    at: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE files SET unreferenced_since = $2
         WHERE id = ANY($1) AND unreferenced_since IS NULL",
        ids,
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Unmarks the files marked as shown by no document that a page's last snapshot or a cover
/// shows again, and those of `shown`, which a document shows since its last snapshot.
pub async fn unmark_referenced(executor: impl PgExecutor<'_>, shown: &[Uuid]) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE files f SET unreferenced_since = NULL
         WHERE f.unreferenced_since IS NOT NULL
           AND (f.id = ANY($1)
                OR EXISTS (SELECT 1 FROM page_file_refs r WHERE r.file_id = f.id)
                OR EXISTS (SELECT 1 FROM pages p WHERE p.cover_file_id = f.id))",
        shown,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes those of `ids` marked as shown by no document since before `before` that no
/// page's last snapshot or cover shows still: their blobs are then left to delete. Says
/// how many went.
pub async fn delete_unreferenced(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM files f
         WHERE f.id = ANY($1) AND f.unreferenced_since < $2
           AND NOT EXISTS (SELECT 1 FROM page_file_refs r WHERE r.file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM pages p WHERE p.cover_file_id = f.id)",
        ids,
        before,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
