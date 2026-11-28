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
| `ghcr.io/lein3000zzz/neon-teamspace-web` | Caddy, serving the app at `/` and these docs at `/docs`, and passing `/api`, `/ws`, `/mcp`, `/hooks` and `/.well-known/` on to the server, all on one origin. Given a domain, it gets and renews its TLS certificate itself. The app's and the docs' license notices are served at `/THIRD_PARTY_NOTICES.txt` and `/docs/THIRD_PARTY_NOTICES.txt`, and Caddy's, with those of the Go modules it is built from, are in `/usr/share/doc/caddy/THIRD_PARTY_NOTICES.txt`. |

A third image runs only with the `updater` [profile](#profiles):

| Image | What it holds |
|---|---|
| `ghcr.io/lein3000zzz/neon-teamspace-updater` | The `neon-updater` binary beside the docker CLI, its Compose plugin and cosign, on Docker's official Alpine image of the CLI. It updates the server and web to a release when an instance admin asks, as [One-click updates](/docs/self-hosting/upgrading/#one-click-updates) describes. The license notices of the crates `neon-updater` is built from are in `/usr/share/doc/neon-updater/THIRD_PARTY_NOTICES.txt`, and those of the docker CLI, its plugins and cosign, with the Go modules each is built from, in `/usr/share/doc/docker/` and `/usr/share/doc/cosign/`. |

You need Docker Engine 25 or later with Compose 2.24.4 or later, and the Compose file with
the example settings beside it: from a release's Compose archive, to run the published
images, or from a clone of the repository, to build both images from it.

### Starting it

Each release on GitHub attaches its Compose archive, `neon-teamspace-compose-` and the version
`.tar.gz`. It holds the Compose file, the example settings, the observability profile's
Grafana files and the license, at the paths they have in the repository, so every command on
this page works from the directory it is unpacked in. The Caddyfile is not in it: it is part
of the web image, which checks it as it builds. Unpack the archive into a directory of its
own, copy the example settings to `deployments/.env`, set the database's password in it, and
start the stack:

```sh
mkdir neon-teamspace && cd neon-teamspace
curl -fsSL https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.2.0/neon-teamspace-compose-0.2.0.tar.gz | tar -xz --strip-components=1
cp deployments/.env.example deployments/.env
openssl rand -hex 24       # a password: put it in deployments/.env as POSTGRES_PASSWORD
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d
```

The instance is then at `http://127.0.0.1:3000/`, and these docs at
`http://127.0.0.1:3000/docs/`; [the first run](#the-first-run) makes its first admin. The
Compose file runs the images of the release it belongs to, which its `image` lines name, and
`NEON_VERSION` in `deployments/.env` names another. It has no build sections, so an image
that cannot be pulled, such as one of a mistyped version, stops the start instead of being
built from whatever the directory holds.

Neither the download nor the pull needs a GitHub account. The GitHub CLI, signed in, can
fetch the archive instead, for `tar` to unpack:

```sh
gh release download v0.2.0 --repo lein3000zzz/neon-teamspace --pattern 'neon-teamspace-compose-*.tar.gz'
tar -xzf neon-teamspace-compose-0.2.0.tar.gz --strip-components=1
```

In a clone, `docker-compose.build.yml` beside the Compose file builds both images from the
checkout instead, after the same `deployments/.env`:

```sh
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build
```

The first start builds the images, which takes some minutes. They are named
`neon-teamspace-server:local` and `neon-teamspace-web:local`, never after a release. With
`make`, `make up` builds and runs the same stack in the foreground, until Ctrl+C stops it,
and `make down` stops it from another terminal.

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

### The first run

A new instance has no accounts. Make its first admin in the server's container, with
`neon admin create`, which asks for the password twice:

```sh
docker compose -f deployments/docker-compose.yml exec server neon admin create --email you@example.com --display-name You
```

Or set `NEON_ADMIN__EMAIL` and `NEON_ADMIN__PASSWORD` in `deployments/server.env`, a copy
of `deployments/server.env.example`: the server makes that admin as it starts, while no
admin can sign in. [Accounts and registration](/docs/guides/accounts/#the-first-admin) has
the rules of both. Set `NEON_SECRETS__KEYS` in the same file too, with the key its comment
shows how to make: it seals the secrets the database keeps, so admins can copy invite codes
again and mail can go out. A change to `server.env` applies once `docker compose -f deployments/docker-compose.yml up -d`
runs again.

Then, in a browser:

1. Open `http://127.0.0.1:3000/` and sign in as the admin. An account in no workspace gets a
   page that says so: **Create a workspace** there asks for a name, and the workspace comes
   with a teamspace named General. [Workspaces and teamspaces](/docs/guides/workspaces/)
   covers the rest.
2. **+** on General's row in the sidebar makes a page; [Editing pages](/docs/guides/editing-pages/)
   covers the editor.
3. Drop a file onto the page, or paste an image: it uploads into a block of its own, as
   [Attachments](/docs/guides/attachments/) describes.
4. For an MCP client, under **Workspace settings**, **Agents** makes an agent, and its card a
   token for it with the `read` and `write` scopes. [Agents over
   MCP](/docs/guides/agents-over-mcp/#connecting-with-a-token) has the client's configuration,
   with `http://127.0.0.1:3000/mcp` as its URL here. Claude Code connects with the
   [Claude Code plugin](/docs/guides/claude-code-plugin/), over OAuth, with no token to copy.

The foot of the sidebar shows the instance's name and the server's version, which
`/api/v1/meta` also answers:

```sh
curl -s http://127.0.0.1:3000/api/v1/meta
```

```json
{"product":"neon-teamspace","name":"Neon Teamspace","version":"0.2.0"}
```

### `deployments/.env` and `deployments/server.env`

The settings are in two files beside the Compose file, both ignored by git. Compose reads
`deployments/.env` for the stack's own settings, which `deployments/.env.example` describes:

| Variable | What it sets |
|---|---|
| `POSTGRES_PASSWORD` | The database's password. Required. Postgres reads it once, when its volume is created, so changing it later takes an `ALTER USER` in the database too. Letters and digits only, as it goes into the server's database URL. |
| `NEON_VERSION` | The release of the published images to run, the same for each, when not the one the Compose file belongs to, which it runs unless set: a version such as `0.2.0`, a minor version such as `0.2`, or `latest`; [Upgrading](/docs/self-hosting/upgrading/#releases) says what each tag follows. Images built from the clone ignore it. |
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

Before the first start, `server.env` wants three things a new instance needs:

```sh
NEON_SERVER__PUBLIC_URL=https://neon.example.com
NEON_SECRETS__KEYS=2026:<output of openssl rand -base64 32>
NEON_ADMIN__EMAIL=you@example.com
NEON_ADMIN__PASSWORD=<a password of 12 characters or more>
```

The public URL is the address people type, exactly: `http://127.0.0.1:3000` for the stack on
its default port, or the domain of [below](#a-domain-and-tls). The
[secret key](/docs/self-hosting/configuration/#secret-keys) seals what the database keeps
secret, mail and webhooks need it, and it belongs in the backups, apart from the database's.
The first admin is made as the server starts, while no admin can sign in. Without the two
`ADMIN` variables, `neon admin create` makes one in the running stack instead, as
[the first run](#the-first-run) shows.

The stack sets four of the server's keys itself, over anything `server.env` says:

- `database.url`, from `POSTGRES_PASSWORD`;
- `server.trusted_proxies`, the private address ranges, as only the stack's containers reach
  the server;
- `observability.admin.addr`, which it sets to `0.0.0.0:9090`;
- `updates.updater_token`, from `NEON_UPDATER_TOKEN` in `deployments/.env`, even when that is
  blank: the updater's token goes there, as
  [Turning it on](/docs/self-hosting/upgrading/#turning-it-on) shows, and not in `server.env`.

On that address, the admin listener answers on the stack's own network, for Prometheus to
scrape, and is never published. The server logs a warning at start that it is not bound to
loopback; in the stack, that is meant. Caddy never passes the admin listener's paths on, so
`/metrics`, `/readyz` and `/debug/pprof` on the instance's address are routes of the app
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
takes the client's address from the proxy's `X-Forwarded-For`. Caddy reads that header from
its end, skips the addresses of the proxies it trusts, and stops at the first that is none of
theirs: that is the client, and whatever the client wrote into the header before it is never
read. A proxy that appends the address it saw, as most do, works so; one that passes the
header on as the client sent it, without its own entry, would let the client choose. The
server only ever sees the client's address, as Caddy works it out.
`Strict-Transport-Security` is then the proxy's to set. The proxy must also pass WebSocket
upgrades on, for the live editing at `/ws`, as most do by default, and pass the server-sent
events of `/mcp` on as they come, without buffering them, for the notifications of MCP
clients.

`NEON_SERVER__PUBLIC_URL` must be the address browsers use: the server refuses requests that
change something, and WebSocket upgrades, whose `Origin` is another.

Caddy sets the same security headers on every answer: a Content-Security-Policy that allows
only the instance's own scripts, styles, images, fonts and connections, with a policy of
their own for these docs; `X-Content-Type-Options: nosniff`; `X-Frame-Options: DENY`; a
`Referrer-Policy`; and a `Permissions-Policy` that turns off the camera, the microphone,
geolocation and other device features.

### Upgrading

A new release brings its own Compose archive: unpacked over the old one, it brings its images
with it. The new server applies the new migrations before it takes requests, and they only go
forward, so going back to an older release means restoring the backup taken before the
upgrade. [Upgrading](/docs/self-hosting/upgrading/) has the steps, the backup to take first,
how migrations run and fail, rolling back, and how to check the result. A server that was
killed rather than stopped may leave pages whose latest text search does not read yet;
`neon search reindex` stores it, as [Search](/docs/guides/search/#rebuilding-the-text)
explains.

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
| `neon_postgres` | The database: everything the instance knows but attachments' bytes. |
| `neon_server-data` | The server's data directory, `/var/lib/neon`: attachments, while they are kept on local disk. |
| `neon_caddy-data` | Caddy's certificates and keys, for `NEON_DOMAIN`. |
| `neon_caddy-config` | Caddy's saved configuration. |
| `neon_otel-lgtm` | With the `observability` profile, the metrics, traces, logs and profiles it collected. |
| `neon_seaweedfs` | With the `seaweedfs` profile, the files SeaweedFS keeps, attachments among them once the server is set to use it. |

A backup takes the database, the attachments' files and the secret keys in
`deployments/server.env`, which open the secrets the database keeps sealed.
[Backups and restore](/docs/self-hosting/backups/) has the commands, the restore onto a new
machine, and what a backup without one of the three loses.

### Profiles

Compose profiles add services next to the stack:

| Profile | What it adds |
|---|---|
| `mail` | [Mailpit](https://mailpit.axllent.org), which accepts mail on `mailpit:1025` and shows it at `http://127.0.0.1:8025/`. Point the server at it in `deployments/server.env`; see [Mail](/docs/self-hosting/mail/#development). |
| `observability` | Grafana with Prometheus, Tempo, Loki and Pyroscope; see [Observability stack](/docs/self-hosting/observability/). |
| `seaweedfs` | [SeaweedFS](https://github.com/seaweedfs/seaweedfs), an S3-compatible store for attachments, on `seaweedfs:8333` inside the network; see [File storage](/docs/self-hosting/storage/#seaweedfs-in-the-compose-stack). |
| `updater` | The updater, on `updater:8080`, reachable from the server alone, which updates the server and web to a release when an instance admin presses **Update** in **Admin > Settings**. It holds the Docker socket, and needs `NEON_UPDATER_TOKEN` in `deployments/.env`; see [One-click updates](/docs/self-hosting/upgrading/#one-click-updates). |

A profile goes on the command that starts the stack, with the same files, for example
`docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml --profile mail up -d --build`
in a clone. `COMPOSE_PROFILES` in `deployments/.env`, such as `COMPOSE_PROFILES=updater`,
turns profiles on instead, for every command that names none with `--profile`, which replaces
it: list every profile there, such as `COMPOSE_PROFILES=updater,mail`. These are separate
programs under their own licenses: Postgres under the PostgreSQL License, Caddy, SeaweedFS,
the docker CLI and its plugins, and cosign under Apache-2.0, Mailpit under MIT, and the
observability stack's Grafana, Loki, Tempo and Pyroscope under AGPL-3.0. None of them is
linked with Neon Teamspace's code.

### The end-to-end tests

`make e2e` builds the images from the checkout, checks that the updater image states the
checkout's version and holds the programs it runs, starts a throwaway stack of the server and
web images under its own Compose project, on a free port of `127.0.0.1` (or the one `E2E_WEB_PORT` names), and
runs the tests in `e2e/` against it in a headless Chromium with Playwright. The stack gets a
first admin with a password made for the run, a secret key, its public URL on that port, and
Mailpit, which takes all its mail and whose API the tests read. The tests start from that
admin and make people of their own. They cover signing up in each registration mode,
verifying an address and resetting a password with the links mailed, and requests to join,
approved or denied; workspaces, joined by invitation and by link; pages edited in two
browsers at once, shared and trashed; databases and boards; comments, mentions, the inbox
and the mail it sends; search and page history; attachments, and who may download them;
import and export; agents, OAuth and MCP clients; webhooks delivered to a receiver on the
stack's network; the layout on a phone's screen; the docs, the security headers, and that
the admin listener is not reachable. The `phone-*.spec.ts` specs run with an iPhone's and an
Android phone's screen, touch and browser name, in the same Chromium: they sign in, find
pages from the drawer and the search, edit and rearrange a page from the toolbar above the
keyboard, comment, open the inbox and edit a board's cards. It removes the stack, its data
included, however the run ends, and leaves Playwright's report in `e2e/playwright-report/`.
It needs Docker, Bun and Node.js; `make e2e-install`, which it runs first, installs the
tests' packages and the browser. The server image it tests is built with Cargo's `e2e`
profile, the release profile without link-time optimisation, so it compiles in a fraction of
the time; the published images are built with `release`, which
`make e2e E2E_CARGO_PROFILE=release` tests.

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
{"product":"neon-teamspace","name":"Neon Teamspace","version":"0.2.0"}
```

The name is the [`instance.name`](/docs/reference/settings/) setting. Everything else the
server reads is in [Configuration](/docs/self-hosting/configuration/).

#### Commands

| Command | What it does |
|---|---|
| `neon serve` | Runs the server. It is what `neon` does when given no command. |
| `neon migrate` | Updates the database schema, then exits. |
| `neon admin create --email ADDRESS [--display-name NAME]` | Creates an instance admin, whatever the registration mode. It asks for the password at a terminal, or reads it from the first line of standard input. |
| `neon search reindex [--all]` | Reads the documents of the pages whose text [search](/docs/guides/search/#rebuilding-the-text) lacks or whose latest changes it lacks, or of every page with `--all`, and stores their text where it differs. Safe while servers run. |
| `neon openapi [--out PATH]` | Prints the API's OpenAPI description, or writes it to `PATH`. |
| `neon settings docs` | Prints the [settings reference](/docs/reference/settings/) as markdown, from the server's settings registry. |
| `neon config check` | Loads and checks the configuration from the config file and the environment, then prints it with secrets redacted. |
| `neon health [--live]` | Asks the admin listener's `/readyz`, or `/livez` with `--live`, within three seconds. It prints nothing and exits with status 0 when the answer is `200`; otherwise it prints why to standard error and exits with status 1. Container health checks run it, as the server image has no shell or `curl`. |

With cargo, the arguments go after `--`, as in `cargo run --bin neon -- migrate`. Like
`neon serve`, `neon migrate`, `neon admin create`, `neon search reindex`, `neon config check`
and `neon health` read the configuration, so run them with `.env` exported (`set -a; . ./.env; set +a`) or
next to a `neon.yaml`. In
the stack, `docker compose -f deployments/docker-compose.yml exec server neon health` runs
one in the server's container, with its configuration.

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
as errors, the Rust docs with warnings as errors, a build, the checks that `api/openapi.json`,
the web app's API types and the settings reference are current, the web app's, the docs' and
the end-to-end tests' own checks, the block fixtures in both directions, the Claude Code
plugin, the version check, which fails unless everything that states the version states the
cargo workspace's, and the license and advisory checks, which read the packages installed in
`web/`, `docs/`, `e2e/` and `tools/blocknote-fixtures/`. `make blocks-install` installs the last of those. Its build checks the SQL against the committed `.sqlx/` cache instead of a
database; see [Data and storage](/docs/architecture/data/#queries). It builds in
`target/offline/`, so that switching between it and the builds checked against the database
does not rebuild `neon-store` and every crate above it each time: `neon-store`'s build script
makes each build read the right source, the cache or the database.
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

The S3 blob store's test needs an S3-compatible server, and is skipped without one.
`make test-s3` runs it against SeaweedFS in a throwaway container, on a free port of
`127.0.0.1`, and removes the container however the run ends; it needs Docker.

### Generated files

The repository commits a few files generated from the code. A check fails when one of them
falls behind, and a target rewrites it:

| File | Rewrite | After changing |
|---|---|---|
| `api/openapi.json` | `make api-spec` | a route, or a type a route uses |
| `web/src/api/schema.d.ts` | `make web-api` | `api/openapi.json` |
| `.sqlx/` | `make sqlx-prepare` | a query or a migration |
| `docs/src/content/docs/reference/settings.md` | `make docs-gen` | a runtime setting, added or changed |
| `crates/blocks/tests/fixtures/blocknote/` and `rust-out/` | `make blocks-fixtures` | the fixture corpus, `crates/blocks`, the editor's own blocks, or the BlockNote version |
