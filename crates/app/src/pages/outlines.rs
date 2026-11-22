//! Outlines: the pages an export writes, in the order of their tree, with what it needs of
//! each besides its content.

use neon_domain::auth::Principal;
use neon_domain::workspaces::{Action, Teamspace};
use neon_store::pages::{self as repo, OutlineNode};
use neon_store::{teamspaces, workspaces};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::operations::teamspace_space;
use super::{NO_TEAMSPACE, PageError, PageService, actor, check_space};

/// A page as an export writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlinePage {
    pub id: Uuid,
    /// The page it is under, among those listed with it; `None` for the first.
    pub parent_id: Option<Uuid>,
    pub title: String,
    /// When its content last changed.
    pub content_updated_at: OffsetDateTime,
}

impl From<OutlineNode> for OutlinePage {
    fn from(node: OutlineNode) -> Self {
        Self {
            id: node.id,
            parent_id: node.parent_id,
            title: node.title,
            content_updated_at: node.content_updated_at,
        }
    }
}

/// A teamspace's pages as an export writes them, and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamspaceOutline {
    pub name: String,
    pub pages: Vec<OutlinePage>,
}

impl PageService {
    /// A page the caller sees and, with `children`, the pages under it, each page then the
    /// pages under it, siblings in order: those out of the trash, or for a page in the
    /// trash, those that went there with it. Who sees a page sees every page under it. At
    /// most `limit` pages, and one more when there are more, so the caller tells.
    pub async fn page_outline(
        &self,
        principal: &Principal,
        id: Uuid,
        children: bool,
        limit: usize,
    ) -> Result<Vec<OutlinePage>, PageError> {
        self.observe("page_outline", async {
            let mut conn = self.pool().acquire().await?;
            let context = self.context(&mut conn, principal, id).await?;
            context.check(Action::ViewPage)?;
            let mut pages: Vec<OutlinePage> = if children {
                let limit = i64::try_from(limit).unwrap_or(i64::MAX - 1) + 1;
                repo::outline(&mut *conn, id, limit)
                    .await?
                    .into_iter()
                    .map(OutlinePage::from)
                    .collect()
            } else {
                let page = context.page();
                vec![OutlinePage {
                    id: page.id,
                    parent_id: None,
                    title: page.title.clone(),
                    content_updated_at: page.content_updated_at,
                }]
            };
            // The page above it may be hidden from the caller.
            if let Some(first) = pages.first_mut() {
                first.parent_id = None;
            }
            Ok(pages)
        })
        .await
    }

    /// Whether the caller may list a teamspace's pages, as
    /// [`teamspace_outline`](Self::teamspace_outline) asks, without reading them.
    pub async fn check_teamspace_pages(
        &self,
        principal: &Principal,
        teamspace_id: Uuid,
    ) -> Result<(), PageError> {
        self.observe("check_teamspace_pages", async {
            let mut conn = self.pool().acquire().await?;
            listed_teamspace(&mut conn, principal, teamspace_id).await?;
            Ok(())
        })
        .await
    }

    /// A teamspace's pages out of the trash, for everyone who may list them, as
    /// [`teamspace_tree`](Self::teamspace_tree) has them. At most `limit`, and one more
    /// when there are more.
    pub async fn teamspace_outline(
        &self,
        principal: &Principal,
        teamspace_id: Uuid,
        limit: usize,
    ) -> Result<TeamspaceOutline, PageError> {
        self.observe("teamspace_outline", async {
            let mut conn = self.pool().acquire().await?;
            let teamspace = listed_teamspace(&mut conn, principal, teamspace_id).await?;
            let workspace_id = teamspace.workspace_id;
            let limit = i64::try_from(limit).unwrap_or(i64::MAX - 1) + 1;
            let pages = repo::space_outline(&mut *conn, workspace_id, teamspace_id, limit)
                .await?
                .into_iter()
                .map(OutlinePage::from)
                .collect();
            Ok(TeamspaceOutline {
                name: teamspace.name,
                pages,
            })
        })
        .await
    }
}

/// A teamspace whose pages the principal may list.
async fn listed_teamspace(
    conn: &mut PgConnection,
    principal: &Principal,
    teamspace_id: Uuid,
) -> Result<Teamspace, PageError> {
    let user = principal.user.id;
    let teamspace = teamspaces::find(&mut *conn, teamspace_id)
        .await?
        .ok_or(PageError::NotFound(NO_TEAMSPACE))?;
    let workspace_id = teamspace.workspace_id;
    let role = workspaces::member_role(&mut *conn, workspace_id, user).await?;
    let space = teamspace_space(conn, workspace_id, teamspace_id, user).await?;
    check_space(
        actor(principal, role),
        space,
        Action::ListPages,
        NO_TEAMSPACE,
    )?;
    Ok(teamspace)
}
