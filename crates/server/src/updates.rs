//! The adapters of the release check and of one-click updates: GitHub's list of releases,
//! and the updater beside the stack, both over HTTP/1.1.

mod client;
mod github;
#[cfg(test)]
mod tests;
mod updater;

use neon_domain::updates::Running;
use semver::Version;

pub use github::{DEFAULT_API_URL, GitHubReleases, REPOSITORY};
pub use updater::{STATUS_PATH, UPDATE_PATH, UpdaterClient};

/// The build this binary is: the cargo workspace's version, and a development build when
/// it was compiled with debug assertions, as `cargo run` and `make run` compile it.
pub fn running() -> Running {
    Running {
        version: Version::parse(env!("CARGO_PKG_VERSION"))
            .expect("the workspace's version is a semantic version"),
        development: cfg!(debug_assertions),
    }
}
