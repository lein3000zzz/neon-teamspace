//! An update, run in a container of its own that sees the stack's Compose files and env files
//! at the paths the stack was started with: it pulls the images of the services and of the
//! updater's own service at the version, and checks the signature of every one, before it
//! changes anything. Then it keeps the version in the last env file and recreates the
//! services, nothing else; last, the updater's own service, as it can.

use std::fs::OpenOptions;
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;

use anyhow::{Context as _, bail};

use crate::output::Output;
use crate::runner::{Program, Runner};
use crate::verify::verify_images;
use crate::version::Version;

/// The variable the Compose file names its images' version by.
pub const VERSION_VAR: &str = "NEON_VERSION";

/// An update of a Compose stack to a release.
#[derive(Debug)]
pub struct Apply<R> {
    pub runner: R,
    /// The release to update to.
    pub version: Version,
    /// The stack's Compose project.
    pub project: String,
    /// Its project directory, which relative paths in its files start from.
    pub project_dir: String,
    /// The Compose files it was started with, in order.
    pub files: Vec<String>,
    /// The env files it was started with, in order; the version is kept in the last.
    pub env_files: Vec<String>,
    /// The services to update.
    pub services: Vec<String>,
    /// The updater's own service: its image is pulled and checked with theirs, and it is
    /// recreated after them, as it can.
    pub own: Option<String>,
    /// Whether each image's signature is checked before anything is recreated from it.
    pub verify: bool,
}

impl<R: Runner> Apply<R> {
    /// Runs the update, telling `out` what it does. Nothing changes unless every image, the
    /// updater's own among them, pulled and, with [`Apply::verify`], checked out; the
    /// updater's own service failing to start leaves the others updated, and the update done.
    ///
    /// # Errors
    ///
    /// When an image, the updater's own among them, does not pull or does not check out, the
    /// env file cannot be written, or the services do not start.
    pub async fn run(&self, out: &dyn Output) -> anyhow::Result<()> {
        if self.project.is_empty()
            || self.project_dir.is_empty()
            || self.files.is_empty()
            || self.env_files.is_empty()
            || self.services.is_empty()
        {
            bail!("an update needs its project, project directory, files, env files and services");
        }

        let compose = self.compose();
        // Over what the env files say, until the last of them says it too.
        let env = [(VERSION_VAR.to_owned(), self.version.to_string())];
        let services = self.services.join(", ");
        // The updater's own image too, so that an image that does not check out never gets as
        // far as the env file, where a later docker compose up would start it.
        let pulled: Vec<String> = self.services.iter().chain(&self.own).cloned().collect();

        out.line(&format!(
            "Pulling {} at {}",
            pulled.join(", "),
            self.version
        ));
        self.runner
            .run(
                Program::Docker,
                &concat(&compose, &["pull", "--quiet"], &pulled),
                &env,
                out,
            )
            .await
            .context("pulling the images")?;

        if self.verify {
            verify_images(&self.runner, &compose, &env, &pulled, &self.version, out)
                .await
                .context("checking the images' signatures; nothing was changed")?;
        } else {
            out.line("Not checking the images' signatures: NEON_UPDATER_VERIFY is false");
        }

        // The last env file wins, so a docker compose up after this stays on the version.
        let Some(last) = self.env_files.last() else {
            bail!("an update needs an env file");
        };
        write_version(Path::new(last), &self.version)
            .with_context(|| format!("keeping the version in {last}"))?;
        out.line(&format!(
            "{VERSION_VAR}={} kept in {last}; starting {services}",
            self.version
        ));

        // Never pulled again: what starts is what was checked.
        self.runner
            .run(
                Program::Docker,
                &concat(
                    &compose,
                    &["up", "--detach", "--no-deps", "--pull", "never"],
                    &self.services,
                ),
                &env,
                out,
            )
            .await
            .context("starting the new containers")?;
        out.line(&format!("Updated to {}", self.version));

        if let Some(own) = &self.own {
            self.update_own(own, &compose, &env, out).await;
        }
        Ok(())
    }

    /// Recreates the updater's own service from the image pulled and checked with the others,
    /// last: this update outlives it. A failure leaves the updater as it is.
    async fn update_own(
        &self,
        own: &str,
        compose: &[String],
        env: &[(String, String)],
        out: &dyn Output,
    ) {
        let own = [own.to_owned()];
        if let Err(error) = self
            .runner
            .run(
                Program::Docker,
                &concat(
                    compose,
                    &["up", "--detach", "--no-deps", "--pull", "never"],
                    &own,
                ),
                env,
                out,
            )
            .await
        {
            out.line(&format!("The updater stays as it is: {error:#}"));
            return;
        }
        out.line(&format!("The updater is at {} too", self.version));
    }

    /// `docker compose` and the arguments that name the stack as it was started.
    fn compose(&self) -> Vec<String> {
        let mut args = vec![
            "compose".to_owned(),
            "--project-name".to_owned(),
            self.project.clone(),
            "--project-directory".to_owned(),
            self.project_dir.clone(),
        ];
        for file in &self.files {
            args.extend(["--file".to_owned(), file.clone()]);
        }
        for file in &self.env_files {
            args.extend(["--env-file".to_owned(), file.clone()]);
        }
        args
    }
}

fn concat(first: &[String], middle: &[&str], last: &[String]) -> Vec<String> {
    first
        .iter()
        .cloned()
        .chain(middle.iter().map(|&arg| arg.to_owned()))
        .chain(last.iter().cloned())
        .collect()
}

/// Sets the version in the env file at `path`, in place: a file Docker mounts on its own
/// cannot be replaced. The new text is written over the old before the file is cut to its
/// length, so a write that stops halfway leaves no file shorter than both.
fn write_version(path: &Path, version: &Version) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let new = set_version(&text, version);
    file.seek(SeekFrom::Start(0))?;
    file.write_all(new.as_bytes())?;
    file.set_len(new.len() as u64)?;
    file.sync_all()
}

/// `env`, the text of an env file, with [`VERSION_VAR`] set to `version`: each line that sets
/// it changed, an `export ` before it kept, or a line added at the end. Comments, the other
/// lines and their line endings stay as they are.
#[must_use]
pub fn set_version(env: &str, version: &Version) -> String {
    let mut set = false;
    let mut out: String = env
        .split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\r', '\n']);
            let ending = &line[body.len()..];
            let trimmed = body.trim();
            let (export, rest) = match trimmed.strip_prefix("export ") {
                Some(rest) => ("export ", rest.trim_start()),
                None => ("", trimmed),
            };
            match rest.split_once('=') {
                Some((name, _)) if name.trim() == VERSION_VAR => {
                    set = true;
                    format!("{export}{VERSION_VAR}='{version}'{ending}")
                }
                _ => line.to_owned(),
            }
        })
        .collect();
    if set {
        return out;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "\n# The release Neon Teamspace's updater last updated to.\n{VERSION_VAR}='{version}'\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Tail;
    use crate::runner::fake::Fake;

    const DIGEST: &str = "sha256:44c87791af0820a6d609825cb3157553d312a3a25f9b4165e5f06e90c2033bd9";

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn set_version_changes_each_line_that_sets_it() {
        let v = version("0.2.1");
        for (name, before, after) in [
            (
                "set",
                "A=1\nNEON_VERSION='0.1.0'\nB=2\n",
                "A=1\nNEON_VERSION='0.2.1'\nB=2\n",
            ),
            ("blank", "NEON_VERSION=\n", "NEON_VERSION='0.2.1'\n"),
            (
                "spaced, unquoted, CRLF",
                "A=1\r\n  NEON_VERSION = 0.1 \r\nB=2\r\n",
                "A=1\r\nNEON_VERSION='0.2.1'\r\nB=2\r\n",
            ),
            (
                "double-quoted",
                "NEON_VERSION=\"latest\"\n",
                "NEON_VERSION='0.2.1'\n",
            ),
            (
                "exported, no newline at the end",
                "export NEON_VERSION=latest",
                "export NEON_VERSION='0.2.1'",
            ),
            (
                "exported with spaces",
                "export   NEON_VERSION=0.1.0\n",
                "export NEON_VERSION='0.2.1'\n",
            ),
            (
                "each time it is set",
                "NEON_VERSION=1\n# between\nNEON_VERSION=2\n",
                "NEON_VERSION='0.2.1'\n# between\nNEON_VERSION='0.2.1'\n",
            ),
        ] {
            assert_eq!(set_version(before, &v), after, "{name}");
        }
    }

    #[test]
    fn set_version_keeps_comments_and_other_variables() {
        let v = version("0.2.1");
        let comment = "# NEON_VERSION='0.1.0'\n#NEON_VERSION=latest\n";
        let added =
            "\n# The release Neon Teamspace's updater last updated to.\nNEON_VERSION='0.2.1'\n";
        for (name, before) in [
            ("commented out", comment),
            ("another variable", "NEON_VERSIONS=1\nMY_NEON_VERSION=2\n"),
            ("no newline at the end", "POSTGRES_PASSWORD=x"),
            ("empty", ""),
        ] {
            let mut expected = before.to_owned();
            if !expected.is_empty() && !expected.ends_with('\n') {
                expected.push('\n');
            }
            expected.push_str(added);
            assert_eq!(set_version(before, &v), expected, "{name}");
        }
    }

    #[test]
    fn the_version_is_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let long = format!(
            "# {}\nNEON_VERSION=0.1.0-a-long-pre-release-name\n",
            "x".repeat(40)
        );
        std::fs::write(&path, &long).unwrap();

        write_version(&path, &version("0.2.1")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("# {}\nNEON_VERSION='0.2.1'\n", "x".repeat(40)),
            "cut to the new, shorter length"
        );
    }

    fn apply(fake: Fake, env_file: &str) -> Apply<Fake> {
        Apply {
            runner: fake,
            version: version("0.2.1"),
            project: "neon".to_owned(),
            project_dir: "/srv/neon/deployments".to_owned(),
            files: vec!["/srv/neon/deployments/docker-compose.yml".to_owned()],
            env_files: vec!["/srv/neon/base.env".to_owned(), env_file.to_owned()],
            services: vec!["server".to_owned(), "web".to_owned()],
            own: None,
            verify: true,
        }
    }

    const COMPOSE: &str = "docker compose --project-name neon --project-directory /srv/neon/deployments \
                           --file /srv/neon/deployments/docker-compose.yml --env-file /srv/neon/base.env --env-file";

    /// A stack whose images all check out.
    fn signed(env_file: &str, services: &[&str]) -> Fake {
        let mut fake = Fake::default();
        let config: Vec<String> = services
            .iter()
            .map(|s| {
                format!(r#""{s}": {{"image": "ghcr.io/lein3000zzz/neon-teamspace-{s}:0.2.1"}}"#)
            })
            .collect();
        let config = format!(r#"{{"services": {{{}}}}}"#, config.join(", "));
        fake.answer(
            &format!(
                "{COMPOSE} {env_file} config --format json {}",
                services.join(" ")
            ),
            &config,
        );
        for s in services {
            fake.answer(
                &format!(
                    "docker image inspect --format {{{{json .RepoDigests}}}} ghcr.io/lein3000zzz/neon-teamspace-{s}:0.2.1"
                ),
                &format!(r#"["ghcr.io/lein3000zzz/neon-teamspace-{s}@{DIGEST}"]"#),
            );
        }
        fake
    }

    fn env_file(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "POSTGRES_PASSWORD='x'\n# NEON_VERSION=latest\nNEON_VERSION=\n",
        )
        .unwrap();
        path.display().to_string()
    }

    #[tokio::test]
    async fn pulls_verifies_keeps_the_version_then_starts() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let update = apply(signed(&env_file, &["server", "web"]), &env_file);
        let tail = Tail::new(4096);

        update.run(&tail).await.unwrap();

        let lines = update.runner.lines();
        let compose = format!("{COMPOSE} {env_file}");
        assert_eq!(lines[0], format!("{compose} pull --quiet server web"));
        assert!(
            lines[1..lines.len() - 1]
                .iter()
                .any(|line| line.starts_with("cosign verify"))
        );
        assert_eq!(
            lines.last().unwrap(),
            &format!("{compose} up --detach --no-deps --pull never server web"),
            "nothing but the services, from the images checked"
        );
        assert!(
            update
                .runner
                .ran()
                .iter()
                .all(|ran| ran.program == Program::Cosign
                    || ran.args.first().is_some_and(|arg| arg == "image")
                    || ran.env == [("NEON_VERSION".to_owned(), "0.2.1".to_owned())]),
            "compose always sees the new version"
        );
        assert_eq!(
            std::fs::read_to_string(&env_file).unwrap(),
            "POSTGRES_PASSWORD='x'\n# NEON_VERSION=latest\nNEON_VERSION='0.2.1'\n",
            "the last env file holds the version"
        );
        let text = tail.text();
        assert!(text.starts_with("Pulling server, web at 0.2.1\n"), "{text}");
        assert!(text.ends_with("Updated to 0.2.1\n"), "{text}");
    }

    #[tokio::test]
    async fn verification_that_fails_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let before = std::fs::read_to_string(&env_file).unwrap();
        let mut fake = signed(&env_file, &["server", "web"]);
        fake.fail("cosign verify", "cosign verify: exit status 1");
        let update = apply(fake, &env_file);

        let error = update.run(&Tail::new(4096)).await.unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "checking the images' signatures; nothing was changed: \
             ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 is not signed by the release workflow of v0.2.1: \
             cosign verify: exit status 1"
        );
        assert!(
            !update
                .runner
                .lines()
                .iter()
                .any(|line| line.contains(" up ")),
            "nothing recreated"
        );
        assert_eq!(std::fs::read_to_string(&env_file).unwrap(), before);
    }

    #[tokio::test]
    async fn without_verification_nothing_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let mut update = apply(Fake::default(), &env_file);
        update.verify = false;
        let tail = Tail::new(4096);

        update.run(&tail).await.unwrap();
        assert_eq!(update.runner.lines().len(), 2, "a pull and an up");
        assert!(tail.text().contains("Not checking the images' signatures"));
    }

    #[tokio::test]
    async fn a_failed_pull_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let before = std::fs::read_to_string(&env_file).unwrap();
        let mut fake = Fake::default();
        fake.fail("docker compose", "docker compose: exit status 1");
        let update = apply(fake, &env_file);

        let error = update.run(&Tail::new(64)).await.unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "pulling the images: docker compose: exit status 1"
        );
        assert_eq!(update.runner.lines().len(), 1, "not started");
        assert_eq!(std::fs::read_to_string(&env_file).unwrap(), before);
    }

    /// An update of the stack and the updater's own service, from `fake`.
    fn apply_with_own(fake: Fake, env_file: &str) -> Apply<Fake> {
        let mut update = apply(fake, env_file);
        update.own = Some("updater".to_owned());
        update
    }

    #[tokio::test]
    async fn the_updater_is_pulled_and_checked_with_the_others_and_started_last() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let compose = format!("{COMPOSE} {env_file}");
        let update = apply_with_own(signed(&env_file, &["server", "web", "updater"]), &env_file);
        let tail = Tail::new(4096);

        update.run(&tail).await.unwrap();

        let lines = update.runner.lines();
        assert_eq!(
            lines[0],
            format!("{compose} pull --quiet server web updater")
        );
        let checked = lines
            .iter()
            .position(|line| {
                line.starts_with("cosign verify ghcr.io/lein3000zzz/neon-teamspace-updater@")
            })
            .expect("the updater's image checked");
        let started = lines
            .iter()
            .position(|line| line.contains(" up "))
            .expect("the services started");
        assert!(
            checked < started,
            "checked before anything starts: {lines:?}"
        );
        assert_eq!(
            lines[started..],
            [
                format!("{compose} up --detach --no-deps --pull never server web"),
                format!("{compose} up --detach --no-deps --pull never updater"),
            ]
        );
        let text = tail.text();
        assert!(
            text.starts_with("Pulling server, web, updater at 0.2.1\n"),
            "{text}"
        );
        assert!(text.ends_with("The updater is at 0.2.1 too\n"), "{text}");
    }

    #[tokio::test]
    async fn an_updater_image_that_does_not_check_out_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let before = std::fs::read(&env_file).unwrap();
        let mut fake = signed(&env_file, &["server", "web", "updater"]);
        fake.fail(
            "cosign verify ghcr.io/lein3000zzz/neon-teamspace-updater",
            "exit status 1",
        );
        let update = apply_with_own(fake, &env_file);

        let error = update.run(&Tail::new(4096)).await.unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "checking the images' signatures; nothing was changed: \
             ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.1 is not signed by the release workflow of v0.2.1: \
             exit status 1"
        );
        assert!(
            !update
                .runner
                .lines()
                .iter()
                .any(|line| line.contains(" up ")),
            "nothing recreated"
        );
        assert_eq!(
            std::fs::read(&env_file).unwrap(),
            before,
            "the env file names no image that did not check out"
        );
    }

    #[tokio::test]
    async fn an_updater_image_that_does_not_pull_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let before = std::fs::read(&env_file).unwrap();
        let mut fake = signed(&env_file, &["server", "web", "updater"]);
        fake.fail(
            &format!("{COMPOSE} {env_file} pull --quiet server web updater"),
            "exit status 1",
        );
        let update = apply_with_own(fake, &env_file);

        let error = update.run(&Tail::new(4096)).await.unwrap_err();
        assert_eq!(format!("{error:#}"), "pulling the images: exit status 1");
        assert_eq!(update.runner.lines().len(), 1, "nothing after the pull");
        assert_eq!(std::fs::read(&env_file).unwrap(), before);
    }

    #[tokio::test]
    async fn the_updater_failing_to_start_leaves_the_update_done() {
        let dir = tempfile::tempdir().unwrap();
        let env_file = env_file(&dir);
        let mut fake = signed(&env_file, &["server", "web", "updater"]);
        fake.fail(
            &format!("{COMPOSE} {env_file} up --detach --no-deps --pull never updater"),
            "exit status 1",
        );
        let update = apply_with_own(fake, &env_file);
        let tail = Tail::new(4096);

        update.run(&tail).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&env_file).unwrap(),
            "POSTGRES_PASSWORD='x'\n# NEON_VERSION=latest\nNEON_VERSION='0.2.1'\n",
            "the server and web updated"
        );
        assert!(
            tail.text()
                .ends_with("The updater stays as it is: exit status 1\n"),
            "{}",
            tail.text()
        );
    }

    #[tokio::test]
    async fn an_update_without_its_parts_runs_nothing() {
        let mut update = apply(Fake::default(), "/srv/neon/.env");
        update.services.clear();
        let error = update.run(&Tail::new(64)).await.unwrap_err();
        assert!(error.to_string().starts_with("an update needs"), "{error}");
        assert!(update.runner.ran().is_empty());
    }
}
