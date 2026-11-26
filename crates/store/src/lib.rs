//! Postgres repositories.
//!
//! Queries are written by hand and checked against the schema at build time. Every
//! function takes an executor: `&PgPool` for a single statement, or `&mut *tx` to run
//! inside the caller's transaction, so the caller decides what commits together.

pub mod agents;
pub mod api_tokens;
pub mod databases;
pub mod documents;
pub mod email_tokens;
pub mod favorites;
pub mod incoming_webhooks;
pub mod invites;
pub mod jobs;
pub mod join_requests;
pub mod oauth;
pub mod page_shares;
pub mod pages;
mod pool;
pub mod sessions;
pub mod settings;
pub mod teamspaces;
pub mod users;
pub mod versions;
pub mod webhooks;
pub mod workspace_invites;
pub mod workspaces;

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

pub use pool::{PoolConfig, connect};

/// The migrations in the repository's `migrations/` directory, embedded at build time.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Applies pending migrations. A lock keeps two servers starting together from racing.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

/// Whether `error` is a write that broke the unique constraint or index `name`.
pub fn is_unique_violation(error: &sqlx::Error, name: &str) -> bool {
    error.as_database_error().is_some_and(|db| {
        db.kind() == sqlx::error::ErrorKind::UniqueViolation && db.constraint() == Some(name)
    })
}

/// A value the database holds that the code cannot read, such as a role a check
/// constraint should have refused.
pub(crate) fn decode_error(message: String) -> sqlx::Error {
    sqlx::Error::Decode(message.into())
}
