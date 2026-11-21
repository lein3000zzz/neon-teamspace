//! Postgres repositories.
//!
//! Queries are written by hand and checked against the schema at build time. Every
//! function takes an executor: `&PgPool` for a single statement, or `&mut *tx` to run
//! inside the caller's transaction, so the caller decides what commits together.

pub mod jobs;
mod pool;
pub mod settings;

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

pub use pool::{PoolConfig, connect};

/// The migrations in the repository's `migrations/` directory, embedded at build time.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Applies pending migrations. A lock keeps two servers starting together from racing.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}
