//! Pages shared with members and guests of their workspace.

use neon_domain::pages::{PageAccess, Share};
use neon_domain::workspaces::Person;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::pages::{WALK_LIMIT, parse_access};
use crate::users::PersonColumns;

/// The constraint an insert breaks when the page is shared with that person already.
pub const ALREADY_SHARED: &str = "page_shares_pkey";

/// Saves a share. Fails on [`ALREADY_SHARED`], and on the foreign key when the person is
/// not in the page's workspace.
pub async fn insert(executor: impl PgExecutor<'_>, share: &Share) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO page_shares (page_id, workspace_id, user_id, access, created_by, created_at,
                                  updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        share.page_id,
        share.workspace_id,
        share.user_id,
        share.access.as_str(),
        share.created_by,
        share.created_at,
        share.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// What a page is shared with a person for, on the page itself.
pub async fn get(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Share>> {
    sqlx::query!(
        "SELECT page_id, workspace_id, user_id, access, created_by, created_at, updated_at
         FROM page_shares WHERE page_id = $1 AND user_id = $2",
        page_id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| {
        Ok(Share {
            page_id: row.page_id,
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            access: parse_access(&row.access)?,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    })
    .transpose()
}

pub async fn set_access(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    user_id: Uuid,
    access: PageAccess,
    at: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE page_shares SET access = $3, updated_at = $4 WHERE page_id = $1 AND user_id = $2",
        page_id,
        user_id,
        access.as_str(),
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM page_shares WHERE page_id = $1 AND user_id = $2",
        page_id,
        user_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// What each of `pages` is shared with `user_id` for, itself, without the shares of the
/// pages above it: for those of them that are shared with them.
pub async fn of_pages(
    executor: impl PgExecutor<'_>,
    pages: &[Uuid],
    user_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, PageAccess)>> {
    sqlx::query!(
        "SELECT page_id, access FROM page_shares WHERE page_id = ANY($1) AND user_id = $2",
        pages,
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| Ok((row.page_id, parse_access(&row.access)?)))
    .collect()
}

/// A share as its page's list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub share: Share,
    pub person: Person,
    /// How far above the page listed the page shared is: 0 for the page itself.
    pub depth: i32,
}

/// The shares on a page and on every page above it, the page's own first, then each
/// page's up the tree, each page's by name.
pub async fn on_chain(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Vec<Listed>> {
    sqlx::query!(
        r#"WITH RECURSIVE chain (id, parent_id, depth) AS (
               SELECT id, parent_id, 0 FROM pages WHERE id = $1
             UNION ALL
               SELECT p.id, p.parent_id, c.depth + 1
               FROM chain c JOIN pages p ON p.id = c.parent_id
               WHERE c.depth < $2
           )
           SELECT s.page_id, s.workspace_id, s.user_id, s.access, s.created_by, s.created_at,
                  s.updated_at, u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  c.depth AS "depth!"
           FROM chain c
           JOIN page_shares s ON s.page_id = c.id
           JOIN users u ON u.id = s.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           ORDER BY c.depth, lower(u.display_name), u.id"#,
        page_id,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(Listed {
            share: Share {
                page_id: row.page_id,
                workspace_id: row.workspace_id,
                user_id: row.user_id,
                access: parse_access(&row.access)?,
                created_by: row.created_by,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            person: Person::try_from(PersonColumns {
                id: row.user_id,
                kind: row.kind,
                display_name: row.display_name,
                email: row.email,
                hue: row.hue,
                owner_id: row.owner_id,
                owner_name: row.owner_name,
                owner_hue: row.owner_hue,
            })?,
            depth: row.depth,
        })
    })
    .collect()
}
