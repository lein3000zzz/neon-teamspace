//! The server's updater client against the updater itself: `neon-updater`'s own router,
//! served on a free port of loopback, with a docker CLI played by a fake runner, so that both
//! sides of the protocol are held to each other rather than each to a stand-in of the other.
//! Nothing here runs Docker.

use std::sync::{Arc, Mutex, PoisonError};

use anyhow::anyhow;
use neon_app::updates::{Updater as _, UpdaterEndpoint, UpdaterError};
use neon_domain::settings::Secret;
use neon_domain::updates::{UpdaterState, UpdaterStatus};
use neon_server::updates::UpdaterClient;
use neon_updater::docker::{BINARY, Docker, ERROR_PREFIX, Job};
use neon_updater::output::Output;
use neon_updater::runner::{Program, Runner};
use neon_updater::server::{Server, Token};
use semver::Version;
use tokio::sync::Semaphore;
use url::Url;

/// The token both sides are given: obviously fake, and long enough for the updater.
const TOKEN: &str = "contract-test-token-not-a-secret-at-all";

/// What `docker inspect` says of the updater's own container: in the stack's project, with
/// the Docker socket.
const UPDATER_CONTAINER: &str = r#"[{"Image": "sha256:updater", "Config": {"Labels": {
    "com.docker.compose.project": "neon", "com.docker.compose.service": "updater"}},
    "Mounts": [{"Source": "/run/docker.sock", "Destination": "/var/run/docker.sock"}]}]"#;

/// What it says of the stack's server: how Compose started it.
const SERVER_CONTAINER: &str = r#"[{"Image": "sha256:server", "Config": {"Labels": {
    "com.docker.compose.project": "neon", "com.docker.compose.service": "server",
    "com.docker.compose.project.config_files": "/srv/neon/deployments/docker-compose.yml",
    "com.docker.compose.project.working_dir": "/srv/neon/deployments",
    "com.docker.compose.project.environment_file": "/srv/neon/deployments/.env"}},
    "Mounts": null}]"#;

/// What the fake docker CLI does with an update's container, which the test steers.
struct Containers {
    /// Lets the update's container through; none until a test adds a permit.
    through: Semaphore,
    /// The error the update's container fails with, printed as `neon-updater apply` prints it.
    fail: Mutex<Option<String>>,
    /// The arguments of every container run.
    runs: Mutex<Vec<Vec<String>>>,
}

impl Containers {
    fn new() -> Self {
        Self {
            through: Semaphore::new(0),
            fail: Mutex::new(None),
            runs: Mutex::new(Vec::new()),
        }
    }
}

/// The docker CLI of a stack of one project, `neon`: it answers what the updater asks of the
/// stack, and runs each update's container as [`Containers`] says.
struct FakeDocker(Arc<Containers>);

impl Runner for FakeDocker {
    async fn output(
        &self,
        program: Program,
        args: &[String],
        _env: &[(String, String)],
    ) -> anyhow::Result<Vec<u8>> {
        let line = format!("{program} {}", args.join(" "));
        let printed = match line.as_str() {
            "docker inspect --type container updater-1" => UPDATER_CONTAINER,
            "docker ps --all --quiet --filter label=com.docker.compose.project=neon \
             --filter label=com.docker.compose.service=server" => "server-1\n",
            "docker inspect --type container server-1" => SERVER_CONTAINER,
            _ => return Err(anyhow!("{line}: no such object")),
        };
        Ok(printed.as_bytes().to_vec())
    }

    async fn run(
        &self,
        program: Program,
        args: &[String],
        _env: &[(String, String)],
        out: &dyn Output,
    ) -> anyhow::Result<()> {
        assert_eq!(program, Program::Docker, "the update runs in a container");
        let containers = &self.0;
        containers
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(args.to_vec());
        containers
            .through
            .acquire()
            .await
            .expect("the gate stays open")
            .forget();
        out.line("pulled the release's images");
        let fail = containers
            .fail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match fail {
            Some(error) => {
                out.line(&format!("{ERROR_PREFIX}{error}"));
                Err(anyhow!("docker run: exit status 1"))
            }
            None => Ok(()),
        }
    }
}

/// The updater, serving its router on a free port of loopback.
struct Running {
    updater: Server,
    docker: Arc<Containers>,
    url: Url,
    serving: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn start() -> Self {
        let docker = Arc::new(Containers::new());
        let job = Docker {
            runner: FakeDocker(Arc::clone(&docker)),
            self_id: "updater-1".to_owned(),
            services: vec!["server".to_owned(), "web".to_owned()],
            project: None,
            image: None,
            binary: BINARY.to_owned(),
            verify: true,
        };
        let updater = Server::new(
            Token::new(TOKEN).expect("a token long enough"),
            Arc::new(job) as Arc<dyn Job>,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port");
        let url = Url::parse(&format!("http://{}", listener.local_addr().expect("bound")))
            .expect("a URL");
        let router = updater.router();
        let serving = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serving");
        });
        Self {
            updater,
            docker,
            url,
            serving,
        }
    }

    /// Where the server finds it, with the token it is given.
    fn endpoint(&self, token: &str) -> UpdaterEndpoint {
        UpdaterEndpoint {
            url: self.url.clone(),
            token: Secret::new(token),
        }
    }

    fn runs(&self) -> Vec<Vec<String>> {
        self.docker
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

fn version(text: &str) -> Version {
    Version::parse(text).expect("a version")
}

#[tokio::test]
async fn the_server_reads_an_idle_updater_s_status() {
    let running = Running::start().await;

    let status = UpdaterClient::default()
        .status(&running.endpoint(TOKEN))
        .await
        .expect("a status");

    assert_eq!(
        status,
        UpdaterStatus {
            state: UpdaterState::Idle,
            version: None,
            error: None,
            log: None,
            started_at: None,
            finished_at: None,
            updater_version: neon_updater::VERSION.to_owned(),
        }
    );
}

#[tokio::test]
async fn the_server_starts_an_update_follows_it_and_is_refused_a_second_meanwhile() {
    let running = Running::start().await;
    let client = UpdaterClient::default();
    let at = running.endpoint(TOKEN);

    let started = client
        .update(&at, &version("0.2.1"))
        .await
        .expect("the update starts");

    assert_eq!(started.state, UpdaterState::Updating);
    assert_eq!(started.version.as_deref(), Some("0.2.1"));
    assert!(started.started_at.is_some());
    assert_eq!(started.finished_at, None);
    assert_eq!(started.updater_version, neon_updater::VERSION);

    // The update's container is held at the gate: the status says it is under way, with the
    // end of its output so far, and a second update is refused with 409.
    let under_way = client.status(&at).await.expect("a status");
    assert_eq!(under_way.state, UpdaterState::Updating);
    assert_eq!(under_way.version.as_deref(), Some("0.2.1"));
    assert_eq!(under_way.started_at, started.started_at);
    assert_eq!(
        under_way.log.as_deref(),
        Some("Updating server, web, updater to 0.2.1\n")
    );
    assert_eq!(
        client.update(&at, &version("0.2.2")).await,
        Err(UpdaterError::Busy)
    );

    running.docker.through.add_permits(1);
    running.updater.wait().await;

    let done = client.status(&at).await.expect("a status");
    assert_eq!(done.state, UpdaterState::Done);
    assert_eq!(done.version.as_deref(), Some("0.2.1"));
    assert_eq!(done.error, None);
    assert_eq!(
        done.log.as_deref(),
        Some("Updating server, web, updater to 0.2.1\npulled the release's images\n")
    );
    assert_eq!(done.started_at, started.started_at);
    assert!(done.finished_at >= done.started_at);

    let runs = running.runs();
    assert_eq!(runs.len(), 1, "the refused update ran nothing: {runs:?}");
    let asked = runs[0]
        .windows(2)
        .any(|pair| pair[0] == "--version" && pair[1] == "0.2.1");
    assert!(
        asked,
        "the update's container is asked for 0.2.1: {:?}",
        runs[0]
    );
}

#[tokio::test]
async fn a_failed_update_reaches_the_server_with_the_update_s_own_error() {
    let running = Running::start().await;
    let client = UpdaterClient::default();
    let at = running.endpoint(TOKEN);
    let why = "checking the images' signatures; nothing was changed: \
               ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 is not signed";
    *running
        .docker
        .fail
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(why.to_owned());
    running.docker.through.add_permits(1);

    client
        .update(&at, &version("0.2.1"))
        .await
        .expect("the update starts");
    running.updater.wait().await;

    let failed = client.status(&at).await.expect("a status");
    assert_eq!(failed.state, UpdaterState::Failed);
    assert_eq!(failed.version.as_deref(), Some("0.2.1"));
    assert_eq!(failed.error.as_deref(), Some(why));
    let log = failed.log.expect("the output");
    assert!(log.ends_with(&format!("{ERROR_PREFIX}{why}\n")), "{log}");
    assert!(failed.finished_at.is_some());

    // A failed update is over: the next one starts.
    running.docker.through.add_permits(1);
    let again = client
        .update(&at, &version("0.2.1"))
        .await
        .expect("a second try starts");
    assert_eq!(again.state, UpdaterState::Updating);
    running.updater.wait().await;
}

#[tokio::test]
async fn a_wrong_token_is_refused_and_starts_nothing() {
    let running = Running::start().await;
    let client = UpdaterClient::default();
    // The right length, one character off.
    let wrong = format!("{}X", &TOKEN[..TOKEN.len() - 1]);
    let at = running.endpoint(&wrong);

    assert_eq!(client.status(&at).await, Err(UpdaterError::Refused));
    assert_eq!(
        client.update(&at, &version("0.2.1")).await,
        Err(UpdaterError::Refused)
    );
    let refused = client
        .update(&at, &version("0.2.1"))
        .await
        .expect_err("refused")
        .to_string();
    assert!(
        !refused.contains(TOKEN) && !refused.contains(&wrong),
        "{refused}"
    );

    assert!(running.runs().is_empty());
    let status = client
        .status(&running.endpoint(TOKEN))
        .await
        .expect("a status");
    assert_eq!(status.state, UpdaterState::Idle);
}

/// The server's versions are semantic versions, build metadata and all; the updater's are
/// also image tags, which take no `+`. It refuses such a version with `400`, and the server
/// passes on its reason, read from the error's shape.
#[tokio::test]
async fn a_version_the_updater_does_not_take_is_refused_with_its_reason() {
    let running = Running::start().await;

    let refused = UpdaterClient::default()
        .update(&running.endpoint(TOKEN), &version("0.2.1+build.5"))
        .await;

    assert_eq!(
        refused,
        Err(UpdaterError::Rejected(
            "\"0.2.1+build.5\" is not a version, such as 0.2.1".to_owned()
        ))
    );
    assert!(running.runs().is_empty());
}

/// An updater URL whose path the updater does not serve: its `404`, in the API's error shape,
/// reaches the server as an updater that does not answer as its protocol says, with the
/// updater's reason.
#[tokio::test]
async fn an_updater_url_with_another_path_gets_the_updater_s_404() {
    let running = Running::start().await;
    let mut at = running.endpoint(TOKEN);
    at.url.set_path("/updater/");

    assert_eq!(
        UpdaterClient::default().status(&at).await,
        Err(UpdaterError::Unreachable(
            "the updater answered 404: no such route".to_owned()
        ))
    );
}
