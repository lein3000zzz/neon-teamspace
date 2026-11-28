//! Starting an update of the Compose stack the updater runs in. The update runs in a
//! container of its own, of the updater's image, as [`crate::apply::Apply`], so it outlives
//! the updater: the stack is found from the labels Compose gave its containers, and its files
//! are mounted into the update's container at the paths it was started with, so that what
//! they name relative to themselves is found as it was.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;

use anyhow::{Context as _, anyhow, bail};
use serde::Deserialize;

use crate::output::{Output, Tail};
use crate::runner::{Program, Runner};
use crate::version::Version;

/// The labels Docker Compose gives each container it starts, which say how it started it,
/// with the host's paths.
const LABEL_PROJECT: &str = "com.docker.compose.project";
const LABEL_SERVICE: &str = "com.docker.compose.service";
const LABEL_CONFIG_FILES: &str = "com.docker.compose.project.config_files";
const LABEL_WORKING_DIR: &str = "com.docker.compose.project.working_dir";
const LABEL_ENV_FILE: &str = "com.docker.compose.project.environment_file";

/// The env file Compose reads from the project directory when given none.
const DEFAULT_ENV_FILE: &str = ".env";

/// Where the Docker socket is, in the updater's container and in the update's.
pub const SOCKET: &str = "/var/run/docker.sock";

/// Where the updater's binary is in its image, where the update runs it again.
pub const BINARY: &str = "/usr/local/bin/neon-updater";

/// The label of the update's container, which names the release it updates to.
const LABEL_UPDATE: &str = "neon.update";

/// What `neon-updater` puts before the error it fails with, alone on the last line it prints.
pub const ERROR_PREFIX: &str = "neon-updater: ";

/// What the docker CLI puts before the error it fails with when it cannot start a container.
const DOCKER_ERROR_PREFIX: &str = "docker: ";

/// How much of the end of the update's output is searched for the error it failed with.
const ERROR_SEARCH_BYTES: usize = 4096;

/// Updates the stack to a release, telling its output where it is.
pub trait Job: Send + Sync + 'static {
    /// Updates the stack to `version`, giving what it does to `out`.
    fn update<'a>(
        &'a self,
        version: &'a Version,
        out: &'a dyn Output,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

/// Updates the Compose stack the updater runs in.
#[derive(Debug)]
pub struct Docker<R> {
    pub runner: R,
    /// The updater's own container, which Docker names its hostname after.
    pub self_id: String,
    /// The stack's services to update, such as `server` and `web`.
    pub services: Vec<String>,
    /// The stack's Compose project; the updater's own when `None`.
    pub project: Option<String>,
    /// The image the update runs in; the updater's own when `None`.
    pub image: Option<String>,
    /// The updater's path in that image.
    pub binary: String,
    /// Whether the update checks the images' signatures.
    pub verify: bool,
}

/// What `docker inspect` says of a container.
#[derive(Debug, Deserialize)]
struct Container {
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Config")]
    config: ContainerConfig,
    #[serde(rename = "Mounts", default)]
    mounts: Option<Vec<Mount>>,
}

#[derive(Debug, Deserialize)]
struct ContainerConfig {
    #[serde(rename = "Labels", default)]
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct Mount {
    #[serde(rename = "Source", default)]
    source: String,
    #[serde(rename = "Destination", default)]
    destination: String,
}

impl Container {
    fn label(&self, name: &str) -> Option<&str> {
        self.config
            .labels
            .as_ref()?
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }
}

/// The update's container, as `docker run` starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    /// `docker`'s arguments.
    pub(crate) args: Vec<String>,
    /// The services it updates, the updater's own last when it is one of the stack's.
    pub(crate) services: Vec<String>,
}

impl<R: Runner> Docker<R> {
    async fn inspect(&self, id: &str) -> anyhow::Result<Container> {
        let args = ["inspect", "--type", "container", id].map(str::to_owned);
        let printed = self.runner.output(Program::Docker, &args, &[]).await?;
        let mut found: Vec<Container> = serde_json::from_slice(&printed)
            .with_context(|| format!("reading what docker inspect said of {id}"))?;
        match (found.pop(), found.is_empty()) {
            (Some(container), true) => Ok(container),
            _ => Err(anyhow!("docker inspect {id}: not one container")),
        }
    }

    /// Runs an update to `version` in a container of its own, until it ends.
    ///
    /// # Errors
    ///
    /// When the stack cannot be found or does not say how it was started, or the update
    /// fails.
    pub async fn update(&self, version: &Version, out: &dyn Output) -> anyhow::Result<()> {
        let plan = self.plan(version).await?;
        out.line(&format!(
            "Updating {} to {version}",
            plan.services.join(", ")
        ));
        let end = Tail::new(ERROR_SEARCH_BYTES);
        let both = Tee(out, &end);
        self.runner
            .run(Program::Docker, &plan.args, &[], &both)
            .await
            .map_err(|error| match failure(&end.text()) {
                Some(said) => anyhow!("{said}"),
                None => error,
            })
    }

    /// The `docker run` of the update to `version`: the stack's files and env files mounted at
    /// the paths it was started with.
    pub(crate) async fn plan(&self, version: &Version) -> anyhow::Result<Plan> {
        let Some(first) = self.services.first() else {
            bail!("no services to update");
        };

        let own = self
            .inspect(&self.self_id)
            .await
            .context("the updater's own container")?;
        let project = match self.project.as_deref() {
            Some(project) => project,
            None => own.label(LABEL_PROJECT).ok_or_else(|| {
                anyhow!("the updater runs in no Compose project: set NEON_UPDATER_PROJECT")
            })?,
        };

        let filters = [
            "ps".to_owned(),
            "--all".to_owned(),
            "--quiet".to_owned(),
            "--filter".to_owned(),
            format!("label={LABEL_PROJECT}={project}"),
            "--filter".to_owned(),
            format!("label={LABEL_SERVICE}={first}"),
        ];
        let ids = self.runner.output(Program::Docker, &filters, &[]).await?;
        let ids = String::from_utf8_lossy(&ids);
        let Some(id) = ids.split_whitespace().next() else {
            bail!("the project {project} has no {first} container");
        };
        let target = self.inspect(id).await?;

        let files = split_list(target.label(LABEL_CONFIG_FILES));
        let Some(work_dir) = target.label(LABEL_WORKING_DIR) else {
            bail!("the {first} container does not say how Compose started it");
        };
        if files.is_empty() {
            bail!("the {first} container does not say how Compose started it");
        }
        // Started without --env-file, Compose read the project directory's .env.
        let mut env_files = split_list(target.label(LABEL_ENV_FILE));
        if env_files.is_empty() {
            env_files.push(
                Path::new(work_dir)
                    .join(DEFAULT_ENV_FILE)
                    .display()
                    .to_string(),
            );
        }

        let socket = own
            .mounts
            .iter()
            .flatten()
            .find(|mount| mount.destination == SOCKET)
            .map(|mount| mount.source.as_str())
            .ok_or_else(|| anyhow!("the updater has no Docker socket at {SOCKET}"))?;
        let image = self.image.as_deref().unwrap_or(&own.image);

        // The updater's own service is updated after the others, where it is one of the
        // stack's: its update outlives it.
        let mut services = self.services.clone();
        let own_service = own.label(LABEL_SERVICE).filter(|service| {
            own.label(LABEL_PROJECT) == Some(project) && !services.iter().any(|s| s == service)
        });

        let mut dirs = vec![work_dir.to_owned()];
        for path in files.iter().chain(&env_files) {
            dirs.push(parent(path)?);
        }
        for path in dirs.iter().chain(&env_files) {
            check_path(path)?;
        }

        // No --network but for the signatures' check: Docker pulls, not the CLI, which talks to
        // it through the socket. Mounts spelled out, as -v's short form can misread a path.
        let mut args: Vec<String> = [
            "run",
            "--rm",
            "--name",
            &format!("{project}-update"),
            "--label",
            &format!("{LABEL_UPDATE}={version}"),
            "--sig-proxy=false",
        ]
        .map(str::to_owned)
        .to_vec();
        if !self.verify {
            args.extend(["--network".to_owned(), "none".to_owned()]);
        }
        args.extend([
            "--mount".to_owned(),
            format!("type=bind,source={socket},target={SOCKET}"),
            "--workdir".to_owned(),
            work_dir.to_owned(),
            "--entrypoint".to_owned(),
            self.binary.clone(),
        ]);
        for dir in mount_dirs(&dirs) {
            args.extend([
                "--mount".to_owned(),
                format!("type=bind,source={dir},target={dir},readonly"),
            ]);
        }
        // The version goes into the last.
        for file in &env_files {
            args.extend([
                "--mount".to_owned(),
                format!("type=bind,source={file},target={file}"),
            ]);
        }
        args.extend(
            [
                image,
                "apply",
                "--version",
                version.as_str(),
                "--project",
                project,
                "--project-directory",
                work_dir,
            ]
            .map(str::to_owned),
        );
        for file in &files {
            args.extend(["--file".to_owned(), file.clone()]);
        }
        for file in &env_files {
            args.extend(["--env-file".to_owned(), file.clone()]);
        }
        for service in &services {
            args.extend(["--service".to_owned(), service.clone()]);
        }
        if let Some(own_service) = own_service {
            args.extend(["--self".to_owned(), own_service.to_owned()]);
            services.push(own_service.to_owned());
        }
        if !self.verify {
            args.push("--no-verify".to_owned());
        }
        Ok(Plan { args, services })
    }
}

impl<R: Runner + 'static> Job for Docker<R> {
    fn update<'a>(
        &'a self,
        version: &'a Version,
        out: &'a dyn Output,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(Docker::update(self, version, out))
    }
}

/// Gives what is written to both.
struct Tee<'a>(&'a dyn Output, &'a dyn Output);

impl Output for Tee<'_> {
    fn write(&self, bytes: &[u8]) {
        self.0.write(bytes);
        self.1.write(bytes);
    }
}

/// The error a failed update said it failed with, from the end of its output: the update's
/// own, or the docker CLI's where it could not start the update's container.
fn failure(output: &str) -> Option<String> {
    let lines = || output.lines().rev().map(str::trim);
    lines()
        .find_map(|line| line.strip_prefix(ERROR_PREFIX))
        .map(str::to_owned)
        .or_else(|| {
            lines()
                .find_map(|line| line.strip_prefix(DOCKER_ERROR_PREFIX))
                .map(|said| format!("docker run: {said}"))
        })
        .filter(|said| !said.trim().is_empty())
}

/// A label Compose lists paths in, separated by commas.
fn split_list(label: Option<&str>) -> Vec<String> {
    label
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The directory `path` is in.
fn parent(path: &str) -> anyhow::Result<String> {
    Path::new(path)
        .parent()
        .map(|dir| dir.display().to_string())
        .filter(|dir| !dir.is_empty())
        .ok_or_else(|| {
            anyhow!("compose named {path}: the updater mounts the directories of its files, and it is in none")
        })
}

/// Refuses a path the update could not mount as it is.
fn check_path(path: &str) -> anyhow::Result<()> {
    let plain = Path::new(path)
        .components()
        .all(|component| matches!(component, Component::RootDir | Component::Normal(_)));
    if !Path::new(path).is_absolute() || !plain {
        bail!("compose named {path}: the updater mounts only absolute paths, with no .. in them");
    }
    if path == "/" {
        bail!(
            "compose named {path}: the updater mounts the directories of its files, and / is none it can"
        );
    }
    if path.contains([':', ',']) {
        bail!("compose named {path}: the updater mounts no path with a comma or a colon");
    }
    Ok(())
}

/// The directories to mount: each once, and none inside another.
fn mount_dirs(dirs: &[String]) -> Vec<String> {
    let mut cleaned: Vec<PathBuf> = dirs.iter().map(|dir| clean(Path::new(dir))).collect();
    cleaned.sort();
    cleaned.dedup();
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in cleaned {
        if !out.iter().any(|kept| dir.starts_with(kept)) {
            out.push(dir);
        }
    }
    out.iter().map(|dir| dir.display().to_string()).collect()
}

/// `path` without `.` and with each `..` taking off the name before it.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Tail;
    use crate::runner::fake::Fake;

    const SELF: &str = r#"[{"Image": "sha256:updater", "Config": {"Labels": {
        "com.docker.compose.project": "neon", "com.docker.compose.service": "updater"}},
        "Mounts": [{"Source": "/srv/neon", "Destination": "/neon"},
                   {"Source": "/run/docker.sock", "Destination": "/var/run/docker.sock"}]}]"#;
    const SERVER: &str = r#"[{"Image": "sha256:server", "Config": {"Labels": {
        "com.docker.compose.project": "neon", "com.docker.compose.service": "server",
        "com.docker.compose.project.config_files": "/srv/neon/deployments/docker-compose.yml,/etc/neon/extra.yml",
        "com.docker.compose.project.working_dir": "/srv/neon/deployments",
        "com.docker.compose.project.environment_file": "/srv/neon/deployments/.env"}},
        "Mounts": null}]"#;
    const PS: &str = "docker ps --all --quiet --filter label=com.docker.compose.project=neon \
                      --filter label=com.docker.compose.service=server";

    fn stack() -> Fake {
        let mut fake = Fake::default();
        fake.answer("docker inspect --type container abc123", SELF)
            .answer(PS, "def456\n")
            .answer("docker inspect --type container def456", SERVER);
        fake
    }

    fn docker(fake: Fake) -> Docker<Fake> {
        Docker {
            runner: fake,
            self_id: "abc123".to_owned(),
            services: vec!["server".to_owned(), "web".to_owned()],
            project: None,
            image: None,
            binary: BINARY.to_owned(),
            verify: true,
        }
    }

    fn version() -> Version {
        "0.2.1".parse().unwrap()
    }

    #[tokio::test]
    async fn the_update_runs_as_the_stack_was_started() {
        let docker = docker(stack());
        let tail = Tail::new(4096);

        docker.update(&version(), &tail).await.unwrap();

        let ran = docker.runner.ran();
        assert_eq!(
            ran.last().unwrap().args,
            [
                "run",
                "--rm",
                "--name",
                "neon-update",
                "--label",
                "neon.update=0.2.1",
                "--sig-proxy=false",
                "--mount",
                "type=bind,source=/run/docker.sock,target=/var/run/docker.sock",
                "--workdir",
                "/srv/neon/deployments",
                "--entrypoint",
                "/usr/local/bin/neon-updater",
                // Each directory once, those inside another in it.
                "--mount",
                "type=bind,source=/etc/neon,target=/etc/neon,readonly",
                "--mount",
                "type=bind,source=/srv/neon/deployments,target=/srv/neon/deployments,readonly",
                "--mount",
                "type=bind,source=/srv/neon/deployments/.env,target=/srv/neon/deployments/.env",
                "sha256:updater",
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
                "/etc/neon/extra.yml",
                "--env-file",
                "/srv/neon/deployments/.env",
                "--service",
                "server",
                "--service",
                "web",
                "--self",
                "updater",
            ]
        );
        assert_eq!(
            tail.text(),
            "Updating server, web, updater to 0.2.1\nran updater\n"
        );
    }

    #[tokio::test]
    async fn without_an_env_file_the_project_directorys_is_used() {
        let mut fake = stack();
        fake.answer(
            "docker inspect --type container def456",
            &SERVER.replace(
                r#""com.docker.compose.project.environment_file": "/srv/neon/deployments/.env""#,
                r#""x": "y""#,
            ),
        );

        let plan = docker(fake).plan(&version()).await.unwrap();
        let args = plan.args.join(" ");
        assert!(
            args.contains(
                "--mount type=bind,source=/srv/neon/deployments/.env,target=/srv/neon/deployments/.env "
            ),
            "{args}"
        );
        assert!(
            args.contains("--env-file /srv/neon/deployments/.env "),
            "{args}"
        );
    }

    #[tokio::test]
    async fn without_verification_the_update_has_no_network() {
        let mut docker = docker(stack());
        docker.verify = false;

        let plan = docker.plan(&version()).await.unwrap();
        let args = plan.args.join(" ");
        assert!(
            args.contains(" --sig-proxy=false --network none --mount "),
            "{args}"
        );
        assert!(args.ends_with(" --self updater --no-verify"), "{args}");
    }

    #[tokio::test]
    async fn its_project_and_image_as_set() {
        let mut fake = stack();
        fake.answer(&PS.replace("=neon ", "=other "), "def456\n");
        let mut docker = docker(fake);
        docker.project = Some("other".to_owned());
        docker.image = Some("ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.0".to_owned());
        docker.services = vec!["server".to_owned()];

        let plan = docker.plan(&version()).await.unwrap();
        let args = plan.args.join(" ");
        assert!(args.contains("--name other-update "), "{args}");
        assert!(
            args.contains(" ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.0 apply "),
            "{args}"
        );
        assert!(
            args.ends_with("--service server"),
            "not the updater of another project: {args}"
        );
        assert_eq!(plan.services, ["server"]);
    }

    #[tokio::test]
    async fn refuses_a_stack_it_cannot_update() {
        let replace = |json: &str, from: &str, to: &str| json.replacen(from, to, 1);
        type Change = Box<dyn Fn(&mut Fake)>;
        let cases: Vec<(&str, Change, &str)> = vec![
            (
                "not in a container",
                Box::new(|f| {
                    f.outputs.remove("docker inspect --type container abc123");
                }),
                "the updater's own container",
            ),
            (
                "no project",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container abc123",
                        &replace(
                            SELF,
                            r#""com.docker.compose.project": "neon""#,
                            r#""x": "y""#,
                        ),
                    );
                }),
                "set NEON_UPDATER_PROJECT",
            ),
            (
                "no server",
                Box::new(|f| {
                    f.answer(PS, "");
                }),
                "the project neon has no server container",
            ),
            (
                "not started by Compose",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container def456",
                        &replace(SERVER, "com.docker.compose.project.working_dir", "x"),
                    );
                }),
                "does not say how Compose started it",
            ),
            (
                "no socket",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container abc123",
                        &replace(SELF, "/var/run/docker.sock", "/elsewhere"),
                    );
                }),
                "no Docker socket",
            ),
            (
                "files at the root",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container def456",
                        &replace(SERVER, "/etc/neon/extra.yml", "/extra.yml"),
                    );
                }),
                "/ is none it can",
            ),
            (
                "a colon",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container def456",
                        &replace(
                            SERVER,
                            "/srv/neon/deployments/.env",
                            "/srv/neon/x:/etc/.env",
                        ),
                    );
                }),
                "no path with a comma or a colon",
            ),
            (
                "relative",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container def456",
                        &replace(SERVER, "/etc/neon/extra.yml", "extra.yml"),
                    );
                }),
                "compose named extra.yml",
            ),
            (
                "up to the root",
                Box::new(move |f| {
                    f.answer(
                        "docker inspect --type container def456",
                        &replace(
                            SERVER,
                            r#""com.docker.compose.project.working_dir": "/srv/neon/deployments""#,
                            r#""com.docker.compose.project.working_dir": "/srv/neon/../..""#,
                        ),
                    );
                }),
                "compose named /srv/neon/../..: the updater mounts only absolute paths, with no .. in them",
            ),
            (
                "two containers",
                Box::new(move |f| {
                    let two = SELF.replacen("[{", "[{}, {", 1);
                    f.answer("docker inspect --type container abc123", &two);
                }),
                "the updater's own container",
            ),
        ];
        for (name, change, expected) in cases {
            let mut fake = stack();
            change(&mut fake);
            let docker = docker(fake);
            let error = docker.update(&version(), &Tail::new(64)).await.unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{name}: {error:#}");
            assert!(
                !docker
                    .runner
                    .lines()
                    .iter()
                    .any(|line| line.starts_with("docker run")),
                "{name}: nothing run"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_update_fails_with_the_error_it_printed() {
        let mut fake = stack();
        fake.fail("docker run", "docker run: exit status 1");
        let docker = docker(fake);
        let tail = Tail::new(4096);

        // The fake prints only its own line, so the runner's error stands.
        let error = docker.update(&version(), &tail).await.unwrap_err();
        assert_eq!(error.to_string(), "docker run: exit status 1");

        assert_eq!(
            failure(
                "Pulling server, web at 0.2.1\n\
                 Error: no signatures found\n\
                 neon-updater: checking the images' signatures; nothing was changed: \
                 ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 is not signed by the release workflow of v0.2.1\n"
            )
            .as_deref(),
            Some(
                "checking the images' signatures; nothing was changed: \
                 ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 is not signed by the release workflow of v0.2.1"
            )
        );
        assert_eq!(
            failure(
                "docker: Error response from daemon: Conflict. The container name \"/neon-update\" is already in use\n\n\
                 Run 'docker run --help' for more information\n"
            )
            .as_deref(),
            Some(
                "docker run: Error response from daemon: Conflict. The container name \"/neon-update\" is already in use"
            )
        );
        assert_eq!(failure("Pulling server, web at 0.2.1\n"), None);
        assert_eq!(failure("neon-updater: \n"), None);
    }

    #[test]
    fn mount_dirs_mounts_each_directory_once() {
        let dirs = [
            "/srv/neon/deployments",
            "/srv/neon",
            "/srv/neon/./deployments/../deployments",
            "/srv/neonx",
            "/etc/neon",
        ]
        .map(str::to_owned);
        assert_eq!(mount_dirs(&dirs), ["/etc/neon", "/srv/neon", "/srv/neonx"]);
    }
}
