//! Links mailed to an account: to verify its address, or to set a new password. Each works
//! once, until it expires, and the database keeps only its token's hash.

use std::fmt;
use std::time::Duration;

/// What a mailed link does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailTokenPurpose {
    /// Verifies the address.
    Verify,
    /// Sets a new password.
    Reset,
}

impl EmailTokenPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
        }
    }

    /// How long a link of this purpose works.
    pub fn lifetime(self) -> Duration {
        match self {
            Self::Verify => VERIFY_LINK_TTL,
            Self::Reset => RESET_LINK_TTL,
        }
    }
}

impl fmt::Display for EmailTokenPurpose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How long a verification link works.
pub const VERIFY_LINK_TTL: Duration = Duration::from_secs(24 * 3600);

/// How long a password reset link works.
pub const RESET_LINK_TTL: Duration = Duration::from_secs(3600);

/// The least time between two links of one purpose to one account.
pub const LINK_GAP: Duration = Duration::from_secs(60);

/// The most links of one purpose an account gets in [`LINK_WINDOW`].
pub const LINKS_PER_WINDOW: i64 = 5;
pub const LINK_WINDOW: Duration = Duration::from_secs(24 * 3600);

/// How long an account that must verify its address may go without, before the hourly
/// cleanup removes it.
pub const UNVERIFIED_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// How long used and expired links are kept, so they still count towards
/// [`LINKS_PER_WINDOW`].
pub const RETENTION: Duration = LINK_WINDOW;

/// The longest token a link may carry. Longer text matches nothing, so it is refused
/// before it is hashed.
pub const MAX_LINK_TOKEN: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_purpose_has_its_lifetime() {
        assert_eq!(EmailTokenPurpose::Verify.lifetime(), VERIFY_LINK_TTL);
        assert_eq!(
            EmailTokenPurpose::Reset.lifetime(),
            Duration::from_secs(3600)
        );
        assert_eq!(EmailTokenPurpose::Reset.to_string(), "reset");
        assert!(RETENTION >= LINK_WINDOW);
    }
}
