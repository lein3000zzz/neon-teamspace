use std::fmt;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// How long a query waits for a free connection before it fails. Short, so an exhausted
/// pool turns into errors quickly instead of piling up requests.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the pool needs from the configuration. `Debug` hides the URL, which usually
/// holds a password.
#[derive(Clone, Copy)]
pub struct PoolConfig<'a> {
    pub url: &'a str,
    pub max_connections: u32,
}

impl fmt::Debug for PoolConfig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolConfig")
            .field("url", &"[redacted]")
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// Opens a pool and checks the database is reachable by connecting once.
pub async fn connect(config: PoolConfig<'_>) -> Result<PgPool, sqlx::Error> {
    let options: PgConnectOptions = config.url.parse()?;
    pool_options(config).connect_with(options).await
}

fn pool_options(config: PoolConfig<'_>) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://neon:hunter2@db.internal/neon";

    fn config() -> PoolConfig<'static> {
        PoolConfig {
            url: URL,
            max_connections: 4,
        }
    }

    #[test]
    fn the_config_hides_the_url() {
        let shown = format!("{:?}", config());

        assert_eq!(
            shown,
            r#"PoolConfig { url: "[redacted]", max_connections: 4 }"#
        );
    }

    #[tokio::test]
    async fn the_pool_does_not_show_its_password() {
        let options: PgConnectOptions = URL.parse().expect("a valid URL");
        let pool = pool_options(config()).connect_lazy_with(options);

        let shown = format!("{pool:?}");

        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("max_connections: 4"), "{shown}");
    }

    #[test]
    fn parse_errors_do_not_show_the_url() {
        let error = "postgres://neon:hunter2@db.internal:port/neon"
            .parse::<PgConnectOptions>()
            .expect_err("the port is not a number");

        assert!(!error.to_string().contains("hunter2"), "{error}");
        assert!(!format!("{error:?}").contains("hunter2"), "{error:?}");
    }
}
