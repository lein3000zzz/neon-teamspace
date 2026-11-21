//! The `neon` binary.

use clap::Parser as _;
use neon_server::cli::{self, Cli};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run(Cli::parse()).await
}
