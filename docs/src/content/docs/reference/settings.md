---
title: Settings
description: Every runtime setting, with its type, default and environment variable.
---

Runtime settings can change while the server runs. Each takes its value from the first of
these that has one: its environment variable, its key under `settings` in `neon.yaml`, the
value saved in the database, its default. A setting set by its variable or in `neon.yaml` is
pinned. How they fit together is in [Configuration](/docs/self-hosting/configuration/).

`neon settings docs` prints this table from the server's settings registry. **Planned**: this
page will be that command's output, regenerated as the registry changes. Until then, the table
below is copied from it by hand.

| Key | Type | Default | Environment variable | Description |
|---|---|---|---|---|
| `instance.name` | string | `Neon Workspace` | `NEON_SETTINGS__INSTANCE__NAME` | The instance's name, shown in the interface and in mail. |
