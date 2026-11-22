//! One page at a time: creating, reading and changing a page, moving it, trashing it,
//! restoring it, deleting it for good, and the way down to it.

use neon_domain::auth::Principal;
use neon_domain::pages::{
    Cover, Font, MAX_DEPTH, Page, PageAccess, PageStyle, Placement, Space, check_title,
};
use neon_domain::workspaces::{Action, PageSpace, Resource, authorize, check_icon};
use neon_store::pages::{self as repo, Move, Restore, Siblings};
use neon_store::{teamspaces, workspaces};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{
    AccessChange, NO_PAGE, NO_TEAMSPACE, NO_WORKSPACE, PageError, PageService, PageView, actor,
    check_space,
};

/// Where a page goes: under another page, at the top of a teamspace, or at the top of the
/// caller's own private pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Under(Uuid),
    Teamspace(Uuid),
    Private,
}

/// What a new page starts with.
#[derive(Debug, Clone, Default)]
pub struct NewPage {
    pub title: String,
    pub icon: Option<String>,
    pub cover: Option<Cover>,
    pub style: PageStyle,
}

/// What a change to a page sets. Each part left out stays as it is; for the icon and the
/// cover, `Some(None)` removes them.
#[derive(Debug, Clone, Default)]
pub struct PageUpdate {
    pub title: Option<String>,
    pub icon: Option<Option<String>>,
    pub cover: Option<Option<Cover>>,
    pub font: Option<Font>,
    pub small_text: Option<bool>,
    pub full_width: Option<bool>,
}

impl PageService {
    /// Makes a page under another page, at the top of a teamspace, or private to the
    /// caller, placed among its siblings. Under a page, it needs edit access to that page;
    /// at the top of a teamspace, being in it.
    pub async fn create_page(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        location: Location,
        placement: Placement,
        new: NewPage,
    ) -> Result<PageView, PageError> {
        self.observe("create_page", async {
            let title = check_title(&new.title).map_err(PageError::invalid)?;
            let icon = match new.icon {
                Some(icon) => check_icon(&icon).map_err(PageError::invalid)?,
                None => None,
            };
            let user = principal.user.id;
            let mut tx = self.pool().begin().await?;
            let role = in_workspace(&mut tx, principal, workspace_id).await?;
            repo::lock_tree(&mut *tx, workspace_id).await?;
            let (space, parent_id) = match location {
                Location::Under(parent) => {
                    let context = self.context(&mut tx, principal, parent).await?;
                    context.check(Action::EditPage)?;
                    if context.page().workspace_id != workspace_id {
                        return Err(PageError::NotFound(NO_PAGE));
                    }
                    if context.depth() >= MAX_DEPTH {
                        return Err(PageError::TooDeep);
                    }
                    (context.page().space, Some(parent))
                }
                Location::Teamspace(id) => {
                    let space = teamspace_space(&mut tx, workspace_id, id, user).await?;
                    check_space(
                        actor(principal, role),
                        space,
                        Action::CreatePage,
                        NO_TEAMSPACE,
                    )?;
                    (Space::Teamspace(id), None)
                }
                Location::Private => {
                    let space = PageSpace::Private { own: true };
                    check_space(
                        actor(principal, role),
                        space,
                        Action::CreatePage,
                        NO_WORKSPACE,
                    )?;
                    (Space::Private { owner: user }, None)
                }
            };
            let siblings = Siblings::of(workspace_id, parent_id, space);
            let position = self.position(&mut tx, siblings, None, placement).await?;
            let now = self.now();
            let page = Page {
                id: Uuid::now_v7(),
                workspace_id,
                space,
                parent_id,
                position,
                title,
                icon,
                cover: new.cover,
                style: new.style,
                created_by: Some(user),
                last_edited_by: Some(user),
                created_at: now,
                updated_at: now,
                content_updated_at: now,
                trashed_at: None,
                trashed_by: None,
                trashed_with: None,
            };
            repo::insert(&mut *tx, &page).await?;
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace_id,
                page_id = %page.id,
                "page created"
            );
            // Whoever may add a page somewhere edits what is there.
            Ok(PageView {
                page,
                access: PageAccess::Edit,
            })
        })
        .await
    }

    /// A page, with what the caller may do with it. One in the trash shows only to those
    /// who could edit it.
    pub async fn page(&self, principal: &Principal, id: Uuid) -> Result<PageView, PageError> {
        self.observe("get_page", async {
            let mut conn = self.pool().acquire().await?;
            let context = self.context(&mut conn, principal, id).await?;
            context.check(Action::ViewPage)?;
            context.view()
        })
        .await
    }

    /// Changes a page's title, icon, cover or style, for those who edit it.
    pub async fn update_page(
        &self,
        principal: &Principal,
        id: Uuid,
        update: PageUpdate,
    ) -> Result<PageView, PageError> {
        self.observe("update_page", async {
            let mut tx = self.pool().begin().await?;
            let mut context = self.context(&mut tx, principal, id).await?;
            context.check(Action::EditPage)?;
            let page = &mut context.chain[0].page;
            if let Some(title) = update.title {
                page.title = check_title(&title).map_err(PageError::invalid)?;
            }
            if let Some(icon) = update.icon {
                page.icon = match icon {
                    Some(icon) => check_icon(&icon).map_err(PageError::invalid)?,
                    None => None,
                };
            }
            if let Some(cover) = update.cover {
                page.cover = cover;
            }
            if let Some(font) = update.font {
                page.style.font = font;
            }
            if let Some(small_text) = update.small_text {
                page.style.small_text = small_text;
            }
            if let Some(full_width) = update.full_width {
                page.style.full_width = full_width;
            }
            page.last_edited_by = Some(principal.user.id);
            page.updated_at = self.now();
            repo::update(&mut *tx, page).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, page_id = %id, "page updated");
            context.view()
        })
        .await
    }

    /// Moves a page, with the pages under it, under another page, to the top of a
    /// teamspace or to the caller's private pages, placed among its new siblings. It needs
    /// edit access to the page and to where it goes, and never goes under itself.
    pub async fn move_page(
        &self,
        principal: &Principal,
        id: Uuid,
        to: Location,
        placement: Placement,
    ) -> Result<PageView, PageError> {
        self.observe("move_page", async {
            let user = principal.user.id;
            let mut tx = self.pool().begin().await?;
            let source = self.locked_context(&mut tx, principal, id).await?;
            source.check(Action::EditPage)?;
            let workspace_id = source.page().workspace_id;
            let height = usize::try_from(repo::height(&mut *tx, id).await?).unwrap_or(MAX_DEPTH);
            let (space, parent_id) = match to {
                Location::Under(parent) => {
                    let target = self.context(&mut tx, principal, parent).await?;
                    target.check(Action::EditPage)?;
                    if target.page().workspace_id != workspace_id {
                        return Err(PageError::InvalidInput(
                            "a page moves within its workspace".to_owned(),
                        ));
                    }
                    if target.chain.iter().any(|link| link.page.id == id) {
                        return Err(PageError::Cycle);
                    }
                    if target.depth() + 1 + height > MAX_DEPTH {
                        return Err(PageError::TooDeep);
                    }
                    (target.page().space, Some(parent))
                }
                Location::Teamspace(teamspace) => {
                    let space = teamspace_space(&mut tx, workspace_id, teamspace, user).await?;
                    check_space(source.actor, space, Action::CreatePage, NO_TEAMSPACE)?;
                    (Space::Teamspace(teamspace), None)
                }
                Location::Private => {
                    let space = PageSpace::Private { own: true };
                    check_space(source.actor, space, Action::CreatePage, NO_WORKSPACE)?;
                    (Space::Private { owner: user }, None)
                }
            };
            if parent_id.is_none() && 1 + height > MAX_DEPTH {
                return Err(PageError::TooDeep);
            }
            let siblings = Siblings::of(workspace_id, parent_id, space);
            let position = self
                .position(&mut tx, siblings, Some(id), placement)
                .await?;
            repo::move_page(
                &mut *tx,
                &Move {
                    id,
                    parent_id,
                    position,
                    space,
                    by: user,
                    at: self.now(),
                },
            )
            .await?;
            let moved = self.context(&mut tx, principal, id).await?;
            tx.commit().await?;
            info!(
                user_id = %user,
                page_id = %id,
                parent_id = ?parent_id,
                space_id = %space.id(),
                "page moved"
            );
            self.changed(AccessChange::Pages {
                workspace_id,
                page_id: id,
                user_id: None,
            });
            moved.view()
        })
        .await
    }

    /// Moves a page to the trash with the pages under it, for those who edit it. Restoring
    /// it brings them back together.
    pub async fn trash_page(&self, principal: &Principal, id: Uuid) -> Result<PageView, PageError> {
        self.observe("trash_page", async {
            let mut tx = self.pool().begin().await?;
            let context = self.locked_context(&mut tx, principal, id).await?;
            context.check(Action::TrashPage)?;
            let workspace_id = context.page().workspace_id;
            let trashed = repo::trash(&mut *tx, id, principal.user.id, self.now()).await?;
            let context = self.context(&mut tx, principal, id).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, page_id = %id, pages = trashed, "page trashed");
            self.changed(AccessChange::Pages {
                workspace_id,
                page_id: id,
                user_id: None,
            });
            context.view()
        })
        .await
    }

    /// Brings a page back from the trash with the pages trashed along with it. It goes back
    /// under its parent, unless that is in the trash too: then to the top of its space,
    /// which the caller must be allowed to add pages to.
    pub async fn restore_page(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<PageView, PageError> {
        self.observe("restore_page", async {
            let mut tx = self.pool().begin().await?;
            let context = self.locked_context(&mut tx, principal, id).await?;
            context.check(Action::RestorePage)?;
            let page = context.page().clone();
            let trashed_with = page
                .trashed_with
                .ok_or_else(|| PageError::Internal(format!("page {id} is trashed with nothing")))?;
            // Every page above a page in the trash is there, but for those above the page
            // trashed on its own: its parent is out of the trash, or the parent went with it.
            let parent_id = page.parent_id.filter(|_| {
                context
                    .chain
                    .get(1)
                    .is_some_and(|link| !link.page.is_trashed())
            });
            if parent_id.is_none() && page.parent_id.is_some() {
                authorize(
                    context.actor,
                    Action::CreatePage,
                    Resource::Pages {
                        space: context.space,
                    },
                )
                .map_err(|_| {
                    PageError::Forbidden(
                        "the page it was under is in the trash, and you may not add pages at \
                         the top of its space: restore that page first",
                    )
                })?;
            }
            let siblings = Siblings::of(page.workspace_id, parent_id, page.space);
            let taken = repo::sibling_positions(&mut *tx, siblings, Some(id))
                .await?
                .iter()
                .any(|(_, position)| *position == page.position);
            let position = if parent_id == page.parent_id && !taken {
                page.position.clone()
            } else {
                self.position(&mut tx, siblings, Some(id), Placement::Last)
                    .await?
            };
            let restored = repo::restore(
                &mut *tx,
                &Restore {
                    id,
                    trashed_with,
                    parent_id,
                    position,
                    at: self.now(),
                },
            )
            .await?;
            let context = self.context(&mut tx, principal, id).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, page_id = %id, pages = restored, "page restored");
            self.changed(AccessChange::Pages {
                workspace_id: page.workspace_id,
                page_id: id,
                user_id: None,
            });
            context.view()
        })
        .await
    }

    /// Deletes a page in the trash for good, with everything under it, for those who could
    /// edit it.
    pub async fn delete_page(&self, principal: &Principal, id: Uuid) -> Result<(), PageError> {
        self.observe("delete_page", async {
            let mut tx = self.pool().begin().await?;
            let context = self.locked_context(&mut tx, principal, id).await?;
            context.check(Action::DeletePage)?;
            let workspace_id = context.page().workspace_id;
            repo::delete(&mut *tx, id).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, page_id = %id, "page deleted");
            self.changed(AccessChange::Pages {
                workspace_id,
                page_id: id,
                user_id: None,
            });
            Ok(())
        })
        .await
    }

    /// The way down to a page: the highest page above it the caller sees, then each page
    /// under that one, the page last.
    pub async fn breadcrumbs(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<Page>, PageError> {
        self.observe("breadcrumbs", async {
            let mut conn = self.pool().acquire().await?;
            let context = self.context(&mut conn, principal, id).await?;
            context.check(Action::ViewPage)?;
            Ok(context.breadcrumbs())
        })
        .await
    }

    /// Locks the tree of a page's workspace, then reads the page as the lock leaves it.
    async fn locked_context(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        id: Uuid,
    ) -> Result<super::Context, PageError> {
        let page = repo::get(&mut *conn, id)
            .await?
            .ok_or(PageError::NotFound(NO_PAGE))?;
        repo::lock_tree(&mut *conn, page.workspace_id).await?;
        self.context(conn, principal, id).await
    }
}

/// The principal's role in a workspace, once the rules let them know of it.
pub(super) async fn in_workspace(
    conn: &mut PgConnection,
    principal: &Principal,
    workspace_id: Uuid,
) -> Result<Option<neon_domain::workspaces::WorkspaceRole>, PageError> {
    let workspace = workspaces::get(&mut *conn, workspace_id)
        .await?
        .ok_or(PageError::NotFound(NO_WORKSPACE))?;
    let role = workspaces::member_role(&mut *conn, workspace_id, principal.user.id).await?;
    authorize(
        actor(principal, role),
        Action::ViewWorkspace,
        Resource::Workspace {
            members_can_create_teamspaces: workspace.members_can_create_teamspaces,
        },
    )
    .map_err(|denied| PageError::denied(denied, NO_WORKSPACE))?;
    Ok(role)
}

/// A teamspace of a workspace as the rules see it for `user`.
pub(super) async fn teamspace_space(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    teamspace_id: Uuid,
    user: Uuid,
) -> Result<PageSpace, PageError> {
    let teamspace = teamspaces::get(&mut *conn, workspace_id, teamspace_id)
        .await?
        .ok_or(PageError::NotFound(NO_TEAMSPACE))?;
    Ok(PageSpace::Teamspace {
        access: teamspace.access,
        membership: teamspaces::member_role(&mut *conn, teamspace_id, user).await?,
    })
}
