//! The `neon` binary.

use clap::Parser as _;
use neon_server::cli::{self, Cli};

/// jemalloc, through the wrapper that lets heap profiles tell it serves the allocations.
#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOCATOR: neon_observability::profiling::Jemalloc = neon_observability::profiling::Jemalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run(Cli::parse()).await
}
