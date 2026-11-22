//! What a new password must be.

/// The longest password accepted, in bytes. It bounds the hashing work one request can
/// cause; passphrases fit comfortably.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// The fewest characters `auth.min_password_length` may ask for, and the most.
pub const MIN_PASSWORD_LENGTH_BOUNDS: (i64, i64) = (8, 128);

/// `auth.min_password_length` unless set.
pub const DEFAULT_MIN_PASSWORD_LENGTH: i64 = 12;

/// The rule a new password is held to. Existing passwords keep working when it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordPolicy {
    /// The fewest characters, counted as Unicode scalar values.
    pub min_length: usize,
}

/// Why a new password is refused. The messages never repeat the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WeakPassword {
    #[error("a password has at least {0} characters")]
    TooShort(usize),
    #[error("a password has at most {MAX_PASSWORD_BYTES} bytes")]
    TooLong,
    #[error("a password cannot be only spaces")]
    Blank,
}

impl PasswordPolicy {
    pub fn check(&self, password: &str) -> Result<(), WeakPassword> {
        if password.chars().count() < self.min_length {
            Err(WeakPassword::TooShort(self.min_length))
        } else if password.len() > MAX_PASSWORD_BYTES {
            Err(WeakPassword::TooLong)
        } else if password.trim().is_empty() {
            Err(WeakPassword::Blank)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: PasswordPolicy = PasswordPolicy { min_length: 12 };

    #[test]
    fn length_counts_characters_not_bytes() {
        assert_eq!(POLICY.check("correct horse"), Ok(()));
        assert_eq!(POLICY.check("éééééééééééé"), Ok(()));
        assert_eq!(POLICY.check("ééééééééééé"), Err(WeakPassword::TooShort(12)));
        assert_eq!(POLICY.check(""), Err(WeakPassword::TooShort(12)));
    }

    #[test]
    fn the_byte_bound_holds_whatever_the_minimum() {
        let longest = "a".repeat(MAX_PASSWORD_BYTES);
        let long = "a".repeat(MAX_PASSWORD_BYTES + 1);
        let wide = "é".repeat(MAX_PASSWORD_BYTES / 2 + 1);

        assert_eq!(POLICY.check(&longest), Ok(()));
        assert_eq!(POLICY.check(&long), Err(WeakPassword::TooLong));
        assert_eq!(POLICY.check(&wide), Err(WeakPassword::TooLong));
    }

    #[test]
    fn blank_passwords_are_refused() {
        assert_eq!(POLICY.check(&" ".repeat(20)), Err(WeakPassword::Blank));
        assert_eq!(POLICY.check("\t\n            "), Err(WeakPassword::Blank));
    }

    #[test]
    fn messages_say_what_is_wanted() {
        assert_eq!(
            WeakPassword::TooShort(12).to_string(),
            "a password has at least 12 characters"
        );
        assert_eq!(
            WeakPassword::TooLong.to_string(),
            "a password has at most 1024 bytes"
        );
    }
}
