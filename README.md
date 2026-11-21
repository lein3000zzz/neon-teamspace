# Neon Workspace

A self-hosted team workspace with block-based pages, databases and kanban boards, real-time
collaborative editing, and AI agents as first-class teammates over MCP.

One deployment hosts many workspaces. Each workspace has teamspaces, nested pages and
databases. Agents connect over MCP, pick up tasks, comment and edit pages next to people.

> **Status:** early development. Nothing here is usable yet.

## Repository layout

| Path | What it holds |
|---|---|
| `crates/domain` | Domain types, validation and permission rules. No I/O. |
| `crates/store` | Postgres repositories. |
| `crates/app` | Use cases, transactions, and the ports for side effects. |
| `crates/observability` | Logs, metrics, traces, profiles, the admin listener and ordered shutdown. |
| `crates/server` | The `neon` binary: HTTP API, realtime sync, MCP, webhooks, wiring. |
| `api/openapi.json` | The HTTP API description, generated from the server code. |

Dependencies only point inward: `server` → `app` → `store` → `domain`. Each crate's
`Cargo.toml` lists what it may use, so the compiler enforces the direction.

## Development

Prerequisites:

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the version.
- [sqlx-cli](https://crates.io/crates/sqlx-cli), for migrations and the query cache:
  `cargo install sqlx-cli --locked --version 0.9.1 --no-default-features --features postgres,rustls`
- [Bun](https://bun.sh) for the web app and the docs.
- Docker, for Postgres and the other local services.

Start Postgres and point the tools at it. The compose file binds Postgres to
`127.0.0.1:55432` with credentials meant only for development. `.env` holds the matching
URLs, and git ignores it.

```sh
docker compose -f deployments/docker-compose.dev.yml up -d --wait
cp .env.example .env
set -a; . ./.env; set +a   # export the variables to this shell
```

Once `DATABASE_URL` is set, in the shell or in `.env`, the build checks every SQL query
against that database, so the tables must exist before the first build. Create them:

```sh
sqlx migrate run
```

Without sqlx-cli, `SQLX_OFFLINE=true cargo run -p neon-server -- migrate` does the same: it
builds from the query cache in `.sqlx/` instead of the database, then applies the
migrations. Run `sqlx migrate run` again after pulling a change that adds a migration.

Then build and run the server. It listens on `0.0.0.0:8080` and applies pending migrations
when it starts. `neon migrate` applies them without serving.

```sh
cargo run -p neon-server            # the same as `neon serve`
curl localhost:8080/api/v1/meta
```

Tests use the same Postgres. Each database test creates a fresh database through
`DATABASE_URL`.

```sh
cargo test --workspace
```

The `neon` binary has these commands:

| Command | What it does |
|---|---|
| `neon serve` | Runs the server. The default when no command is given. |
| `neon migrate` | Applies pending migrations and exits. |
| `neon openapi [--out PATH]` | Prints the OpenAPI description, or writes it to `PATH`. |
| `neon settings docs` | Prints the runtime settings reference as markdown. |
| `neon config check` | Loads and checks the configuration, then prints it with secrets redacted. |

### Generated files

The repository commits two generated files. A test fails when `api/openapi.json` no longer
matches the routes, and `cargo sqlx prepare --workspace --check` fails when `.sqlx/` no
longer matches the queries.

```sh
# api/openapi.json, after changing a route or a type it uses
cargo run -p neon-server -- openapi --out api/openapi.json

# .sqlx/, the query cache that lets the build check SQL without a database,
# after adding or changing a query or a migration
cargo sqlx prepare --workspace
SQLX_OFFLINE=true cargo build --workspace   # confirms the cache is complete
```

### Configuration

The server reads `neon.yaml` from its working directory, or the file that `NEON_CONFIG`
names. [`neon.example.yaml`](neon.example.yaml) lists every key with its default. Copy it
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
