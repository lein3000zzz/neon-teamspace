//! Log output until the observability setup takes it over.

use tracing_subscriber::EnvFilter;

/// Used when `RUST_LOG` is unset. Postgres notices, such as "relation already exists,
/// skipping" on every start, are only worth seeing when they warn.
const DEFAULT_FILTER: &str = "info,sqlx::postgres::notice=warn";

/// Writes readable log lines to stderr, filtered by `RUST_LOG`. Stderr keeps stdout free
/// for command output such as `neon openapi`.
pub(crate) fn init() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    // Fails only when a subscriber is already installed, which then keeps logging.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
