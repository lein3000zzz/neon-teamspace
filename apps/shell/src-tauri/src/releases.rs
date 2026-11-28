//! The newest release of the apps, as the Android app looks for one on GitHub.
//!
//! Tauri's updater has no Android side, so the Android app asks GitHub's API for the
//! repository's latest release, `GET /repos/lein3000zzz/neon-teamspace/releases/latest`, which
//! leaves drafts and pre-releases out. It sends no credentials, waits [`TIMEOUT`] at most, reads
//! at most [`MAX_ANSWER_BYTES`], and follows a redirect only within `https://api.github.com`,
//! where a renamed repository's API sends its callers. A release is newer when the version of
//! its tag, `v` and a semantic version, is above the app's. Its APK is the asset named
//! `…_android.apk`, which only a release signed with the project's key has, and only at the
//! address where the repository's releases keep that asset: the app opens nothing else.

use std::time::Duration;

use reqwest::header::{ACCEPT, HeaderValue};
use reqwest::{StatusCode, redirect};
use semver::Version;
use serde::Deserialize;
use url::Url;

/// The repository whose releases the apps come from, as `owner/name`.
pub const REPOSITORY: &str = "lein3000zzz/neon-teamspace";

/// The API's address of the repository's latest release.
pub const LATEST_RELEASE: &str =
    "https://api.github.com/repos/lein3000zzz/neon-teamspace/releases/latest";

/// The largest answer read. A release's, its notes and assets included, is a few KiB.
pub const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// How long the whole request may take.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// The only host a redirect of the API may lead to.
const API_HOST: &str = "api.github.com";

/// The most redirects followed.
const MAX_REDIRECTS: usize = 3;

/// The end of the signed APK's name, as the release workflow names it.
const APK_SUFFIX: &str = "_android.apk";

/// A release, as the app offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Its version, from its tag.
    pub version: Version,
    /// Its page on GitHub, which lists its files and links its notes.
    pub page: Url,
    /// The address of its signed APK, if it has one.
    pub apk: Option<Url>,
}

/// Why the latest release could not be had.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseError {
    /// The request failed: `reason` is the innermost cause, such as "connection refused".
    #[error("The app could not reach GitHub, where its releases are published: {reason}.")]
    Unreachable { reason: String },
    #[error("GitHub did not answer within {seconds} seconds.")]
    TimedOut { seconds: u64 },
    /// GitHub answers `403` or `429` once a network has asked too often without signing in.
    #[error("GitHub takes only so many questions an hour from one network. Try again later.")]
    RateLimited,
    /// `404`: the repository has published no release yet.
    #[error("GitHub has no release of the app yet.")]
    NoRelease,
    #[error("GitHub answered {status} when asked for the latest release.")]
    Status { status: u16 },
    #[error("GitHub's answer was not a release the app could read: {what}.")]
    Unreadable { what: String },
}

impl ReleaseError {
    /// A stable name for the picker and the tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable { .. } => "unreachable",
            Self::TimedOut { .. } => "timed_out",
            Self::RateLimited => "rate_limited",
            Self::NoRelease => "no_release",
            Self::Status { .. } => "status",
            Self::Unreadable { .. } => "unreadable",
        }
    }
}

/// Where the latest release is asked for, and how long that may take. Tests point it at a
/// server of their own.
#[derive(Debug, Clone)]
pub struct ReleaseOptions {
    pub url: Url,
    pub timeout: Duration,
}

impl Default for ReleaseOptions {
    fn default() -> Self {
        Self {
            url: Url::parse(LATEST_RELEASE).unwrap_or_else(|_| unreachable!("a valid URL")),
            timeout: TIMEOUT,
        }
    }
}

/// Asks GitHub for the repository's latest release.
pub async fn latest(options: &ReleaseOptions) -> Result<Release, ReleaseError> {
    let client = crate::meta::client_builder(options.timeout)
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < MAX_REDIRECTS
                && attempt.url().scheme() == "https"
                && attempt.url().host_str() == Some(API_HOST)
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|error| request_error(options, &error))?;
    let mut response = client
        .get(options.url.clone())
        .header(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        )
        .header(
            "x-github-api-version",
            HeaderValue::from_static("2022-11-28"),
        )
        .send()
        .await
        .map_err(|error| request_error(options, &error))?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => return Err(ReleaseError::NoRelease),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            return Err(ReleaseError::RateLimited);
        }
        status => {
            return Err(ReleaseError::Status {
                status: status.as_u16(),
            });
        }
    }
    let too_large = || ReleaseError::Unreadable {
        what: format!("it is larger than {} KiB", MAX_ANSWER_BYTES / 1024),
    };
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ANSWER_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| request_error(options, &error))?
    {
        if body.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    read_release(&body)
}

/// The fields of the API's answer the app reads.
#[derive(Deserialize)]
struct Answer {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Reads the API's answer for a release.
pub fn read_release(body: &[u8]) -> Result<Release, ReleaseError> {
    let unreadable = |what: &str| ReleaseError::Unreadable {
        what: what.to_owned(),
    };
    let answer: Answer =
        serde_json::from_slice(body).map_err(|_| unreadable("it is not the JSON of a release"))?;
    if answer.draft || answer.prerelease {
        return Err(unreadable("it is a draft or a pre-release"));
    }
    let version = answer
        .tag_name
        .strip_prefix('v')
        .and_then(|version| Version::parse(version).ok())
        .ok_or_else(|| unreadable("its tag is not a version"))?;
    let tag = format!("v{version}");
    if answer.tag_name != tag {
        return Err(unreadable("its tag is not a version"));
    }
    let page = github_url(&["releases", "tag", &tag]);
    let apk = answer
        .assets
        .iter()
        .filter(|asset| asset.name.ends_with(APK_SUFFIX))
        .find_map(|asset| {
            let expected = github_url(&["releases", "download", &tag, &asset.name]);
            (Url::parse(&asset.browser_download_url).ok()? == expected).then_some(expected)
        });
    Ok(Release { version, page, apk })
}

/// Whether `release` is newer than `current`, as semantic versions rank them: a pre-release
/// below its release, and build metadata of no account.
pub fn is_newer(release: &Version, current: &Version) -> bool {
    release.cmp_precedence(current).is_gt()
}

/// The address of a page of the repository on GitHub, under `segments`.
fn github_url(segments: &[&str]) -> Url {
    let mut url = Url::parse("https://github.com").unwrap_or_else(|_| unreachable!("a valid URL"));
    if let Ok(mut path) = url.path_segments_mut() {
        path.clear().extend(REPOSITORY.split('/')).extend(segments);
    }
    url
}

fn request_error(options: &ReleaseOptions, error: &reqwest::Error) -> ReleaseError {
    if error.is_timeout() {
        return ReleaseError::TimedOut {
            seconds: options.timeout.as_secs_f64().ceil() as u64,
        };
    }
    // The innermost cause says the most, such as "connection refused".
    let mut cause: &(dyn std::error::Error + 'static) = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    ReleaseError::Unreachable {
        reason: cause.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// The API's answer for the release `tag`, with assets of these names at the addresses
    /// GitHub gives them.
    fn answer(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|name| {
                format!(
                    r#"{{"name":"{name}","browser_download_url":"https://github.com/{REPOSITORY}/releases/download/{tag}/{name}","size":1}}"#
                )
            })
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","name":"{tag}","draft":false,"prerelease":false,"html_url":"https://github.com/{REPOSITORY}/releases/tag/{tag}","body":"Notes","assets":[{}]}}"#,
            assets.join(",")
        )
    }

    fn unreadable(body: &str) -> String {
        match read_release(body.as_bytes()) {
            Err(ReleaseError::Unreadable { what }) => what,
            other => panic!("{body} was read as {other:?}"),
        }
    }

    #[test]
    fn a_higher_version_is_newer_as_semantic_versions_rank_them() {
        for (release, current) in [
            ("0.2.0", "0.1.0"),
            ("0.1.1", "0.1.0"),
            ("1.0.0", "0.9.9"),
            ("0.10.0", "0.9.0"),
            ("0.2.0", "0.2.0-rc.1"),
            ("0.2.0-rc.2", "0.2.0-rc.1"),
        ] {
            assert!(
                is_newer(&version(release), &version(current)),
                "{release} over {current}"
            );
        }
        for (release, current) in [
            ("0.1.0", "0.1.0"),
            ("0.1.0", "0.2.0"),
            ("0.2.0-rc.1", "0.2.0"),
            ("0.9.0", "0.10.0"),
            ("0.1.0+build.2", "0.1.0+build.1"),
            ("0.1.0", "0.1.0+local"),
        ] {
            assert!(
                !is_newer(&version(release), &version(current)),
                "{release} over {current}"
            );
        }
    }

    #[test]
    fn a_release_gives_its_version_its_page_and_its_signed_apk() {
        let body = answer(
            "v0.2.0",
            &[
                "Neon-Teamspace_0.2.0_aarch64.dmg",
                "Neon-Teamspace_0.2.0_android.apk",
                "SHA256SUMS",
            ],
        );
        let release = read_release(body.as_bytes()).unwrap();
        assert_eq!(release.version, version("0.2.0"));
        assert_eq!(
            release.page.as_str(),
            "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0"
        );
        assert_eq!(
            release.apk.as_ref().map(Url::as_str),
            Some(
                "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0/Neon-Teamspace_0.2.0_android.apk"
            )
        );
    }

    #[test]
    fn an_unsigned_apk_or_none_is_no_apk_to_offer() {
        for assets in [
            &["Neon-Teamspace_0.2.0_android-unsigned.apk"][..],
            &["Neon-Teamspace_0.2.0_x64-setup.exe", "SHA256SUMS"][..],
            &[][..],
        ] {
            let release = read_release(answer("v0.2.0", assets).as_bytes()).unwrap();
            assert_eq!(release.apk, None, "{assets:?}");
            assert_eq!(release.version, version("0.2.0"));
        }
    }

    #[test]
    fn an_apk_anywhere_but_the_releases_own_address_is_not_offered() {
        for url in [
            "https://evil.example/Neon-Teamspace_0.2.0_android.apk",
            "http://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0/Neon-Teamspace_0.2.0_android.apk",
            "https://github.com/someone/neon-teamspace/releases/download/v0.2.0/Neon-Teamspace_0.2.0_android.apk",
            "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.1.0/Neon-Teamspace_0.2.0_android.apk",
            "https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0/other.apk",
            "https://github.com.evil.example/lein3000zzz/neon-teamspace/releases/download/v0.2.0/Neon-Teamspace_0.2.0_android.apk",
            "not a url",
        ] {
            let body = format!(
                r#"{{"tag_name":"v0.2.0","assets":[{{"name":"Neon-Teamspace_0.2.0_android.apk","browser_download_url":"{url}"}}]}}"#
            );
            let release = read_release(body.as_bytes()).unwrap();
            assert_eq!(release.apk, None, "{url}");
        }
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_refused() {
        for tag in ["0.2.0", "v0.2", "release-1", "v0.2.0 ", "v01.2.0", ""] {
            assert_eq!(
                unreadable(&answer(tag, &[])),
                "its tag is not a version",
                "{tag:?}"
            );
        }
    }

    #[test]
    fn anything_but_a_published_release_is_refused() {
        assert_eq!(unreadable("not json"), "it is not the JSON of a release");
        assert_eq!(unreadable("[]"), "it is not the JSON of a release");
        assert_eq!(
            unreadable(r#"{"message":"Not Found"}"#),
            "it is not the JSON of a release"
        );
        for flag in ["draft", "prerelease"] {
            let body = format!(r#"{{"tag_name":"v0.2.0","{flag}":true,"assets":[]}}"#);
            assert_eq!(unreadable(&body), "it is a draft or a pre-release");
        }
    }

    #[test]
    fn every_failure_has_a_code_and_a_sentence() {
        let errors = [
            ReleaseError::Unreachable {
                reason: "connection refused".to_owned(),
            },
            ReleaseError::TimedOut { seconds: 15 },
            ReleaseError::RateLimited,
            ReleaseError::NoRelease,
            ReleaseError::Status { status: 502 },
            ReleaseError::Unreadable {
                what: "it is not the JSON of a release".to_owned(),
            },
        ];
        let codes: Vec<&str> = errors.iter().map(ReleaseError::code).collect();
        assert_eq!(
            codes,
            [
                "unreachable",
                "timed_out",
                "rate_limited",
                "no_release",
                "status",
                "unreadable"
            ]
        );
        for error in errors {
            let message = error.to_string();
            assert!(message.ends_with('.'), "{message}");
            assert!(message.contains("GitHub"), "{message}");
        }
    }
}
