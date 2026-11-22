//! Who may create an account.

use std::fmt;

/// The instance's registration mode, from `auth.registration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationMode {
    /// Anyone may sign up.
    Open,
    /// Signing up needs an invite an admin made. The default.
    Invite,
    /// Only admins create accounts.
    Closed,
}

impl RegistrationMode {
    pub const ALL: [Self; 3] = [Self::Open, Self::Invite, Self::Closed];

    /// The setting's options, in the order the docs list them.
    pub const OPTIONS: &[&str] = &["open", "invite", "closed"];

    pub const DEFAULT: Self = Self::Invite;

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Invite => "invite",
            Self::Closed => "closed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == text)
    }
}

impl fmt::Display for RegistrationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_the_modes_as_text() {
        let texts: Vec<&str> = RegistrationMode::ALL.iter().map(|m| m.as_str()).collect();

        assert_eq!(texts, RegistrationMode::OPTIONS);
        for mode in RegistrationMode::ALL {
            assert_eq!(RegistrationMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(RegistrationMode::parse("Invite"), None);
        assert_eq!(RegistrationMode::DEFAULT, RegistrationMode::Invite);
    }
}
