//! Pages' documents: each one's log of updates and its latest snapshot.
//!
//! A page's document is a Yjs document. Its updates are logged as they come, each with its
//! author, and from time to time the whole document is saved as a snapshot that takes in
//! the updates logged before it, which are deleted in the same transaction. The document is
//! the snapshot with the updates logged after it applied, in any order: Yjs updates commute.
//!
//! These queries only move bytes. Encoding and decoding them is the live documents' work.

use sqlx::{PgExecutor, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

/// A page's latest snapshot, as saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The whole document, encoded as one Yjs update.
    pub state: Vec<u8>,
    /// The last logged update it holds.
    pub last_seq: i64,
}

/// An update in a page's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedUpdate {
    pub seq: i64,
    pub update: Vec<u8>,
    pub author_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
}

/// An update to log.
#[derive(Debug, Clone, Copy)]
pub struct NewUpdate<'a> {
    pub update: &'a [u8],
    pub author_id: Uuid,
    pub created_at: OffsetDateTime,
}

/// A snapshot to save in place of the page's last one.
#[derive(Debug, Clone, Copy)]
pub struct NewSnapshot<'a> {
    pub state: &'a [u8],
    /// The document's Yjs state vector, encoded.
    pub state_vector: &'a [u8],
    pub last_seq: i64,
    pub at: OffsetDateTime,
}

/// The latest change to a page's document: when it was made, and by whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edited {
    pub at: OffsetDateTime,
    pub by: Option<Uuid>,
}

pub async fn snapshot(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
) -> sqlx::Result<Option<Snapshot>> {
    sqlx::query_as!(
        Snapshot,
        "SELECT state, last_seq FROM page_snapshots WHERE page_id = $1",
        page_id,
    )
    .fetch_optional(executor)
    .await
}

/// The updates of a page's log after `after`, in the order they were logged.
pub async fn updates_after(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    after: i64,
) -> sqlx::Result<Vec<LoggedUpdate>> {
    sqlx::query_as!(
        LoggedUpdate,
        r#"SELECT seq, "update", author_id, created_at FROM page_updates
           WHERE page_id = $1 AND seq > $2
           ORDER BY seq"#,
        page_id,
        after,
    )
    .fetch_all(executor)
    .await
}

/// A page's document as saved: its snapshot, if any, and the updates logged after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub snapshot: Option<Snapshot>,
    pub updates: Vec<LoggedUpdate>,
}

/// A page's snapshot and the updates logged after it, read in one transaction that sees the
/// database as it was at one moment: a compaction between the two reads would otherwise
/// delete updates the snapshot read lacks.
pub async fn saved(pool: &PgPool, page_id: Uuid) -> sqlx::Result<Saved> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let snapshot = snapshot(&mut *tx, page_id).await?;
    let after = snapshot.as_ref().map_or(0, |snapshot| snapshot.last_seq);
    let updates = updates_after(&mut *tx, page_id, after).await?;
    tx.commit().await?;
    Ok(Saved { snapshot, updates })
}

/// Logs updates in the order given, in one statement. Returns the number of the last one
/// logged, or `None` when there were none.
pub async fn append(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    updates: &[NewUpdate<'_>],
) -> sqlx::Result<Option<i64>> {
    if updates.is_empty() {
        return Ok(None);
    }
    let bodies: Vec<&[u8]> = updates.iter().map(|update| update.update).collect();
    let authors: Vec<Uuid> = updates.iter().map(|update| update.author_id).collect();
    let times: Vec<OffsetDateTime> = updates.iter().map(|update| update.created_at).collect();
    let numbers = sqlx::query_scalar!(
        r#"INSERT INTO page_updates (page_id, "update", author_id, created_at)
           SELECT $1, u.body, u.author_id, u.created_at
           FROM UNNEST($2::bytea[], $3::uuid[], $4::timestamptz[])
               WITH ORDINALITY AS u (body, author_id, created_at, n)
           ORDER BY u.n
           RETURNING seq"#,
        page_id,
        &bodies as &[&[u8]],
        &authors,
        &times,
    )
    .fetch_all(executor)
    .await?;
    Ok(numbers.into_iter().max())
}

/// Saves a page's snapshot in place of its last one.
pub async fn save_snapshot(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    snapshot: NewSnapshot<'_>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO page_snapshots (page_id, state, state_vector, last_seq, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (page_id) DO UPDATE
         SET state = EXCLUDED.state, state_vector = EXCLUDED.state_vector,
             last_seq = EXCLUDED.last_seq, updated_at = EXCLUDED.updated_at",
        page_id,
        snapshot.state,
        snapshot.state_vector,
        snapshot.last_seq,
        snapshot.at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Deletes the updates of a page's log up to and including `seq`, which a snapshot holds.
pub async fn delete_updates_through(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    seq: i64,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM page_updates WHERE page_id = $1 AND seq <= $2",
        page_id,
        seq,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Records what a snapshot says of the page: the text search reads, when there is one, and
/// the document's latest change, when there was one. The change moves `content_updated_at`
/// forward only, and names the page's last editor unless the page itself changed later.
/// Returns whether the page exists.
pub async fn set_content(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    search_text: Option<&str>,
    edited: Option<Edited>,
) -> sqlx::Result<bool> {
    let (at, by) = match edited {
        Some(edited) => (Some(edited.at), edited.by),
        None => (None, None),
    };
    let result = sqlx::query!(
        "UPDATE pages
         SET search_text = COALESCE($2, search_text),
             content_updated_at = GREATEST(content_updated_at, COALESCE($3, content_updated_at)),
             last_edited_by = CASE
                 WHEN $3::timestamptz >= GREATEST(updated_at, content_updated_at) THEN $4
                 ELSE last_edited_by
             END
         WHERE id = $1",
        page_id,
        search_text,
        at,
        by,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Whether `error` is a write that found no page to belong to: the page was deleted.
pub fn is_page_gone(error: &sqlx::Error) -> bool {
    error.as_database_error().is_some_and(|db| {
        db.kind() == sqlx::error::ErrorKind::ForeignKeyViolation
            && matches!(
                db.constraint(),
                Some(
                    "page_updates_page_id_fkey"
                        | "page_snapshots_page_id_fkey"
                        | "page_versions_page_id_fkey"
                )
            )
    })
}
