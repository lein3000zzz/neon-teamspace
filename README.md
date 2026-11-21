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
- [Bun](https://bun.sh) for the web app and the docs.
- Docker, for Postgres and the other local services.

```sh
cargo build
cargo test
```

## License

[PolyForm Noncommercial 1.0.0](LICENSE).
