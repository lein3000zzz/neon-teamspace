# Neon Teamspace

A self-hosted team workspace with block-based pages, databases and kanban boards, real-time
collaborative editing, and AI agents as first-class teammates over MCP.

One deployment hosts many workspaces. Each workspace has teamspaces, nested pages and
databases. Agents connect over MCP, pick up tasks, comment and edit pages next to people.

> **Status:** early development. Nothing here is usable yet.

## Repository layout

| Path | What it holds |
|---|---|
| `crates/domain` | Domain types, validation and permission rules. No I/O. |
| `crates/blocks` | A page's block document: its model, read from and written to the page's Yjs document, and markdown. No I/O. |
| `crates/store` | Postgres repositories. |
| `crates/app` | Use cases, transactions, and the ports for side effects. |
| `crates/observability` | Logs, metrics, traces, profiles, the admin listener and ordered shutdown. |
| `crates/server` | The `neon` binary: HTTP API, realtime sync, MCP, webhooks, wiring. |
| `api/openapi.json` | The HTTP API description, generated from the server code. |
| `web` | The web app: React, Vite and TanStack. Its `Dockerfile` and `Caddyfile` make the web image. |
| `docs` | The documentation site, Astro Starlight. |
| `deployments` | The server image's `Dockerfile`, the Compose files, and the observability stack's Grafana and Prometheus files. |
| `e2e` | End-to-end tests of the whole stack, with Playwright. |
| `scripts` | Tooling shared by the web app, the docs and the tests, such as the license check. |
| `tools/blocknote-fixtures` | A Bun project that runs BlockNote on the server, to write and check `crates/blocks`' fixtures. |

Dependencies only point inward: `server` → `app` → `store` → `domain`. `blocks` stands
beside `domain`, depending on neither. Each crate's `Cargo.toml` lists what it may use, so the
compiler enforces the direction.

## Running it with Docker

Two images make up an instance, both for `linux/amd64` and `linux/arm64`:
`ghcr.io/lein3000zzz/neon-teamspace-server`, the `neon` binary on distroless, and
`ghcr.io/lein3000zzz/neon-teamspace-web`, Caddy with the app and the docs, which passes the
server's paths on to it on the same origin. `deployments/docker-compose.yml` runs them with
Postgres. On its own it pulls the published images of the release `NEON_VERSION` names;
`deployments/docker-compose.build.yml` added to it builds both from the checkout instead,
under local names:

```sh
cp deployments/.env.example deployments/.env   # then set POSTGRES_PASSWORD in it
make up                                        # http://127.0.0.1:3000/, Ctrl+C stops it
```

`make up` builds the images and runs the stack in the foreground; `make down` stops it and
keeps the data. Without make,
`docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build`
runs it in the background. `deployments/.env.example` lists the stack's settings, among them
`NEON_DOMAIN`, a domain for which Caddy gets its TLS certificate itself. The server's own
settings go in `deployments/server.env`, which only the server reads; `server.env.example`
beside it lists the usual ones. The
[install page](docs/src/content/docs/self-hosting/install.md) covers both files, the
published images, domains and TLS, upgrades, stopping, and where the data lives.

The Compose profiles `mail` (Mailpit) and `observability` (Grafana's otel-lgtm image, with
a dashboard of the server's metrics) add services for development. `make obs-up` starts the
observability stack on its own, beside `make run`; see
[its page](docs/src/content/docs/self-hosting/observability.md).

`make e2e` builds both images, starts a throwaway stack of them on a free local port, runs
the Playwright tests in `e2e/` against it, and removes it, leaving the report in
`e2e/playwright-report/`. It needs Docker, Bun and Node.js, and installs the tests' packages
and headless Chromium itself. The stack is the Compose project `neon-e2e` and its images are
tagged `local`; `E2E_PROJECT` and `E2E_TAG` name others, so two runs on one machine keep
apart: `make e2e E2E_PROJECT=neon-e2e-2 E2E_TAG=e2e-2`. The images stay after the run, under
their tag, for the build cache.

## Development

Prerequisites:

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the version.
- [sqlx-cli](https://crates.io/crates/sqlx-cli), for migrations and the query cache:
  `cargo install sqlx-cli --locked --version 0.9.1 --no-default-features --features postgres,rustls`
- [cargo-deny](https://crates.io/crates/cargo-deny), for the license and advisory checks:
  `cargo install cargo-deny --locked`, then `cargo deny fetch` once, which downloads the
  RustSec advisory database that `make check` reads offline.
- [Bun](https://bun.sh), for the web app, the docs and `scripts/`.
- Docker, for Postgres.
- GNU make. The 3.81 that macOS ships is enough.

The root `Makefile` runs every task below. `make` on its own lists its targets.

Start Postgres, then point the tools at it. The compose file binds Postgres to
`127.0.0.1:55432` with credentials meant only for development, and `.env` holds the
matching URLs. Git ignores `.env`. It also starts Mailpit, which shows the server's mail at
`http://127.0.0.1:58025/`; the commented block at the end of `.env.example` sends mail
there, as the [mail page](docs/src/content/docs/self-hosting/mail.md#development) explains.

```sh
make db-up                 # make db-down stops it and keeps the data
cp .env.example .env
make migrate               # creates the tables
```

Once `DATABASE_URL` is set, in the shell or in `.env`, the build checks every SQL query
against that database, so the tables must exist before the first build. That is why
`make migrate`, which runs sqlx-cli, comes first. Run it again after pulling a change that
adds a migration. Without sqlx-cli, the server can apply them. It does not read `.env`
itself, so export the file first; `SQLX_OFFLINE=true` builds from the query cache in `.sqlx/`
instead of the database:

```sh
set -a; . ./.env; set +a   # export the variables to this shell
SQLX_OFFLINE=true cargo run -p neon-server -- migrate
```

Then run the server. `make run` exports `.env` and starts `neon serve`, which listens on
`0.0.0.0:8080`, applies pending migrations as it starts, and serves its admin listener
(metrics, health checks) on `127.0.0.1:9090`.

```sh
make run
curl localhost:8080/api/v1/meta
```

The web app and the docs each run a development server that reloads as files change. Install
their packages once; `make docs-install` also installs the headless Chromium that draws the
docs' diagrams.

```sh
make web-install
make web-dev               # http://localhost:5173/, passing /api and the server's paths to :8080
make docs-install
make docs-dev              # http://localhost:4321/docs/
make e2e-install           # the end-to-end tests' packages, which make check checks too
make blocks-install        # BlockNote, for the block fixtures, which make check checks too
```

`make check` runs every check that needs no Docker and, once the crates, the packages and
the advisory database are downloaded, no network: formatting, clippy, a build from the
`.sqlx/` cache, the generated files, the web app's, the docs' and the end-to-end tests'
checks, the block fixtures in both directions, and the license and advisory checks. Its cargo commands build in `target/offline/`: cargo does not rebuild a
crate when only `SQLX_OFFLINE` changes, so in `target/` the build could reuse queries last
checked against the database instead of the cache. cargo-deny reads the advisory database it
last fetched and fails once that copy is more than 90 days old; `make licenses` fetches a new
one. The Rust tests and the query cache check need Postgres. Each database test creates a
fresh database through `DATABASE_URL`.

```sh
make check
make test
make sqlx-check
```

The `neon` binary has these commands:

| Command | What it does |
|---|---|
| `neon serve` | Runs the server. The default when no command is given. |
| `neon migrate` | Applies pending migrations and exits. |
| `neon admin create --email ADDRESS` | Creates an admin account, whatever the registration mode. Asks for the password at a terminal, or reads it from stdin. |
| `neon openapi [--out PATH]` | Prints the OpenAPI description, or writes it to `PATH`. |
| `neon settings docs` | Prints the runtime settings reference as markdown. |
| `neon config check` | Loads and checks the configuration, then prints it with secrets redacted. |
| `neon health [--live]` | Asks the admin listener's `/readyz`, or `/livez`, and exits 0 when it answers 200, else 1. For container health checks. |

### Generated files

The repository commits these generated files. Each has a target that rewrites it and a check
that fails when it is stale.

| File | Rewrite it after changing | Rewrite | Check |
|---|---|---|---|
| `api/openapi.json` | a route, or a type a route uses | `make api-spec` | `make api-check`, and a server test |
| `web/src/api/schema.d.ts` | `api/openapi.json` | `make web-api` | `make web-check` |
| `.sqlx/`, the query cache that lets the build check SQL without a database | a query or a migration | `make sqlx-prepare` | `make sqlx-check` |
| `docs/src/content/docs/reference/settings.md` | a runtime setting, added or changed | `make docs-gen` | `make docs-check`, and a server test |
| `crates/blocks/tests/fixtures/blocknote/` and `rust-out/`, the Yjs updates and blocks BlockNote and `neon-blocks` write for the fixture corpus | the corpus, `crates/blocks`, the configs of the editor's own blocks in `web/src/components/editor/custom-blocks.ts`, or the BlockNote version in `tools/blocknote-fixtures` | `make blocks-fixtures` | `make blocks-check` |

The web app's route tree, `web/src/routeTree.gen.ts`, is generated and committed too. It has
no target: the TanStack Router plugin rewrites it whenever `make web-dev` or `make web-build`
runs, so commit it with the route change.

### Continuous integration

GitHub Actions runs three workflows.

- **CI** runs on every pull request into `dev` or `main` and on every push to them. A newer push cancels the older run on the same branch or pull request.
  - A changes job starts only the jobs the changed files affect. A change to the Makefile or to a workflow starts all of them.
  - **Rust:** `make fmt-check`, `cargo deny check`, `make migrate`, `make lint`, `make test`, `make sqlx-check` and `make api-check`, against a Postgres 18 service container.
  - **Web app:** `make web-install web-check web-build` and the license check for `web/`.
  - **Docs:** the packages and headless Chromium, the docs' type and script checks, `make docs-build`, and the license check for `docs/`.
  - **E2E test code:** `make e2e-check` and the license check for `e2e/`.
  - **Block fixtures:** `make blocks-check`, which tests `neon-blocks` against the updates BlockNote wrote and has BlockNote load the ones `neon-blocks` wrote, and the license check for `tools/blocknote-fixtures/`.
  - **Secret scan:** gitleaks over the run's new commits.
  - **All checks** passes when every other job passed or was skipped. It is the one check branch protection needs to require.
- **E2E** runs `make e2e`: it builds both images, starts a throwaway Compose stack and runs the Playwright tests against it.
  - It runs on every pull request into `main`, and on pull requests into `dev` that touch the images, Caddy, Compose, the end-to-end tests or the Makefile.
  - It can also be started by hand from the Actions tab.
  - When the tests fail, the Playwright report is attached to the run.
- **Release** runs on a `v*` tag.
  - It runs the end-to-end tests, then builds the server and web images for `linux/amd64` and `linux/arm64` with an SBOM and build provenance.
  - It pushes them to `ghcr.io/lein3000zzz/neon-teamspace-server` and `-web`, then creates a GitHub release with generated notes.
  - Images are signed with cosign keyless signing only once the repository is public, because keyless signing records the repository's identity in Sigstore's public logs.

Dependabot opens one grouped pull request per ecosystem each month, targeting `dev`. A few pins in the workflows are bumped by hand; `.github/dependabot.yml` lists them.

## Licenses

Every dependency must be under a license that `deny.toml` allows. `make licenses` runs all of
cargo-deny's checks on the crates (licenses, advisories, bans and sources), then checks the
packages installed in `web/`, `docs/`, `e2e/` and `tools/blocknote-fixtures/` with
`scripts/licenses.ts`, which reads the same allow list. A package outside it needs a build-only exception in `scripts/licenses/policy.ts`,
with the reason none of it ships.

What ships carries its dependencies' license notices. The web app's and the docs' builds
write `THIRD_PARTY_NOTICES.txt` beside their files, served at `/THIRD_PARTY_NOTICES.txt` and
`/docs/THIRD_PARTY_NOTICES.txt`. The server image holds the crates' notices at
`/usr/share/doc/neon/THIRD_PARTY_NOTICES.txt`, which [cargo-about](https://github.com/EmbarkStudios/cargo-about)
writes as the image builds, as `about.toml` sets out; its accepted licenses are the allow list
of `deny.toml`, and a test keeps the two the same.

### Configuration

The server reads `neon.yaml` from its working directory, or the file that `NEON_CONFIG`
names. [`neon.example.yaml`](neon.example.yaml) lists every key with its default, as a
server test checks for the runtime settings. Copy it
to `neon.yaml`, which git ignores.

An environment variable can set any key. Its name is `NEON_` followed by the key's path in
capitals, with `__` between levels, so `database.url` is `NEON_DATABASE__URL`. Other tools
use the `NEON_` prefix too, such as Neon's Postgres tooling with `NEON_API_KEY`, so a
variable that sets no key only gets a warning naming it. An unknown key in the file, or
an unknown `NEON_SETTINGS__` variable, stops the start, since it is almost always a typo.

For the `server` and `database` sections, the environment wins over the file, and the file
wins over the defaults. Runtime settings, under `settings`, have one more layer. They
resolve in this order:

1. the environment
2. the file
3. the value saved on the admin page
4. the default

A setting that the environment or the file sets is pinned. The admin page shows it but
cannot change it. `neon settings docs` lists every setting.

## License

[PolyForm Noncommercial 1.0.0](LICENSE).
