//! Invitations into workspaces, and invite links.

use neon_domain::workspaces::{
    Invitation, InvitationState, InviteLink, Person, Workspace, WorkspaceRole,
};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;
use crate::users::parse_hue;
use crate::workspaces::{Row as WorkspaceRow, parse_role};

/// The index an insert breaks when the account has a pending invitation to the workspace.
pub const ALREADY_INVITED: &str = "workspace_invitations_pending";

struct InvitationRow {
    id: Uuid,
    workspace_id: Uuid,
    user_id: Uuid,
    role: String,
    invited_by: Option<Uuid>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    state: String,
    decided_at: Option<OffsetDateTime>,
}

impl TryFrom<InvitationRow> for Invitation {
    type Error = sqlx::Error;

    fn try_from(row: InvitationRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            role: parse_role(&row.role)?,
            invited_by: row.invited_by,
            created_at: row.created_at,
            expires_at: row.expires_at,
            state: InvitationState::parse(&row.state)
                .ok_or_else(|| decode_error(format!("unknown state {:?}", row.state)))?,
            decided_at: row.decided_at,
        })
    }
}

/// Saves a new invitation. Fails on [`ALREADY_INVITED`] when one is pending for the same
/// workspace and account, expired or not: [`set_aside_expired`] first.
pub async fn insert(executor: impl PgExecutor<'_>, invitation: &Invitation) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO workspace_invitations (id, workspace_id, user_id, role, invited_by,
                                            created_at, expires_at, state, decided_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        invitation.id,
        invitation.workspace_id,
        invitation.user_id,
        invitation.role.as_str(),
        invitation.invited_by,
        invitation.created_at,
        invitation.expires_at,
        invitation.state.as_str(),
        invitation.decided_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Deletes a pending invitation of the account into the workspace that expired by `now`,
/// so a new one can be made.
pub async fn set_aside_expired(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM workspace_invitations
         WHERE workspace_id = $1 AND user_id = $2 AND state = 'pending' AND expires_at <= $3",
        workspace_id,
        user_id,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// The invitations a workspace sent that wait for an answer at `now`, with who each is
/// for, newest first.
pub async fn pending_for_workspace(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Vec<(Invitation, Person)>> {
    sqlx::query!(
        "SELECT i.id, i.workspace_id, i.user_id, i.role, i.invited_by, i.created_at,
                i.expires_at, i.state, i.decided_at,
                u.display_name, u.email, u.hue
         FROM workspace_invitations i JOIN users u ON u.id = i.user_id
         WHERE i.workspace_id = $1 AND i.state = 'pending' AND i.expires_at > $2
         ORDER BY i.created_at DESC, i.id DESC",
        workspace_id,
        now,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let invitee = Person {
            id: row.user_id,
            display_name: row.display_name,
            email: row.email,
            hue_override: parse_hue(row.hue)?,
        };
        let invitation = Invitation::try_from(InvitationRow {
            id: row.id,
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            role: row.role,
            invited_by: row.invited_by,
            created_at: row.created_at,
            expires_at: row.expires_at,
            state: row.state,
            decided_at: row.decided_at,
        })?;
        Ok((invitation, invitee))
    })
    .collect()
}

/// The invitations an account waits on at `now`, with their workspace and who sent each
/// while their account exists, newest first.
pub async fn pending_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Vec<(Invitation, Workspace, Option<Person>)>> {
    sqlx::query!(
        r#"SELECT i.id, i.workspace_id, i.user_id, i.role, i.invited_by, i.created_at,
                  i.expires_at, i.state, i.decided_at,
                  w.name, w.slug, w.icon, w.hue, w.members_can_create_teamspaces,
                  w.created_by AS workspace_created_by, w.created_at AS workspace_created_at,
                  w.updated_at AS workspace_updated_at,
                  u.display_name AS "inviter_name?", u.email AS "inviter_email?",
                  u.hue AS inviter_hue
           FROM workspace_invitations i
           JOIN workspaces w ON w.id = i.workspace_id
           LEFT JOIN users u ON u.id = i.invited_by
           WHERE i.user_id = $1 AND i.state = 'pending' AND i.expires_at > $2
           ORDER BY i.created_at DESC, i.id DESC"#,
        user_id,
        now,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let workspace = Workspace::try_from(WorkspaceRow {
            id: row.workspace_id,
            name: row.name,
            slug: row.slug,
            icon: row.icon,
            hue: row.hue,
            members_can_create_teamspaces: row.members_can_create_teamspaces,
            created_by: row.workspace_created_by,
            created_at: row.workspace_created_at,
            updated_at: row.workspace_updated_at,
        })?;
        let inviter = match (row.invited_by, row.inviter_name, row.inviter_email) {
            (Some(id), Some(display_name), Some(email)) => Some(Person {
                id,
                display_name,
                email,
                hue_override: parse_hue(row.inviter_hue)?,
            }),
            _ => None,
        };
        let invitation = Invitation::try_from(InvitationRow {
            id: row.id,
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            role: row.role,
            invited_by: row.invited_by,
            created_at: row.created_at,
            expires_at: row.expires_at,
            state: row.state,
            decided_at: row.decided_at,
        })?;
        Ok((invitation, workspace, inviter))
    })
    .collect()
}

/// An account's invitation, if it waits for an answer at `now`.
pub async fn open(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Option<Invitation>> {
    sqlx::query_as!(
        InvitationRow,
        "SELECT id, workspace_id, user_id, role, invited_by, created_at, expires_at, state,
                decided_at
         FROM workspace_invitations
         WHERE id = $1 AND user_id = $2 AND state = 'pending' AND expires_at > $3",
        id,
        user_id,
        now,
    )
    .fetch_optional(executor)
    .await?
    .map(Invitation::try_from)
    .transpose()
}

/// Like [`open`], locking the invitation until the transaction ends so it is answered
/// once.
pub async fn lock_open(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Option<Invitation>> {
    sqlx::query_as!(
        InvitationRow,
        "SELECT id, workspace_id, user_id, role, invited_by, created_at, expires_at, state,
                decided_at
         FROM workspace_invitations
         WHERE id = $1 AND user_id = $2 AND state = 'pending' AND expires_at > $3
         FOR UPDATE",
        id,
        user_id,
        now,
    )
    .fetch_optional(executor)
    .await?
    .map(Invitation::try_from)
    .transpose()
}

/// Answers or revokes an invitation that is pending. False when it is not.
pub async fn decide(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
    state: InvitationState,
    at: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspace_invitations SET state = $3, decided_at = $4
         WHERE workspace_id = $1 AND id = $2 AND state = 'pending'",
        workspace_id,
        id,
        state.as_str(),
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes the invitations answered, revoked or expired before `before`.
pub async fn delete_ended(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM workspace_invitations WHERE expires_at < $1 OR decided_at < $1",
        before,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

struct LinkRow {
    id: Uuid,
    workspace_id: Uuid,
    role: String,
    max_uses: i32,
    uses: i32,
    created_by: Option<Uuid>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    revoked_by: Option<Uuid>,
    token_kept: bool,
}

impl TryFrom<LinkRow> for InviteLink {
    type Error = sqlx::Error;

    fn try_from(row: LinkRow) -> sqlx::Result<Self> {
        let role = parse_role(&row.role)?;
        if !matches!(role, WorkspaceRole::Member | WorkspaceRole::Guest) {
            return Err(decode_error(format!("an invite link admits no {role}")));
        }
        Ok(Self {
            id: row.id,
            workspace_id: row.workspace_id,
            role,
            max_uses: row.max_uses,
            uses: row.uses,
            created_by: row.created_by,
            created_at: row.created_at,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            revoked_by: row.revoked_by,
            token_kept: row.token_kept,
        })
    }
}

/// Saves a new invite link with its token's hash and, when a key was there to seal it,
/// the sealed token.
pub async fn insert_link(
    executor: impl PgExecutor<'_>,
    link: &InviteLink,
    token_hash: &[u8],
    token_sealed: Option<&[u8]>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO workspace_invite_links (id, workspace_id, role, token_hash, token_sealed,
                                             max_uses, uses, created_by, created_at,
                                             expires_at, revoked_at, revoked_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        link.id,
        link.workspace_id,
        link.role.as_str(),
        token_hash,
        token_sealed,
        link.max_uses,
        link.uses,
        link.created_by,
        link.created_at,
        link.expires_at,
        link.revoked_at,
        link.revoked_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A workspace's invite links, newest first.
pub async fn links(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<InviteLink>> {
    sqlx::query_as!(
        LinkRow,
        r#"SELECT id, workspace_id, role, max_uses, uses, created_by, created_at, expires_at,
                  revoked_at, revoked_by, token_sealed IS NOT NULL AS "token_kept!"
           FROM workspace_invite_links WHERE workspace_id = $1
           ORDER BY created_at DESC, id DESC"#,
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(InviteLink::try_from)
    .collect()
}

/// A link's sealed token: `None` without such a link in the workspace, `Some(None)` when
/// its token was not kept.
pub async fn sealed_token(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Option<Vec<u8>>>> {
    sqlx::query_scalar!(
        "SELECT token_sealed FROM workspace_invite_links WHERE workspace_id = $1 AND id = $2",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Ends a link. One revoked before keeps when and by whom. False when there is none.
pub async fn revoke_link(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
    by: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspace_invite_links
         SET revoked_at = COALESCE(revoked_at, $4), revoked_by = COALESCE(revoked_by, $3)
         WHERE workspace_id = $1 AND id = $2",
        workspace_id,
        id,
        by,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The link of a token hash, if it still admits people at `now`.
pub async fn open_link(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> sqlx::Result<Option<InviteLink>> {
    sqlx::query_as!(
        LinkRow,
        r#"SELECT id, workspace_id, role, max_uses, uses, created_by, created_at, expires_at,
                  revoked_at, revoked_by, token_sealed IS NOT NULL AS "token_kept!"
           FROM workspace_invite_links
           WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > $2
                 AND uses < max_uses"#,
        token_hash,
        now,
    )
    .fetch_optional(executor)
    .await?
    .map(InviteLink::try_from)
    .transpose()
}

/// Takes one use of a link, in one conditional update, so people racing for its last use
/// cannot both have it. False when it no longer admits anyone at `now`. Run it in the
/// transaction that adds the member.
pub async fn consume_link(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspace_invite_links SET uses = uses + 1
         WHERE id = $1 AND revoked_at IS NULL AND expires_at > $2 AND uses < max_uses",
        id,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes the links that expired, or were revoked, before `before`.
pub async fn delete_ended_links(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM workspace_invite_links WHERE expires_at < $1 OR revoked_at < $1",
        before,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
