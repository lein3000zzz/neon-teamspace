use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// How long a query waits for a free connection before it fails. Short, so an exhausted
/// pool turns into errors quickly instead of piling up requests.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the pool needs from the configuration.
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig<'a> {
    pub url: &'a str,
    pub max_connections: u32,
}

/// Opens a pool and checks the database is reachable by connecting once.
pub async fn connect(config: PoolConfig<'_>) -> Result<PgPool, sqlx::Error> {
    let options: PgConnectOptions = config.url.parse()?;

    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(options)
        .await
}
