//! Creating workspaces, listing one's own, and changing and deleting them.

use neon_domain::auth::Principal;
use neon_domain::hue::check_hue;
use neon_domain::workspaces::{
    Action, LimitOverrides, Limits, Resource, Teamspace, TeamspaceAccess, TeamspaceRole, Workspace,
    WorkspaceRole, authorize, check_icon, check_name, check_slug, slug_base, suffixed_slug,
    usable_as_is,
};
use neon_store::{is_unique_violation, teamspaces, workspaces as repo};
use tracing::info;
use uuid::Uuid;

use super::{
    NO_WORKSPACE, WorkspaceError, WorkspaceService, actor, locked_membership, membership,
    require_session,
};

/// The name a workspace's default teamspace starts with.
pub const DEFAULT_TEAMSPACE_NAME: &str = "General";

/// How many slugs with a suffix a new workspace tries before giving up.
const SLUG_ATTEMPTS: usize = 8;

/// What a new workspace starts with. Its slug comes from its name unless one is given.
#[derive(Debug, Clone, Default)]
pub struct NewWorkspace {
    pub name: String,
    pub slug: Option<String>,
    pub icon: Option<String>,
    pub hue: Option<i64>,
}

/// What a change to a workspace sets. Each part left out stays as it is; for the icon and
/// the hue, `Some(None)` removes them.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceUpdate {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub icon: Option<Option<String>>,
    pub hue: Option<Option<i64>>,
    pub members_can_create_teamspaces: Option<bool>,
}

/// A workspace as one of its members sees it.
#[derive(Debug, Clone)]
pub struct WorkspaceDetails {
    pub workspace: Workspace,
    /// The caller's role in it.
    pub role: WorkspaceRole,
    /// How many people are in it.
    pub members: i64,
    pub limits: Limits,
}

/// A workspace the caller just joined, and their role in it.
#[derive(Debug, Clone)]
pub struct Joined {
    pub workspace: Workspace,
    pub role: WorkspaceRole,
}

impl WorkspaceService {
    /// Makes a workspace, as `workspaces.creation` allows, with its default teamspace and
    /// its creator as the owner of both, together.
    pub async fn create_workspace(
        &self,
        principal: &Principal,
        new: NewWorkspace,
    ) -> Result<WorkspaceDetails, WorkspaceError> {
        self.observe("create_workspace", async {
            let instance = Resource::Instance {
                creation: self.creation(),
            };
            authorize(actor(principal, None), Action::CreateWorkspace, instance)
                .map_err(|denied| WorkspaceError::denied(denied, NO_WORKSPACE))?;
            let name = check_name(&new.name).map_err(WorkspaceError::invalid)?;
            let icon = match new.icon {
                Some(icon) => check_icon(&icon).map_err(WorkspaceError::invalid)?,
                None => None,
            };
            let hue_override = new
                .hue
                .map(check_hue)
                .transpose()
                .map_err(WorkspaceError::invalid)?;
            let typed = new
                .slug
                .filter(|slug| !slug.trim().is_empty())
                .map(|slug| check_slug(&slug))
                .transpose()
                .map_err(WorkspaceError::invalid)?;

            let now = self.now();
            let user = principal.user.id;
            let mut workspace = Workspace {
                id: Uuid::now_v7(),
                name,
                slug: String::new(),
                icon,
                hue_override,
                members_can_create_teamspaces: true,
                created_by: Some(user),
                created_at: now,
                updated_at: now,
            };
            let mut tx = self.pool().begin().await?;
            let mut saved = false;
            for slug in slug_candidates(typed.as_deref(), &workspace.name) {
                workspace.slug = slug;
                if repo::insert(&mut *tx, &workspace).await? {
                    saved = true;
                    break;
                }
            }
            if !saved {
                return Err(if typed.is_some() {
                    WorkspaceError::SlugTaken
                } else {
                    WorkspaceError::Internal(format!(
                        "no free slug for a workspace after {SLUG_ATTEMPTS} tries"
                    ))
                });
            }
            let general = Teamspace {
                id: Uuid::now_v7(),
                workspace_id: workspace.id,
                name: DEFAULT_TEAMSPACE_NAME.to_owned(),
                icon: None,
                hue_override: None,
                access: TeamspaceAccess::Default,
                created_by: Some(user),
                created_at: now,
                updated_at: now,
            };
            teamspaces::insert(&mut *tx, &general).await?;
            repo::add_member(&mut *tx, workspace.id, user, WorkspaceRole::Owner, now).await?;
            teamspaces::add_member(&mut *tx, &general, user, TeamspaceRole::Owner, now).await?;
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace.id,
                slug = %workspace.slug,
                "workspace created"
            );
            Ok(WorkspaceDetails {
                workspace,
                role: WorkspaceRole::Owner,
                members: 1,
                limits: self.limits(LimitOverrides::default()),
            })
        })
        .await
    }

    /// The workspaces the principal is in, with their role in each, by name.
    pub async fn list_workspaces(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(Workspace, WorkspaceRole)>, WorkspaceError> {
        self.observe("list_workspaces", async {
            Ok(repo::list_for_user(self.pool(), principal.user.id).await?)
        })
        .await
    }

    /// A workspace the principal is in.
    pub async fn workspace(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<WorkspaceDetails, WorkspaceError> {
        self.observe("get_workspace", async {
            let mut conn = self.pool().acquire().await?;
            let membership = membership(&mut conn, principal, id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let role = membership.role()?;
            self.details(&mut conn, membership.workspace, role).await
        })
        .await
    }

    /// Changes a workspace's name, slug, icon, hue or settings, for its owners and admins.
    /// A new name leaves the slug as it is, so links keep working.
    pub async fn update_workspace(
        &self,
        principal: &Principal,
        id: Uuid,
        update: WorkspaceUpdate,
    ) -> Result<WorkspaceDetails, WorkspaceError> {
        self.observe("update_workspace", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, id).await?;
            membership.check(principal, Action::UpdateWorkspace)?;
            let role = membership.role()?;
            let mut workspace = membership.workspace;
            if let Some(name) = update.name {
                workspace.name = check_name(&name).map_err(WorkspaceError::invalid)?;
            }
            if let Some(slug) = update.slug {
                workspace.slug = check_slug(&slug).map_err(WorkspaceError::invalid)?;
            }
            if let Some(icon) = update.icon {
                workspace.icon = match icon {
                    Some(icon) => check_icon(&icon).map_err(WorkspaceError::invalid)?,
                    None => None,
                };
            }
            if let Some(hue) = update.hue {
                workspace.hue_override = hue
                    .map(check_hue)
                    .transpose()
                    .map_err(WorkspaceError::invalid)?;
            }
            if let Some(allowed) = update.members_can_create_teamspaces {
                workspace.members_can_create_teamspaces = allowed;
            }
            workspace.updated_at = self.now();
            repo::update(&mut *tx, &workspace).await.map_err(|error| {
                if is_unique_violation(&error, repo::SLUG_TAKEN) {
                    WorkspaceError::SlugTaken
                } else {
                    WorkspaceError::Database(error)
                }
            })?;
            let details = self.details(&mut tx, workspace, role).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, workspace_id = %id, "workspace updated");
            Ok(details)
        })
        .await
    }

    /// Deletes a workspace with everything in it, for an owner who types its name to
    /// confirm. Only from a browser session: a token that leaked cannot delete a team's
    /// work.
    pub async fn delete_workspace(
        &self,
        principal: &Principal,
        id: Uuid,
        confirm_name: &str,
    ) -> Result<(), WorkspaceError> {
        self.observe("delete_workspace", async {
            require_session(
                principal,
                "deleting a workspace needs a browser session: API tokens cannot",
            )?;
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, id).await?;
            membership.check(principal, Action::DeleteWorkspace)?;
            if confirm_name.trim() != membership.workspace.name {
                return Err(WorkspaceError::ConfirmationMismatch);
            }
            repo::delete(&mut *tx, id).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, workspace_id = %id, "workspace deleted");
            Ok(())
        })
        .await
    }

    async fn details(
        &self,
        conn: &mut sqlx::PgConnection,
        workspace: Workspace,
        role: WorkspaceRole,
    ) -> Result<WorkspaceDetails, WorkspaceError> {
        let members = repo::count_members(&mut *conn, workspace.id).await?;
        let limits = self.limits_of(&mut *conn, workspace.id).await?;
        Ok(WorkspaceDetails {
            workspace,
            role,
            members,
            limits,
        })
    }
}

/// The slugs a new workspace tries in turn: the one typed alone, or else the name's, then
/// the name's with a suffix. A name's slug that is one of the app's paths, or looks like
/// an id, goes straight to the suffixes.
fn slug_candidates(typed: Option<&str>, name: &str) -> Vec<String> {
    if let Some(slug) = typed {
        return vec![slug.to_owned()];
    }
    let base = slug_base(name);
    let plain = usable_as_is(&base).then(|| base.clone());
    plain
        .into_iter()
        .chain((0..SLUG_ATTEMPTS).map(|_| suffixed_slug(&base, &slug_suffix())))
        .collect()
}

/// Six random characters of lowercase letters and digits, without the ones easy to
/// confuse (l, o, 0, 1).
fn slug_suffix() -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 6];
    rand::fill(&mut bytes);
    bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_slug_is_the_only_candidate() {
        assert_eq!(slug_candidates(Some("team"), "Design"), ["team"]);
    }

    #[test]
    fn a_name_tries_its_slug_then_suffixes() {
        let candidates = slug_candidates(None, "Design Team");

        assert_eq!(candidates[0], "design-team");
        assert_eq!(candidates.len(), 1 + SLUG_ATTEMPTS);
        for candidate in &candidates[1..] {
            let suffix = candidate.strip_prefix("design-team-").expect("a suffix");
            assert_eq!(suffix.len(), 6);
            assert_eq!(check_slug(candidate).as_deref(), Ok(candidate.as_str()));
        }
    }

    #[test]
    fn reserved_names_go_straight_to_suffixes() {
        let candidates = slug_candidates(None, "Settings");

        assert_eq!(candidates.len(), SLUG_ATTEMPTS);
        assert!(candidates.iter().all(|slug| slug.starts_with("settings-")));
    }

    #[test]
    fn suffixes_avoid_confusable_characters() {
        for _ in 0..200 {
            let suffix = slug_suffix();
            assert!(
                suffix
                    .chars()
                    .all(|c| (c.is_ascii_lowercase() || c.is_ascii_digit()) && !"lo01".contains(c)),
                "{suffix}"
            );
        }
    }
}
