//! Checking the images an update pulled before anything is recreated from them: each image's
//! digest, as Docker pulled it, must carry a cosign signature that the release workflow made
//! for the release's tag, as the docs' "Checking the images" shows by hand.

use std::collections::HashMap;

use anyhow::{Context as _, anyhow, bail};
use serde::Deserialize;

use crate::output::Output;
use crate::runner::{Program, Runner};
use crate::version::Version;

/// Who issues the identity of a GitHub Actions workflow.
pub const ISSUER: &str = "https://token.actions.githubusercontent.com";

/// The workflow that builds, pushes and signs each release's images.
pub const WORKFLOW: &str =
    "https://github.com/lein3000zzz/neon-teamspace/.github/workflows/release.yml";

/// The identity the signatures of `version`'s images carry: the release workflow, run for the
/// version's tag.
#[must_use]
pub fn identity(version: &Version) -> String {
    format!("{WORKFLOW}@refs/tags/v{version}")
}

/// What `docker compose config --format json` says of the services.
#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    services: HashMap<String, Service>,
}

#[derive(Debug, Deserialize)]
struct Service {
    image: Option<String>,
}

/// Checks the signature of the image each of `services` runs, with `compose`, the arguments
/// that name the stack, and `env` over its env files. Each image must have been pulled. Fails
/// on the first that does not check out.
pub(crate) async fn verify_images<R: Runner>(
    runner: &R,
    compose: &[String],
    env: &[(String, String)],
    services: &[String],
    version: &Version,
    out: &dyn Output,
) -> anyhow::Result<()> {
    // Named, the services are in the configuration even where a profile they are in, such as
    // the updater's own, was not turned on.
    let mut args = compose.to_vec();
    args.extend(["config", "--format", "json"].map(str::to_owned));
    args.extend(services.iter().cloned());
    let printed = runner
        .output(Program::Docker, &args, env)
        .await
        .context("reading the Compose file")?;
    let config: Config =
        serde_json::from_slice(&printed).context("reading what docker compose config printed")?;

    let identity = identity(version);
    for service in services {
        let image = config
            .services
            .get(service)
            .and_then(|service| service.image.as_deref())
            .ok_or_else(|| anyhow!("the Compose file gives {service} no image"))?;
        let pinned = pulled_digest(runner, image).await?;
        out.line(&format!("Checking the signature of {pinned}"));
        let args = [
            "verify",
            &pinned,
            "--certificate-identity",
            &identity,
            "--certificate-oidc-issuer",
            ISSUER,
            "--output",
            "text",
        ]
        .map(str::to_owned);
        runner
            .run(Program::Cosign, &args, &[], out)
            .await
            .with_context(|| {
                format!("{image} is not signed by the release workflow of v{version}")
            })?;
    }
    Ok(())
}

/// The image `image` names, by the digest Docker pulled it at: `repository@sha256:...`.
async fn pulled_digest<R: Runner>(runner: &R, image: &str) -> anyhow::Result<String> {
    let args = [
        "image",
        "inspect",
        "--format",
        "{{json .RepoDigests}}",
        image,
    ]
    .map(str::to_owned);
    let printed = runner
        .output(Program::Docker, &args, &[])
        .await
        .with_context(|| format!("reading the digest of {image}"))?;
    let digests: Option<Vec<String>> = serde_json::from_slice(&printed)
        .with_context(|| format!("reading the digest of {image}"))?;
    pick_digest(image, &digests.unwrap_or_default())
}

/// The digest of `digests`, as `docker image inspect` lists them, that belongs to `image`'s
/// repository.
fn pick_digest(image: &str, digests: &[String]) -> anyhow::Result<String> {
    let repository = normalized(repository(image));
    let mut found = digests.iter().filter(|pinned| {
        pinned
            .split_once('@')
            .is_some_and(|(name, digest)| normalized(name) == repository && is_digest(digest))
    });
    match (found.next(), found.next()) {
        (Some(pinned), None) => Ok(pinned.clone()),
        (None, _) => bail!(
            "{image} has no digest from its registry: only an image pulled from one can be checked"
        ),
        (Some(_), Some(_)) => bail!("{image} has more than one digest from its registry"),
    }
}

/// An image's repository: its name without a tag or a digest.
fn repository(image: &str) -> &str {
    let name = image.split_once('@').map_or(image, |(name, _)| name);
    match name.rfind(':') {
        Some(colon) if !name[colon..].contains('/') => &name[..colon],
        _ => name,
    }
}

/// A repository as Docker Hub's images are named in full: `postgres` is
/// `docker.io/library/postgres`.
fn normalized(repository: &str) -> String {
    let Some((first, _)) = repository.split_once('/') else {
        return format!("docker.io/library/{repository}");
    };
    if first.contains('.') || first.contains(':') || first == "localhost" {
        repository.to_owned()
    } else {
        format!("docker.io/{repository}")
    }
}

/// `sha256:` and 64 lowercase hexadecimal digits.
fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Tail;
    use crate::runner::fake::Fake;

    const DIGEST: &str = "sha256:44c87791af0820a6d609825cb3157553d312a3a25f9b4165e5f06e90c2033bd9";

    fn version() -> Version {
        "0.2.1".parse().unwrap()
    }

    #[test]
    fn the_identity_is_the_release_workflow_at_the_tag() {
        assert_eq!(
            identity(&version()),
            "https://github.com/lein3000zzz/neon-teamspace/.github/workflows/release.yml@refs/tags/v0.2.1"
        );
    }

    #[test]
    fn repositories_lose_their_tag_and_digest() {
        assert_eq!(
            repository("ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1"),
            "ghcr.io/lein3000zzz/neon-teamspace-server"
        );
        assert_eq!(
            repository("localhost:5000/neon:0.2.1"),
            "localhost:5000/neon"
        );
        assert_eq!(repository("localhost:5000/neon"), "localhost:5000/neon");
        assert_eq!(repository(&format!("postgres:18@{DIGEST}")), "postgres");
        assert_eq!(normalized("postgres"), "docker.io/library/postgres");
        assert_eq!(
            normalized("grafana/otel-lgtm"),
            "docker.io/grafana/otel-lgtm"
        );
    }

    #[test]
    fn the_digest_is_the_one_of_the_images_repository() {
        let image = "ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1";
        let pinned = format!("ghcr.io/lein3000zzz/neon-teamspace-server@{DIGEST}");
        let digests = vec![format!("mirror.example/neon@{DIGEST}"), pinned.clone()];
        assert_eq!(pick_digest(image, &digests).unwrap(), pinned);

        let error = pick_digest(image, &[]).unwrap_err();
        assert!(
            error.to_string().contains("no digest from its registry"),
            "{error}"
        );
        let error = pick_digest(
            image,
            &["ghcr.io/lein3000zzz/neon-teamspace-server@sha256:x".to_owned()],
        )
        .unwrap_err();
        assert!(error.to_string().contains("no digest"), "{error}");

        assert_eq!(
            pick_digest("postgres:18", &[format!("postgres@{DIGEST}")]).unwrap(),
            format!("postgres@{DIGEST}")
        );
    }

    fn stack(fake: &mut Fake) {
        let config = r#"{"services": {
            "server": {"image": "ghcr.io/lein3000zzz/neon-teamspace-server:0.2.1"},
            "web": {"image": "ghcr.io/lein3000zzz/neon-teamspace-web:0.2.1"},
            "builder": {"build": {"context": "."}}
        }}"#;
        for services in ["server web", "web", "builder"] {
            fake.answer(
                &format!("docker compose -p neon config --format json {services}"),
                config,
            );
        }
        for image in ["server", "web"] {
            fake.answer(
                &format!(
                    "docker image inspect --format {{{{json .RepoDigests}}}} ghcr.io/lein3000zzz/neon-teamspace-{image}:0.2.1"
                ),
                &format!(r#"["ghcr.io/lein3000zzz/neon-teamspace-{image}@{DIGEST}"]"#),
            );
        }
    }

    fn compose() -> Vec<String> {
        ["compose", "-p", "neon"].map(str::to_owned).to_vec()
    }

    #[tokio::test]
    async fn each_image_is_verified_by_its_pulled_digest() {
        let mut fake = Fake::default();
        stack(&mut fake);
        let env = [("NEON_VERSION".to_owned(), "0.2.1".to_owned())];
        let services = ["server".to_owned(), "web".to_owned()];
        let tail = Tail::new(4096);

        verify_images(&fake, &compose(), &env, &services, &version(), &tail)
            .await
            .unwrap();

        let ran = fake.ran();
        assert_eq!(
            ran[0].line(),
            "docker compose -p neon config --format json server web",
            "the services named, whatever their profiles"
        );
        assert_eq!(
            ran[0].env, env,
            "the images of the release being updated to"
        );
        let cosign: Vec<String> = ran
            .iter()
            .filter(|ran| ran.program == Program::Cosign)
            .map(|ran| ran.line())
            .collect();
        assert_eq!(
            cosign,
            ["server", "web"].map(|image| format!(
                "cosign verify ghcr.io/lein3000zzz/neon-teamspace-{image}@{DIGEST} \
                 --certificate-identity https://github.com/lein3000zzz/neon-teamspace/.github/workflows/release.yml@refs/tags/v0.2.1 \
                 --certificate-oidc-issuer https://token.actions.githubusercontent.com --output text"
            ))
        );
        assert!(
            tail.text()
                .starts_with("Checking the signature of ghcr.io/")
        );
    }

    #[tokio::test]
    async fn a_signature_that_does_not_check_out_fails_it() {
        let mut fake = Fake::default();
        stack(&mut fake);
        fake.fail(
            "cosign verify ghcr.io/lein3000zzz/neon-teamspace-web",
            "cosign verify: exit status 1",
        );
        let services = ["server".to_owned(), "web".to_owned()];

        let error = verify_images(
            &fake,
            &compose(),
            &[],
            &services,
            &version(),
            &Tail::new(64),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ghcr.io/lein3000zzz/neon-teamspace-web:0.2.1 is not signed by the release workflow of v0.2.1"
        );
    }

    #[tokio::test]
    async fn an_image_without_a_digest_or_a_service_without_an_image_fails_it() {
        let mut fake = Fake::default();
        stack(&mut fake);
        fake.answer(
            "docker image inspect --format {{json .RepoDigests}} ghcr.io/lein3000zzz/neon-teamspace-web:0.2.1",
            "[]",
        );
        let tail = Tail::new(64);

        let error = verify_images(
            &fake,
            &compose(),
            &[],
            &["web".to_owned()],
            &version(),
            &tail,
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("no digest from its registry"),
            "{error}"
        );

        let error = verify_images(
            &fake,
            &compose(),
            &[],
            &["builder".to_owned()],
            &version(),
            &tail,
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "the Compose file gives builder no image");
        assert!(
            !fake.ran().iter().any(|ran| ran.program == Program::Cosign),
            "nothing to check, nothing checked"
        );
    }
}
