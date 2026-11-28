//! The `neon-updater` command line.

use std::io::Write as _;
use std::sync::Arc;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use tokio::net::TcpListener;

use crate::apply::Apply;
use crate::config::ServeConfig;
use crate::docker::{BINARY, Docker};
use crate::output::Stdout;
use crate::runner::Cli as Programs;
use crate::server::Server;
use crate::version::Version;

#[derive(Debug, Parser)]
#[command(
    name = "neon-updater",
    version,
    about = "Neon Teamspace's updater: it updates the Compose stack it runs beside to a release when the server asks."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the server's asks to update. This is the default when no command is given.
    ///
    /// Reads NEON_UPDATER_TOKEN, the token the server sends, at least 32 characters;
    /// NEON_UPDATER_ADDR, 0.0.0.0:8080 unless set; NEON_UPDATER_SERVICES, the Compose services
    /// to update, server,web unless set; NEON_UPDATER_PROJECT and NEON_UPDATER_IMAGE, the
    /// stack's Compose project and the image an update runs in, its own unless set; and
    /// NEON_UPDATER_VERIFY, whether an update checks the images' signatures, true unless set.
    Serve,
    /// Update the stack to a release. The updater runs this in a container of its own, which
    /// sees the stack's files at the paths it was started with.
    Apply(ApplyArgs),
    /// Print the updater's version.
    Version,
}

#[derive(Debug, Args)]
struct ApplyArgs {
    /// The release to update to, such as 0.2.1.
    #[arg(long)]
    version: Version,
    /// The stack's Compose project.
    #[arg(long)]
    project: String,
    /// Its project directory.
    #[arg(long = "project-directory", value_name = "DIR")]
    project_directory: String,
    /// A Compose file it was started with; once for each, in order.
    #[arg(long = "file", value_name = "FILE", required = true)]
    files: Vec<String>,
    /// An env file it was started with; once for each, in order. The version is kept in the
    /// last.
    #[arg(long = "env-file", value_name = "FILE", required = true)]
    env_files: Vec<String>,
    /// A service to update; once for each.
    #[arg(long = "service", value_name = "SERVICE", required = true)]
    services: Vec<String>,
    /// The updater's own service, updated after the others as it can.
    #[arg(long = "self", value_name = "SERVICE")]
    own: Option<String>,
    /// Recreate the services without checking the images' signatures.
    #[arg(long)]
    no_verify: bool,
}

impl Cli {
    /// Whether the command is `serve`, which sheds root before it starts.
    #[must_use]
    pub fn serves(&self) -> bool {
        matches!(self.command, None | Some(Command::Serve))
    }
}

/// Runs the command.
///
/// # Errors
///
/// When serving fails to start, or an update fails.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None | Some(Command::Serve) => serve(ServeConfig::from_env()?).await,
        Some(Command::Apply(args)) => apply(args).await,
        Some(Command::Version) => {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "neon-updater {}", crate::VERSION)?;
            Ok(())
        }
    }
}

async fn serve(config: ServeConfig) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stdout)
        .init();

    // Docker names a container's hostname after it, and sets HOSTNAME to it.
    let self_id = match std::env::var("HOSTNAME") {
        Ok(name) if !name.trim().is_empty() => name.trim().to_owned(),
        _ => std::fs::read_to_string("/etc/hostname")
            .context("finding the updater's own container: neither HOSTNAME nor /etc/hostname")?
            .trim()
            .to_owned(),
    };
    let docker = Docker {
        runner: Programs,
        self_id,
        services: config.services.clone(),
        project: config.project.clone(),
        image: config.image.clone(),
        binary: BINARY.to_owned(),
        verify: config.verify,
    };
    let server = Server::new(config.token, Arc::new(docker));
    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("listening on {}", config.addr))?;
    let addr = listener.local_addr()?;
    tracing::info!(
        %addr,
        version = crate::VERSION,
        services = %config.services.join(","),
        verify = config.verify,
        "listening"
    );
    if !config.verify {
        tracing::warn!(
            "NEON_UPDATER_VERIFY is false: updates recreate the services without checking the images' signatures"
        );
    }
    axum::serve(listener, server.router())
        .with_graceful_shutdown(stopped())
        .await
        .context("serving")?;
    tracing::info!("stopped");
    Ok(())
}

/// Resolves on SIGTERM, as `docker stop` sends, or on Ctrl+C.
async fn stopped() {
    let interrupt = async {
        // Without a handler there is nothing to wait for but the other signal.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

async fn apply(args: ApplyArgs) -> anyhow::Result<()> {
    let update = Apply {
        runner: Programs,
        version: args.version,
        project: args.project,
        project_dir: args.project_directory,
        files: args.files,
        env_files: args.env_files,
        services: args.services,
        own: args.own,
        verify: !args.no_verify,
    };
    update.run(&Stdout).await
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn serve_is_the_default() {
        assert!(Cli::parse_from(["neon-updater"]).serves());
        assert!(Cli::parse_from(["neon-updater", "serve"]).serves());
        assert!(!Cli::parse_from(["neon-updater", "version"]).serves());
    }

    #[test]
    fn apply_takes_each_file_and_service_in_order() {
        let cli = Cli::parse_from([
            "neon-updater",
            "apply",
            "--version",
            "0.2.1",
            "--project",
            "neon",
            "--project-directory",
            "/srv/neon/deployments",
            "--file",
            "/srv/neon/deployments/docker-compose.yml",
            "--file",
            "/srv/neon/extra.yml",
            "--env-file",
            "/srv/neon/deployments/.env",
            "--service",
            "server",
            "--service",
            "web",
            "--self",
            "updater",
            "--no-verify",
        ]);
        let Some(Command::Apply(args)) = cli.command else {
            panic!("not apply");
        };
        assert_eq!(args.version.as_str(), "0.2.1");
        assert_eq!(
            args.files,
            [
                "/srv/neon/deployments/docker-compose.yml",
                "/srv/neon/extra.yml"
            ]
        );
        assert_eq!(args.services, ["server", "web"]);
        assert_eq!(args.own.as_deref(), Some("updater"));
        assert!(args.no_verify);
    }

    #[test]
    fn apply_refuses_a_version_that_is_none() {
        let error = Cli::try_parse_from([
            "neon-updater",
            "apply",
            "--version",
            "0.2.1 --pull always",
            "--project",
            "neon",
            "--project-directory",
            "/srv",
            "--file",
            "/srv/a.yml",
            "--env-file",
            "/srv/.env",
            "--service",
            "server",
        ])
        .unwrap_err();
        assert!(error.to_string().contains("is not a version"), "{error}");
    }
}
