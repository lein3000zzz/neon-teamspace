//! Accounts: who they are and what they may do on the instance.

use std::fmt;

use time::OffsetDateTime;
use uuid::Uuid;

use super::email::Email;
use crate::hue::hue_or_hashed;

/// The longest display name, in characters.
pub const MAX_DISPLAY_NAME: usize = 100;

/// What an account may do on the instance. Admins manage accounts, invites and settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    /// Every role, as stored and sent.
    pub const ALL: [Self; 2] = [Self::Admin, Self::User];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == text)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An account. Its password hash stays with the repository and the use cases that check
/// it, so it never travels with the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// UUIDv7, so ids sort by when accounts were made.
    pub id: Uuid,
    pub email: Email,
    pub display_name: String,
    pub role: Role,
    /// The hue they pinned for themselves, if any; see [`hue`](Self::hue).
    pub hue_override: Option<u16>,
    /// Unset until the address is verified. Accounts admins make, and those that signed
    /// up with an invite mailed to them, are verified from the start.
    pub email_verified_at: Option<OffsetDateTime>,
    /// Set while the account is disabled: it cannot sign in, and its sessions and tokens
    /// stop working.
    pub disabled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }

    pub fn is_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }

    /// An admin who can sign in. The instance always keeps at least one.
    pub fn is_active_admin(&self) -> bool {
        self.is_admin() && !self.is_disabled()
    }

    /// The hue they are drawn in: the one they pinned, or else their id's.
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }
}

/// A display name that cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidDisplayName {
    #[error("a display name has at most {MAX_DISPLAY_NAME} characters")]
    TooLong,
    #[error("a display name cannot hold control characters such as line breaks")]
    Control,
}

/// The display name to keep: the one given, trimmed, or the address's local part when
/// none is given.
pub fn display_name(given: Option<&str>, email: &Email) -> Result<String, InvalidDisplayName> {
    let name = given.map(str::trim).filter(|name| !name.is_empty());
    let name = name.unwrap_or_else(|| email.local_part());
    if name.chars().any(char::is_control) {
        return Err(InvalidDisplayName::Control);
    }
    if name.chars().count() > MAX_DISPLAY_NAME {
        return Err(InvalidDisplayName::TooLong);
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email() -> Email {
        Email::parse("ada@example.com").expect("valid")
    }

    fn user(role: Role, disabled: bool) -> User {
        let now = OffsetDateTime::UNIX_EPOCH;
        User {
            id: Uuid::now_v7(),
            email: email(),
            display_name: "Ada".into(),
            role,
            hue_override: None,
            email_verified_at: None,
            disabled_at: disabled.then_some(now),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn roles_round_trip_through_text() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("Admin"), None);
        assert_eq!(Role::parse("owner"), None);
    }

    #[test]
    fn only_enabled_admins_are_active_admins() {
        assert!(user(Role::Admin, false).is_active_admin());
        assert!(!user(Role::Admin, true).is_active_admin());
        assert!(!user(Role::User, false).is_active_admin());
        assert!(!user(Role::User, false).is_verified());
    }

    #[test]
    fn a_pinned_hue_wins_over_the_hashed_one() {
        let mut ada = user(Role::User, false);
        let hashed = crate::hue::hue_for_uuid(ada.id);

        assert_eq!(ada.hue(), hashed);
        ada.hue_override = Some(7);
        assert_eq!(ada.hue(), 7);
    }

    #[test]
    fn display_names_default_to_the_local_part() {
        assert_eq!(display_name(None, &email()), Ok("ada".to_owned()));
        assert_eq!(display_name(Some("   "), &email()), Ok("ada".to_owned()));
        assert_eq!(
            display_name(Some("  Ada Lovelace "), &email()),
            Ok("Ada Lovelace".to_owned())
        );
    }

    #[test]
    fn display_names_are_bounded() {
        let long = "x".repeat(MAX_DISPLAY_NAME + 1);
        let longest = "é".repeat(MAX_DISPLAY_NAME);

        assert_eq!(
            display_name(Some(&long), &email()),
            Err(InvalidDisplayName::TooLong)
        );
        assert_eq!(display_name(Some(&longest), &email()), Ok(longest.clone()));
        assert_eq!(
            display_name(Some("Ada\nLovelace"), &email()),
            Err(InvalidDisplayName::Control)
        );
    }
}
