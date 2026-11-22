//! Lists of pages: the tree of a teamspace, of the caller's private pages and of the pages
//! shared with them, and a workspace's trash.
//!
//! A tree comes in the order a sidebar shows it, each page then the pages under it, up to
//! [`MAX_LIST`] at a time. A list that has more says where the next part starts, an opaque
//! cursor to pass back.

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, Space};
use neon_domain::workspaces::{Action, Actor, PageSpace};
use neon_store::pages::{self as repo, PageFacts, TreeNode};
use neon_store::{teamspaces, workspaces};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::operations::{in_workspace, teamspace_space};
use super::{
    NO_TEAMSPACE, NO_WORKSPACE, PageError, PageService, PageView, access_of, actor, check_space,
    seen,
};

/// The most pages a list answers with at once, and how many unless told fewer.
pub const MAX_LIST: i64 = 1000;

/// How many batches of the trash a list reads, at most, looking for pages the caller may
/// see, before it answers with what it found and where to go on.
const TRASH_ROUNDS: usize = 10;

/// The longest cursor taken.
const MAX_CURSOR: usize = 8192;

/// A page in a tree, as a sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreePage {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub space: Space,
    pub title: String,
    pub icon: Option<String>,
    /// How many pages are under it, out of the trash.
    pub child_count: i64,
    pub access: PageAccess,
}

/// A part of a tree, and where the next part starts when there is more.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub pages: Vec<TreePage>,
    pub next: Option<String>,
}

/// A part of a workspace's trash, latest first, and where the next part starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashList {
    pub pages: Vec<PageView>,
    pub next: Option<String>,
}

impl PageService {
    /// A teamspace's pages out of the trash, for everyone who may see the teamspace in
    /// full: a private one's for its members only.
    pub async fn teamspace_tree(
        &self,
        principal: &Principal,
        teamspace_id: Uuid,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Tree, PageError> {
        self.observe("teamspace_tree", async {
            let limit = list_limit(limit)?;
            let cursor = cursor.map(tree_cursor).transpose()?;
            let user = principal.user.id;
            let mut conn = self.pool().acquire().await?;
            let teamspace = teamspaces::find(&mut *conn, teamspace_id)
                .await?
                .ok_or(PageError::NotFound(NO_TEAMSPACE))?;
            let workspace_id = teamspace.workspace_id;
            let role = workspaces::member_role(&mut *conn, workspace_id, user).await?;
            let actor = actor(principal, role);
            let space = teamspace_space(&mut conn, workspace_id, teamspace_id, user).await?;
            check_space(actor, space, Action::ListPages, NO_TEAMSPACE)?;
            let nodes = repo::space_tree(
                &mut *conn,
                workspace_id,
                teamspace_id,
                user,
                cursor,
                limit + 1,
            )
            .await?;
            Ok(tree(nodes, limit, |_| Some(space), actor))
        })
        .await
    }

    /// The caller's private pages in a workspace, out of the trash.
    pub async fn private_tree(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Tree, PageError> {
        self.observe("private_tree", async {
            let limit = list_limit(limit)?;
            let cursor = cursor.map(tree_cursor).transpose()?;
            let user = principal.user.id;
            let mut conn = self.pool().acquire().await?;
            let actor = actor(
                principal,
                in_workspace(&mut conn, principal, workspace_id).await?,
            );
            let space = PageSpace::Private { own: true };
            check_space(actor, space, Action::ListPages, NO_WORKSPACE)?;
            let nodes =
                repo::space_tree(&mut *conn, workspace_id, user, user, cursor, limit + 1).await?;
            Ok(tree(nodes, limit, |_| Some(space), actor))
        })
        .await
    }

    /// The pages of a workspace shared with the caller, out of the trash, each with the
    /// pages under it: how guests find theirs, and members the pages they reach through
    /// shares alone.
    pub async fn shared_tree(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Tree, PageError> {
        self.observe("shared_tree", async {
            let limit = list_limit(limit)?;
            let cursor = cursor.map(tree_cursor).transpose()?;
            let user = principal.user.id;
            let mut conn = self.pool().acquire().await?;
            let actor = actor(
                principal,
                in_workspace(&mut conn, principal, workspace_id).await?,
            );
            let nodes =
                repo::shared_tree(&mut *conn, workspace_id, user, cursor, limit + 1).await?;
            let mut spaces = HashMap::new();
            for node in &nodes {
                if let Space::Teamspace(id) = node.space
                    && !spaces.contains_key(&id)
                {
                    let space = teamspace_space(&mut conn, workspace_id, id, user).await?;
                    spaces.insert(id, space);
                }
            }
            let space_of = |space: Space| match space {
                Space::Teamspace(id) => spaces.get(&id).copied(),
                Space::Private { owner } => Some(PageSpace::Private { own: owner == user }),
            };
            Ok(tree(nodes, limit, space_of, actor))
        })
        .await
    }

    /// The pages of a workspace in the trash that the caller could edit, latest first. A
    /// page trashed with the pages under it is listed alone, standing for them.
    pub async fn trash(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<TrashList, PageError> {
        self.observe("list_trash", async {
            let limit = list_limit(limit)?;
            let mut after = cursor.map(trash_cursor).transpose()?;
            let mut conn = self.pool().acquire().await?;
            in_workspace(&mut conn, principal, workspace_id).await?;
            let mut list = TrashList::default();
            for _ in 0..TRASH_ROUNDS {
                let batch = repo::trashed(&mut *conn, workspace_id, after, limit).await?;
                let exhausted = i64::try_from(batch.len()).unwrap_or(i64::MAX) < limit;
                let facts = facts_by_id(&mut conn, &batch, principal).await?;
                for page in batch {
                    after = page.trashed_at.map(|at| (at, page.id));
                    if let Some(view) = facts
                        .get(&page.id)
                        .map(|f| seen(principal, f))
                        .transpose()?
                        .flatten()
                    {
                        list.pages.push(view);
                        if i64::try_from(list.pages.len()).unwrap_or(i64::MAX) == limit {
                            list.next = after.map(|(at, id)| trash_position(at, id));
                            return Ok(list);
                        }
                    }
                }
                if exhausted {
                    return Ok(list);
                }
            }
            list.next = after.map(|(at, id)| trash_position(at, id));
            Ok(list)
        })
        .await
    }
}

/// The facts of each page of `pages` for the principal, by id.
async fn facts_by_id(
    conn: &mut PgConnection,
    pages: &[neon_domain::pages::Page],
    principal: &Principal,
) -> Result<HashMap<Uuid, PageFacts>, PageError> {
    let ids: Vec<Uuid> = pages.iter().map(|page| page.id).collect();
    Ok(repo::facts(&mut *conn, &ids, principal.user.id)
        .await?
        .into_iter()
        .map(|facts| (facts.page.id, facts))
        .collect())
}

/// The nodes of a tree with what the actor may do with each, the `limit` first, and where
/// the rest start, when there are more.
fn tree(
    mut nodes: Vec<TreeNode>,
    limit: i64,
    space_of: impl Fn(Space) -> Option<PageSpace>,
    actor: Actor,
) -> Tree {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let more = nodes.len() > limit;
    nodes.truncate(limit);
    let next = more
        .then(|| nodes.last().map(|node| node.path.clone()))
        .flatten();
    let pages = nodes
        .into_iter()
        .filter_map(|node| {
            let access = access_of(actor, space_of(node.space)?, node.shared)?;
            Some(TreePage {
                id: node.id,
                parent_id: node.parent_id,
                space: node.space,
                title: node.title,
                icon: node.icon,
                child_count: node.child_count,
                access,
            })
        })
        .collect();
    Tree { pages, next }
}

fn list_limit(limit: Option<i64>) -> Result<i64, PageError> {
    match limit {
        None => Ok(MAX_LIST),
        Some(limit) if (1..=MAX_LIST).contains(&limit) => Ok(limit),
        Some(_) => Err(PageError::InvalidInput(format!(
            "a list holds 1 to {MAX_LIST} pages at a time"
        ))),
    }
}

const INVALID_CURSOR: &str = "this cursor comes from no list: pass back the one a list answered";

/// A tree's cursor: the place in the tree of the last page listed, made of position keys,
/// ids and the `!` between levels.
fn tree_cursor(cursor: &str) -> Result<&str, PageError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '!' || c == '-';
    if cursor.is_empty() || cursor.len() > MAX_CURSOR || !cursor.chars().all(allowed) {
        return Err(PageError::InvalidInput(INVALID_CURSOR.to_owned()));
    }
    Ok(cursor)
}

/// Where a trash list goes on from: when the last page listed went to the trash, in
/// microseconds, and its id.
fn trash_position(at: OffsetDateTime, id: Uuid) -> String {
    format!("{}.{id}", at.unix_timestamp_nanos() / 1000)
}

fn trash_cursor(cursor: &str) -> Result<(OffsetDateTime, Uuid), PageError> {
    let invalid = || PageError::InvalidInput(INVALID_CURSOR.to_owned());
    let (micros, id) = cursor.split_once('.').ok_or_else(invalid)?;
    let micros: i128 = micros.parse().map_err(|_| invalid())?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(micros * 1000).map_err(|_| invalid())?;
    Ok((at, Uuid::try_parse(id).map_err(|_| invalid())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_stay_within_bounds() {
        assert_eq!(list_limit(None).ok(), Some(MAX_LIST));
        assert_eq!(list_limit(Some(1)).ok(), Some(1));
        assert_eq!(list_limit(Some(MAX_LIST)).ok(), Some(MAX_LIST));
        for bad in [0, -1, MAX_LIST + 1] {
            assert!(list_limit(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn cursors_are_checked() {
        assert!(tree_cursor("a0!a1V").is_ok());
        assert!(tree_cursor("0193a5b2-0000-7000-8000-0000000000a1!a0").is_ok());
        for bad in ["", "a0 a1", "a0;drop", &"a".repeat(MAX_CURSOR + 1)] {
            assert!(tree_cursor(bad).is_err(), "{bad:?}");
        }

        let at = OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a time");
        let id = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00a1);
        let cursor = trash_position(at, id);
        assert_eq!(
            cursor,
            "1795348800000000.0193a5b2-0000-7000-8000-0000000000a1"
        );
        assert_eq!(trash_cursor(&cursor).ok(), Some((at, id)));
        for bad in [
            "",
            "123",
            "x.0193a5b2-0000-7000-8000-0000000000a1",
            "1.nope",
        ] {
            assert!(trash_cursor(bad).is_err(), "{bad:?}");
        }
    }
}
