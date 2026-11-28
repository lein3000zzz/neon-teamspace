//! The ports the updates reach out through: the list of releases on GitHub, and the updater
//! beside the stack.

use async_trait::async_trait;
use neon_domain::settings::Secret;
use neon_domain::updates::{Release, UpdaterStatus};
use semver::Version;
use url::Url;

/// Lists the releases of Neon Teamspace.
#[async_trait]
pub trait Releases: Send + Sync + 'static {
    /// The latest releases, as the source lists them, drafts and pre-releases among them. A
    /// release whose tag is no version, or whose page is no https URL, is left out.
    async fn list(&self) -> Result<Vec<Release>, ReleasesError>;
}

/// Why the releases could not be listed, said for admins.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReleasesError(pub String);

/// Where the updater is, and the token it takes. `Debug` hides the token.
#[derive(Debug, Clone)]
pub struct UpdaterEndpoint {
    pub url: Url,
    pub token: Secret,
}

/// The updater beside the stack, which pulls a release's images and restarts the services
/// with them.
#[async_trait]
pub trait Updater: Send + Sync + 'static {
    /// Its latest update.
    async fn status(&self, at: &UpdaterEndpoint) -> Result<UpdaterStatus, UpdaterError>;

    /// Asks it to update the stack to `version`; answers where it is with that.
    async fn update(
        &self,
        at: &UpdaterEndpoint,
        version: &Version,
    ) -> Result<UpdaterStatus, UpdaterError>;
}

/// Why the updater did not do what it was asked. The messages are said for admins and
/// never hold the token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdaterError {
    /// It did not answer, or not as its protocol says.
    #[error("{0}")]
    Unreachable(String),
    /// It refused the token.
    #[error(
        "the updater refused the server's token: updates.updater_token must be the updater's \
         NEON_UPDATER_TOKEN"
    )]
    Refused,
    /// An update is under way already.
    #[error("an update is under way already")]
    Busy,
    /// It refused the version, saying why.
    #[error("{0}")]
    Rejected(String),
}
