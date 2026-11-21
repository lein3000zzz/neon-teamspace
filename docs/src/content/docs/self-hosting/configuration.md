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
| `observability` | `logging`, `metrics`, `tracing`, `profiling`, and `admin`, the internal [admin listener](/docs/architecture/observability/#the-admin-listener), whose `addr` is `127.0.0.1:9090` unless set. |
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
  admin:
    addr: 127.0.0.1:9090

settings:
  instance:
    name: Acme Workspace
```

The repository's `.gitignore` keeps `neon.yaml` and `*.local.yaml` out of git, as they often
hold a password. Secrets are better kept in the environment than in a file.

## Environment variables

Every key has a variable: `NEON_`, then the key's path in capitals, with `__`, two
underscores, between its levels.

| Key | Variable |
|---|---|
| `server.listen` | `NEON_SERVER__LISTEN` |
| `database.url` | `NEON_DATABASE__URL` |
| `database.max_connections` | `NEON_DATABASE__MAX_CONNECTIONS` |
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
