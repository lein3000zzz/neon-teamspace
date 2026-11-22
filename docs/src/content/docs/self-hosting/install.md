---
title: Install
description: Run Neon Teamspace with Docker Compose, or from source for development.
---

Neon Teamspace runs in two ways:

- **With Docker**: Postgres, the server and Caddy as containers, from the Compose file in
  `deployments/`. This is the way to run an instance.
- **From source**: Postgres in Docker, the server with cargo, and the web app and these docs
  with Bun, each reloading as its files change. This is the way to work on the code.

## With Docker

Two images make up an instance, both built for `linux/amd64` and `linux/arm64`:

| Image | What it holds |
|---|---|
| `ghcr.io/lein3000zzz/neon-teamspace-server` | The `neon` binary on a minimal base image without a shell, distroless `cc`, running as an unprivileged user. It listens on port 8080 for the HTTP API and serves its [admin listener](/docs/architecture/observability/#the-admin-listener) on 9090. The license notices of the crates it is built from are in `/usr/share/doc/neon/THIRD_PARTY_NOTICES.txt`. |
| `ghcr.io/lein3000zzz/neon-teamspace-web` | Caddy, serving the app at `/` and these docs at `/docs`, and passing `/api`, `/ws`, `/mcp`, `/hooks` and `/.well-known/oauth-*` on to the server, all on one origin. Given a domain, it gets and renews its TLS certificate itself. The app's and the docs' license notices are served at `/THIRD_PARTY_NOTICES.txt` and `/docs/THIRD_PARTY_NOTICES.txt`. |

You need Docker Engine 25 or later with Compose 2.24.4 or later, and the repository's
`deployments/` directory, which holds the Compose files: a clone, to build both images from
it, or a copy of `deployments/` alone to run the published ones.

### Starting it

Copy the example settings to `deployments/.env`, which git ignores, and set the database's
password in it. In a clone, `docker-compose.build.yml` beside the Compose file then builds
both images from the checkout:

```sh
cp deployments/.env.example deployments/.env
openssl rand -hex 24       # a password: put it in deployments/.env as POSTGRES_PASSWORD
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build
```

The first start builds the images, which takes some minutes. They are named
`neon-teamspace-server:local` and `neon-teamspace-web:local`, never after a release. The
instance is then at `http://127.0.0.1:3000/`, and these docs at
`http://127.0.0.1:3000/docs/`. With `make`, `make up` builds and runs the same stack in the
foreground, until Ctrl+C stops it, and `make down` stops it from another terminal.

To run the published images instead, set `NEON_VERSION` in `deployments/.env` to a release,
such as `0.1.0`, and use `docker-compose.yml` alone:

```sh
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d
```

That file has no build sections, so an image that cannot be pulled, such as one of a
mistyped version, stops the start instead of being built from whatever the directory holds.
Without `NEON_VERSION`, it runs `latest`, which moves on to each new release as it is
pulled. While the images are private on GitHub's container registry, `docker login ghcr.io`
with a token that can read packages comes first.

The stack has three services, which start in order, each once the one before it is healthy:

| Service | What it runs | Healthy when |
|---|---|---|
| `postgres` | Postgres 18, with its data in a named volume. Not published. | `pg_isready` answers. |
| `server` | `neon serve`, which applies pending migrations as it starts. Not published: clients reach it through `web`. | `neon health` succeeds: the admin listener's `/readyz` answers `200`, once the server listens and while Postgres answers. |
| `web` | Caddy, on `127.0.0.1:3000` unless `deployments/.env` says otherwise. | Caddy answers on its own health address, on loopback inside the container. |

Every `up` names the same files as the first one did: on a stack built from the clone, an
`up` with `docker-compose.yml` alone replaces the built images with the published ones.
Other commands, such as `ps`, `logs`, `exec`, `stop` and `down`, find the running stack by
its project name, `neon`, so `docker compose -f deployments/docker-compose.yml` is enough for
them. `docker compose -f deployments/docker-compose.yml ps` shows each service's state, and
`docker compose -f deployments/docker-compose.yml logs server` the server's logs, one JSON
object per line.

### `deployments/.env` and `deployments/server.env`

The settings are in two files beside the Compose file, both ignored by git. Compose reads
`deployments/.env` for the stack's own settings, which `deployments/.env.example` describes:

| Variable | What it sets |
|---|---|
| `POSTGRES_PASSWORD` | The database's password. Required. Postgres reads it once, when its volume is created, so changing it later takes an `ALTER USER` in the database too. Letters and digits only, as it goes into the server's database URL. |
| `NEON_VERSION` | The release of the published images to run, such as `0.1.0`, the same for both. `latest` unless set. Images built from the clone ignore it. |
| `NEON_DOMAIN` | The domain people reach the instance at; see [below](#a-domain-and-tls). Unset, Caddy serves plain HTTP. |
| `NEON_HTTP_PORT`, `NEON_HTTPS_PORT` | Where the web container's ports 80 and 443 are published: a port, or an address and a port. `127.0.0.1:3000` and `127.0.0.1:3443` unless set, which only this machine reaches. |
| `NEON_WEB_TRUSTED_PROXIES` | Address ranges, separated by spaces, of proxies in front of Caddy whose `X-Forwarded-For` it believes. None unless set. |

None of these reaches the server. Its own settings go in `deployments/server.env`, which only
the server reads, as its environment: a `NEON_` variable for any key that
[Configuration](/docs/self-hosting/configuration/) describes, runtime settings such as
`NEON_SETTINGS__INSTANCE__NAME` among them, and the standard `OTEL_` variables.
`deployments/server.env.example` lists the usual ones. The file is optional, and a blank
value leaves its key to the default. Compose expands `$` in its values, so a value that holds
one goes in single quotes. A change applies once the start command runs again, which
recreates the server's container with the new environment.

The stack sets two of the server's keys itself, over anything `server.env` says:
`database.url`, from `POSTGRES_PASSWORD`, and `observability.admin.addr`, which it sets to
`0.0.0.0:9090`. The admin listener then answers on the stack's own network, for Prometheus
to scrape, and is never published. The server logs a warning at start that it is not bound
to loopback; in the stack, that is meant. Caddy never passes the admin listener's paths on,
so `/metrics`, `/readyz` and `/debug/pprof` on the instance's address are routes of the app
like any other.

### A domain and TLS

Given a domain, Caddy gets a certificate for it from a public certificate authority, such as
Let's Encrypt, renews it before it expires, serves HTTPS, and redirects plain HTTP to it. In
`deployments/.env`:

```sh
NEON_DOMAIN=neon.example.com
NEON_HTTP_PORT=80
NEON_HTTPS_PORT=443
```

And in `deployments/server.env`, the address the server gives out for itself:

```sh
NEON_SERVER__PUBLIC_URL=https://neon.example.com
```

The domain's DNS records must point at the machine, and ports 80 and 443 must be reachable
from the internet, as the certificate authority checks the domain through them. Caddy keeps
its certificates in the `caddy-data` volume. Over HTTPS, every answer also carries
`Strict-Transport-Security`, so browsers use HTTPS for the domain from then on.

TLS can also end in front of Caddy instead, at a load balancer, a tunnel or another proxy.
Leave `NEON_DOMAIN` unset, so Caddy serves plain HTTP, point the proxy at `NEON_HTTP_PORT`,
and set `NEON_WEB_TRUSTED_PROXIES` to the proxy's addresses, such as `10.0.0.0/8`, so Caddy
takes the client's address from the proxy's `X-Forwarded-For`. The server only ever sees the
client's address, as Caddy works it out. `Strict-Transport-Security` is then the proxy's to
set. The proxy must also pass WebSocket upgrades on, for the live editing at `/ws`, as most
do by default.

`NEON_SERVER__PUBLIC_URL` must be the address browsers use: the server refuses requests that
change something, and WebSocket upgrades, whose `Origin` is another.

Caddy sets the same security headers on every answer: a Content-Security-Policy that allows
only the instance's own scripts, styles, images, fonts and connections, with a policy of
their own for these docs; `X-Content-Type-Options: nosniff`; `X-Frame-Options: DENY`; a
`Referrer-Policy`; and a `Permissions-Policy` that turns off the camera, the microphone,
geolocation and other device features.

### Upgrading

Built from the clone, pull the new code and build again:

```sh
git pull
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build
```

With the published images, set the new release in `NEON_VERSION`, then:

```sh
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d
```

Compose replaces the containers whose image changed. The new server applies the new
migrations before it takes requests, and `/readyz` answers `503` until it has. Migrations
only go forward, so take a backup first (see [below](#where-the-data-lives)): going back to
an older version means restoring the backup taken before the upgrade.

### Stopping

`docker compose -f deployments/docker-compose.yml stop`, or `down`, sends the server SIGTERM.
It then [shuts down in order](/docs/architecture/observability/#shutdown): `/readyz` turns
to `503`, open requests get up to 30 seconds to finish, then running jobs up to 30 more, and
the database pool closes and the telemetry flushes last. The Compose file gives it 90
seconds, its `stop_grace_period`, before Docker kills it. A job cut off by that is not lost:
it is queued again once its claim runs out, 15 minutes after a worker took it, and the
attempt counts.

`down` removes the containers and keeps the volumes, so `up` starts again with the same data.
`down --volumes` deletes the data too.

### Where the data lives

Everything lasting is in named volumes, which Docker keeps until they are deleted. Compose
prefixes their names with the project's, `neon`:

| Volume | What it holds |
|---|---|
| `neon_postgres` | The database: everything the instance knows. |
| `neon_caddy-data` | Caddy's certificates and keys, for `NEON_DOMAIN`. |
| `neon_caddy-config` | Caddy's saved configuration. |
| `neon_otel-lgtm` | With the `observability` profile, the metrics, traces, logs and profiles it collected. |

To back the database up into `neon.sql`, and to restore that over what the database holds:

```sh
docker compose -f deployments/docker-compose.yml exec -T postgres pg_dump -U neon --clean --if-exists neon > neon.sql
docker compose -f deployments/docker-compose.yml exec -T postgres psql -U neon -d neon < neon.sql
```

Stop the server before restoring, with
`docker compose -f deployments/docker-compose.yml stop server`, and start it again after.

### Development profiles

Compose profiles add services that development needs, next to the stack:

| Profile | What it adds |
|---|---|
| `mail` | [Mailpit](https://mailpit.axllent.org), which accepts mail on `mailpit:1025` and shows it at `http://127.0.0.1:8025/`. Point the server at it in `deployments/server.env`; see [Mail](/docs/self-hosting/mail/#development). |
| `observability` | Grafana with Prometheus, Tempo, Loki and Pyroscope; see [Observability stack](/docs/self-hosting/observability/). |

A profile goes on the command that starts the stack, with the same files, for example
`docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml --profile mail up -d --build`
in a clone. These are separate programs under their own licenses: Postgres under the PostgreSQL License,
Caddy under Apache-2.0, Mailpit under MIT, and the observability stack's Grafana, Loki,
Tempo and Pyroscope under AGPL-3.0. None of them is linked with Neon Teamspace's code.

### The end-to-end tests

`make e2e` builds both images from the checkout, starts a throwaway stack of them under its
own Compose project, on a free port of `127.0.0.1`, and runs the tests in `e2e/` against it
in a headless Chromium with Playwright: the app and its API, the docs, the security headers,
and that the admin listener is not reachable. The stack gets a first admin with a password
made for the run, and a secret key; the tests sign in as that admin, make an invite, sign up
with it in a second browser and sign out, and read the settings. It removes the stack, its data included,
however the run ends, and leaves Playwright's report in `e2e/playwright-report/`. It needs
Docker, Bun and Node.js; `make e2e-install`, which it runs first, installs the tests'
packages and the browser.

## From source

### What you need

- Rust through [rustup](https://rustup.rs). The repository's `rust-toolchain.toml` pins the
  version, and rustup installs it on the first build.
- [sqlx-cli](https://crates.io/crates/sqlx-cli), which applies the migrations and keeps the
  query cache:
  `cargo install sqlx-cli --locked --version 0.9.1 --no-default-features --features postgres,rustls`
- [cargo-deny](https://crates.io/crates/cargo-deny), for the license and advisory checks:
  `cargo install cargo-deny --locked`, then `cargo deny fetch` once, which downloads the
  RustSec advisory database that `make check` reads offline.
- [Bun](https://bun.sh), for the web app, these docs and the end-to-end tests.
- Docker, for Postgres, and for the stack the end-to-end tests run against.
- [Node.js](https://nodejs.org), which Playwright's test runner runs on, for `make e2e`.
- GNU make. The 3.81 that macOS ships is enough.

The steps below run the targets of the repository's `Makefile`, from the repository's root.
`make` on its own lists them all, each with a line on what it does.

### Postgres and Mailpit

The repository's development Compose file runs Postgres 18 with its data in a named volume,
and Mailpit, which takes the server's mail and shows it in a browser:

```sh
make db-up
```

Postgres listens on `127.0.0.1:55432`, reachable from this machine only, on a port that does
not clash with a Postgres installed the usual way. The user, password and database are all
`neon`, for development only. Mailpit takes mail on `127.0.0.1:51025` and shows it at
`http://127.0.0.1:58025/`; [Mail](/docs/self-hosting/mail/#development) has the settings that
send the server's mail there. `make db-down` stops both and keeps the database;
`docker compose -p neon-dev -f deployments/docker-compose.dev.yml down --volumes` deletes it
too.

### The server

Copy `.env.example` to `.env`, which git ignores, then create the tables:

```sh
cp .env.example .env
make migrate
```

`.env` sets `DATABASE_URL`, which sqlx's tools and the tests read from `.env` themselves, and
`NEON_DATABASE__URL`, the database the server uses. With `DATABASE_URL` set, the build checks
every SQL query against that database, so the tables must exist before the first build:
`make migrate` applies the migrations with sqlx-cli. Run it again after pulling a change that
adds one.

The server does not read `.env` itself. `make run` exports it, then starts the server:

```sh
make run
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
{"name":"Neon Teamspace","version":"0.1.0"}
```

The name is the [`instance.name`](/docs/reference/settings/) setting. Everything else the
server reads is in [Configuration](/docs/self-hosting/configuration/).

#### Commands

| Command | What it does |
|---|---|
| `neon serve` | Runs the server. It is what `neon` does when given no command. |
| `neon migrate` | Updates the database schema, then exits. |
| `neon openapi [--out PATH]` | Prints the API's OpenAPI description, or writes it to `PATH`. |
| `neon settings docs` | Prints the [settings reference](/docs/reference/settings/) as markdown, from the server's settings registry. |
| `neon config check` | Loads and checks the configuration from the config file and the environment, then prints it with secrets redacted. |
| `neon health [--live]` | Asks the admin listener's `/readyz`, or `/livez` with `--live`, within three seconds. It prints nothing and exits with status 0 when the answer is `200`; otherwise it prints why to standard error and exits with status 1. Container health checks run it, as the server image has no shell or `curl`. |

With cargo, the arguments go after `--`, as in `cargo run --bin neon -- migrate`. Like
`neon serve`, `neon migrate`, `neon config check` and `neon health` read the configuration,
so run them with `.env` exported (`set -a; . ./.env; set +a`) or next to a `neon.yaml`. In
the stack, `docker compose -f deployments/docker-compose.yml exec server neon health` runs
one in the server's container, with its configuration.

**Planned.** `neon admin create`, to create an instance admin from the command line.

### The web app

The app is in `web/`. Install its packages once, then start Vite's development server, which
reloads as files change:

```sh
make web-install
make web-dev
```

It is then at `http://localhost:5173/`. The development server passes `/api` and the
server's other paths on to the server on port 8080, so in development, too, the app and the
API share one origin.

### These docs

The docs are in `docs/`. Their diagrams are drawn as the docs build, in a headless Chromium
that Playwright runs, so installing their packages installs that too:

```sh
make docs-install
make docs-dev
```

They are then at `http://localhost:4321/docs/`. `make docs-build` builds them into
`docs/dist/`, and `docs/README.md` covers the rest.

### The end-to-end tests' packages

The tests in `e2e/` have packages of their own, which `make check` checks too, and drive a
headless Chromium of their own. Install both once:

```sh
make e2e-install
```

`make e2e` then runs them against a throwaway stack of images built from the checkout; see
[above](#the-end-to-end-tests).

### Checks

```sh
make check
```

`make check` runs everything that needs neither Docker nor, once the crates, the packages and
the advisory database are downloaded, the network: `cargo fmt --check`, clippy with warnings
as errors, a build, the checks that `api/openapi.json`, the web app's API types and the
settings reference are current, the web app's, the docs' and the end-to-end tests' own
checks, and the license and advisory checks, which read the packages installed in `web/`,
`docs/` and `e2e/`. Its build checks the SQL against the committed `.sqlx/` cache instead of a
database; see [Data and storage](/docs/architecture/data/#queries). It builds in
`target/offline/`, because cargo does not rebuild a crate when only `SQLX_OFFLINE` changes:
in `target/`, it could reuse queries last checked against the database instead of the cache.
cargo-deny reads the advisory database it last fetched and fails once that copy is more than
90 days old; `make licenses` fetches a new one.

Two checks need Postgres:

```sh
make test
make sqlx-check
```

Tests that use the database each get a fresh one, which `#[sqlx::test]` creates on the
server `DATABASE_URL` points at. `make sqlx-check` fails when `.sqlx/` no longer matches the
queries.

### Generated files

The repository commits a few files generated from the code. A check fails when one of them
falls behind, and a target rewrites it:

| File | Rewrite | After changing |
|---|---|---|
| `api/openapi.json` | `make api-spec` | a route, or a type a route uses |
| `web/src/api/schema.d.ts` | `make web-api` | `api/openapi.json` |
| `.sqlx/` | `make sqlx-prepare` | a query or a migration |
| `docs/src/content/docs/reference/settings.md` | `make docs-gen` | a runtime setting, added or changed |
