//! Running the docker CLI and cosign. Their arguments go to them as they are, never through a
//! shell, and tests run them through a fake [`Runner`].

use std::fmt;
use std::future::Future;
use std::process::{ExitStatus, Stdio};

use anyhow::{Context as _, anyhow};
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::Command;

use crate::output::Output;

#[cfg(test)]
pub(crate) mod fake;

/// A program the updater runs, from the `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Program {
    /// The docker CLI, with its Compose plugin.
    Docker,
    /// cosign, which checks the images' signatures.
    Cosign,
}

impl Program {
    /// Its command's name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Cosign => "cosign",
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Runs the updater's programs.
pub trait Runner: Send + Sync {
    /// Runs `program` with `args` and `env` added to its environment, and returns what it
    /// printed to its standard output, or fails with what it printed to its standard error.
    fn output(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
    ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;

    /// Runs `program` with `args` and `env` added to its environment, giving what it prints on
    /// either stream to `out` as it comes, and fails unless it exits with status 0.
    fn run(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
        out: &dyn Output,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// Runs the programs found on the `PATH`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cli;

impl Runner for Cli {
    async fn output(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
    ) -> anyhow::Result<Vec<u8>> {
        let output = Command::new(program.name())
            .args(args)
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("running {program}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!(
                "{}: {}: {}",
                described(program, args),
                exit(output.status),
                said.trim()
            ))
        }
    }

    async fn run(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
        out: &dyn Output,
    ) -> anyhow::Result<()> {
        let mut child = Command::new(program.name())
            .args(args)
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("running {program}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        tokio::join!(copy(stdout, out), copy(stderr, out));
        let status = child
            .wait()
            .await
            .with_context(|| format!("running {program}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("{}: {}", described(program, args), exit(status)))
        }
    }
}

/// Gives what `stream` holds to `out` until it ends.
async fn copy(stream: Option<impl AsyncRead + Unpin>, out: &dyn Output) {
    let Some(mut stream) = stream else { return };
    let mut buffer = vec![0; 8192];
    // A stream that fails ends like one that closed: the exit status says the rest.
    while let Ok(read @ 1..) = stream.read(&mut buffer).await {
        out.write(&buffer[..read]);
    }
}

/// The program and its first argument, its command, as errors name them: `docker compose`.
fn described(program: Program, args: &[String]) -> String {
    match args.first() {
        Some(command) => format!("{program} {command}"),
        None => program.to_string(),
    }
}

fn exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => "killed by a signal".to_owned(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;
    use crate::output::Tail;

    #[test]
    fn errors_name_the_command_and_its_status() {
        assert_eq!(
            described(Program::Docker, &["compose".to_owned(), "pull".to_owned()]),
            "docker compose"
        );
        assert_eq!(described(Program::Cosign, &[]), "cosign");
    }

    /// A `docker` of the test's own, found through the `PATH` given in the child's environment,
    /// so that the process's own stays as it is.
    #[tokio::test]
    async fn runs_the_program_with_its_arguments_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let docker = dir.path().join("docker");
        std::fs::write(
            &docker,
            r#"#!/bin/sh
for arg in "$@"; do printf '[%s]' "$arg"; done
printf '\n'
printf 'to stderr %s\n' "$NEON_VERSION" >&2
[ "$1" != fail ] || exit 3
"#,
        )
        .unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = [
            (
                "PATH".to_owned(),
                dir.path().display().to_string() + ":/usr/bin:/bin",
            ),
            ("NEON_VERSION".to_owned(), "0.2.1".to_owned()),
        ];

        let tail = Tail::new(1024);
        let args = [
            "compose".to_owned(),
            "$HOME".to_owned(),
            "a b; c".to_owned(),
        ];
        Cli.run(Program::Docker, &args, &env, &tail).await.unwrap();
        let text = tail.text();
        assert!(text.contains("[compose][$HOME][a b; c]\n"), "{text}");
        assert!(text.contains("to stderr 0.2.1\n"), "{text}");

        let error = Cli
            .run(Program::Docker, &["fail".to_owned()], &env, &tail)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "docker fail: exit status 3");

        let printed = Cli.output(Program::Docker, &args, &env).await.unwrap();
        assert_eq!(
            String::from_utf8(printed).unwrap(),
            "[compose][$HOME][a b; c]\n"
        );
        let error = Cli
            .output(Program::Docker, &["fail".to_owned()], &env)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "docker fail: exit status 3: to stderr 0.2.1"
        );
    }
}
