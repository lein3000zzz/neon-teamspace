//! Instance invites: codes admins hand out that let people sign up while registration is
//! by invite.
//!
//! A code is typed by the admin, or made by the server: [`GENERATED_PREFIX`] and 32 random
//! bytes. Either way it is matched ignoring case, so the database keeps the hash of its
//! [`lookup_form`].

use std::fmt;
use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

/// Starts every code the server makes.
pub const GENERATED_PREFIX: &str = "inv_";

/// The shortest typed code, and the longest, in characters.
pub const CODE_LENGTH: (usize, usize) = (8, 64);

/// The longest code a lookup reads. Longer text matches no invite, so it is refused before
/// it is hashed.
pub const MAX_CODE_INPUT: usize = 128;

/// The most sign-ups one invite admits.
pub const MAX_USES: i32 = 1000;

/// The sign-ups an invite admits unless told otherwise.
pub const DEFAULT_USES: i32 = 1;

/// The furthest ahead an invite may end.
pub const MAX_LIFETIME: Duration = Duration::from_secs(365 * 24 * 3600);

/// How long an invite stays listed after it expired or was revoked, before the hourly
/// cleanup removes it with its list of sign-ups.
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);

/// What is wrong with a typed code, `None` when nothing is.
pub fn code_problem(code: &str) -> Option<&'static str> {
    let length = code.chars().count();
    if length < CODE_LENGTH.0 || length > CODE_LENGTH.1 {
        Some("a code has 8 to 64 characters")
    } else if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some("a code has letters, digits, - and _ only")
    } else {
        None
    }
}

/// The form of a code whose hash finds its invite: trimmed and in lowercase, so codes
/// match whatever their case.
pub fn lookup_form(code: &str) -> String {
    code.trim().to_lowercase()
}

/// Something an admin asked of an invite that it cannot have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidInviteSpec {
    #[error("an invite admits 1 to {MAX_USES} sign-ups")]
    Uses,
    #[error("an invite ends after now and within a year")]
    Expiry,
}

/// How many sign-ups an invite admits, [`DEFAULT_USES`] when not given.
pub fn check_uses(max_uses: Option<i32>) -> Result<i32, InvalidInviteSpec> {
    let uses = max_uses.unwrap_or(DEFAULT_USES);
    if (1..=MAX_USES).contains(&uses) {
        Ok(uses)
    } else {
        Err(InvalidInviteSpec::Uses)
    }
}

/// When an invite ends: the time given, or `default_ttl` from now. It must be after now
/// and within [`MAX_LIFETIME`].
pub fn check_expiry(
    expires_at: Option<OffsetDateTime>,
    default_ttl: Duration,
    now: OffsetDateTime,
) -> Result<OffsetDateTime, InvalidInviteSpec> {
    let expires_at = expires_at.unwrap_or(now + default_ttl);
    if expires_at > now && expires_at <= now + MAX_LIFETIME {
        Ok(expires_at)
    } else {
        Err(InvalidInviteSpec::Expiry)
    }
}

/// An invite, without its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub id: Uuid,
    /// The admin who made it; unset once their account is gone.
    pub created_by: Option<Uuid>,
    /// The address it is bound to: it alone signs up with it. Set for invites mailed by an
    /// admin or made by approving a request to join.
    pub email: Option<String>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub max_uses: i32,
    /// The sign-ups it admitted.
    pub uses: i32,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by: Option<Uuid>,
    /// Whether its code was kept sealed, so admins can copy it again.
    pub code_kept: bool,
}

/// Whether an invite admits sign-ups now, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    Open,
    UsedUp,
    Expired,
    Revoked,
}

impl InviteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::UsedUp => "used",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }
}

impl fmt::Display for InviteStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Invite {
    /// Revoked wins over used up, which wins over expired.
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

/// A sign-up with an invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteUse {
    /// The account's id and address, both unset once it is gone.
    pub user_id: Option<Uuid>,
    pub email: Option<String>,
    pub used_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn invite() -> Invite {
        Invite {
            id: Uuid::now_v7(),
            created_by: None,
            email: None,
            created_at: now(),
            expires_at: now() + HOUR,
            max_uses: 2,
            uses: 0,
            revoked_at: None,
            revoked_by: None,
            code_kept: true,
        }
    }

    #[test]
    fn typed_codes_are_8_to_64_url_safe_characters() {
        assert_eq!(code_problem("welcome1"), None);
        assert_eq!(code_problem("Team_2026-Spring"), None);
        assert_eq!(code_problem(&"a".repeat(64)), None);
        assert_eq!(code_problem("short"), Some("a code has 8 to 64 characters"));
        assert_eq!(
            code_problem(&"a".repeat(65)),
            Some("a code has 8 to 64 characters")
        );
        for bad in [
            "with space",
            "dots.dots",
            "ümlauts1",
            "slash/slash",
            "emoji😀xx",
        ] {
            assert_eq!(
                code_problem(bad),
                Some("a code has letters, digits, - and _ only"),
                "{bad}"
            );
        }
    }

    #[test]
    fn codes_are_found_ignoring_case() {
        assert_eq!(lookup_form(" Welcome-2026 "), "welcome-2026");
        assert_eq!(lookup_form("inv_AbC"), lookup_form("INV_abc"));
    }

    #[test]
    fn uses_default_to_one_and_stay_within_bounds() {
        assert_eq!(check_uses(None), Ok(1));
        assert_eq!(check_uses(Some(1000)), Ok(1000));
        assert_eq!(check_uses(Some(0)), Err(InvalidInviteSpec::Uses));
        assert_eq!(check_uses(Some(1001)), Err(InvalidInviteSpec::Uses));
        assert_eq!(check_uses(Some(-1)), Err(InvalidInviteSpec::Uses));
    }

    #[test]
    fn expiry_defaults_to_the_ttl_and_stays_within_a_year() {
        let week = 7 * 24 * HOUR;

        assert_eq!(check_expiry(None, week, now()), Ok(now() + week));
        assert_eq!(
            check_expiry(Some(now() + MAX_LIFETIME), week, now()),
            Ok(now() + MAX_LIFETIME)
        );
        assert_eq!(
            check_expiry(Some(now() + MAX_LIFETIME + HOUR), week, now()),
            Err(InvalidInviteSpec::Expiry)
        );
        assert_eq!(
            check_expiry(Some(now()), week, now()),
            Err(InvalidInviteSpec::Expiry)
        );
        assert_eq!(
            check_expiry(None, MAX_LIFETIME * 2, now()),
            Err(InvalidInviteSpec::Expiry)
        );
    }

    #[test]
    fn status_says_why_an_invite_admits_no_one() {
        let open = invite();
        let used = Invite {
            uses: 2,
            ..invite()
        };
        let revoked = Invite {
            revoked_at: Some(now()),
            uses: 2,
            ..invite()
        };

        assert_eq!(open.status(now()), InviteStatus::Open);
        assert_eq!(open.status(now() + HOUR), InviteStatus::Expired);
        assert_eq!(used.status(now()), InviteStatus::UsedUp);
        assert_eq!(used.status(now() + HOUR), InviteStatus::UsedUp);
        assert_eq!(revoked.status(now()), InviteStatus::Revoked);
        assert_eq!(InviteStatus::UsedUp.to_string(), "used");
    }
}
