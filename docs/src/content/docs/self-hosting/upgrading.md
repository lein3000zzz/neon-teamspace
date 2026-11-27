---
title: Upgrading
description: How releases are numbered and tagged, their notes, moving a Compose install to a new release from its Compose archive or a clone, how the server migrates the database as it starts and what a failed migration leaves, rolling back with a backup, and checking the images.
---

## Releases

A release is a Git tag, `v` and a version such as `v0.1.0`, and the images built from it. It
has one version for the server image, the web image, the Compose file and the
[Claude Code plugin](/docs/guides/claude-code-plugin/). The server says its version in
`GET /api/v1/meta` and in `neon --version`.

Versions follow [semantic versioning](https://semver.org). Until 1.0.0, a new minor version,
such as 0.2.0 after 0.1.x, may change the API, the settings or the behaviour in a way that
asks something of whoever runs or calls the server, and its notes say what; a patch version,
such as 0.1.1, fixes what its minor version shipped. A pre-release, such as `0.2.0-rc.1`,
comes before the release of its number.

Each release publishes both images, `ghcr.io/lein3000zzz/neon-teamspace-server` and
`ghcr.io/lein3000zzz/neon-teamspace-web`, for `linux/amd64` and `linux/arm64`, under three
tags:

| Tag | Moves |
|---|---|
| `0.1.0`, the version | Never: it names one release. |
| `0.1`, the minor version | To each patch release of 0.1 as it is published. |
| `latest` | To each release as it is published. |

From 1.0.0 on, the major version, such as `1`, is a tag too, and moves to each release of 1.
A pre-release, such as `v0.2.0-rc.1`, gets its own version as its tag alone, and never moves
`latest` or the others. Its GitHub release is marked as a pre-release. A release candidate,
`-rc.` and a number, rehearses the release of its version on the same commit, so its Compose
archive is that release's: it runs the candidate's images only with `NEON_VERSION` set to the
candidate's version, such as `0.2.0-rc.1`.

The Compose file runs the images of its own release, which its `image` lines name, unless
`NEON_VERSION` in `deployments/.env` names another. Set to `0.1`, it follows patch releases
as they are pulled; set to `latest`, it follows every release, and each `pull` may then take
the instance to a release whose migrations cannot be undone. On an instance whose data
matters, leave it unset, or name a full version: the instance then changes only when you
upgrade it, and the backup you take first matches what runs.

## The release notes

Read them before an upgrade, for every release between the one running and the new one.
They are in two places:

- `CHANGELOG.md` at the repository's root, by area, with a section for each release. A
  change that needs something of you, a setting renamed, a route that answers differently, a
  step to run after the upgrade, is called out there under **Breaking changes**, with what to
  do.
- The release's own page among the repository's
  [releases](https://github.com/lein3000zzz/neon-teamspace/releases). Its notes name the
  images, their platforms and their tags, say how to check their signatures, and link the
  install page, this page and the changelog; the pull requests merged since the full release
  before it come after, as GitHub lists them. The release's Compose archive is attached to it.

## Upgrading a Compose install

1. Read the notes of every release between the one running and the new one.
2. [Back up](/docs/self-hosting/backups/) the database, the files and the secrets. Migrations
   only go forward, so this backup is the way back.
3. Get `deployments/` of the new release, as its Compose file may change with it, and start
   it: from its [Compose archive](#with-the-compose-archive), or
   [built from a clone](#built-from-a-clone).
4. Check `GET /api/v1/meta` for the new version, and the server's log for warnings, as
   [What happens](#what-happens) shows.

### With the Compose archive

In the directory the [install](/docs/self-hosting/install/#starting-it) unpacked the archive
in, unpack the new release's over it, then pull and start its images:

```sh
curl -fsSL https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.1.0/neon-teamspace-compose-0.1.0.tar.gz | tar -xz --strip-components=1
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d --wait
```

With the GitHub CLI, signed in, `gh release download v0.1.0 --repo lein3000zzz/neon-teamspace --pattern 'neon-teamspace-compose-*.tar.gz' --clobber`
fetches the archive instead, for `tar -xzf neon-teamspace-compose-0.1.0.tar.gz --strip-components=1`
to unpack.

Unpacking replaces the Compose file and the two example files, and keeps `deployments/.env`
and `deployments/server.env`, which the archive does not hold. A release may add settings, so
compare the new examples with the files made from the old ones:

```sh
diff deployments/.env.example deployments/.env
diff deployments/server.env.example deployments/server.env
```

With `NEON_VERSION` set in `deployments/.env`, set it to the new release too, or leave it
blank to run the archive's own.

### Built from a clone

Check out the release's tag, then build and start both images from it:

```sh
git fetch --tags
git checkout v0.1.0
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build --wait
```

## What happens

Compose replaces the containers whose image or settings changed. The old server stops as
[Stopping](/docs/self-hosting/install/#stopping) describes. The new one migrates the
database, as [below](#migrations), and logs `database migrations applied` when it is done,
then takes requests; `/readyz` answers `503` until then. Meanwhile Caddy answers `502`, and
open pages show **Reconnecting** and keep what is typed until the server is back. The new web
container starts once the new server is healthy. MCP sessions live in the server's memory,
so MCP clients start new ones. A server that was killed rather than stopped may leave pages
whose latest text search does not read yet; `neon search reindex` stores it, as
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

## Migrations

The server applies pending migrations as it starts, before it takes requests, unless
`database.auto_migrate` is `false`; `neon admin create` and `neon search reindex` do too,
before their own work. `neon migrate` applies them whatever the setting says, and exits.
The migrations are built into the binary. The server:

1. takes a Postgres advisory lock, so two servers that start at once do not both migrate;
2. checks the migrations the database lists in `_sqlx_migrations` against its own: each must
   be one it has, unchanged;
3. applies each pending one in order, each in a transaction of its own with its row in
   `_sqlx_migrations`, so a migration is applied whole or not at all.

While it migrates, the admin listener answers `/livez` with `200` and `/readyz` with `503`,
so the container is not healthy yet; the Compose file gives it two minutes to start before
failed checks count. Migrations are never changed once released: a change to the schema is a
new migration.

A migration that fails rolls back alone: the database stays as the migrations before it left
it, those of the same upgrade included. The server logs `stopping after a failure` with the
error, prints it, and exits with status 1, such as:

```txt
Error: applying database migrations

Caused by:
    0: while executing migration 20261121165322: error returned from database: relation "jobs" already exists at line 1488
    1: error returned from database: relation "jobs" already exists at line 1488
    2: relation "jobs" already exists at line 1488
```

Docker starts it again, as the Compose file says `restart: unless-stopped`, and each start
tries the same migration, so the server keeps restarting and never turns healthy:
`up --wait` stops with `dependency failed to start: container neon-server-1 is unhealthy`, and
`docker compose -f deployments/docker-compose.yml logs server` shows the error. Restore the
backup and go back to the release before, as [below](#rolling-back), then report the error.

To migrate apart from the start, to watch it say, set
`NEON_DATABASE__AUTO_MIGRATE=false` in `deployments/server.env`, pull the new images, and
run the migrations in a one-off container of the new server before starting it:

```sh
docker compose -f deployments/docker-compose.yml run --rm --no-deps server migrate
docker compose -f deployments/docker-compose.yml up -d --wait
```

A server with `auto_migrate` off never checks the database: it must not start on a database
older or newer than its own migrations.

## Rolling back

A server refuses a database that a newer release migrated, naming the migration it does not
know:

```txt
Error: applying database migrations

Caused by:
    migration 20270101000000 was previously applied but is missing in the resolved migrations
```

So rolling back is a restore of the backup taken before the upgrade. First point the stack
at the older release: unpack its Compose archive over the directory again, or check out its
tag in a clone, and set `NEON_VERSION` back if it names the newer one. Setting `NEON_VERSION`
to the older release runs its images too, with the newer Compose file. Then restore the
backup [over the instance](/docs/self-hosting/backups/#over-an-instance-that-runs), whose
last command starts the older release's images; built from a clone, start them with
`docker-compose.build.yml` and `--build` instead, as [above](#built-from-a-clone).
Everything changed since the backup is lost. An upgrade without migrations can go back by its
image alone, but the notes do not promise which releases have none: take the backup either
way.

## Checking the images

Each image carries a software bill of materials, which lists the packages in it, the crates
the `neon` binary links among them in the server image, and the provenance of its build,
which records the commit and the workflow that built it:

```sh
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .SBOM }}'
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .Provenance }}'
```

Each release's images are signed with [cosign](https://docs.sigstore.dev/cosign/)'s keyless
signing: the release workflow signs both images, and each platform's image in them, with its
own GitHub identity, which the signature's certificate and Sigstore's public transparency log
record with the repository and the commit. To check one:

```sh
cosign verify ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/lein3000zzz/neon-teamspace/\.github/workflows/release\.yml@refs/tags/v'
```

The command needs cosign 3, or cosign 2.6 or later with `--new-bundle-format`. The workflow
writes each signature as a Sigstore bundle, and as GHCR has no OCI referrers API, cosign
stores the bundle under a tag of the image's package named `sha256-` and the signed image's
digest. Older cosign, and tools that read only cosign's older `.sig` tags, find no signature.
