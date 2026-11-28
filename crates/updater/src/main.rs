//! The `neon-updater` binary.

use std::io::Write as _;
use std::process::ExitCode;

use clap::Parser as _;
use neon_updater::cli::{self, Cli};
use neon_updater::docker::ERROR_PREFIX;
use neon_updater::privileges;

/// Runs the command, and fails with its error on one line, the last it prints, which the
/// updater that started an update reads it from.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let mut stderr = std::io::stderr().lock();
            // Nothing can be done about a standard error that is gone.
            let _ = writeln!(stderr, "{ERROR_PREFIX}{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.serves() {
        privileges::shed_root()?;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(cli::run(cli))
}
