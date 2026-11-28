//! Releases of Neon Teamspace and the updater that moves a stack to one: which release
//! counts, which is the newest, whether the server runs behind it, and where an update
//! stands.
//!
//! Versions compare by semantic versioning's precedence, which build metadata does not
//! change: `0.2.0+abc` is neither newer nor older than `0.2.0`.

use std::cmp::Ordering;
use std::fmt;

use semver::Version;
use time::OffsetDateTime;
use url::Url;

/// The fewest characters the updater's token has. The updater refuses to start with a
/// shorter one, and `updates.updater_token` refuses one too.
pub const MIN_UPDATER_TOKEN: usize = 32;

/// A release of Neon Teamspace, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Its tag without the `v`, such as `0.2.1` or `0.3.0-rc.1`.
    pub version: Version,
    /// GitHub marks it a pre-release.
    pub prerelease: bool,
    /// A draft, which only the repository's maintainers see. It never counts.
    pub draft: bool,
    /// Its page on GitHub, with its notes.
    pub url: Url,
    /// When it was published; unset for a draft.
    pub published_at: Option<OffsetDateTime>,
}

impl Release {
    /// A pre-release: GitHub marks it one, or its version has a pre-release part, as
    /// `0.3.0-rc.1` has.
    pub fn is_prerelease(&self) -> bool {
        self.prerelease || !self.version.pre.is_empty()
    }

    /// Whether it counts as a release to update to: never a draft, and a pre-release only
    /// while `prereleases` is on.
    pub fn counts(&self, prereleases: bool) -> bool {
        !self.draft && (prereleases || !self.is_prerelease())
    }
}

/// The newest of `releases` that counts, by precedence. Of two with the same precedence,
/// the one listed first.
pub fn newest(releases: &[Release], prereleases: bool) -> Option<&Release> {
    releases
        .iter()
        .filter(|release| release.counts(prereleases))
        .reduce(|best, release| {
            if release.version.cmp_precedence(&best.version) == Ordering::Greater {
                release
            } else {
                best
            }
        })
}

/// Whether `candidate` comes after `than` by precedence.
pub fn is_newer(candidate: &Version, than: &Version) -> bool {
    candidate.cmp_precedence(than) == Ordering::Greater
}

/// The build the server runs: its version, the cargo workspace's, and whether it is a
/// development build, one compiled with debug assertions, as `cargo run` makes. A
/// development build is never behind a release: it is updated from its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub version: Version,
    pub development: bool,
}

impl Running {
    /// Whether a release of `version` is an update for this build: newer, and never for a
    /// development build.
    pub fn is_behind(&self, version: &Version) -> bool {
        !self.development && is_newer(version, &self.version)
    }
}

impl fmt::Display for Running {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.development {
            write!(f, "{}, a development build", self.version)
        } else {
            write!(f, "{}", self.version)
        }
    }
}

text_enum! {
    /// Where the updater is with updating the stack.
    UpdaterState {
        /// Asked nothing since it started.
        Idle => "idle",
        /// Pulling the release's images and restarting the services with them.
        Updating => "updating",
        /// The last update finished.
        Done => "done",
        /// The last update failed, and the stack runs what it ran before.
        Failed => "failed",
    }
}

/// The updater's latest update, as it reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterStatus {
    pub state: UpdaterState,
    /// The release it updates or updated to, such as `0.2.1`.
    pub version: Option<String>,
    /// Why the update failed.
    pub error: Option<String>,
    /// The end of what it did, 16 KiB at most.
    pub log: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    /// The updater's own version.
    pub updater_version: String,
}

/// Checks the updater's token: empty, for no updater, or at least [`MIN_UPDATER_TOKEN`]
/// characters, each printable ASCII other than a space, as it goes in a header. The error
/// never quotes the token.
pub fn check_updater_token(token: &str) -> Result<(), String> {
    let printable = |c: char| c.is_ascii_graphic();
    if token.is_empty()
        || (token.chars().count() >= MIN_UPDATER_TOKEN && token.chars().all(printable))
    {
        Ok(())
    } else {
        Err(format!(
            "must be at least {MIN_UPDATER_TOKEN} characters, with no spaces, such as the \
             output of openssl rand -hex 32"
        ))
    }
}

/// Checks the updater's address: an http or https URL with a host, and no user, password,
/// query or fragment, as the updater's paths come after it.
pub fn check_updater_url(value: &str) -> Result<(), String> {
    let refused = || "want an http or https URL, such as http://updater:8080".to_owned();
    let url = Url::parse(value).map_err(|_| refused())?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if plain { Ok(()) } else { Err(refused()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: Version::parse(version).expect("a version"),
            prerelease: false,
            draft: false,
            url: Url::parse(&format!(
                "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v{version}"
            ))
            .expect("a URL"),
            published_at: None,
        }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).expect("a version")
    }

    #[test]
    fn the_newest_is_by_precedence_not_by_order() {
        let releases = [release("0.1.1"), release("0.2.0"), release("0.1.10")];

        assert_eq!(
            newest(&releases, false).map(|r| r.version.to_string()),
            Some("0.2.0".to_owned())
        );
        assert_eq!(newest(&[], false), None);
    }

    #[test]
    fn pre_releases_count_only_when_asked_and_drafts_never() {
        let candidate = release("0.3.0-rc.1");
        let marked = Release {
            prerelease: true,
            ..release("0.2.5")
        };
        let draft = Release {
            draft: true,
            ..release("0.9.0")
        };
        let releases = [release("0.2.0"), candidate, marked, draft];

        assert_eq!(
            newest(&releases, false).map(|r| r.version.to_string()),
            Some("0.2.0".to_owned()),
            "stable releases alone"
        );
        assert_eq!(
            newest(&releases, true).map(|r| r.version.to_string()),
            Some("0.3.0-rc.1".to_owned()),
            "the draft never counts"
        );
        assert!(releases[2].is_prerelease(), "GitHub's mark is enough");
        assert!(releases[1].is_prerelease(), "and so is the version's");
    }

    #[test]
    fn a_release_is_newer_by_precedence() {
        assert!(is_newer(&version("0.2.0"), &version("0.1.9")));
        assert!(is_newer(&version("0.2.0"), &version("0.2.0-rc.1")));
        assert!(!is_newer(&version("0.2.0-rc.1"), &version("0.2.0")));
        assert!(!is_newer(&version("0.2.0+abc"), &version("0.2.0")));
        assert!(!is_newer(&version("0.1.0"), &version("0.2.0")));
    }

    #[test]
    fn a_development_build_is_never_behind() {
        let release = Running {
            version: version("0.1.0"),
            development: false,
        };
        let development = Running {
            development: true,
            ..release.clone()
        };

        assert!(release.is_behind(&version("0.2.0")));
        assert!(!release.is_behind(&version("0.1.0")));
        assert!(!development.is_behind(&version("0.2.0")));
        assert_eq!(development.to_string(), "0.1.0, a development build");
        assert_eq!(release.to_string(), "0.1.0");
    }

    #[test]
    fn the_token_is_empty_or_long_and_printable() {
        assert_eq!(check_updater_token(""), Ok(()));
        assert_eq!(check_updater_token(&"a".repeat(32)), Ok(()));
        for refused in [
            "a".repeat(31),
            format!("{} b", "a".repeat(32)),
            "é".repeat(32),
        ] {
            let error = check_updater_token(&refused).expect_err("refused");
            assert!(!error.contains(&refused), "{error}");
        }
    }

    #[test]
    fn the_updater_is_at_an_http_url() {
        for fine in [
            "http://updater:8080",
            "https://updater.internal",
            "http://10.0.0.5:8080/updater",
        ] {
            assert_eq!(check_updater_url(fine), Ok(()), "{fine}");
        }
        for refused in [
            "",
            "updater:8080",
            "ftp://updater",
            "http://user:pass@updater:8080",
            "http://updater:8080?x=1",
            "http://updater:8080#here",
        ] {
            assert!(check_updater_url(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn updater_states_have_the_protocol_s_texts() {
        assert_eq!(
            UpdaterState::TEXTS,
            ["idle", "updating", "done", "failed"].as_slice()
        );
    }
}
