//! Who may create an account.

text_enum! {
    /// The instance's registration mode, from `auth.registration`.
    RegistrationMode {
        /// Anyone may sign up.
        Open => "open",
        /// Signing up needs an invite an admin made. The default.
        Invite => "invite",
        /// Only admins create accounts.
        Closed => "closed",
    }
}

impl RegistrationMode {
    pub const DEFAULT: Self = Self::Invite;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_text() {
        for mode in RegistrationMode::ALL {
            assert_eq!(RegistrationMode::parse(mode.as_str()), Some(*mode));
        }
        assert_eq!(RegistrationMode::parse("Invite"), None);
        assert_eq!(RegistrationMode::DEFAULT, RegistrationMode::Invite);
    }
}
