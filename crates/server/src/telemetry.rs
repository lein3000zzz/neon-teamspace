//! Installing `neon-observability` for the commands that run against the database.
//!
//! Commands that print their output, such as `neon openapi`, install nothing: logs go to
//! stdout, which is theirs.

use neon_app::{auth, jobs, pages, sync, transfer, webhooks, workspaces};
use neon_observability::red::Buckets;
use neon_observability::{InitError, ServiceInfo, Telemetry, http};

/// The RED subsystems with the histogram buckets that fit what they time: requests, the
/// auth, workspace and page use cases, a password hash included, and loading and saving
/// documents take milliseconds to seconds, webhooks' deliveries and test events up to their
/// time limit of seconds, imports and exports up to theirs, jobs up to minutes.
const SUBSYSTEMS: [(&str, Buckets); 8] = [
    (http::SUBSYSTEM, Buckets::Medium),
    (auth::SUBSYSTEM, Buckets::Medium),
    (workspaces::SUBSYSTEM, Buckets::Medium),
    (pages::SUBSYSTEM, Buckets::Medium),
    (sync::SUBSYSTEM, Buckets::Medium),
    (webhooks::SUBSYSTEM, Buckets::MediumLong),
    (transfer::SUBSYSTEM, Buckets::XLong),
    (jobs::SUBSYSTEM, Buckets::Long),
];

/// Installs logs, metrics, traces and profiling for the process, and logs the warnings of
/// the configuration's `observability` section.
pub(crate) fn init(config: &neon_observability::Config) -> Result<Telemetry, InitError> {
    let service = ServiceInfo::new("neon", env!("CARGO_PKG_VERSION"));
    neon_observability::init(config, service, &SUBSYSTEMS)
}
