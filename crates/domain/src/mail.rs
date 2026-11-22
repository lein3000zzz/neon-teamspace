//! Rules of outgoing mail: how the connection to the mail server is secured, what its
//! host may be, and where a password may go without TLS.

use std::net::IpAddr;

/// How the connection to the mail server is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// A plain connection upgraded with STARTTLS. A server that does not offer it is
    /// refused, so nothing goes in the clear.
    StartTls,
    /// TLS from the first byte, as on port 465.
    Tls,
    /// A plain connection, for a relay on this host or a network the operator trusts.
    None,
}

impl Security {
    /// The setting's options, in the order the docs list them.
    pub const OPTIONS: &'static [&'static str] = &["starttls", "tls", "none"];

    pub const DEFAULT: Self = Self::StartTls;

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartTls => "starttls",
            Self::Tls => "tls",
            Self::None => "none",
        }
    }

    /// Whether the connection is encrypted before anything but the greeting is sent.
    pub fn encrypts(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The longest host name DNS allows.
const MAX_HOST: usize = 253;

/// Checks a mail server's host: a name like `smtp.example.com`, or an IP address, without
/// a scheme, a port or a user. Empty passes: it means no server is set.
pub fn check_host(host: &str) -> Result<(), String> {
    if host.is_empty() || host.parse::<IpAddr>().is_ok() || is_host_name(host) {
        Ok(())
    } else {
        Err(
            "want a host alone, such as smtp.example.com or 192.0.2.10, without a scheme or \
             port: the port is mail.port"
                .to_owned(),
        )
    }
}

/// Dot-separated labels of letters, digits and inner hyphens.
fn is_host_name(host: &str) -> bool {
    host.len() <= MAX_HOST
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Whether `host` is this machine: `localhost`, or a loopback address. Over a connection
/// without TLS, a password is only ever sent there, where nobody on the network can read
/// it on the way.
pub fn is_this_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Whether a mail server may be signed in to with these settings: always over TLS, and
/// without it only on this host.
pub fn may_sign_in(security: Security, host: &str) -> bool {
    security.encrypts() || is_this_host(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_options_round_trip() {
        for option in Security::OPTIONS {
            let security = Security::parse(option).expect("an option");
            assert_eq!(security.as_str(), *option);
        }
        assert_eq!(Security::parse("ssl"), None);
        assert_eq!(Security::DEFAULT.as_str(), "starttls");
        assert!(Security::StartTls.encrypts() && Security::Tls.encrypts());
        assert!(!Security::None.encrypts());
    }

    #[test]
    fn hosts_are_names_or_addresses_alone() {
        for good in [
            "",
            "smtp.example.com",
            "mailpit",
            "mail-1.example.com",
            "192.0.2.10",
            "::1",
        ] {
            assert_eq!(check_host(good), Ok(()), "{good}");
        }
        for bad in [
            "smtp.example.com:587",
            "smtps://smtp.example.com",
            "user@smtp.example.com",
            "smtp example.com",
            "-smtp.example.com",
            "smtp..example.com",
            "[::1]",
        ] {
            assert!(check_host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn passwords_go_in_the_clear_only_to_this_host() {
        for local in ["localhost", "LOCALHOST", "127.0.0.1", "127.1.2.3", "::1"] {
            assert!(is_this_host(local), "{local}");
            assert!(may_sign_in(Security::None, local), "{local}");
        }
        for remote in [
            "smtp.example.com",
            "10.0.0.5",
            "mailpit",
            "localhost.example.com",
        ] {
            assert!(!is_this_host(remote), "{remote}");
            assert!(!may_sign_in(Security::None, remote), "{remote}");
            assert!(may_sign_in(Security::StartTls, remote));
            assert!(may_sign_in(Security::Tls, remote));
        }
    }
}
