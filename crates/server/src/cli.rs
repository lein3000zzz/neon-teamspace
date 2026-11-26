//! The `neon` command line.

use std::io::{self, BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use neon_app::auth::NewAccount;
use neon_app::settings::registry;
use neon_domain::auth::Role;
use neon_domain::settings::Secret;

use crate::health::{self, Probe};
use crate::{config, http, runtime};

#[derive(Debug, Parser)]
#[command(
    name = "neon",
    version,
    about = "Neon Teamspace, a self-hosted team workspace."
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
    /// Accounts.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Print the OpenAPI description of the HTTP API.
    Openapi {
        /// Write it to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Runtime settings.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// Search.
    #[command(subcommand)]
    Search(SearchCommand),
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
enum AdminCommand {
    /// Create an admin account, whatever the registration mode and however many admins
    /// there are. The password is asked for at the terminal, or read from the first line
    /// of stdin when that is not a terminal.
    Create {
        /// The admin's address.
        #[arg(long)]
        email: String,
        /// The local part of the address when not given.
        #[arg(long)]
        display_name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsCommand {
    /// Print the settings reference as a markdown page.
    Docs,
}

#[derive(Debug, Subcommand)]
enum SearchCommand {
    /// Read pages' documents again and store their text for search: those whose text was
    /// never stored, such as one whose server stopped before saving it, or with --all every
    /// page. Applies pending migrations first, as serve does. Safe while servers run.
    Reindex {
        /// Every page with a document, not only those whose text is missing.
        #[arg(long)]
        all: bool,
    },
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
        Command::Admin(AdminCommand::Create {
            email,
            display_name,
        }) => {
            let loaded = config::load(&registry())?;
            let password = read_password()?;
            let account = NewAccount {
                email,
                password,
                display_name,
                role: Role::Admin,
            };
            let user = runtime::create_admin(loaded, account).await?;
            print(&format!(
                "created the admin {} ({})\n",
                user.address(),
                user.id
            ))
        }
        Command::Openapi { out: Some(path) } => std::fs::write(&path, http::spec_json())
            .with_context(|| format!("writing {}", path.display())),
        Command::Openapi { out: None } => print(&http::spec_json()),
        Command::Settings(SettingsCommand::Docs) => print(&registry().reference_markdown()),
        Command::Search(SearchCommand::Reindex { all }) => {
            let stored = runtime::reindex(config::load(&registry())?, all).await?;
            print(&format!("stored the search text of {stored} pages\n"))
        }
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

/// A password typed twice at the terminal without echo, or else the first line of stdin.
fn read_password() -> anyhow::Result<Secret> {
    if io::stdin().is_terminal() {
        let first = rpassword::prompt_password("Password: ").context("reading the password")?;
        let again =
            rpassword::prompt_password("The password again: ").context("reading the password")?;
        if first != again {
            bail!("the two passwords differ");
        }
        return Ok(Secret::new(first));
    }
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the password from stdin")?;
    Ok(Secret::new(line.trim_end_matches(['\r', '\n'])))
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
    fn reindexing_stores_what_is_missing_unless_told_all() {
        let missing = Cli::try_parse_from(["neon", "search", "reindex"]).expect("parses");
        let all = Cli::try_parse_from(["neon", "search", "reindex", "--all"]).expect("parses");

        assert!(matches!(
            missing.command,
            Some(Command::Search(SearchCommand::Reindex { all: false }))
        ));
        assert!(matches!(
            all.command,
            Some(Command::Search(SearchCommand::Reindex { all: true }))
        ));
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
    fn admin_create_takes_an_address_and_a_name() {
        let cli = Cli::try_parse_from([
            "neon",
            "admin",
            "create",
            "--email",
            "root@example.com",
            "--display-name",
            "Root",
        ])
        .expect("parses");

        assert!(matches!(
            cli.command,
            Some(Command::Admin(AdminCommand::Create { email, display_name: Some(name) }))
                if email == "root@example.com" && name == "Root"
        ));
        assert!(Cli::try_parse_from(["neon", "admin", "create"]).is_err());
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
