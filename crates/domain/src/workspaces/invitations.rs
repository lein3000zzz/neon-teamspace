//! Invitations to a workspace, which only go to accounts that exist, and invite links.
//!
//! An invitation names one account, which accepts or declines it; nothing is joined until
//! it accepts. An invite link admits whoever opens it while signed in, up to its number of
//! uses, until it expires. Neither ever makes an account.

use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

use super::roles::WorkspaceRole;
use crate::auth::InviteStatus;

/// How long an invitation waits for its answer.
pub const INVITATION_TTL: Duration = Duration::from_secs(14 * 24 * 3600);

/// How long an invitation or invite link stays stored after it was answered, revoked, used
/// up or expired, before the hourly cleanup removes it.
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);

/// Starts every invite link's token.
pub const LINK_TOKEN_PREFIX: &str = "wsl_";

/// The longest token a join reads. Longer text matches no link, so it is refused before
/// it is hashed.
pub const MAX_LINK_TOKEN_INPUT: usize = 128;

/// The most people one link admits, and how many it admits unless told otherwise.
pub const MAX_LINK_USES: i32 = 1000;
pub const DEFAULT_LINK_USES: i32 = 10;

/// How long a link works unless told otherwise, and the longest it may.
pub const DEFAULT_LINK_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
pub const MAX_LINK_LIFETIME: Duration = Duration::from_secs(365 * 24 * 3600);

/// Where an invitation stands, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationState {
    Pending,
    Accepted,
    Declined,
    Revoked,
}

impl InvitationState {
    pub const ALL: [Self; 4] = [Self::Pending, Self::Accepted, Self::Declined, Self::Revoked];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }
}

/// An invitation of one account into a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The account invited.
    pub user_id: Uuid,
    /// The role it joins with: admin, member or guest.
    pub role: WorkspaceRole,
    /// Who invited; unset once their account is gone.
    pub invited_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub state: InvitationState,
    /// When it was accepted, declined or revoked.
    pub decided_at: Option<OffsetDateTime>,
}

/// An invite link, without its token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The role people join with: member or guest.
    pub role: WorkspaceRole,
    pub max_uses: i32,
    /// How many joined with it.
    pub uses: i32,
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by: Option<Uuid>,
    /// Whether its token was kept sealed, so it can be copied again.
    pub token_kept: bool,
}

impl InviteLink {
    /// Revoked wins over used up, which wins over expired, as for instance invites.
    pub fn status(&self, now: OffsetDateTime) -> InviteStatus {
        if self.revoked_at.is_some() {
            InviteStatus::Revoked
        } else if self.uses >= self.max_uses {
            InviteStatus::UsedUp
        } else if now >= self.expires_at {
            InviteStatus::Expired
        } else {
            InviteStatus::Open
        }
    }
}

/// Something asked of a new invite link that it cannot have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidLinkSpec {
    #[error("an invite link admits 1 to {MAX_LINK_USES} people")]
    Uses,
    #[error("an invite link ends after now and within a year")]
    Expiry,
}

/// How many people a link admits, [`DEFAULT_LINK_USES`] when not given.
pub fn check_link_uses(max_uses: Option<i32>) -> Result<i32, InvalidLinkSpec> {
    let uses = max_uses.unwrap_or(DEFAULT_LINK_USES);
    if (1..=MAX_LINK_USES).contains(&uses) {
        Ok(uses)
    } else {
        Err(InvalidLinkSpec::Uses)
    }
}

/// When a link ends: the time given, or [`DEFAULT_LINK_TTL`] from now; after now, and
/// within [`MAX_LINK_LIFETIME`].
pub fn check_link_expiry(
    expires_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> Result<OffsetDateTime, InvalidLinkSpec> {
    let expires_at = expires_at.unwrap_or(now + DEFAULT_LINK_TTL);
    if expires_at > now && expires_at <= now + MAX_LINK_LIFETIME {
        Ok(expires_at)
    } else {
        Err(InvalidLinkSpec::Expiry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn link() -> InviteLink {
        InviteLink {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            role: WorkspaceRole::Member,
            max_uses: 2,
            uses: 0,
            created_by: None,
            created_at: now(),
            expires_at: now() + HOUR,
            revoked_at: None,
            revoked_by: None,
            token_kept: true,
        }
    }

    #[test]
    fn states_round_trip_through_text() {
        for state in InvitationState::ALL {
            assert_eq!(InvitationState::parse(state.as_str()), Some(state));
        }
        assert_eq!(InvitationState::parse("expired"), None);
    }

    #[test]
    fn link_uses_default_to_ten_and_stay_within_bounds() {
        assert_eq!(check_link_uses(None), Ok(DEFAULT_LINK_USES));
        assert_eq!(check_link_uses(Some(1)), Ok(1));
        assert_eq!(check_link_uses(Some(1000)), Ok(1000));
        for bad in [0, -1, 1001] {
            assert_eq!(check_link_uses(Some(bad)), Err(InvalidLinkSpec::Uses));
        }
    }

    #[test]
    fn link_expiry_defaults_to_a_week_and_stays_within_a_year() {
        assert_eq!(check_link_expiry(None, now()), Ok(now() + DEFAULT_LINK_TTL));
        assert_eq!(
            check_link_expiry(Some(now() + MAX_LINK_LIFETIME), now()),
            Ok(now() + MAX_LINK_LIFETIME)
        );
        assert_eq!(
            check_link_expiry(Some(now() + MAX_LINK_LIFETIME + HOUR), now()),
            Err(InvalidLinkSpec::Expiry)
        );
        assert_eq!(
            check_link_expiry(Some(now()), now()),
            Err(InvalidLinkSpec::Expiry)
        );
    }

    #[test]
    fn link_status_says_why_it_admits_no_one() {
        let open = link();
        let used = InviteLink { uses: 2, ..link() };
        let revoked = InviteLink {
            revoked_at: Some(now()),
            uses: 2,
            ..link()
        };

        assert_eq!(open.status(now()), InviteStatus::Open);
        assert_eq!(open.status(now() + HOUR), InviteStatus::Expired);
        assert_eq!(used.status(now() + HOUR), InviteStatus::UsedUp);
        assert_eq!(revoked.status(now()), InviteStatus::Revoked);
    }
}
