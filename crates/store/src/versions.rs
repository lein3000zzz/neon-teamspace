//! Pages' versions: their documents' blocks as the snapshots that changed them left them,
//! with who changed them and how much.
//!
//! A version is written in the transaction that saves the snapshot it comes from. These
//! queries move what the history service decides; which version to keep is the domain's
//! [`history`](neon_domain::history) rules'.

use neon_domain::history::Dated;
use serde_json::Value as Json;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// A version to keep.
#[derive(Debug, Clone, Copy)]
pub struct NewVersion<'a> {
    pub id: Uuid,
    pub page_id: Uuid,
    pub created_at: OffsetDateTime,
    pub authors: &'a [Uuid],
    /// Its blocks, as a JSON array in BlockNote's form.
    pub content: &'a Json,
    pub added: i32,
    pub changed: i32,
    pub removed: i32,
    /// For a version a restore made: the version restored, and when that one was made.
    pub restored_from: Option<(Uuid, OffsetDateTime)>,
}

/// A version without its blocks, as a page's history lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub id: Uuid,
    pub page_id: Uuid,
    pub created_at: OffsetDateTime,
    pub authors: Vec<Uuid>,
    pub added: i32,
    pub changed: i32,
    pub removed: i32,
    pub restored_from: Option<Uuid>,
    pub restored_from_at: Option<OffsetDateTime>,
}

/// A version with its blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub info: VersionInfo,
    pub content: Json,
}

/// Keeps a version. One restored from that is gone already, thinned out meanwhile, is named
/// by its time alone.
pub async fn insert(executor: impl PgExecutor<'_>, version: NewVersion<'_>) -> sqlx::Result<()> {
    let (from, from_at) = version.restored_from.unzip();
    sqlx::query!(
        "INSERT INTO page_versions (id, page_id, created_at, authors, content, added, changed,
                                    removed, restored_from, restored_from_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                 (SELECT id FROM page_versions WHERE id = $9), $10)",
        version.id,
        version.page_id,
        version.created_at,
        version.authors,
        version.content,
        version.added,
        version.changed,
        version.removed,
        from,
        from_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The blocks of a page's latest version, if it has one.
pub async fn latest_content(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
) -> sqlx::Result<Option<Json>> {
    sqlx::query_scalar!(
        "SELECT content FROM page_versions WHERE page_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        page_id,
    )
    .fetch_optional(executor)
    .await
}

/// A page's versions, latest first, after the one made at `after`, if given; at most
/// `limit`.
pub async fn list(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> sqlx::Result<Vec<VersionInfo>> {
    let (at, id) = after.unzip();
    sqlx::query_as!(
        VersionInfo,
        "SELECT id, page_id, created_at, authors, added, changed, removed, restored_from,
                restored_from_at
         FROM page_versions
         WHERE page_id = $1
           AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3::uuid))
         ORDER BY created_at DESC, id DESC
         LIMIT $4",
        page_id,
        at,
        id,
        limit,
    )
    .fetch_all(executor)
    .await
}

/// A version of a page, with its blocks.
pub async fn get(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Version>> {
    Ok(sqlx::query!(
        "SELECT id, page_id, created_at, authors, added, changed, removed, restored_from,
                restored_from_at, content
         FROM page_versions
         WHERE page_id = $1 AND id = $2",
        page_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| Version {
        info: VersionInfo {
            id: row.id,
            page_id: row.page_id,
            created_at: row.created_at,
            authors: row.authors,
            added: row.added,
            changed: row.changed,
            removed: row.removed,
            restored_from: row.restored_from,
            restored_from_at: row.restored_from_at,
        },
        content: row.content,
    }))
}

/// The versions of each page the cleanup may thin, a part at a time: pages after `after`,
/// in the order of their ids, with more than one version and some made before `before`, or
/// more than `max`; at most `pages` of them.
pub async fn to_thin(
    executor: impl PgExecutor<'_>,
    after: Option<Uuid>,
    before: OffsetDateTime,
    max: i64,
    pages: i64,
) -> sqlx::Result<Vec<(Uuid, Vec<Dated>)>> {
    Ok(sqlx::query!(
        r#"SELECT page_id, array_agg(id) AS "ids!", array_agg(created_at) AS "times!"
           FROM page_versions
           WHERE $1::uuid IS NULL OR page_id > $1
           GROUP BY page_id
           HAVING count(*) > 1 AND (min(created_at) < $2 OR count(*) > $3)
           ORDER BY page_id
           LIMIT $4"#,
        after,
        before,
        max,
        pages,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let dated = row
            .ids
            .into_iter()
            .zip(row.times)
            .map(|(id, at)| Dated { id, at })
            .collect();
        (row.page_id, dated)
    })
    .collect())
}

/// Deletes versions. Returns how many went.
pub async fn delete(executor: impl PgExecutor<'_>, ids: &[Uuid]) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM page_versions WHERE id = ANY($1)", ids)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}
