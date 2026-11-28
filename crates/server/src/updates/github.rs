//! The [`Releases`] port over GitHub's REST API: the latest releases of Neon Teamspace's
//! repository, asked without credentials, with a `User-Agent` that names the product and
//! the server's version and nothing else.

use std::time::Duration;

use async_trait::async_trait;
use axum::http::header::{ACCEPT, USER_AGENT};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use neon_app::updates::{Releases, ReleasesError};
use neon_domain::updates::Release;
use semver::Version;
use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use super::client::{Client, Outgoing};

/// The GitHub API the release check asks unless `updates.github_api_url` names another.
pub const DEFAULT_API_URL: &str = "https://api.github.com";

/// The repository whose releases are Neon Teamspace's.
pub const REPOSITORY: &str = "lein3000zzz/neon-teamspace";

/// How many of the latest releases are listed, newest first: the newest version is among
/// them unless an older line had as many releases since.
const PER_PAGE: u32 = 30;

/// How long asking may take: less than a request to the API may, as an admin's Check now
/// waits for it.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The most of GitHub's answer that is read. A release takes a few KiB with its notes and
/// its assets listed.
const MAX_ANSWER: usize = 4 * 1024 * 1024;

/// The version of GitHub's REST API the answer is read as.
const API_VERSION: &str = "2022-11-28";

/// Lists the releases on GitHub; see the module docs.
pub struct GitHubReleases {
    url: Url,
    user_agent: HeaderValue,
    client: Client,
}

impl GitHubReleases {
    /// Asks the GitHub API at `api`, such as [`DEFAULT_API_URL`], or a mirror of it at a
    /// path of its own.
    pub fn new(api: &Url) -> Self {
        let mut url = api.clone();
        url.set_path(&format!(
            "{}/repos/{REPOSITORY}/releases",
            api.path().trim_end_matches('/')
        ));
        url.set_query(Some(&format!("per_page={PER_PAGE}")));
        Self {
            url,
            user_agent: HeaderValue::from_static(concat!(
                "NeonTeamspace/",
                env!("CARGO_PKG_VERSION")
            )),
            client: Client::default(),
        }
    }
}

/// A release as GitHub lists it; the rest of its fields are not read.
#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
}

#[async_trait]
impl Releases for GitHubReleases {
    async fn list(&self) -> Result<Vec<Release>, ReleasesError> {
        let answer = self
            .client
            .send(Outgoing {
                method: Method::GET,
                url: &self.url,
                headers: vec![
                    (USER_AGENT, self.user_agent.clone()),
                    (
                        ACCEPT,
                        HeaderValue::from_static("application/vnd.github+json"),
                    ),
                    (
                        HeaderName::from_static("x-github-api-version"),
                        HeaderValue::from_static(API_VERSION),
                    ),
                ],
                json: None,
                timeout: TIMEOUT,
                max_answer: MAX_ANSWER,
            })
            .await
            .map_err(|error| ReleasesError(format!("GitHub could not be asked: {error}")))?;
        match answer.status {
            StatusCode::OK => {}
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                return Err(ReleasesError(format!(
                    "GitHub answered {}: it limits how often one address asks, so the next \
                     check tries again",
                    answer.status.as_u16()
                )));
            }
            StatusCode::NOT_FOUND => {
                return Err(ReleasesError(format!(
                    "GitHub answered 404: it shows no repository {REPOSITORY}"
                )));
            }
            other => {
                return Err(ReleasesError(format!("GitHub answered {}", other.as_u16())));
            }
        }
        let listed: Vec<Listed> = serde_json::from_slice(&answer.body)
            .map_err(|_| ReleasesError("GitHub's answer is not a list of releases".to_owned()))?;
        Ok(listed.into_iter().filter_map(release).collect())
    }
}

/// A release as the port gives it: none for a tag that is no version, `v` and a version
/// such as `v0.2.1`, or for a page that is no https URL.
fn release(listed: Listed) -> Option<Release> {
    let tag = listed
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&listed.tag_name);
    let version = Version::parse(tag).ok()?;
    let url = Url::parse(&listed.html_url)
        .ok()
        .filter(|url| url.scheme() == "https")?;
    let published_at = listed
        .published_at
        .and_then(|at| OffsetDateTime::parse(&at, &Rfc3339).ok());
    Some(Release {
        version,
        prerelease: listed.prerelease,
        draft: listed.draft,
        url,
        published_at,
    })
}
