---
title: Neon Workspace
description: A self-hosted team workspace with block-based pages, databases and boards, edited together in real time, and agents as teammates over MCP.
template: splash
hero:
  tagline: A self-hosted team workspace. Block-based pages, databases and boards, edited together in real time, with agents working beside people over MCP.
  actions:
    - text: Read the introduction
      link: /docs/guides/introduction/
      icon: right-arrow
    - text: Run it locally
      link: /docs/self-hosting/install/
      variant: secondary
---

:::note[Early development]
Neon Workspace is not usable yet. The foundation exists: the server's crates, configuration,
the database layer and job queue, observability, the first API endpoint, the web app's shell,
these docs, and the Docker images that serve them. Pages say what exists today and mark the rest of the design as **Planned**.
:::

## Start here

- **[Introduction](/docs/guides/introduction/)**: what Neon Workspace is, who it is for, and
  what is built so far.
- **[Concepts](/docs/guides/concepts/)**: the instance, workspaces, teamspaces, pages,
  databases and views, members, guests, agents and integrations.

## Running an instance

- **[Install](/docs/self-hosting/install/)**: the Docker deployment, and the development setup.
- **[Configuration](/docs/self-hosting/configuration/)**: `neon.yaml`, `NEON_` variables, and
  which value wins.

## Reference

- **[Settings](/docs/reference/settings/)**: every runtime setting, with its default and
  variable.
- **[API](/docs/reference/api/)**: the HTTP API, from `api/openapi.json`.

The API reference is generated from the server's description of its API as the docs build,
so it matches the version it was built with. The settings page is generated from the server's
settings registry, and a test fails while the committed page differs from it.

## How it is built

- **[Overview](/docs/architecture/overview/)**: the crates, the direction their dependencies
  point, and the way a request takes.
- **[Key decisions](/docs/architecture/decisions/)**: the choices that shape the project, such
  as one Yjs document per page and agents as workspace members, and why.
- The rest of **Architecture**: [data](/docs/architecture/data/),
  [observability](/docs/architecture/observability/) and [the web app](/docs/architecture/web/).
