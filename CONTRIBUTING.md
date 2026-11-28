# Contributing to Neon Teamspace

This page covers what a change needs and the `Makefile` does not say: the development setup,
the generated files, the checks a pull request must pass, the conventions, and the
dependencies' licenses. `make` on its own lists every target, each with a line on what it
does. Vulnerabilities are reported privately instead; see [Security issues](#security-issues).

## Repository layout

| Path | What it holds |
|---|---|
| `crates/domain` | Domain types, validation and permission rules. No I/O. |
| `crates/blocks` | A page's block document: its model, read from and written to the page's Yjs document, and markdown. No I/O. |
| `crates/store` | Postgres repositories. |
| `crates/app` | Use cases, transactions, and the ports for side effects. |
| `crates/observability` | Logs, metrics, traces, profiles, the admin listener and ordered shutdown. |
| `crates/server` | The `neon` binary: HTTP API, realtime sync, MCP, webhooks, wiring. |
| `migrations` | The database's migrations, plain SQL, built into the binary, which applies them as it starts. |
| `.sqlx` | The query cache, which lets the build check SQL without a database. |
| `api/openapi.json` | The HTTP API description, generated from the server code. |
| `web` | The web app: React, Vite and TanStack. Its `Dockerfile` and `Caddyfile` make the web image. |
| `apps/shell` | The desktop and mobile apps: a Tauri project whose bundled server picker opens the chosen server's web app. Its Rust side, in `src-tauri/`, is a Cargo workspace of its own. |
| `docs` | The documentation site, Astro Starlight. |
| `deployments` | The server image's `Dockerfile`, the Compose files, and the observability stack's Grafana and Prometheus files. |
| `e2e` | End-to-end tests of the whole stack, with Playwright. |
| `scripts` | Tooling shared by the web app, the docs and the tests: the version check, the license check and the Claude Code plugin's check. |
| `.claude-plugin`, `integrations/claude-code` | The Claude Code plugin marketplace, and its plugin: the MCP server's configuration and the skills. |
| `tools/blocknote-fixtures` | A Bun project that runs BlockNote on the server, to write and check `crates/blocks`' fixtures. |
| `.github` | The CI, end-to-end and release workflows, Dependabot's configuration and the security policy. |

Dependencies only point inward: `server` → `app` → `store` → `domain`. `blocks` stands beside
`domain`, depending on neither. Each crate's `Cargo.toml` lists what it may use, so the
compiler enforces the direction. The [architecture overview](docs/src/content/docs/architecture/overview.md)
goes further.

## Development setup

### What you need

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the version, and
  rustup installs it on the first build.
- [sqlx-cli](https://crates.io/crates/sqlx-cli), for the migrations and the query cache:
  `cargo install sqlx-cli --locked --version 0.9.1 --no-default-features --features postgres,rustls`
- [cargo-deny](https://crates.io/crates/cargo-deny), for the license and advisory checks:
  `cargo install cargo-deny --locked`, then `cargo deny fetch` once, which downloads the
  RustSec advisory database that `make check` reads offline.
- [cargo-about](https://crates.io/crates/cargo-about), for the apps' license notices, which
  every build of the apps' server picker writes: `cargo install cargo-about --locked --features cli`.
- [Bun](https://bun.sh), for the web app, the docs, the end-to-end tests, the apps' picker
  and `scripts/`.
- Docker, for Postgres and for the stack the end-to-end tests run against.
- [Node.js](https://nodejs.org), which Playwright's test runner runs on, for `make e2e`.
- GNU make 3.81 or later, which macOS ships.
- [Claude Code](https://code.claude.com), optionally: `make plugin-check` runs its plugin
  validation when `claude` is installed, and skips it otherwise.

### Postgres, Mailpit and the server

`make db-up` starts the development Compose file: Postgres 18 on `127.0.0.1:55432`, with
credentials meant only for development, and Mailpit, which shows the server's mail at
`http://127.0.0.1:58025/`. `.env.example` holds the matching URLs, and its commented block at
the end sends mail to Mailpit, as the [mail page](docs/src/content/docs/self-hosting/mail.md#development)
explains. Git ignores `.env`.

```sh
make db-up                 # make db-down stops it and keeps the data
cp .env.example .env
make migrate               # creates the tables
make run                   # the server on :8080
```

With `DATABASE_URL` set, in the shell or in `.env`, the build checks every SQL query against
that database, so the tables must exist before the first build. That is why `make migrate`,
which runs sqlx-cli, comes first. Run it again after pulling a change that adds a migration.
Without sqlx-cli, `neon migrate` applies them, built with `SQLX_OFFLINE=true` so the build
checks its SQL against the query cache in `.sqlx/` instead. The server does not read `.env`,
so export the file first:

```sh
set -a; . ./.env; set +a   # export the variables to this shell
SQLX_OFFLINE=true cargo run -p neon-server -- migrate
```

`make run` exports `.env` and starts `neon serve`, which listens on `0.0.0.0:8080`, applies
pending migrations as it starts, and serves its admin listener, with metrics and health
checks, on `127.0.0.1:9090`. `curl localhost:8080/api/v1/meta` answers with the instance's
name and version. The [install page](docs/src/content/docs/self-hosting/install.md#commands)
lists the `neon` binary's other commands, and `neon.example.yaml` every configuration key with
its default.

`make obs-up` starts Grafana with Prometheus, Tempo, Loki and Pyroscope on
`http://127.0.0.1:3001`, beside `make run`, and prints the variables that send traces and
profiles to it; see the [observability stack](docs/src/content/docs/self-hosting/observability.md).

### The web app and the docs

Each has a development server that reloads as files change. Install their packages once;
`make docs-install` also installs the headless Chromium that draws the docs' diagrams.

```sh
make web-install
make web-dev               # http://localhost:5173/, passing /api and the server's paths to :8080
make docs-install
make docs-dev              # http://localhost:4321/docs/
```

[web/README.md](web/README.md) and [docs/README.md](docs/README.md) describe each project,
its scripts and its license notices among the rest.

### The apps

The desktop and mobile apps are the Tauri project in `apps/shell/`: the server picker in
TypeScript, built with Vite, and the Rust side in `apps/shell/src-tauri/`, a Cargo workspace
of its own. Beside what [What you need](#what-you-need) lists, cargo-about among them, a build
needs [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/) for its platform:

- **Linux**: WebKitGTK 4.1 and the other libraries Tauri lists, on Debian or Ubuntu
  `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev
  libayatana-appindicator3-dev librsvg2-dev`.
- **macOS**: Xcode's command-line tools for the desktop app; full Xcode and CocoaPods
  (`brew install cocoapods`) for iOS.
- **Windows**: the Microsoft C++ build tools and WebView2.
- **Android**: JDK 17, the Android SDK with its platform tools and NDK, and the Rust targets
  `aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`.
  `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` must point at them.

```sh
make app-install           # the picker's packages and Tauri's CLI, pinned in apps/shell/bun.lock
make app-dev               # the desktop app, the picker reloading as it changes
make app-check             # the picker's types, lints and formatting; the Rust side's
                           # formatting, clippy, docs and tests (part of make check)
make app-browsers          # once: the headless Chromium and WebKit for the next line
make app-test              # the picker on its own, in Chromium and WebKit at desktop and phone sizes
make app-build             # this machine's desktop app and installers
make app-webview-check     # on Linux, after make app-build: the app against a hostile server
make app-android           # an APK for every ABI, signed with the debug key, for testing
make app-ios               # the iOS app, signed for an Apple developer team
```

`make app-build` writes to `apps/shell/src-tauri/target/release/bundle/`: on macOS the `.app`
and a `.dmg`, on Windows the `.msi` and setup `.exe`, on Linux the `.deb`, `.rpm` and AppImage.
It sets `CI=true`, which makes the disk image's script skip the AppleScript that arranges its
Finder window; that script needs permission to control Finder. `make app-android` writes
`apps/shell/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`,
which `adb install` puts on a device or an emulator.

`make app-webview-check` runs the built Linux app in WebKitGTK against a stand-in server whose
page calls the picker's commands while it sends the window to the app's own origin, and fails
if one call goes through. It needs a display: CI runs it under `xvfb-run -a`.

The iOS app builds on a Mac with full Xcode, and needs an Apple developer team's certificate
to sign it. Set `APPLE_DEVELOPMENT_TEAM` to the team's ID, or `bundle.iOS.developmentTeam` in
`apps/shell/src-tauri/tauri.conf.json`. `make app-ios` archives the app and exports it for App
Store Connect. `bun run tauri ios dev` in `apps/shell` runs it in a simulator. The Xcode
project is generated in `apps/shell/src-tauri/gen/apple/`; `bun run tauri ios init` in
`apps/shell` makes it again after Tauri's template changes.

The [Apps guide](docs/src/content/docs/guides/apps.md) covers installing the apps, connecting
them to a server, and their security model, which a change to the apps must keep.

### The whole stack from the checkout

`make up` builds both images from the checkout and runs the stack in the foreground on
`http://127.0.0.1:3000/`, once `deployments/.env` is copied from `deployments/.env.example`
with `POSTGRES_PASSWORD` set. `make down` stops it and keeps the data. The server's own
settings go in `deployments/server.env`, as the [install page](docs/src/content/docs/self-hosting/install.md)
describes, along with the Compose profiles `mail`, `observability` and `seaweedfs`.

## Generated files

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

## Checks

A pull request must pass `make check`, the tests and the end-to-end tests. One that changes
the apps must pass `make app-test` and CI's **App shell** jobs too.

`make check` runs every check that needs no Docker and, once the crates, the packages and the
advisory database are downloaded, no network: formatting, clippy with warnings as errors, the
Rust docs with warnings as errors, a build from the `.sqlx/` cache, the generated files, the
web app's, the docs' and the end-to-end tests' own checks, the block fixtures in both
directions, the Claude Code plugin, the apps' picker checks and their Rust side's formatting,
clippy, docs and tests, the version check, and the license and advisory checks. It reads
the packages of every JS project, so install them first:

```sh
make web-install docs-install e2e-install blocks-install app-install
make check
```

Its cargo commands for the server build in `target/offline/`, so that switching between them
and the builds checked against the database does not rebuild `neon-store` and every crate
above it each time: `neon-store`'s build script makes each build read the right source, the
cache or the database. The apps' Rust side builds in `apps/shell/src-tauri/target/`, where
Tauri's CLI looks for it. cargo-deny reads the advisory database it last fetched and fails
once that copy is more than 90 days old; `make licenses` fetches a new one.

`make web-build` and `make docs-build` build the web app and the docs, as CI does too. The
docs' build fails on a broken link or anchor, so run it after changing a page.

The tests need the development database, from `make db-up` and `make migrate`. Each database
test creates a fresh database through `DATABASE_URL`. The S3 blob store's test runs against
an S3-compatible server that `NEON_TEST_S3_ENDPOINT` names, and is skipped without one;
`make test-s3` starts SeaweedFS in a throwaway container for it.

```sh
make test
make test-s3               # needs Docker
make sqlx-check            # fails when .sqlx/ no longer matches the queries
```

`make e2e` builds both images, starts a throwaway stack of them on a free local port, with
Mailpit catching its mail, runs the Playwright tests in `e2e/` against it in a headless
Chromium, and removes it, leaving the report in `e2e/playwright-report/`. It installs the
tests' packages and the browser itself. The server image it tests builds with Cargo's `e2e`
profile, the release profile without link-time optimisation, which compiles in a fraction of
the time; `make e2e E2E_CARGO_PROFILE=release` tests the build the published images have.
`E2E_PROJECT`, `E2E_SERVER_IMAGE`, `E2E_WEB_IMAGE` and `E2E_WEB_PORT` name the Compose
project, the image tags and the port, so runs from two checkouts can go side by side.
`E2E_SERVER_PREBUILT` names a directory holding a `neon` binary built for Linux and the
notices `make server-notices` writes, which `make e2e` then puts into the server image's
`prebuilt` target instead of compiling the server, as CI does.

CI runs the same `make` targets. On each pull request into `dev` or `main`, and each push to
them, a job per area runs the checks for the files that changed, and a change to the
`Makefile` or a workflow runs them all. The Rust code compiles once, in Cargo's `ci` profile:
**Rust build** archives every test binary with [cargo-nextest](https://nexte.st), runs the
doctests, which nextest does not run, and checks the OpenAPI description. Three **Rust tests**
jobs share the archived tests out, each against a Postgres service container, with the
settings in `.config/nextest.toml`, and one of them runs `make test-s3` too. **E2E** runs
`make e2e` on the build's `neon` binary, on each push, on each pull request into `main`, and
on those into `dev` that touch the images, Caddy, the Compose files, the end-to-end tests or
the `Makefile`; when the tests fail, the Playwright report is attached to the run. Before a
release, the E2E workflow runs `make e2e` on both images built in full, the server in the
release profile. The **App shell** job runs on Ubuntu 22.04 with WebKitGTK, as the release
builds the Linux app: the apps' license checks, `make app-check`, clippy for Android,
`make app-test`, a `.deb` build of the Linux app, and `make app-webview-check` against that
build under Xvfb, and **App shell (iOS)** runs clippy for iOS on macOS. Gitleaks
scans the new commits for secrets, and **All checks** passes once every job but E2E passed
or was skipped.

Dependabot opens one grouped pull request per ecosystem each month, into `dev`. The few pins
it cannot read, which `.github/dependabot.yml` lists, are bumped by hand.

## Rust code

- A module with submodules lives in `foo.rs` beside its directory `foo/`, never in
  `foo/mod.rs`: clippy's `mod_module_files` fails `make check` on one. The code that an
  integration test's files share stays in `tests/support/mod.rs`, so that cargo does not
  build it as a test of its own.
- `neon-app`'s integration tests are the modules of one binary, `crates/app/tests/app/`,
  which links the crate once for all of them. A new file of them goes there and in its
  `main.rs`, unless it needs spans or logs recorded, as the `*_telemetry.rs` files and
  `webhooks.rs` do: such a file is a binary of its own, as tracing decides whether a span or
  an event is enabled for every thread of the process at once.
- The lints are the root `Cargo.toml`'s `[workspace.lints]`, which the apps' crate repeats
  in its own `[lints]`. A lint is silenced only where a fix would be wrong, on the item it
  fires on and with a reason: `#[expect(lint, reason = "...")]`, which fails once the lint no
  longer fires, or `#[allow(lint, reason = "...")]` where it fires on some builds only. Each
  `unsafe` block has a `// SAFETY:` comment that says why it is sound.
- Doc comments name other items with intra-doc links, which `make doc-check` checks: it
  builds the docs of every crate, private items included, and fails on a warning, such as a
  link that resolves to nothing or one from a public item's docs to a private item. Public
  docs name a private item in a code span instead.

## Branches and commits

- Branch from `dev` and open the pull request into `dev`. `main` takes `dev`'s changes for
  each release.
- Write commits as [Conventional Commits](https://www.conventionalcommits.org): a type, an
  optional scope, and a summary in the imperative, such as
  `fix(web): keep table columns at their saved widths`. The types in use are `feat`, `fix`,
  `refactor`, `docs`, `test`, `ci`, `build` and `chore`, and a scope names a part, such as
  `web`, `server`, `e2e` or `deps`.
- Keep each commit to one change, with the generated files it needs. A pull request lands on
  `dev` as one commit with its title as the subject, so write the title the same way.

## Versions

The cargo workspace's version, in the root `Cargo.toml`, is the single source of the version
that the crates, the images, the Compose file, the apps, the plugin and the packages share.
`make version-check`, part of `make check`, fails unless every file that states it, the docs
and the changelog among them, states the same one. The apps' crate, in
`apps/shell/src-tauri/Cargo.toml`, belongs to a workspace of its own and states the version
itself, as do its `Cargo.lock` and `apps/shell/package.json`; the check holds them to it too.
[Upgrading](docs/src/content/docs/self-hosting/upgrading.md#releases) explains how releases
are numbered and tagged.

## Dependency licenses

Every dependency must be under a license that `deny.toml` allows. `make licenses` runs all of
cargo-deny's checks, licenses, advisories, bans and sources, on the crates of the server's
workspace and of the apps' (`apps/shell/src-tauri/`), then checks the packages installed in
`web/`, `docs/`, `e2e/`, `tools/blocknote-fixtures/` and `apps/shell/` with
`scripts/licenses.ts`, which reads the same allow list. A package outside it needs a
build-only exception in `scripts/licenses/policy.ts`, with its exact license and the reason
none of it ships, and the check fails on an exception nothing needs any more.

What ships carries its dependencies' license notices:

- The web app's and the docs' builds write `THIRD_PARTY_NOTICES.txt` beside their files,
  served at `/THIRD_PARTY_NOTICES.txt` and `/docs/THIRD_PARTY_NOTICES.txt`, with each
  package's `NOTICE` files and where to get the source of each package under MPL-2.0. The
  web app's also carries shadcn/ui's MIT license, for the components copied into
  `web/src/components/ui`, where the license is too. A shipped package without a license
  text fails the build.
- The server image holds the crates' notices at `/usr/share/doc/neon/THIRD_PARTY_NOTICES.txt`,
  which [cargo-about](https://github.com/EmbarkStudios/cargo-about) writes as the image
  builds, as `about.toml` sets out.
- The web image holds Caddy's notices at `/usr/share/doc/caddy/THIRD_PARTY_NOTICES.txt`: a
  stage of `web/Dockerfile` downloads each Go module that `caddy build-info` lists, checks it
  against the hash the binary records, and copies its license files, failing when a module
  has none.
- The apps embed `THIRD_PARTY_NOTICES.txt` with their server picker, which links to it. The
  picker's build writes the notices of its packages and, with cargo-about and
  `apps/shell/about.toml`, of the crates the apps link on every platform, with where to get
  the source of each package and crate under MPL-2.0.

Both `about.toml` files' accepted licenses are the allow list of `deny.toml`, and a test keeps
them the same.

[web/README.md](web/README.md#licenses) and [docs/README.md](docs/README.md#licenses) cover
their projects' exceptions and how the builds find what ships.

## Security issues

Report a vulnerability privately, as the [security policy](.github/SECURITY.md) describes,
and not in an issue, a pull request or a discussion, which are public.
