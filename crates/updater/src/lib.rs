//! Neon Teamspace's updater: a small server beside the Compose stack, holding the Docker
//! socket, that updates the stack to a release when the server asks with its token.
//!
//! - `neon-updater serve`, the default, serves the protocol of [`protocol`]: one update at a
//!   time, with its status and the end of its output held in memory. Each update runs as
//!   [`docker::Docker::update`] has it, in a container of its own that outlives the updater.
//! - `neon-updater apply` is that update, [`apply::Apply`]: it pulls the images of the
//!   services and of the updater's own service at the release, and checks their signatures
//!   as [`verify`] does, before it changes anything. Then it keeps the release in the stack's
//!   env file as `NEON_VERSION`, so a later `docker compose up` stays on it, and recreates the
//!   services, nothing else, and last the updater's own.
//! - `neon-updater version` prints its version.
//!
//! It runs the docker CLI and cosign through a [`runner::Runner`], with their arguments as
//! they are and never through a shell.

pub mod apply;
pub mod cli;
pub mod config;
pub mod docker;
pub mod output;
pub mod privileges;
pub mod protocol;
pub mod runner;
pub mod server;
pub mod verify;
pub mod version;

/// The updater's version, the cargo workspace's, which every release shares.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
