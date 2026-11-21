---
title: Install
description: Run Neon Workspace from source, with Postgres in Docker.
---

Neon Workspace runs from source today: Postgres in Docker, the server with cargo, and the web
app and these docs with Bun. This is a development setup, not one to put on the internet.

**Planned.** Two images, both for `linux/amd64` and `linux/arm64`, and a Compose file that
runs them with Postgres:

- `server`, the `neon` binary on a minimal base image.
- `web`, Caddy, serving the app and these docs and passing `/api`, `/ws`, `/mcp`, `/hooks`
  and `/.well-known/oauth-*` on to the server, all on one origin. Given a domain, it gets and
  renews its TLS certificate itself.

Compose profiles will add SeaweedFS for attachments, Mailpit to catch mail in development,
and an observability stack.

## What you need

- Rust through [rustup](https://rustup.rs). The repository's `rust-toolchain.toml` pins the
  version, and rustup installs it on the first build.
- [Bun](https://bun.sh), for the web app and these docs.
- Docker, for Postgres.

## Postgres

The repository's development Compose file runs Postgres 18 with its data in a named volume.
From the repository's root:

```sh
docker compose -f deployments/docker-compose.dev.yml up -d
```

It listens on `127.0.0.1:55432`, reachable from this machine only, on a port that does not
clash with a Postgres installed the usual way. The user, password and database are all
`neon`, for development only. `docker compose -f deployments/docker-compose.dev.yml down`
stops it and keeps the data; adding `--volumes` deletes the data too.

## The server

Copy `.env.example` to `.env`, which git ignores:

```sh
cp .env.example .env
```

It sets `DATABASE_URL`, which sqlx's tools and the tests read from `.env` themselves, and
`NEON_DATABASE__URL`, the database the server uses. The server does not read `.env`, so
export the file into the shell it runs from, then start it from the repository's root:

```sh
set -a; . ./.env; set +a
cargo run --bin neon -- serve
```

Instead of the environment, the server can be told where the database is in `neon.yaml`, in
the directory it runs from; the environment wins when both say. `neon.yaml` is ignored by
git, so a local one stays local.

```yaml
database:
  url: postgres://neon:neon@127.0.0.1:55432/neon
```

The server updates the database schema as it starts, unless `database.auto_migrate` is
`false`. It listens on port 8080, and its [admin listener](/docs/architecture/observability/#the-admin-listener)
on `127.0.0.1:9090`. To check it answers:

```sh
curl http://localhost:8080/api/v1/meta
```

```json
{"name":"Neon Workspace","version":"0.1.0"}
```

The name is the [`instance.name`](/docs/reference/settings/) setting. Everything else the
server reads is in [Configuration](/docs/self-hosting/configuration/).

### Commands

| Command | What it does |
|---|---|
| `neon serve` | Runs the server. It is what `neon` does when given no command. |
| `neon migrate` | Updates the database schema, then exits. |
| `neon openapi [--out PATH]` | Prints the API's OpenAPI description, or writes it to `PATH`. `--out api/openapi.json` refreshes the committed copy. |
| `neon settings docs` | Prints the [settings reference](/docs/reference/settings/) as markdown, from the server's settings registry. |
| `neon config check` | Loads and checks the configuration from the config file and the environment, then prints it with secrets redacted. |

With cargo, the arguments go after `--`, as in `cargo run --bin neon -- migrate`.

**Planned.** `neon admin create`, to create an instance admin from the command line.

## The web app

The app is in `web/`. Install its packages once, then start Vite's development server, which
reloads as files change:

```sh
cd web
bun install
bun run dev
```

It is then at `http://localhost:5173/`. The development server passes `/api` and the
server's other paths on to the server on port 8080, so in development, too, the app and the
API share one origin.

## These docs

The docs are in `docs/`. Their diagrams are drawn as the docs build, in a headless Chromium
that Playwright runs, so install that once too:

```sh
cd docs
bun install
bun run chromium
bun run dev
```

They are then at `http://localhost:4321/docs/`. `docs/README.md` lists the other commands.

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests that use the database each get a fresh one, which `#[sqlx::test]` creates on the
server `DATABASE_URL` points at, so Postgres must be running for them. A build with
`SQLX_OFFLINE=true` needs no database. It checks the SQL against the committed `.sqlx/`
cache instead; see [Data and storage](/docs/architecture/data/#queries).
