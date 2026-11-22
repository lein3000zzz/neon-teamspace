//! The `neon` command line.

use std::io::{self, Write as _};
use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use neon_app::settings::registry;

use crate::health::{self, Probe};
use crate::{config, http, runtime};

#[derive(Debug, Parser)]
#[command(
    name = "neon",
    version,
    about = "Neon Workspace, a self-hosted team workspace."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server. This is the default when no command is given.
    Serve,
    /// Apply pending database migrations, then exit.
    Migrate,
    /// Print the OpenAPI description of the HTTP API.
    Openapi {
        /// Write it to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Runtime settings.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// The configuration from the config file and the environment.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Ask the admin listener whether the server is ready, for container health checks.
    /// Prints nothing and exits 0 when it is; else prints why and exits 1.
    Health {
        /// Ask /livez, whether the process is up, instead of /readyz, whether it takes
        /// traffic.
        #[arg(long)]
        live: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    /// Print the settings reference as a markdown page.
    Docs,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Load and check the configuration, then print it with secrets redacted.
    Check,
}

/// Runs a command. `serve` and `migrate` load the configuration, then install telemetry,
/// which logs to stdout. `main` prints a returned error to stderr, which is the only
/// place an invalid configuration is reported, as telemetry is not up yet. The other
/// commands print their output to stdout and install no telemetry, so nothing else is
/// written there. `health` prints nothing unless it fails, and then only to stderr.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => runtime::serve(config::load(&registry())?).await,
        Command::Migrate => runtime::migrate(config::load(&registry())?).await,
        Command::Openapi { out: Some(path) } => std::fs::write(&path, http::spec_json())
            .with_context(|| format!("writing {}", path.display())),
        Command::Openapi { out: None } => print(&http::spec_json()),
        Command::Settings(SettingsCommand::Docs) => print(&registry().reference_markdown()),
        Command::Config(ConfigCommand::Check) => {
            let registry = registry();
            let loaded = config::load(&registry)?;
            print(&loaded.render(&registry)?)
        }
        Command::Health { live } => {
            let loaded = config::load(&registry())?;
            let probe = if live { Probe::Live } else { Probe::Ready };
            health::check(&loaded.config.observability.admin, probe).await
        }
    }
}

/// Writes command output. Unlike `print!`, a closed pipe, as in `neon openapi | head`,
/// is an error to report rather than a panic.
fn print(text: &str) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn serve_is_the_default() {
        let cli = Cli::try_parse_from(["neon"]).expect("parses");

        assert!(cli.command.is_none());
    }

    #[test]
    fn health_asks_for_readiness_unless_told_live() {
        let ready = Cli::try_parse_from(["neon", "health"]).expect("parses");
        let live = Cli::try_parse_from(["neon", "health", "--live"]).expect("parses");

        assert!(matches!(
            ready.command,
            Some(Command::Health { live: false })
        ));
        assert!(matches!(live.command, Some(Command::Health { live: true })));
    }

    #[test]
    fn openapi_takes_an_output_path() {
        let cli =
            Cli::try_parse_from(["neon", "openapi", "--out", "api/openapi.json"]).expect("parses");

        assert!(matches!(
            cli.command,
            Some(Command::Openapi { out: Some(path) }) if path.to_str() == Some("api/openapi.json")
        ));
    }
}
