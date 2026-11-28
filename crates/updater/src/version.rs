//! The versions releases are tagged with: `major.minor.patch`, and a pre-release after a
//! hyphen, such as `0.3.0-rc.1`, as [semver.org](https://semver.org) has them. No leading `v`
//! and no build metadata: a version is also an image tag, which takes no `+`.

use std::fmt;
use std::str::FromStr;

/// The longest version taken. An image tag holds 128 characters, and no release needs many.
const MAX_LEN: usize = 64;

/// A release's version, checked: only digits, ASCII letters, dots and hyphens, so it is safe
/// in an image tag, a container label and a quoted value of an env file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version(String);

impl Version {
    /// The version as text, such as `0.2.1`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Text that is not a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAVersion(String);

impl fmt::Display for NotAVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a version, such as 0.2.1", self.0)
    }
}

impl std::error::Error for NotAVersion {}

impl FromStr for Version {
    type Err = NotAVersion;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if is_version(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotAVersion(text.to_owned()))
        }
    }
}

/// Whether `text` is a version: three numbers without leading zeros, then perhaps a hyphen
/// and dot-separated identifiers of digits, letters and hyphens, the numeric ones without
/// leading zeros.
fn is_version(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_LEN {
        return false;
    }
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers.iter().all(|number| is_number(number))
        && pre.is_none_or(|pre| pre.split('.').all(is_identifier))
}

/// Digits, and no leading zero but in `0` itself.
fn is_number(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}

/// A pre-release identifier: digits, letters and hyphens; a number has no leading zero.
fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && (!text.bytes().all(|b| b.is_ascii_digit()) || is_number(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_releases_and_pre_releases() {
        for good in [
            "0.2.1",
            "1.0.0",
            "10.20.30",
            "0.3.0-rc.1",
            "1.0.0-alpha-1.beta.0x",
        ] {
            assert!(good.parse::<Version>().is_ok(), "{good}");
        }
    }

    #[test]
    fn refuses_anything_else() {
        for bad in [
            "",
            "v0.2.1",
            "0.2",
            "0.2.1.4",
            "01.2.3",
            "0.2.1-",
            "0.2.1-rc..1",
            "0.2.1-rc.01",
            "0.2.1+build.5",
            "0.2.1; reboot",
            "0.2.1 --pull always",
            "0.2.1'",
            "latest",
            &format!("0.2.1-{}", "a".repeat(60)),
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
    }

    #[test]
    fn says_what_it_refused() {
        let error = "latest".parse::<Version>().unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"latest\" is not a version, such as 0.2.1"
        );
    }
}
