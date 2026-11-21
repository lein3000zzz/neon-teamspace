---
title: Configuration
description: The config file, environment variables, runtime settings, and which value wins.
---

The server reads two kinds of configuration:

- **Bootstrap configuration** is what the server needs before it can reach its database:
  where to listen, the database's URL, and how to report logs, metrics and traces. It is read
  once, at start.
- **Runtime settings** are everything else, such as the instance's name. They can change while
  the server runs, and their values can be kept in the database. The
  [settings reference](/docs/reference/settings/) lists them all.

Both come from the same two places, a YAML file and the environment.

## The config file

The server reads `neon.yaml` in the directory it runs from, or the file `NEON_CONFIG` names
instead. It has four sections:

| Section | What it holds |
|---|---|
| `server` | `listen`, the address the app's port binds, `0.0.0.0:8080` unless set; `public_url`, the address people use to reach the instance. |
| `database` | `url`, the Postgres connection URL; `max_connections`, the size of the connection pool; `auto_migrate`, whether the server updates the schema as it starts, `true` unless set. |
| `observability` | Logs, metrics, traces, profiles and the internal [admin listener](/docs/architecture/observability/#the-admin-listener), in five sections; see [below](#the-observability-section). |
| `settings` | Runtime settings, each under its key, nested at its dots. |

For example:

```yaml
server:
  listen: 0.0.0.0:8080
  public_url: https://neon.example.com

database:
  url: postgres://neon:secret@db.internal:5432/neon
  max_connections: 20

observability:
  logging:
    level: info,sqlx=warn
  tracing:
    enabled: true
    endpoint: http://otel-lgtm:4318
  admin:
    addr: 127.0.0.1:9090

settings:
  instance:
    name: Acme Workspace
```

The repository's `.gitignore` keeps `neon.yaml` and `*.local.yaml` out of git, as they often
hold a password. Secrets are better kept in the environment than in a file.

The file is YAML 1.2, so only `true` and `false` are booleans. Words that older YAML read as
booleans, such as `on`, `off`, `yes` and `no`, are plain words, and `mode: off` needs no
quotes. `neon.example.yaml` in the repository lists every key with its default.

## The observability section

The server reads it as it starts, and installs logs, metrics, traces and profiling from it
before anything else. Every key is optional.

| Key | Default | What it sets |
|---|---|---|
| `logging.level` | `info` | A filter directive, such as `info` or `info,sqlx=warn`. It filters what is logged, not what is traced. A level below `info`, such as `debug`, also traces spans down to it. |
| `logging.format` | `json` | `json`, one object per line, or `pretty`, indented text for reading in a terminal. |
| `metrics.enabled` | `true` | Records metrics and serves them at `/metrics` on the admin listener. |
| `tracing.enabled` | `false` | Exports spans over OTLP. Trace IDs exist and pass on to job rows either way. |
| `tracing.endpoint` | unset | The collector's base URL, such as `http://otel-lgtm:4318`; `/v1/traces` is added unless the path ends with it. Unset, the standard `OTEL_EXPORTER_OTLP_*` variables apply, then `http://localhost:4318`. Plain `http` only: TLS belongs to a collector on the internal network. |
| `tracing.protocol` | `http` | OTLP/HTTP with protobuf bodies. `grpc` is not built in and fails the check. |
| `tracing.sample_ratio` | `1.0` | The share of new traces exported, from 0 to 1. A trace continued from a job row, or from a trusted caller, keeps the decision it started with. |
| `tracing.trust_incoming` | `false` | Continues the `traceparent` and keeps the `baggage` that HTTP callers send. While off, each request starts a new trace that links to the caller's. Turn it on only when every caller is trusted, for example behind a proxy that removes or rewrites these headers; see [Traces](/docs/architecture/observability/#traces). |
| `profiling.mode` | `off` | `off`, `pull` to serve profiles on the admin listener, or `push` to send them to Pyroscope. |
| `profiling.profiles` | `[cpu]` | The profiles to collect, `cpu`, `heap` or both. Heap profiles have [requirements](/docs/architecture/observability/#heap-profiles). |
| `profiling.push.server_address` | unset | The Pyroscope server, such as `http://pyroscope:4040`. Required in `push` mode. |
| `profiling.push.upload_interval_seconds` | `10` | Seconds between uploads, at least 1. |
| `profiling.push.tags` | none | Labels on every profile, such as `env: prod`. |
| `admin.enabled` | `true` | Starts the admin listener, with `/metrics`, `/livez`, `/readyz` and profiles. |
| `admin.addr` | `127.0.0.1:9090` | Its address. Keep it on loopback or a private network. |

The server checks the whole section as it starts. A setting that cannot work, such as a
`sample_ratio` of 2, `push` mode without a `server_address`, or `pull` mode with the admin
listener off, is an error: the server reports it with any other configuration error and does
not start. A setting that only has no effect, such as an `endpoint` while `tracing.enabled`
is `false`, is a warning, which the server logs once logging is up. `neon config check`
checks the section the same way and prints it, with the warnings as comments at the top.

## Environment variables

Every key has a variable: `NEON_`, then the key's path in capitals, with `__`, two
underscores, between its levels.

| Key | Variable |
|---|---|
| `server.listen` | `NEON_SERVER__LISTEN` |
| `database.url` | `NEON_DATABASE__URL` |
| `database.max_connections` | `NEON_DATABASE__MAX_CONNECTIONS` |
| `observability.logging.level` | `NEON_OBSERVABILITY__LOGGING__LEVEL` |
| `observability.tracing.trust_incoming` | `NEON_OBSERVABILITY__TRACING__TRUST_INCOMING` |
| `observability.admin.addr` | `NEON_OBSERVABILITY__ADMIN__ADDR` |
| `settings.instance.name` | `NEON_SETTINGS__INSTANCE__NAME` |

The separator is two underscores because keys have single ones of their own:
`NEON_DATABASE__MAX_CONNECTIONS` can only mean `database.max_connections`.

## Which value wins

For bootstrap configuration, the first of these with a value wins:

1. The environment variable.
2. `neon.yaml`.
3. The default.

Runtime settings have one more place, between the file and the default: the database, where
the admin page saves them.

```mermaid
flowchart LR
  env[Environment variable] -- unset --> yaml[neon.yaml] -- unset --> db[Saved in the database] -- unset --> def[Default]
```

So an operator can fix any value with the deployment, and admins change the rest while the
server runs.

**Planned.** The admin page, where instance admins see every setting, where its value comes
from, and change it. It arrives with accounts.

## Pinned settings

A runtime setting that an environment variable or `neon.yaml` sets is **pinned**. Its value
comes from there whatever the database holds, and the admin page will show it locked,
naming the variable or the file. Pinning suits values managed with the deployment, such as the
instance's name on a hosted instance.

Removing the variable or the key unpins the setting: a value saved in the database applies
again, or else the default.

## Changing settings while the server runs

A value saved in the database applies without a restart. Before a value is saved, the
settings registry checks it, and checks it against the other settings where they depend on
each other, so a combination that cannot work is refused rather than discovered later. Once
mail exists, for example, email verification cannot be turned on while no mail server is
configured.

## The settings audit

Each change to a setting kept in the database is recorded in an audit table: the setting,
its value before and after, who changed it and when. Secret values are never recorded, only
that they changed.
