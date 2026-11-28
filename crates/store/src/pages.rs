//! Pages and the trees they make. Which pages someone may see is the rules' business, not
//! these queries': they return what the rules need to know, such as what a person's shares
//! on a page and on the pages above it give.
//!
//! Changes to a workspace's tree, adding, moving, trashing, restoring and deleting pages,
//! take [`lock_tree`] first, so they run one at a time: positions are worked out from
//! siblings no other change is moving, and two moves cannot make a cycle between them.

use neon_domain::pages::{Cover, Font, MAX_DEPTH, Page, PageAccess, PageKind, PageStyle, Space};
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;

/// The class of the advisory locks on page trees, one per workspace: "page" in ASCII.
const TREE_LOCK: i32 = 0x7061_6765;

/// How far the walks up and down a tree go at most, past [`MAX_DEPTH`], which the use
/// cases keep to.
pub(crate) const WALK_LIMIT: i32 = MAX_DEPTH as i32 + 1;

/// A `pages` row.
pub(crate) struct PageColumns {
    pub(crate) id: Uuid,
    pub(crate) workspace_id: Uuid,
    pub(crate) kind: String,
    pub(crate) teamspace_id: Option<Uuid>,
    pub(crate) owner_id: Option<Uuid>,
    pub(crate) parent_id: Option<Uuid>,
    pub(crate) position: String,
    pub(crate) title: String,
    pub(crate) icon: Option<String>,
    pub(crate) cover: Option<String>,
    pub(crate) cover_file_id: Option<Uuid>,
    pub(crate) font: String,
    pub(crate) small_text: bool,
    pub(crate) full_width: bool,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) last_edited_by: Option<Uuid>,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
    pub(crate) content_updated_at: OffsetDateTime,
    pub(crate) trashed_at: Option<OffsetDateTime>,
    pub(crate) trashed_by: Option<Uuid>,
    pub(crate) trashed_with: Option<Uuid>,
}

impl TryFrom<PageColumns> for Page {
    type Error = sqlx::Error;

    fn try_from(row: PageColumns) -> sqlx::Result<Self> {
        let space = match (row.teamspace_id, row.owner_id) {
            (Some(teamspace), None) => Space::Teamspace(teamspace),
            (None, Some(owner)) => Space::Private { owner },
            _ => {
                return Err(decode_error(format!(
                    "page {} is in a teamspace or private, not both or neither",
                    row.id
                )));
            }
        };
        Ok(Self {
            id: row.id,
            workspace_id: row.workspace_id,
            kind: parse_kind(&row.kind)?,
            space,
            parent_id: row.parent_id,
            position: row.position,
            title: row.title,
            icon: row.icon,
            cover: row
                .cover
                .map(|cover| {
                    Cover::parse(&cover)
                        .ok_or_else(|| decode_error(format!("unknown cover {cover:?}")))
                })
                .transpose()?,
            cover_image: row.cover_file_id,
            style: PageStyle {
                font: Font::parse(&row.font)
                    .ok_or_else(|| decode_error(format!("unknown font {:?}", row.font)))?,
                small_text: row.small_text,
                full_width: row.full_width,
            },
            created_by: row.created_by,
            last_edited_by: row.last_edited_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            content_updated_at: row.content_updated_at,
            trashed_at: row.trashed_at,
            trashed_by: row.trashed_by,
            trashed_with: row.trashed_with,
        })
    }
}

/// The page of a `pages` row read by a query of another module.
pub(crate) fn page_of(columns: PageColumns) -> sqlx::Result<Page> {
    Page::try_from(columns)
}

/// What a share's access ranks as in the queries that take the most of several: 1 to
/// view, 2 to comment, 3 to edit, 0 or nothing for none.
fn from_rank(rank: Option<i32>) -> Option<PageAccess> {
    match rank {
        Some(3) => Some(PageAccess::Edit),
        Some(2) => Some(PageAccess::Comment),
        Some(1) => Some(PageAccess::View),
        _ => None,
    }
}

pub(crate) fn parse_kind(text: &str) -> sqlx::Result<PageKind> {
    PageKind::parse(text).ok_or_else(|| decode_error(format!("unknown page kind {text:?}")))
}

pub(crate) fn parse_access(text: &str) -> sqlx::Result<PageAccess> {
    PageAccess::parse(text).ok_or_else(|| decode_error(format!("unknown page access {text:?}")))
}

/// Locks a workspace's page tree until the transaction ends; see the module docs.
pub async fn lock_tree(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        r#"SELECT true AS "locked!" FROM pg_advisory_xact_lock($1, hashtext($2::text))"#,
        TREE_LOCK,
        workspace_id.to_string(),
    )
    .fetch_one(executor)
    .await?;
    Ok(())
}

pub async fn insert(executor: impl PgExecutor<'_>, page: &Page) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO pages (id, workspace_id, kind, teamspace_id, owner_id, parent_id, position,
                            title, icon, cover, cover_file_id, font, small_text, full_width,
                            created_by, last_edited_by, created_at, updated_at,
                            content_updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                 $18, $19)",
        page.id,
        page.workspace_id,
        page.kind.as_str(),
        page.space.teamspace_id(),
        page.space.owner_id(),
        page.parent_id,
        page.position,
        page.title,
        page.icon,
        page.cover.map(Cover::as_str),
        page.cover_image,
        page.style.font.as_str(),
        page.style.small_text,
        page.style.full_width,
        page.created_by,
        page.last_edited_by,
        page.created_at,
        page.updated_at,
        page.content_updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Page>> {
    sqlx::query_as!(
        PageColumns,
        "SELECT id, workspace_id, kind, teamspace_id, owner_id, parent_id, position, title, icon,
                cover, cover_file_id, font, small_text, full_width, created_by, last_edited_by,
                created_at, updated_at, content_updated_at, trashed_at, trashed_by, trashed_with
         FROM pages WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Page::try_from)
    .transpose()
}

/// A page and the pages above it, the page first and its top-level ancestor last, each
/// with what it is shared with `user_id` for, if anything. Empty when there is no page.
pub async fn chain(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<(Page, Option<PageAccess>)>> {
    sqlx::query!(
        r#"WITH RECURSIVE chain (id, parent_id, depth) AS (
               SELECT id, parent_id, 0 FROM pages WHERE id = $1
             UNION ALL
               SELECT p.id, p.parent_id, c.depth + 1
               FROM chain c JOIN pages p ON p.id = c.parent_id
               WHERE c.depth < $3
           )
           SELECT p.id, p.workspace_id, p.kind, p.teamspace_id, p.owner_id, p.parent_id,
                  p.position, p.title, p.icon, p.cover, p.cover_file_id, p.font, p.small_text,
                  p.full_width, p.created_by, p.last_edited_by, p.created_at, p.updated_at, p.content_updated_at,
                  p.trashed_at, p.trashed_by, p.trashed_with, s.access AS "shared?"
           FROM chain c
           JOIN pages p ON p.id = c.id
           LEFT JOIN page_shares s ON s.page_id = p.id AND s.user_id = $2
           ORDER BY c.depth"#,
        id,
        user_id,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let shared = row.shared.as_deref().map(parse_access).transpose()?;
        let page = Page::try_from(PageColumns {
            id: row.id,
            workspace_id: row.workspace_id,
            kind: row.kind,
            teamspace_id: row.teamspace_id,
            owner_id: row.owner_id,
            parent_id: row.parent_id,
            position: row.position,
            title: row.title,
            icon: row.icon,
            cover: row.cover,
            cover_file_id: row.cover_file_id,
            font: row.font,
            small_text: row.small_text,
            full_width: row.full_width,
            created_by: row.created_by,
            last_edited_by: row.last_edited_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            content_updated_at: row.content_updated_at,
            trashed_at: row.trashed_at,
            trashed_by: row.trashed_by,
            trashed_with: row.trashed_with,
        })?;
        Ok((page, shared))
    })
    .collect()
}

/// What the rules need to know of a page for one person.
#[derive(Debug, Clone)]
pub struct PageFacts {
    pub page: Page,
    /// Their role in the page's workspace, if they are in it.
    pub workspace_role: Option<WorkspaceRole>,
    /// For a teamspace's page, the teamspace's access, and their role in it if any.
    pub teamspace_access: Option<TeamspaceAccess>,
    pub teamspace_role: Option<TeamspaceRole>,
    /// The most that the page, or a page above it, is shared with them for.
    pub shared: Option<PageAccess>,
}

/// [`PageFacts`] for each of `ids` that exists, for `user_id`, in no order.
pub async fn facts(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    user_id: Uuid,
) -> sqlx::Result<Vec<PageFacts>> {
    sqlx::query!(
        r#"WITH RECURSIVE up (start, id, depth) AS (
               SELECT id, id, 0 FROM pages WHERE id = ANY($1)
             UNION ALL
               SELECT u.start, p.parent_id, u.depth + 1
               FROM up u JOIN pages p ON p.id = u.id
               WHERE p.parent_id IS NOT NULL AND u.depth < $3
           ), best AS (
               SELECT u.start, max(CASE s.access WHEN 'edit' THEN 3 WHEN 'comment' THEN 2 WHEN 'view' THEN 1 END) AS rank
               FROM up u JOIN page_shares s ON s.page_id = u.id AND s.user_id = $2
               GROUP BY u.start
           )
           SELECT p.id AS "id!", p.workspace_id AS "workspace_id!", p.kind AS "kind!",
                  p.teamspace_id,
                  p.owner_id, p.parent_id, p.position AS "position!", p.title AS "title!",
                  p.icon, p.cover, p.cover_file_id, p.font AS "font!",
                  p.small_text AS "small_text!",
                  p.full_width AS "full_width!", p.created_by, p.last_edited_by,
                  p.created_at AS "created_at!", p.updated_at AS "updated_at!",
                  p.content_updated_at AS "content_updated_at!", p.trashed_at, p.trashed_by,
                  p.trashed_with, w.role AS "workspace_role?", t.access AS "teamspace_access?",
                  m.role AS "teamspace_role?", b.rank AS "shared?"
           FROM pages p
           LEFT JOIN workspace_members w ON w.workspace_id = p.workspace_id AND w.user_id = $2
           LEFT JOIN teamspaces t ON t.id = p.teamspace_id
           LEFT JOIN teamspace_members m ON m.teamspace_id = p.teamspace_id AND m.user_id = $2
           LEFT JOIN best b ON b.start = p.id
           WHERE p.id = ANY($1)"#,
        ids,
        user_id,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let page = Page::try_from(PageColumns {
            id: row.id,
            workspace_id: row.workspace_id,
            kind: row.kind,
            teamspace_id: row.teamspace_id,
            owner_id: row.owner_id,
            parent_id: row.parent_id,
            position: row.position,
            title: row.title,
            icon: row.icon,
            cover: row.cover,
            cover_file_id: row.cover_file_id,
            font: row.font,
            small_text: row.small_text,
            full_width: row.full_width,
            created_by: row.created_by,
            last_edited_by: row.last_edited_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            content_updated_at: row.content_updated_at,
            trashed_at: row.trashed_at,
            trashed_by: row.trashed_by,
            trashed_with: row.trashed_with,
        })?;
        Ok(PageFacts {
            page,
            workspace_role: row
                .workspace_role
                .as_deref()
                .map(crate::workspaces::parse_role)
                .transpose()?,
            teamspace_access: row
                .teamspace_access
                .as_deref()
                .map(|access| {
                    TeamspaceAccess::parse(access)
                        .ok_or_else(|| decode_error(format!("unknown access {access:?}")))
                })
                .transpose()?,
            teamspace_role: row
                .teamspace_role
                .as_deref()
                .map(crate::teamspaces::parse_role)
                .transpose()?,
            shared: from_rank(row.shared),
        })
    })
    .collect()
}

/// Saves a page's title, icon, cover and style, and who changed them when.
pub async fn update(executor: impl PgExecutor<'_>, page: &Page) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE pages
         SET title = $2, icon = $3, cover = $4, cover_file_id = $10, font = $5,
             small_text = $6, full_width = $7, last_edited_by = $8, updated_at = $9
         WHERE id = $1",
        page.id,
        page.title,
        page.icon,
        page.cover.map(Cover::as_str),
        page.style.font.as_str(),
        page.style.small_text,
        page.style.full_width,
        page.last_edited_by,
        page.updated_at,
        page.cover_image,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The pages beside one another: those under a page, at the top of a teamspace, or at the
/// top of a member's private pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Siblings {
    pub workspace_id: Uuid,
    /// The parent page's id, or else the id of the space.
    pub under: Uuid,
}

impl Siblings {
    pub fn of(workspace_id: Uuid, parent_id: Option<Uuid>, space: Space) -> Self {
        Self {
            workspace_id,
            under: parent_id.unwrap_or_else(|| space.id()),
        }
    }
}

/// The ids and positions of the pages out of the trash among `siblings`, in order, but for
/// `except`.
pub async fn sibling_positions(
    executor: impl PgExecutor<'_>,
    siblings: Siblings,
    except: Option<Uuid>,
) -> sqlx::Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query!(
        "SELECT id, position FROM pages
         WHERE workspace_id = $1 AND COALESCE(parent_id, teamspace_id, owner_id) = $2
           AND trashed_at IS NULL AND ($3::uuid IS NULL OR id <> $3)
         ORDER BY position, id",
        siblings.workspace_id,
        siblings.under,
        except,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.id, row.position))
    .collect())
}

/// Gives each of `ids` the position at the same index of `positions`.
pub async fn set_positions(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    positions: &[String],
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE pages p SET position = u.position
         FROM UNNEST($1::uuid[], $2::text[]) AS u (id, position)
         WHERE p.id = u.id",
        ids,
        positions,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// How many levels of pages are under a page, in the trash or not: 0 for one without
/// subpages.
pub async fn height(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<i32> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE down (id, level) AS (
               SELECT id, 0 FROM pages WHERE id = $1
             UNION ALL
               SELECT c.id, d.level + 1
               FROM down d JOIN pages c ON c.parent_id = d.id
               WHERE d.level < $2
           )
           SELECT COALESCE(max(level), 0) AS "height!" FROM down"#,
        id,
        WALK_LIMIT,
    )
    .fetch_one(executor)
    .await
}

/// Where a page goes when it moves: its new parent, position and space.
#[derive(Debug, Clone)]
pub struct Move {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: String,
    pub space: Space,
    pub by: Uuid,
    pub at: OffsetDateTime,
}

/// Moves a page, and the pages under it to its new space, in one statement, so every
/// page's parent is in its space when the statement ends.
pub async fn move_page(executor: impl PgExecutor<'_>, to: &Move) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "WITH RECURSIVE down (id) AS (
             SELECT id FROM pages WHERE id = $1
           UNION ALL
             SELECT c.id FROM down d JOIN pages c ON c.parent_id = d.id
         )
         UPDATE pages p
         SET teamspace_id = $2, owner_id = $3,
             parent_id = CASE WHEN p.id = $1 THEN $4 ELSE p.parent_id END,
             position = CASE WHEN p.id = $1 THEN $5 ELSE p.position END,
             updated_at = CASE WHEN p.id = $1 THEN $6 ELSE p.updated_at END,
             last_edited_by = CASE WHEN p.id = $1 THEN $7 ELSE p.last_edited_by END
         WHERE p.id IN (SELECT id FROM down)",
        to.id,
        to.space.teamspace_id(),
        to.space.owner_id(),
        to.parent_id,
        to.position,
        to.at,
        to.by,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Moves a page and the pages under it out of the trash to the trash: those not there
/// already, which were trashed on their own before. Returns how many went.
pub async fn trash(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    by: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "WITH RECURSIVE down (id) AS (
             SELECT id FROM pages WHERE id = $1
           UNION ALL
             SELECT c.id FROM down d JOIN pages c ON c.parent_id = d.id
         )
         UPDATE pages SET trashed_at = $2, trashed_by = $3, trashed_with = $1
         WHERE id IN (SELECT id FROM down) AND trashed_at IS NULL",
        id,
        at,
        by,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Where a page comes back to from the trash.
#[derive(Debug, Clone)]
pub struct Restore {
    pub id: Uuid,
    /// What it went to the trash with; see [`Page::trashed_with`].
    pub trashed_with: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: String,
    pub at: OffsetDateTime,
}

/// Brings a page back from the trash, with the pages under it that went with it, but not
/// those trashed on their own before. Returns how many came back.
pub async fn restore(executor: impl PgExecutor<'_>, to: &Restore) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "WITH RECURSIVE batch (id) AS (
             SELECT id FROM pages WHERE id = $1
           UNION ALL
             SELECT c.id FROM batch b JOIN pages c ON c.parent_id = b.id AND c.trashed_with = $2
         )
         UPDATE pages p
         SET trashed_at = NULL, trashed_by = NULL, trashed_with = NULL,
             parent_id = CASE WHEN p.id = $1 THEN $3 ELSE p.parent_id END,
             position = CASE WHEN p.id = $1 THEN $4 ELSE p.position END,
             updated_at = CASE WHEN p.id = $1 THEN $5 ELSE p.updated_at END
         WHERE p.id IN (SELECT id FROM batch)",
        to.id,
        to.trashed_with,
        to.parent_id,
        to.position,
        to.at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes a page and, by the cascade, everything under it and in it.
pub async fn delete(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM pages WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// A page the trash's purge deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Trashed on its own, rather than along with a page above it.
    pub on_its_own: bool,
}

/// Deletes what went to the trash before `before`, with everything under it, and says
/// what it deleted.
pub async fn purge_trash(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<Vec<Purged>> {
    Ok(sqlx::query!(
        "DELETE FROM pages WHERE trashed_at < $1 RETURNING id, workspace_id, trashed_with",
        before
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| Purged {
        id: row.id,
        workspace_id: row.workspace_id,
        on_its_own: row.trashed_with == Some(row.id),
    })
    .collect())
}

/// A workspace's pages and databases out of the trash, rows left out, most recently changed
/// first, at most `limit`.
pub async fn recent(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    limit: i64,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT id FROM pages
         WHERE workspace_id = $1 AND trashed_at IS NULL AND kind <> 'row'
         ORDER BY GREATEST(updated_at, content_updated_at) DESC, id
         LIMIT $2",
        workspace_id,
        limit,
    )
    .fetch_all(executor)
    .await
}

/// Of a workspace's pages out of the trash that `user_id`'s space or shares let them see,
/// those whose titles hold `text`, ignoring case, rows and databases included, the most
/// recently changed first, at most `limit`. Empty text takes every title.
///
/// What they may see is narrowed first, as the rules give it: the pages of a teamspace
/// they are in, of one open to every member unless it is private, their own private
/// pages, and every page shared with them and those under it; for a `guest`, only those
/// shared. The title is matched among those alone, so neither the pages found nor the
/// work of finding them turns on what they may not see. The rules still have the last
/// word, as an agent sees only what its owner sees too.
pub async fn titled_for(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    guest: bool,
    text: &str,
    limit: i64,
) -> sqlx::Result<Vec<Uuid>> {
    let pattern = format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    sqlx::query_scalar!(
        r#"WITH RECURSIVE shared (id, depth) AS (
               SELECT s.page_id, 0 FROM page_shares s
               JOIN pages r ON r.id = s.page_id
               WHERE s.user_id = $2 AND r.workspace_id = $1
             UNION
               SELECT c.id, d.depth + 1 FROM shared d
               JOIN pages c ON c.parent_id = d.id
               WHERE d.depth < $6
           ),
           visible AS MATERIALIZED (
               SELECT p.id FROM pages p
               LEFT JOIN teamspaces t ON t.id = p.teamspace_id
               WHERE p.workspace_id = $1 AND p.trashed_at IS NULL
                 AND ((NOT $3 AND p.teamspace_id IS NOT NULL
                       AND (t.access <> 'private'
                            OR EXISTS (SELECT 1 FROM teamspace_members m
                                       WHERE m.teamspace_id = p.teamspace_id
                                         AND m.user_id = $2)))
                      OR (NOT $3 AND p.owner_id = $2)
                      OR p.id IN (SELECT id FROM shared))
           )
           SELECT p.id FROM pages p
           JOIN visible v ON v.id = p.id
           WHERE p.title ILIKE $4 ESCAPE '\'
           ORDER BY GREATEST(p.updated_at, p.content_updated_at) DESC, p.id
           LIMIT $5"#,
        workspace_id,
        user_id,
        guest,
        pattern,
        limit,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await
}

/// Which of `ids` are `root` or under it, or no longer exist: the pages a change to
/// `root` and the pages under it may concern, of those open.
pub async fn within(
    executor: impl PgExecutor<'_>,
    root: Uuid,
    ids: &[Uuid],
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE up (start, id, depth) AS (
               SELECT id, id, 0 FROM pages WHERE id = ANY($2)
             UNION ALL
               SELECT u.start, p.parent_id, u.depth + 1
               FROM up u JOIN pages p ON p.id = u.id
               WHERE p.parent_id IS NOT NULL AND u.depth < $3
           )
           SELECT start AS "id!" FROM up WHERE id = $1
           UNION
           SELECT gone.id AS "id!" FROM unnest($2::uuid[]) AS gone (id)
           WHERE NOT EXISTS (SELECT 1 FROM pages p WHERE p.id = gone.id)"#,
        root,
        ids,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await
}

/// The pages of a workspace trashed on their own, rather than along with a page above
/// them, latest first, after the one trashed at `after`, if given.
pub async fn trashed(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> sqlx::Result<Vec<Page>> {
    let (at, id) = after.unzip();
    sqlx::query_as!(
        PageColumns,
        "SELECT id, workspace_id, kind, teamspace_id, owner_id, parent_id, position, title, icon,
                cover, cover_file_id, font, small_text, full_width, created_by, last_edited_by,
                created_at, updated_at, content_updated_at, trashed_at, trashed_by, trashed_with
         FROM pages
         WHERE workspace_id = $1 AND trashed_at IS NOT NULL AND trashed_with = id
           AND ($2::timestamptz IS NULL OR (trashed_at, id) < ($2, $3::uuid))
         ORDER BY trashed_at DESC, id DESC
         LIMIT $4",
        workspace_id,
        at,
        id,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Page::try_from)
    .collect()
}

/// A page in a tree as listed: what a sidebar shows of it. Rows are left out of trees: their
/// database stands for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub id: Uuid,
    pub kind: PageKind,
    pub parent_id: Option<Uuid>,
    pub space: Space,
    pub title: String,
    pub icon: Option<String>,
    /// How many of its subpages are out of the trash, rows left out.
    pub child_count: i64,
    /// The most that it, or a page above it in the tree listed, is shared with the person
    /// listing for.
    pub shared: Option<PageAccess>,
    /// Where it is in the tree's order, for the next page of the list to start after.
    pub path: String,
}

fn space_of(id: Uuid, teamspace_id: Option<Uuid>, owner_id: Option<Uuid>) -> sqlx::Result<Space> {
    match (teamspace_id, owner_id) {
        (Some(teamspace), None) => Ok(Space::Teamspace(teamspace)),
        (None, Some(owner)) => Ok(Space::Private { owner }),
        _ => Err(decode_error(format!(
            "page {id} is in a teamspace or private, not both or neither"
        ))),
    }
}

/// Every page out of the trash in a space of a workspace, in the order of the tree: each
/// page, then the pages under it, siblings by position, each with the most it is shared with
/// `viewer`, the person the tree is listed for. At most `limit`, those after `after`, a path a
/// node of an earlier call returned.
pub async fn space_tree(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    space: Space,
    viewer: Uuid,
    after: Option<&str>,
    limit: i64,
) -> sqlx::Result<Vec<TreeNode>> {
    sqlx::query!(
        r#"WITH RECURSIVE tree (id, path, shared, depth) AS (
               SELECT p.id, p.position COLLATE "C",
                      CASE s.access WHEN 'edit' THEN 3 WHEN 'comment' THEN 2 WHEN 'view' THEN 1 ELSE 0 END, 1
               FROM pages p
               LEFT JOIN page_shares s ON s.page_id = p.id AND s.user_id = $3
               WHERE p.workspace_id = $1 AND COALESCE(p.parent_id, p.teamspace_id, p.owner_id) = $2
                 AND p.parent_id IS NULL AND p.trashed_at IS NULL
             UNION ALL
               SELECT c.id, (t.path || '!' || c.position) COLLATE "C",
                      GREATEST(t.shared,
                               CASE s.access WHEN 'edit' THEN 3 WHEN 'comment' THEN 2 WHEN 'view' THEN 1 ELSE 0 END),
                      t.depth + 1
               FROM tree t
               JOIN pages c ON c.parent_id = t.id AND c.trashed_at IS NULL AND c.kind <> 'row'
               LEFT JOIN page_shares s ON s.page_id = c.id AND s.user_id = $3
               WHERE t.depth < $6
           )
           SELECT p.id, p.kind, p.parent_id, p.teamspace_id, p.owner_id, p.title, p.icon,
                  t.path AS "path!", t.shared AS "shared!",
                  (SELECT count(*) FROM pages k
                   WHERE k.parent_id = p.id AND k.trashed_at IS NULL AND k.kind <> 'row')
                      AS "child_count!"
           FROM tree t JOIN pages p ON p.id = t.id
           WHERE $4::text IS NULL OR t.path > ($4::text COLLATE "C")
           ORDER BY t.path COLLATE "C"
           LIMIT $5"#,
        workspace_id,
        space.id(),
        viewer,
        after,
        limit,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(TreeNode {
            space: space_of(row.id, row.teamspace_id, row.owner_id)?,
            kind: parse_kind(&row.kind)?,
            id: row.id,
            parent_id: row.parent_id,
            title: row.title,
            icon: row.icon,
            child_count: row.child_count,
            shared: from_rank(Some(row.shared)),
            path: row.path,
        })
    })
    .collect()
}

/// The pages shared with `user_id` in a workspace and out of the trash, with every page
/// under them, in the order of the tree as [`space_tree`] has it, but those in a teamspace
/// they are in, whose tree they have already. A shared page under another one shared with
/// them is listed under that one only. The pages shared are in the order their ids sort,
/// which is the order they were made in, and without their parent, which they are listed
/// at the top in place of.
pub async fn shared_tree(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    after: Option<&str>,
    limit: i64,
) -> sqlx::Result<Vec<TreeNode>> {
    sqlx::query!(
        r#"WITH RECURSIVE shared (id) AS (
               SELECT s.page_id
               FROM page_shares s JOIN pages p ON p.id = s.page_id
               WHERE s.workspace_id = $1 AND s.user_id = $2 AND p.trashed_at IS NULL
                 AND p.kind <> 'row'
                 AND NOT EXISTS (
                     SELECT 1 FROM teamspace_members m
                     WHERE m.teamspace_id = p.teamspace_id AND m.user_id = $2
                 )
           ), up (root, id, depth) AS (
               SELECT p.id, p.parent_id, 1 FROM pages p JOIN shared ON shared.id = p.id
             UNION ALL
               SELECT u.root, p.parent_id, u.depth + 1
               FROM up u JOIN pages p ON p.id = u.id
               WHERE u.depth < $5
           ), roots (id) AS (
               SELECT shared.id FROM shared
               WHERE NOT EXISTS (
                   SELECT 1 FROM up u JOIN page_shares s ON s.page_id = u.id AND s.user_id = $2
                   WHERE u.root = shared.id
               )
           ), tree (id, path, shared, depth) AS (
               SELECT r.id, r.id::text COLLATE "C",
                      CASE s.access WHEN 'edit' THEN 3 WHEN 'comment' THEN 2 WHEN 'view' THEN 1 ELSE 0 END, 1
               FROM roots r
               JOIN page_shares s ON s.page_id = r.id AND s.user_id = $2
             UNION ALL
               SELECT c.id, (t.path || '!' || c.position) COLLATE "C",
                      GREATEST(t.shared,
                               CASE s.access WHEN 'edit' THEN 3 WHEN 'comment' THEN 2 WHEN 'view' THEN 1 ELSE 0 END),
                      t.depth + 1
               FROM tree t
               JOIN pages c ON c.parent_id = t.id AND c.trashed_at IS NULL AND c.kind <> 'row'
               LEFT JOIN page_shares s ON s.page_id = c.id AND s.user_id = $2
               WHERE t.depth < $5
           )
           SELECT p.id, p.kind, CASE WHEN t.depth > 1 THEN p.parent_id END AS parent_id,
                  p.teamspace_id, p.owner_id, p.title, p.icon,
                  t.path AS "path!", t.shared AS "shared!",
                  (SELECT count(*) FROM pages k
                   WHERE k.parent_id = p.id AND k.trashed_at IS NULL AND k.kind <> 'row')
                      AS "child_count!"
           FROM tree t JOIN pages p ON p.id = t.id
           WHERE $3::text IS NULL OR t.path > ($3::text COLLATE "C")
           ORDER BY t.path COLLATE "C"
           LIMIT $4"#,
        workspace_id,
        user_id,
        after,
        limit,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(TreeNode {
            space: space_of(row.id, row.teamspace_id, row.owner_id)?,
            kind: parse_kind(&row.kind)?,
            id: row.id,
            parent_id: row.parent_id,
            title: row.title,
            icon: row.icon,
            child_count: row.child_count,
            shared: from_rank(Some(row.shared)),
            path: row.path,
        })
    })
    .collect()
}

/// A page as an export lists it: its place in the tree, its title, and when its content
/// last changed. A database's rows are under it, as its subpages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    pub id: Uuid,
    pub kind: PageKind,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub content_updated_at: OffsetDateTime,
}

/// A page, then every page under it that is where it is: out of the trash for a page out
/// of it, or for a page in it, those that went to the trash with it. In the order of the
/// tree, as [`space_tree`] has it; at most `limit`.
pub async fn outline(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    limit: i64,
) -> sqlx::Result<Vec<OutlineNode>> {
    sqlx::query!(
        r#"WITH RECURSIVE tree (id, batch, path, depth) AS (
               SELECT id, trashed_with, position COLLATE "C", 1 FROM pages WHERE id = $1
             UNION ALL
               SELECT c.id, t.batch, (t.path || '!' || c.position) COLLATE "C", t.depth + 1
               FROM tree t
               JOIN pages c ON c.parent_id = t.id AND c.trashed_with IS NOT DISTINCT FROM t.batch
               WHERE t.depth < $3
           )
           SELECT p.id, p.kind, p.parent_id, p.title, p.content_updated_at
           FROM tree t JOIN pages p ON p.id = t.id
           ORDER BY t.path COLLATE "C", p.id
           LIMIT $2"#,
        id,
        limit,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(OutlineNode {
            id: row.id,
            kind: parse_kind(&row.kind)?,
            parent_id: row.parent_id,
            title: row.title,
            content_updated_at: row.content_updated_at,
        })
    })
    .collect()
}

/// Every page out of the trash in a space of a workspace, `space` being the teamspace's or
/// the owner's id, in the order of the tree, as [`space_tree`] has it; at most `limit`.
pub async fn space_outline(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    space: Uuid,
    limit: i64,
) -> sqlx::Result<Vec<OutlineNode>> {
    sqlx::query!(
        r#"WITH RECURSIVE tree (id, path, depth) AS (
               SELECT id, position COLLATE "C", 1 FROM pages
               WHERE workspace_id = $1 AND COALESCE(parent_id, teamspace_id, owner_id) = $2
                 AND parent_id IS NULL AND trashed_at IS NULL
             UNION ALL
               SELECT c.id, (t.path || '!' || c.position) COLLATE "C", t.depth + 1
               FROM tree t
               JOIN pages c ON c.parent_id = t.id AND c.trashed_at IS NULL
               WHERE t.depth < $4
           )
           SELECT p.id, p.kind, p.parent_id, p.title, p.content_updated_at
           FROM tree t JOIN pages p ON p.id = t.id
           ORDER BY t.path COLLATE "C", p.id
           LIMIT $3"#,
        workspace_id,
        space,
        limit,
        WALK_LIMIT,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(OutlineNode {
            id: row.id,
            kind: parse_kind(&row.kind)?,
            parent_id: row.parent_id,
            title: row.title,
            content_updated_at: row.content_updated_at,
        })
    })
    .collect()
}
