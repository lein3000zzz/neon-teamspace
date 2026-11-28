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
    /// The setting's options, in the order the docs list them.
    pub const OPTIONS: &[&str] = &["open", "invite", "closed"];

    pub const DEFAULT: Self = Self::Invite;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_the_modes_as_text() {
        let texts: Vec<&str> = RegistrationMode::ALL.iter().map(|m| m.as_str()).collect();

        assert_eq!(texts, RegistrationMode::OPTIONS);
        for mode in RegistrationMode::ALL {
            assert_eq!(RegistrationMode::parse(mode.as_str()), Some(*mode));
        }
        assert_eq!(RegistrationMode::parse("Invite"), None);
        assert_eq!(RegistrationMode::DEFAULT, RegistrationMode::Invite);
    }
}
