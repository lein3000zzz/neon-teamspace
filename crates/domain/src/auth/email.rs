//! Email addresses as accounts use them.
//!
//! An address is kept as typed, trimmed, and found by its lowercase form, so
//! `Ada@Example.com` and `ada@example.com` are one account.

use std::fmt;

/// The longest address accepted, in bytes: what fits in SMTP's forward path.
pub const MAX_EMAIL_LENGTH: usize = 254;

/// The longest part before the `@`, in bytes.
const MAX_LOCAL_LENGTH: usize = 64;

/// The longest label of a domain, in bytes.
const MAX_LABEL_LENGTH: usize = 63;

/// Characters an unquoted local part may hold besides letters, digits and dots.
const LOCAL_SYMBOLS: &str = "!#$%&'*+-/=?^_`{|}~";

/// Not an address the server takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not an email address like name@example.com")]
pub struct InvalidEmail;

/// An email address, as typed and trimmed, with the form accounts are found by.
#[derive(Clone, PartialEq, Eq)]
pub struct Email {
    address: String,
    normalized: String,
}

impl Email {
    /// Parses a bare address, `name@example.com`, after trimming spaces around it. Display
    /// names, comments and quoted local parts are refused, and so is anything longer than
    /// [`MAX_EMAIL_LENGTH`].
    pub fn parse(raw: &str) -> Result<Self, InvalidEmail> {
        let address = raw.trim();
        if address.len() > MAX_EMAIL_LENGTH {
            return Err(InvalidEmail);
        }
        let (local, domain) = address.split_once('@').ok_or(InvalidEmail)?;
        if !valid_local(local) || !valid_domain(domain) {
            return Err(InvalidEmail);
        }
        Ok(Self {
            address: address.to_owned(),
            normalized: normalize(address),
        })
    }

    /// An address read back from the database, which was checked when it was saved.
    pub fn from_stored(address: String) -> Self {
        let normalized = normalize(&address);
        Self {
            address,
            normalized,
        }
    }

    /// The address as typed.
    pub fn as_str(&self) -> &str {
        &self.address
    }

    /// The lowercase form accounts are found by.
    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    /// The part before the `@`.
    pub fn local_part(&self) -> &str {
        self.address
            .split_once('@')
            .map_or(self.address.as_str(), |(local, _)| local)
    }

    /// Whether two addresses are one, ignoring case.
    pub fn same_as(&self, other: &str) -> bool {
        self.normalized == normalize(other)
    }
}

impl fmt::Debug for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Email({:?})", self.address)
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.address)
    }
}

/// The form an address is found by: trimmed and in lowercase. Text that is no address
/// normalizes too, so a lookup of it simply finds nothing.
pub fn normalize(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Dot-separated atoms of letters, digits and the symbols RFC 5322 allows unquoted.
fn valid_local(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= MAX_LOCAL_LENGTH
        && local.split('.').all(|atom| {
            !atom.is_empty()
                && atom
                    .chars()
                    .all(|c| c.is_alphanumeric() || LOCAL_SYMBOLS.contains(c))
        })
}

/// Dot-separated labels of letters, digits and inner hyphens. One label alone, such as
/// `localhost`, is a domain too, for instances on a private network.
fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_LENGTH
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_addresses_parse_trimmed_with_a_lowercase_form() {
        let email = Email::parse("  Ada.Lovelace+neon@Example.COM ").expect("valid");

        assert_eq!(email.as_str(), "Ada.Lovelace+neon@Example.COM");
        assert_eq!(email.normalized(), "ada.lovelace+neon@example.com");
        assert_eq!(email.local_part(), "Ada.Lovelace+neon");
        assert!(email.same_as(" ADA.lovelace+NEON@example.com"));
        assert!(!email.same_as("ada@example.com"));
    }

    #[test]
    fn unusual_but_valid_addresses_parse() {
        for raw in [
            "admin@localhost",
            "o'brien@example.ie",
            "x@a-b.example",
            "{tag}@example.com",
            "jörg@bücher.example",
        ] {
            assert!(Email::parse(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        let long_local = format!("{}@example.com", "a".repeat(65));
        let long = format!("a@{}.com", "b".repeat(250));
        let long_label = format!("a@{}.com", "b".repeat(64));
        for raw in [
            "",
            "   ",
            "ada",
            "@example.com",
            "ada@",
            "ada@@example.com",
            "ada@exa@mple.com",
            "ada lovelace@example.com",
            "Ada <ada@example.com>",
            "\"ada\"@example.com",
            ".ada@example.com",
            "ada.@example.com",
            "a..da@example.com",
            "ada@example..com",
            "ada@-example.com",
            "ada@example-.com",
            "ada@exam_ple.com",
            "ada@example.com.",
            "ada@[127.0.0.1]",
            long_local.as_str(),
            long.as_str(),
            long_label.as_str(),
        ] {
            assert_eq!(Email::parse(raw), Err(InvalidEmail), "{raw:?}");
        }
    }

    #[test]
    fn stored_addresses_keep_their_form() {
        let email = Email::from_stored("Ada@Example.com".to_owned());

        assert_eq!(email.to_string(), "Ada@Example.com");
        assert_eq!(email.normalized(), "ada@example.com");
    }
}
