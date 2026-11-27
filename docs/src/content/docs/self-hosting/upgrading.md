---
title: Upgrading
description: How releases are numbered and tagged, moving a Compose install to a new release, how the server migrates the database as it starts and what a failed migration leaves, rolling back with a backup, and the release notes.
---

## Releases

A release is a Git tag, `v` and a version such as `v0.1.0`, and the images built from it.
Versions follow [semantic versioning](https://semver.org). Until 1.0.0, a new minor version,
such as 0.2.0 after 0.1.x, may change the API, the settings or the behaviour in a way that
asks something of whoever runs or calls the server, and its notes say what; a patch version,
such as 0.1.1, fixes what its minor version shipped.

Each release publishes both images, `ghcr.io/lein3000zzz/neon-teamspace-server` and
`ghcr.io/lein3000zzz/neon-teamspace-web`, for `linux/amd64` and `linux/arm64`, under three
tags:

| Tag | Moves |
|---|---|
| `0.1.0`, the version | Never: it names one release. |
| `0.1`, the minor version | To each patch release of 0.1 as it is published. |
| `latest` | To each release as it is published. |

From 1.0.0 on, the major version, such as `1`, is a tag too. A pre-release, such as
`v0.2.0-rc.1`, gets its own tag alone, and never moves `latest` or the others. The server
says its version in `GET /api/v1/meta` and in `neon --version`, and the
[Claude Code plugin](/docs/guides/claude-code-plugin/)'s version is the server's.

`NEON_VERSION` in `deployments/.env` picks the tag both images run. Name a full version: the
instance then changes only when you change it, and the backup you take first matches what
runs. Unset, it is `latest`, which moves to a new release, migrations included, at the next
`pull`.

## The release notes

Read them before an upgrade. Each release has its notes on the repository's
[releases page](https://github.com/lein3000zzz/neon-teamspace/releases), with the images'
names and the pull requests it merged, and in `CHANGELOG.md` at the repository's root, by
area. A change that needs something of you, a setting renamed, a route that answers
differently, a step to run after the upgrade, is called out there under
**Breaking changes**, with what to do.

## Upgrading a Compose install

1. Read the notes of every release between the one running and the new one.
2. [Back up](/docs/self-hosting/backups/) the database, the files and the secrets. Migrations
   only go forward, so this backup is the way back.
3. Get `deployments/` of the new release, as its Compose file may change with it. In a
   clone, `git fetch --tags` and `git checkout v0.2.0`; with a copy of the directory, copy
   the new release's files over it, keeping `.env` and `server.env`.
4. Set `NEON_VERSION` in `deployments/.env` to the new version, then pull and start:

   ```sh
   docker compose -f deployments/docker-compose.yml pull
   docker compose -f deployments/docker-compose.yml up -d --wait
   ```

   Built from a clone instead, check out the tag and build:

   ```sh
   docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build --wait
   ```

5. Check `GET /api/v1/meta` for the new version, and the server's log for warnings.

Compose replaces the containers whose image or settings changed. The old server stops as
[Stopping](/docs/self-hosting/install/#stopping) describes, and the new one migrates the
database, then takes requests. Meanwhile Caddy answers `502`, and open pages show
**Reconnecting** and keep what is typed until the server is back. MCP sessions live in the
server's memory, so MCP clients start new ones.

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
```

Docker starts it again, as the Compose file says `restart: unless-stopped`, and each start
tries the same migration, so the server keeps restarting and never turns healthy:
`up --wait` stops with `container neon-server-1 is unhealthy`, and
`docker compose -f deployments/docker-compose.yml logs server` shows the error. Restore the
backup and go back to the release before, then report the error.

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

A server refuses a database that a newer release migrated, as its migrations do not include
the newer ones:

```txt
Error: applying database migrations

Caused by:
    migration 20270101000000 was previously applied but is missing in the resolved migrations
```

So rolling back is a restore. Set `NEON_VERSION` back to the release before, bring back its
`deployments/`, and restore the backup taken before the upgrade
[over the instance](/docs/self-hosting/backups/#over-an-instance-that-runs). Everything
changed since the backup is lost. An upgrade without migrations can go back by its image
alone, but the notes do not promise which releases have none: take the backup either way.
