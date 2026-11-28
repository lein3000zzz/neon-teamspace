//! `neon-updater apply`, the real binary, against a fake `docker` and a fake `cosign` on the
//! `PATH`, which keep each call's arguments, and a real env file.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use support::{lines, path, program};

const DIGEST: &str = "sha256:44c87791af0820a6d609825cb3157553d312a3a25f9b4165e5f06e90c2033bd9";

/// The fake docker: Compose's configuration from the file beside it, each image's digest from
/// its name, and everything else done.
fn docker() -> String {
    format!(
        r#"#!/bin/sh
printf 'docker %s | NEON_VERSION=%s\n' "$*" "$NEON_VERSION" >> "$FAKE_DIR/calls"
case "$*" in
*" config --format json "*) cat "$FAKE_DIR/config.json" ;;
"image inspect "*) printf '["%s@{DIGEST}"]\n' "${{5%:*}}" ;;
*) echo "did $*" ;;
esac
"#
    )
}

/// The fake cosign: every signature checks out, but those of the images whose names hold what
/// the file `unsigned` holds, while it is there: all of them when it is empty.
const COSIGN: &str = r#"#!/bin/sh
printf 'cosign %s\n' "$*" >> "$FAKE_DIR/calls"
if [ -e "$FAKE_DIR/unsigned" ]; then
  case "$2" in *"$(cat "$FAKE_DIR/unsigned")"*)
    echo "Error: no signatures found" >&2; exit 10 ;;
  esac
fi
echo "Verification for $2 --"
"#;

const CONFIG: &str = r#"{"services": {
    "server": {"image": "ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1"},
    "web": {"image": "ghcr.io/lein3000zzz/neon-teamspace-web:0.2.1"},
    "updater": {"image": "ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.1"}}}"#;

const ENV_FILE: &str =
    "# Compose's settings.\nPOSTGRES_PASSWORD='not-a-real-password'\nNEON_VERSION=\n";

fn apply(dir: &Path, env_file: &Path) -> Output {
    apply_with(dir, env_file, &[])
}

/// `neon-updater apply` of the stack, with `more` arguments after the usual ones.
fn apply_with(dir: &Path, env_file: &Path, more: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neon-updater"))
        .args(["apply", "--version", "0.2.1", "--project", "neon"])
        .args(["--project-directory", "/srv/neon/deployments"])
        .args(["--file", "/srv/neon/deployments/docker-compose.yml"])
        .arg("--env-file")
        .arg(env_file)
        .args(["--service", "server", "--service", "web"])
        .args(more)
        .env_clear()
        .env("PATH", path(dir))
        .env("FAKE_DIR", dir)
        .output()
        .expect("running neon-updater apply")
}

fn stack(dir: &Path) -> std::path::PathBuf {
    program(dir, "docker", &docker());
    program(dir, "cosign", COSIGN);
    std::fs::write(dir.join("config.json"), CONFIG).expect("writing the configuration");
    let env_file = dir.join(".env");
    std::fs::write(&env_file, ENV_FILE).expect("writing the env file");
    env_file
}

#[test]
fn pulls_checks_keeps_the_version_and_starts() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let env_file = stack(dir);

    let output = apply(dir, &env_file);
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{printed}{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let compose = format!(
        "docker compose --project-name neon --project-directory /srv/neon/deployments \
         --file /srv/neon/deployments/docker-compose.yml --env-file {}",
        env_file.display()
    );
    let identity = "--certificate-identity https://github.com/lein3000zzz/neon-teamspace/.github/workflows/release.yml@refs/tags/v0.2.1 \
                    --certificate-oidc-issuer https://token.actions.githubusercontent.com --output text";
    assert_eq!(
        lines(dir, "calls"),
        [
            format!("{compose} pull --quiet server web | NEON_VERSION=0.2.1"),
            format!("{compose} config --format json server web | NEON_VERSION=0.2.1"),
            "docker image inspect --format {{json .RepoDigests}} ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 | NEON_VERSION=".to_owned(),
            format!("cosign verify ghcr.io/lein3000zzz/neon-teamspace-server@{DIGEST} {identity}"),
            "docker image inspect --format {{json .RepoDigests}} ghcr.io/lein3000zzz/neon-teamspace-web:0.2.1 | NEON_VERSION=".to_owned(),
            format!("cosign verify ghcr.io/lein3000zzz/neon-teamspace-web@{DIGEST} {identity}"),
            format!("{compose} up --detach --no-deps --pull never server web | NEON_VERSION=0.2.1"),
        ]
    );
    assert_eq!(
        std::fs::read_to_string(&env_file).unwrap(),
        "# Compose's settings.\nPOSTGRES_PASSWORD='not-a-real-password'\nNEON_VERSION='0.2.1'\n"
    );
    assert!(
        printed.starts_with("Pulling server, web at 0.2.1\n"),
        "{printed}"
    );
    assert!(
        printed.contains(&format!(
            "Verification for ghcr.io/lein3000zzz/neon-teamspace-web@{DIGEST} --\n"
        )),
        "{printed}"
    );
    assert!(printed.ends_with("Updated to 0.2.1\n"), "{printed}");
}

#[test]
fn an_image_that_does_not_check_out_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let env_file = stack(dir);
    std::fs::write(dir.join("unsigned"), "").unwrap();

    let output = apply(dir, &env_file);
    assert!(!output.status.success());
    let printed = String::from_utf8_lossy(&output.stdout);
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        said.contains("neon-updater: checking the images' signatures; nothing was changed"),
        "{said}"
    );
    assert!(
        said.contains("ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1 is not signed by the release workflow of v0.2.1"),
        "{said}"
    );
    assert!(printed.contains("Error: no signatures found"), "{printed}");

    let calls = lines(dir, "calls");
    assert!(
        calls.last().unwrap().starts_with("cosign verify"),
        "{calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains(" up ")),
        "nothing started: {calls:?}"
    );
    assert_eq!(std::fs::read_to_string(&env_file).unwrap(), ENV_FILE);
}

#[test]
fn an_updater_image_that_does_not_check_out_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let env_file = stack(dir);
    std::fs::write(dir.join("unsigned"), "neon-teamspace-updater@").unwrap();

    let output = apply_with(dir, &env_file, &["--self", "updater"]);
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        said.contains("neon-updater: checking the images' signatures; nothing was changed: \
                       ghcr.io/lein3000zzz/neon-teamspace-updater:0.2.1 is not signed by the release workflow of v0.2.1"),
        "{said}"
    );

    let calls = lines(dir, "calls");
    assert!(
        calls[0].ends_with(" pull --quiet server web updater | NEON_VERSION=0.2.1"),
        "pulled with the others: {calls:?}"
    );
    assert!(
        calls.iter().any(
            |call| call.starts_with("cosign verify ghcr.io/lein3000zzz/neon-teamspace-server@")
        ),
        "the others checked first: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.contains(" up ")),
        "nothing started: {calls:?}"
    );
    assert_eq!(std::fs::read_to_string(&env_file).unwrap(), ENV_FILE);
}
