---
title: Upgrading
description: Move an instance to a new release, with a backup first, the release's Compose archive or a clone, and a check of what runs after.
---

A release of Neon Teamspace has one version, such as `0.1.0`, for the server image, the web
image, the Compose file and the [Claude Code plugin](/docs/guides/claude-code-plugin/). The
[changelog](https://github.com/lein3000zzz/neon-teamspace/blob/main/CHANGELOG.md) says what
each release changes, and what it needs of the instance's admins as they upgrade. Read the
sections between the release an instance runs and the one it goes to before upgrading. The
release's page on GitHub names its images and their tags, lists the pull requests it brings,
links its changelog, and has its Compose archive.

## Versions and tags

Versions are major, minor and patch numbers. Before 1.0, a minor release, such as 0.2.0, may
change what admins set up, and its section in the changelog says how; a patch release, such
as 0.1.1, fixes what the release before it got wrong. A pre-release, such as `0.2.0-rc.1`,
comes before the release of its number.

Each release pushes both images to `ghcr.io/lein3000zzz` under these tags:

| Tag | Names |
|---|---|
| The version, such as `0.1.0` | That release, for good. |
| The minor version, such as `0.1` | The newest patch release of 0.1. |
| `latest` | The newest release. |
| The major version, such as `1`, from 1.0 on | The newest release of 1. |

A pre-release is tagged with its version alone, and moves none of the others.

The Compose file runs the images of its own release, which its `image` lines name, unless
`NEON_VERSION` in `deployments/.env` says otherwise. Set to `0.1`, it follows patch releases
as they are pulled; set to `latest`, it follows every release, and each `pull` may then take
the instance to a release whose migrations cannot be undone. On an instance whose data
matters, leave it unset and upgrade by choice, as below.

## Before upgrading

Back up the database, and then the attachments' files, as
[Install](/docs/self-hosting/install/#where-the-data-lives) and
[File storage](/docs/self-hosting/storage/#backups) show. Migrations only go forward: going
back to an older release means restoring the backup taken before the upgrade.

## With the Compose archive

In the directory the [install](/docs/self-hosting/install/#starting-it) unpacked the archive
in, unpack the new release's over it, then pull and start its images:

```sh
gh release download v0.1.0 --repo lein3000zzz/neon-teamspace --pattern 'neon-teamspace-compose-*.tar.gz' --clobber
tar -xzf neon-teamspace-compose-0.1.0.tar.gz --strip-components=1
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d
```

Unpacking replaces the Compose file and the two example files, and keeps `deployments/.env`
and `deployments/server.env`, which the archive does not hold. A release may add settings, so
compare the new examples with the files made from the old ones:

```sh
diff deployments/.env.example deployments/.env
diff deployments/server.env.example deployments/server.env
```

With `NEON_VERSION` set in `deployments/.env`, set it to the new release too, or leave it
blank to run the archive's own.

## Built from a clone

Check out the release's tag, then build and start both images from it:

```sh
git fetch --tags
git checkout v0.1.0
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build
```

## What happens

Compose replaces the containers whose image changed. The new server applies the new
migrations before it takes requests, and logs `database migrations applied` when they are
done; `/readyz` answers `503` until then, and the web container waits until the server is
healthy. A server that was killed rather than stopped may leave pages whose latest text
search does not read yet; `neon search reindex` stores it, as
[Search](/docs/guides/search/#rebuilding-the-text) explains.

Then check that every service is healthy and that the new release runs:

```sh
docker compose -f deployments/docker-compose.yml ps
docker compose -f deployments/docker-compose.yml exec server neon --version
curl -s http://127.0.0.1:3000/api/v1/meta
```

```json
{"name":"Neon Teamspace","version":"0.1.0"}
```

The foot of the app's sidebar shows the same version.

## Going back

The older server refuses a database that a newer one migrated, naming a migration it does not
know, so going back starts from the backup taken before the upgrade. Restore it into the
emptied database, as [Install](/docs/self-hosting/install/#where-the-data-lives) shows,
but leave the server stopped. Then unpack the older release's Compose archive again, or set
`NEON_VERSION` to it, and start its images with
`docker compose -f deployments/docker-compose.yml up -d`. What changed since the backup is
lost.

## Checking the images

Each image carries a software bill of materials and the provenance of its build, which
record the packages in it, and the commit and the workflow that built it:

```sh
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .SBOM }}'
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .Provenance }}'
```

Releases are signed with [cosign](https://docs.sigstore.dev/cosign/)'s keyless signing once
the repository is public, as the signature records the repository's identity in Sigstore's
public log; a release made while the repository is private has no signature. To check one:

```sh
cosign verify ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/lein3000zzz/neon-teamspace/\.github/workflows/release\.yml@refs/tags/v'
```
