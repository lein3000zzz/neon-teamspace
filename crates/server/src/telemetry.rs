//! Installing `neon-observability` for the commands that run against the database.
//!
//! Commands that print their output, such as `neon openapi`, install nothing: logs go to
//! stdout, which is theirs.

use neon_app::{auth, jobs, workspaces};
use neon_observability::red::Buckets;
use neon_observability::{InitError, ServiceInfo, Telemetry, http};

/// The RED subsystems with the histogram buckets that fit what they time: requests and
/// the auth and workspace use cases, a password hash included, take milliseconds to
/// seconds, jobs up to minutes.
const SUBSYSTEMS: [(&str, Buckets); 4] = [
    (http::SUBSYSTEM, Buckets::Medium),
    (auth::SUBSYSTEM, Buckets::Medium),
    (workspaces::SUBSYSTEM, Buckets::Medium),
    (jobs::SUBSYSTEM, Buckets::Long),
];

/// Installs logs, metrics, traces and profiling for the process, and logs the warnings of
/// the configuration's `observability` section.
pub(crate) fn init(config: &neon_observability::Config) -> Result<Telemetry, InitError> {
    let service = ServiceInfo::new("neon", env!("CARGO_PKG_VERSION"));
    neon_observability::init(config, service, &SUBSYSTEMS)
}
