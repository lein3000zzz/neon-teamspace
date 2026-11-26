//! Threads of comments on pages, their comments, who follows a page's comments, and whom a
//! page's text mentions.

use neon_domain::comments::Anchor;
use neon_domain::workspaces::Person;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::users::PersonColumns;

/// A thread of comments on a page: on the whole page, or anchored to a range of its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub page_id: Uuid,
    pub anchor: Option<Anchor>,
    pub resolved_at: Option<OffsetDateTime>,
    pub resolved_by: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
}

/// A comment of a thread: its first, or a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: Uuid,
    pub thread_id: Uuid,
    /// Unset once their account is gone.
    pub author_id: Option<Uuid>,
    pub body: String,
    pub created_at: OffsetDateTime,
    pub edited_at: Option<OffsetDateTime>,
}

/// A comment with who wrote it, while their account is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub comment: Comment,
    pub author: Option<Person>,
}

struct ThreadRow {
    id: Uuid,
    workspace_id: Uuid,
    page_id: Uuid,
    anchor_start: Option<Vec<u8>>,
    anchor_end: Option<Vec<u8>>,
    quote: Option<String>,
    resolved_at: Option<OffsetDateTime>,
    resolved_by: Option<Uuid>,
    created_by: Option<Uuid>,
    created_at: OffsetDateTime,
}

impl From<ThreadRow> for Thread {
    fn from(row: ThreadRow) -> Self {
        let anchor = match (row.anchor_start, row.anchor_end, row.quote) {
            (Some(start), Some(end), Some(quote)) => Some(Anchor { start, end, quote }),
            _ => None,
        };
        Self {
            id: row.id,
            workspace_id: row.workspace_id,
            page_id: row.page_id,
            anchor,
            resolved_at: row.resolved_at,
            resolved_by: row.resolved_by,
            created_by: row.created_by,
            created_at: row.created_at,
        }
    }
}

pub async fn insert_thread(executor: impl PgExecutor<'_>, thread: &Thread) -> sqlx::Result<()> {
    let anchor = thread.anchor.as_ref();
    sqlx::query!(
        "INSERT INTO comment_threads (id, workspace_id, page_id, anchor_start, anchor_end, quote,
                                      resolved_at, resolved_by, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        thread.id,
        thread.workspace_id,
        thread.page_id,
        anchor.map(|anchor| anchor.start.as_slice()),
        anchor.map(|anchor| anchor.end.as_slice()),
        anchor.map(|anchor| anchor.quote.as_str()),
        thread.resolved_at,
        thread.resolved_by,
        thread.created_by,
        thread.created_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn insert_comment(executor: impl PgExecutor<'_>, comment: &Comment) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO comments (id, thread_id, author_id, body, created_at, edited_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
        comment.id,
        comment.thread_id,
        comment.author_id,
        comment.body,
        comment.created_at,
        comment.edited_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A page's threads, oldest first.
pub async fn threads(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Vec<Thread>> {
    Ok(sqlx::query_as!(
        ThreadRow,
        "SELECT id, workspace_id, page_id, anchor_start, anchor_end, quote, resolved_at,
                resolved_by, created_by, created_at
         FROM comment_threads WHERE page_id = $1 ORDER BY id",
        page_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Thread::from)
    .collect())
}

/// How many threads a page has.
pub async fn count_threads(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM comment_threads WHERE page_id = $1"#,
        page_id,
    )
    .fetch_one(executor)
    .await
}

/// A thread by its id.
pub async fn thread(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Thread>> {
    Ok(sqlx::query_as!(
        ThreadRow,
        "SELECT id, workspace_id, page_id, anchor_start, anchor_end, quote, resolved_at,
                resolved_by, created_by, created_at
         FROM comment_threads WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Thread::from))
}

/// A thread, locked until the transaction ends, so its comments change one at a time.
pub async fn lock_thread(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Thread>> {
    Ok(sqlx::query_as!(
        ThreadRow,
        "SELECT id, workspace_id, page_id, anchor_start, anchor_end, quote, resolved_at,
                resolved_by, created_by, created_at
         FROM comment_threads WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Thread::from))
}

/// Resolves a thread, `by` someone at a time, or reopens it with `None`.
pub async fn set_resolved(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    resolved: Option<(OffsetDateTime, Uuid)>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE comment_threads SET resolved_at = $2, resolved_by = $3 WHERE id = $1",
        id,
        resolved.map(|(at, _)| at),
        resolved.map(|(_, by)| by),
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Deletes a thread that has no comments left. Says whether it did.
pub async fn delete_thread_if_empty(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM comment_threads t
         WHERE t.id = $1 AND NOT EXISTS (SELECT 1 FROM comments c WHERE c.thread_id = t.id)",
        id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The comments of a page's threads, with their authors, by thread and in order.
pub async fn comments_of_page(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
) -> sqlx::Result<Vec<Written>> {
    sqlx::query!(
        r#"SELECT c.id, c.thread_id, c.author_id, c.body, c.created_at, c.edited_at,
                  u.kind AS "kind?", u.display_name AS "display_name?", u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue
           FROM comments c
           JOIN comment_threads t ON t.id = c.thread_id
           LEFT JOIN users u ON u.id = c.author_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE t.page_id = $1
           ORDER BY c.thread_id, c.id"#,
        page_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let author = match (row.author_id, row.kind, row.display_name) {
            (Some(id), Some(kind), Some(display_name)) => Some(Person::try_from(PersonColumns {
                id,
                kind,
                display_name,
                email: row.email,
                hue: row.hue,
                owner_id: row.owner_id,
                owner_name: row.owner_name,
                owner_hue: row.owner_hue,
            })?),
            _ => None,
        };
        Ok(Written {
            comment: Comment {
                id: row.id,
                thread_id: row.thread_id,
                author_id: row.author_id,
                body: row.body,
                created_at: row.created_at,
                edited_at: row.edited_at,
            },
            author,
        })
    })
    .collect()
}

/// The comments of one thread, in order.
pub async fn comments_of_thread(
    executor: impl PgExecutor<'_>,
    thread_id: Uuid,
) -> sqlx::Result<Vec<Comment>> {
    sqlx::query_as!(
        Comment,
        "SELECT id, thread_id, author_id, body, created_at, edited_at
         FROM comments WHERE thread_id = $1 ORDER BY id",
        thread_id,
    )
    .fetch_all(executor)
    .await
}

/// A comment by its id.
pub async fn comment(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Comment>> {
    sqlx::query_as!(
        Comment,
        "SELECT id, thread_id, author_id, body, created_at, edited_at
         FROM comments WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Changes a comment's text, as edited at `at`.
pub async fn set_body(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    body: &str,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE comments SET body = $2, edited_at = $3 WHERE id = $1",
        id,
        body,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn delete_comment(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM comments WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Whether someone follows a page by choice, `Some(true)`, or stopped, `Some(false)`.
pub async fn follow_choice(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<bool>> {
    sqlx::query_scalar!(
        "SELECT follows FROM page_follows WHERE page_id = $1 AND user_id = $2",
        page_id,
        user_id,
    )
    .fetch_optional(executor)
    .await
}

/// Records that someone follows a page, or stopped, by choice.
pub async fn set_follow(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    page_id: Uuid,
    user_id: Uuid,
    follows: bool,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO page_follows (page_id, workspace_id, user_id, follows, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (page_id, user_id) DO UPDATE SET follows = $4, updated_at = $5",
        page_id,
        workspace_id,
        user_id,
        follows,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Has each of `users` follow a page, but those who chose either way already: who comments
/// on a page, or is mentioned on it, follows it from then on, unless they stopped.
pub async fn follow_unless_chosen(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    page_id: Uuid,
    users: &[Uuid],
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO page_follows (page_id, workspace_id, user_id, follows, updated_at)
         SELECT $1, $2, u.id, true, $4 FROM users u WHERE u.id = ANY($3)
         ON CONFLICT (page_id, user_id) DO NOTHING",
        page_id,
        workspace_id,
        users,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Who follows a page's comments: its creator unless they stopped, and whoever follows it.
pub async fn followers(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        r#"SELECT f.user_id AS "user_id!" FROM page_follows f WHERE f.page_id = $1 AND f.follows
           UNION
           SELECT p.created_by AS "user_id!" FROM pages p
           WHERE p.id = $1 AND p.created_by IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM page_follows f
                             WHERE f.page_id = p.id AND f.user_id = p.created_by)"#,
        page_id,
    )
    .fetch_all(executor)
    .await
}

/// Whom a page's text mentions, as its latest snapshot holds it, each with the first block
/// it does so in.
pub async fn page_mentions(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, Option<String>)>> {
    Ok(sqlx::query!(
        "SELECT user_id, block_id FROM page_mentions WHERE page_id = $1",
        page_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.user_id, row.block_id))
    .collect())
}

/// Records whom a page's text mentions now: those of `mentioned` whose accounts exist, each
/// with its block, and nobody else. Returns those it did not mention before.
pub async fn set_page_mentions(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    mentioned: &[(Uuid, Option<String>)],
) -> sqlx::Result<Vec<(Uuid, Option<String>)>> {
    let (users, blocks): (Vec<Uuid>, Vec<Option<String>>) = mentioned.iter().cloned().unzip();
    Ok(sqlx::query!(
        r#"WITH wanted (user_id, block_id) AS (
               SELECT * FROM unnest($2::uuid[], $3::text[])
           ),
           gone AS (
               DELETE FROM page_mentions m
               WHERE m.page_id = $1 AND m.user_id <> ALL($2)
           )
           INSERT INTO page_mentions (page_id, user_id, block_id)
           SELECT $1, w.user_id, w.block_id FROM wanted w
           JOIN users u ON u.id = w.user_id
           ON CONFLICT (page_id, user_id) DO NOTHING
           RETURNING user_id, block_id"#,
        page_id,
        &users,
        &blocks as &[Option<String>],
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.user_id, row.block_id))
    .collect())
}
