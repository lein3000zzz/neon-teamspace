//! Installing `neon-observability` for the commands that run against the database.
//!
//! Commands that print their output, such as `neon openapi`, install nothing: logs go to
//! stdout, which is theirs.

use neon_app::{auth, jobs, pages, sync, transfer, webhooks, workspaces};
use neon_observability::red::Buckets;
use neon_observability::{InitError, ServiceInfo, Telemetry, http};

/// The RED subsystems with the histogram buckets that fit what they time: requests, the
/// auth, workspace and page use cases, a password hash included, loading and saving
/// documents and MCP tools take milliseconds to seconds, webhooks' deliveries and test
/// events up to their time limit of seconds, imports and exports up to theirs, jobs up to
/// minutes.
const SUBSYSTEMS: [(&str, Buckets); 9] = [
    (http::SUBSYSTEM, Buckets::Medium),
    (auth::SUBSYSTEM, Buckets::Medium),
    (workspaces::SUBSYSTEM, Buckets::Medium),
    (pages::SUBSYSTEM, Buckets::Medium),
    (sync::SUBSYSTEM, Buckets::Medium),
    (webhooks::SUBSYSTEM, Buckets::MediumLong),
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
/// content with them, so it logs at `info` at most, unless the level names its target. A
/// level of `info` or less verbose holds for it as it is.
pub(crate) fn quiet_mcp_sdk(level: &str) -> String {
    let verbose = level.split(',').map(str::trim).any(|directive| {
        ["debug", "trace", "4", "5"]
            .iter()
            .any(|name| directive.eq_ignore_ascii_case(name))
    });
    if verbose && !level.contains("rmcp") {
        format!("{level},rmcp=info")
    } else {
        level.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::quiet_mcp_sdk;

    #[test]
    fn the_mcp_sdk_logs_no_requests_unless_asked() {
        assert_eq!(quiet_mcp_sdk("debug"), "debug,rmcp=info");
        assert_eq!(
            quiet_mcp_sdk("TRACE,sqlx=warn"),
            "TRACE,sqlx=warn,rmcp=info"
        );
        assert_eq!(quiet_mcp_sdk("info,rmcp=debug"), "info,rmcp=debug");
        assert_eq!(quiet_mcp_sdk("debug,rmcp=debug"), "debug,rmcp=debug");
        // Quieter than info, it stays as quiet for the SDK too.
        for level in ["info", "warn", "error,neon_server=debug", "off"] {
            assert_eq!(quiet_mcp_sdk(level), level);
        }
    }
}
