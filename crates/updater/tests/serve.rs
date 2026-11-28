//! `neon-updater serve`, the real binary, against a fake `docker` on the `PATH` that answers as
//! the CLI would in a Compose stack named `neon`, and keeps each call's arguments.

#![cfg(unix)]

mod support;

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use support::{TOKEN, eventually, lines, path, program, request};

/// The fake docker: `inspect` and `ps` answer from the files beside it, and `run`, the
/// update's container, prints the release its label names, waits for the file `go`, and fails
/// if the file `fail` is there, saying why as `neon-updater apply` does.
const DOCKER: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_DOCKER_DIR/calls"
case "$1" in
inspect)
    case "$4" in
    abc123) cat "$FAKE_DOCKER_DIR/self.json" ;;
    def456) cat "$FAKE_DOCKER_DIR/server.json" ;;
    *) echo "Error: No such container: $4" >&2; exit 1 ;;
    esac ;;
ps) echo def456 ;;
run)
    for arg; do
        case "$arg" in neon.update=*) version="${arg#neon.update=}" ;; esac
    done
    echo "Pulling server, web at $version"
    while [ ! -e "$FAKE_DOCKER_DIR/go" ]; do sleep 0.02; done
    echo "a warning on stderr" >&2
    if [ -e "$FAKE_DOCKER_DIR/fail" ]; then
        echo "neon-updater: pulling the images: docker compose: exit status 1" >&2
        exit 1
    fi
    echo "Updated to $version" ;;
*) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#;

const SELF: &str = r#"[{"Image": "sha256:updater", "Config": {"Labels": {
    "com.docker.compose.project": "neon", "com.docker.compose.service": "updater"}},
    "Mounts": [{"Source": "/var/run/docker.sock", "Destination": "/var/run/docker.sock"}]}]"#;

/// Started without --env-file: Compose read the project directory's .env.
const SERVER: &str = r#"[{"Image": "sha256:server", "Config": {"Labels": {
    "com.docker.compose.project": "neon", "com.docker.compose.service": "server",
    "com.docker.compose.project.config_files": "/srv/neon/deployments/docker-compose.yml",
    "com.docker.compose.project.working_dir": "/srv/neon/deployments"}}}]"#;

/// A release `minor` minor versions after the updater's own, so that the tests hold whatever
/// the workspace's version is.
fn newer(minor: u64) -> String {
    let own = semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("a semantic version");
    semver::Version::new(own.major, own.minor + minor, 0).to_string()
}

/// An update request's body for `version`.
fn asking(version: &str) -> String {
    format!(r#"{{"version": "{version}"}}"#)
}

/// The updater, stopped when dropped, and every line it logged.
struct Updater {
    child: Child,
    addr: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Drop for Updater {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(dir: &Path) -> Updater {
    let mut child = Command::new(env!("CARGO_BIN_EXE_neon-updater"))
        .arg("serve")
        .env_clear()
        .env("PATH", path(dir))
        .env("HOSTNAME", "abc123")
        .env("FAKE_DOCKER_DIR", dir)
        .env("NEON_UPDATER_TOKEN", TOKEN)
        .env("NEON_UPDATER_ADDR", "127.0.0.1:0")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("starting neon-updater serve");

    // Its log, read as it comes, so that it never waits on a full pipe.
    let log = Arc::new(Mutex::new(Vec::new()));
    let stdout = BufReader::new(child.stdout.take().expect("its standard output"));
    let (found, addr) = std::sync::mpsc::channel();
    let kept = Arc::clone(&log);
    std::thread::spawn(move || {
        for line in stdout.lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<serde_json::Value>(&line)
                && record["message"] == "listening"
            {
                let _ = found.send(record["addr"].as_str().unwrap_or_default().to_owned());
            }
            kept.lock().expect("the log's lock").push(line);
        }
    });
    let addr = addr
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the updater says where it listens");
    Updater { child, addr, log }
}

#[test]
fn serves_updates_one_at_a_time_through_docker() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    program(dir, "docker", DOCKER);
    std::fs::write(dir.join("self.json"), SELF).unwrap();
    std::fs::write(dir.join("server.json"), SERVER).unwrap();
    let updater = start(dir);
    let addr = updater.addr.as_str();
    let (first, second) = (newer(1), newer(2));

    let health = request(addr, "GET", "/healthz", None, "");
    assert_eq!((health.status, health.body.as_str()), (200, "ok\n"));

    let refused = request(addr, "GET", "/v1/status", Some("not-the-token"), "");
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["error"]["code"], "unauthenticated");

    let idle = request(addr, "GET", "/v1/status", Some(TOKEN), "");
    assert_eq!(idle.status, 200);
    assert_eq!(
        idle.json(),
        serde_json::json!({"state": "idle", "updater_version": env!("CARGO_PKG_VERSION")})
    );

    let older = request(addr, "POST", "/v1/update", Some(TOKEN), &asking("0.0.1"));
    assert_eq!(older.status, 400, "{}", older.body);
    assert_eq!(older.json()["error"]["code"], "older_release");

    let started = request(addr, "POST", "/v1/update", Some(TOKEN), &asking(&first));
    assert_eq!(started.status, 202, "{}", started.body);
    assert_eq!(started.json()["state"], "updating");

    let busy = request(addr, "POST", "/v1/update", Some(TOKEN), &asking(&second));
    assert_eq!(busy.status, 409, "{}", busy.body);
    assert_eq!(busy.json()["error"]["code"], "update_in_progress");

    // The update's output, as it comes.
    let pulling = format!("Pulling server, web at {first}");
    let running = eventually("the update's first line", || {
        let status = request(addr, "GET", "/v1/status", Some(TOKEN), "").json();
        status["log"]
            .as_str()
            .is_some_and(|log| log.contains(&pulling))
            .then_some(status)
    });
    assert_eq!(running["state"], "updating");
    assert_eq!(
        running["log"].as_str().unwrap().lines().next(),
        Some(format!("Updating server, web, updater to {first}").as_str())
    );

    std::fs::write(dir.join("go"), "").unwrap();
    let done = eventually("the update to end", || {
        let status = request(addr, "GET", "/v1/status", Some(TOKEN), "").json();
        (status["state"] != "updating").then_some(status)
    });
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["version"], first.as_str());
    // Both streams, each read as it comes, so the two lines may come in either order.
    let log = done["log"].as_str().unwrap();
    assert!(log.contains("a warning on stderr\n"), "{log}");
    assert!(log.contains(&format!("Updated to {first}\n")), "{log}");
    assert!(done["finished_at"].is_string());

    let calls = lines(dir, "calls");
    assert_eq!(
        calls[..3],
        [
            "inspect --type container abc123",
            "ps --all --quiet --filter label=com.docker.compose.project=neon \
             --filter label=com.docker.compose.service=server",
            "inspect --type container def456",
        ]
    );
    assert_eq!(
        calls[3],
        format!(
            "run --rm --name neon-update --label neon.update={first} --sig-proxy=false \
             --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
             --workdir /srv/neon/deployments --entrypoint /usr/local/bin/neon-updater \
             --mount type=bind,source=/srv/neon/deployments,target=/srv/neon/deployments,readonly \
             --mount type=bind,source=/srv/neon/deployments/.env,target=/srv/neon/deployments/.env \
             sha256:updater apply --version {first} --project neon \
             --project-directory /srv/neon/deployments \
             --file /srv/neon/deployments/docker-compose.yml --env-file /srv/neon/deployments/.env \
             --service server --service web --self updater"
        )
    );

    // Each line of the update's output is in the updater's own log too.
    let logged = updater.log.lock().unwrap().join("\n");
    assert!(
        logged.contains(&format!(r#""output":"{pulling}""#)),
        "{logged}"
    );

    // Another update may start once one ended, and its failure is kept.
    std::fs::write(dir.join("fail"), "").unwrap();
    let again = request(addr, "POST", "/v1/update", Some(TOKEN), &asking(&first));
    assert_eq!(again.status, 202, "{}", again.body);
    let failed = eventually("the second update to end", || {
        let status = request(addr, "GET", "/v1/status", Some(TOKEN), "").json();
        (status["state"] != "updating").then_some(status)
    });
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(
        failed["error"], "pulling the images: docker compose: exit status 1",
        "the update's own error"
    );
    assert!(
        failed["log"]
            .as_str()
            .unwrap()
            .contains("a warning on stderr")
    );
}

#[test]
fn refuses_to_start_with_a_short_token() {
    let output = Command::new(env!("CARGO_BIN_EXE_neon-updater"))
        .arg("serve")
        .env_clear()
        .env("NEON_UPDATER_TOKEN", "too-short-a-token")
        .env("NEON_UPDATER_ADDR", "127.0.0.1:0")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("NEON_UPDATER_TOKEN"), "{said}");
    assert!(said.contains("at least 32 characters"), "{said}");
}

#[test]
fn fails_with_its_error_on_its_last_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_neon-updater"))
        .args(["apply", "--version", "0.2.1", "--project", "neon"])
        .args(["--project-directory", "/srv/neon/deployments"])
        .args(["--file", "/srv/neon/deployments/docker-compose.yml"])
        .args([
            "--env-file",
            "/srv/neon/deployments/.env",
            "--service",
            "server",
        ])
        .env_clear()
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    let last = said.lines().last().unwrap_or_default();
    assert!(
        last.starts_with("neon-updater: pulling the images: running docker"),
        "{said}"
    );
}

#[test]
fn says_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_neon-updater"))
        .arg("version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("neon-updater {}\n", env!("CARGO_PKG_VERSION"))
    );
}
