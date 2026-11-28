//! Postgres repositories.
//!
//! Queries are written by hand and checked against the schema at build time. Every
//! function takes an executor: `&PgPool` for a single statement, or `&mut *tx` to run
//! inside the caller's transaction, so the caller decides what commits together.

pub mod agents;
pub mod api_tokens;
pub mod comments;
pub mod databases;
pub mod documents;
pub mod email_tokens;
pub mod favorites;
pub mod files;
pub mod incoming_webhooks;
pub mod invites;
pub mod jobs;
pub mod join_requests;
pub mod notifications;
pub mod oauth;
pub mod page_shares;
pub mod pages;
mod pool;
pub mod search;
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

/// Reads an enum from the text a column holds with its `parse`, such as `Role::parse`. A
/// text that names no value is a decode error that says what was read and the text, such
/// as `unknown workspace role "boss"`.
pub(crate) fn decode<T>(
    what: &str,
    text: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> sqlx::Result<T> {
    parse(text).ok_or_else(|| decode_error(format!("unknown {what} {text:?}")))
}

#[cfg(test)]
mod tests {
    use neon_domain::auth::Role;

    use super::*;

    #[test]
    fn decode_reads_a_known_text() {
        let role = decode("role", "admin", Role::parse).expect("a known role");

        assert_eq!(role, Role::Admin);
    }

    #[test]
    fn decode_names_what_it_could_not_read_and_the_text() {
        let error = decode("role", "boss", Role::parse).expect_err("an unknown role");

        let sqlx::Error::Decode(message) = error else {
            panic!("not a decode error: {error:?}");
        };
        assert_eq!(message.to_string(), r#"unknown role "boss""#);
    }
}
