//! Installing `neon-observability` for the commands that run against the database.
//!
//! Commands that print their output, such as `neon openapi`, install nothing: logs go to
//! stdout, which is theirs.

use neon_app::{auth, jobs, pages, sync, transfer, workspaces};
use neon_observability::red::Buckets;
use neon_observability::{InitError, ServiceInfo, Telemetry, http};

/// The RED subsystems with the histogram buckets that fit what they time: requests, the
/// auth, workspace and page use cases, a password hash included, loading and saving
/// documents and MCP tools take milliseconds to seconds, imports and exports up to their
/// time limit, jobs up to minutes.
const SUBSYSTEMS: [(&str, Buckets); 8] = [
    (http::SUBSYSTEM, Buckets::Medium),
    (auth::SUBSYSTEM, Buckets::Medium),
    (workspaces::SUBSYSTEM, Buckets::Medium),
    (pages::SUBSYSTEM, Buckets::Medium),
    (sync::SUBSYSTEM, Buckets::Medium),
    (transfer::SUBSYSTEM, Buckets::XLong),
    (jobs::SUBSYSTEM, Buckets::Long),
    (crate::mcp::SUBSYSTEM, Buckets::Medium),
];

/// Installs logs, metrics, traces and profiling for the process, and logs the warnings of
/// the configuration's `observability` section.
pub(crate) fn init(config: &neon_observability::Config) -> Result<Telemetry, InitError> {
    let service = ServiceInfo::new("neon", env!("CARGO_PKG_VERSION"));
    let mut config = config.clone();
    config.logging.level = quiet_mcp_sdk(&config.logging.level);
    neon_observability::init(&config, service, &SUBSYSTEMS)
}

/// The MCP SDK logs each request and answer whole at `debug`, tool arguments and page
/// content with them, so it logs at `info` at most, unless the level names its target.
fn quiet_mcp_sdk(level: &str) -> String {
    if level.contains("rmcp") {
        level.to_owned()
    } else {
        format!("{level},rmcp=info")
    }
}

#[cfg(test)]
mod tests {
    use super::quiet_mcp_sdk;

    #[test]
    fn the_mcp_sdk_logs_no_requests_unless_asked() {
        assert_eq!(quiet_mcp_sdk("debug"), "debug,rmcp=info");
        assert_eq!(quiet_mcp_sdk("info,sqlx=warn"), "info,sqlx=warn,rmcp=info");
        assert_eq!(quiet_mcp_sdk("info,rmcp=debug"), "info,rmcp=debug");
    }
}
