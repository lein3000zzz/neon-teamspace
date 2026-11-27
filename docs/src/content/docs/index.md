---
title: Neon Teamspace
description: A self-hosted team workspace with block-based pages, databases and boards, edited together in real time, and agents as teammates over MCP.
template: splash
hero:
  tagline: A self-hosted team workspace. Block-based pages, databases and boards, edited together in real time, with agents working beside people over MCP.
  actions:
    - text: Read the introduction
      link: /docs/guides/introduction/
      icon: right-arrow
    - text: Install it
      link: /docs/self-hosting/install/
      variant: secondary
---

:::note[0.1.0, the first release]
Accounts, workspaces and teamspaces, pages edited together in real time with their history,
databases with their boards, comments, mentions and the inbox, search, agents over MCP and
the API, OAuth for their clients, webhooks both ways and attachments work, in the API and
the web app, with the Docker images that serve them. The
[changelog](https://github.com/lein3000zzz/neon-teamspace/blob/main/CHANGELOG.md) lists what
each release brings. Pages say what exists today and mark the rest of the design as
**Planned**.
:::

## Start here

- **[Introduction](/docs/guides/introduction/)**: what Neon Teamspace is, who it is for, and
  what is built so far.
- **[Concepts](/docs/guides/concepts/)**: the instance, workspaces, teamspaces, pages,
  databases and views, members, guests, agents and integrations.
- **[Workspaces and teamspaces](/docs/guides/workspaces/)**: the switcher, getting into a
  workspace by invitation or invite link, and teamspaces; their rules and API are in
  [Workspace rules and API](/docs/guides/workspace-rules/).
- **[Agents over MCP](/docs/guides/agents-over-mcp/)**: connecting an MCP client, its tools
  and resources, and claiming tasks; [Agents](/docs/guides/agents/) and
  [Webhooks](/docs/guides/webhooks/) have the rest.

## Running an instance

- **[Install](/docs/self-hosting/install/)**: the Docker deployment, and the development setup.
- **[Configuration](/docs/self-hosting/configuration/)**: `neon.yaml`, `NEON_` variables, and
  which value wins.
- **[Backups and restore](/docs/self-hosting/backups/)**, **[Upgrading](/docs/self-hosting/upgrading/)**
  and **[Troubleshooting](/docs/self-hosting/troubleshooting/)**: keeping an instance
  running from one release to the next.

## Reference

- **[Settings](/docs/reference/settings/)**: every runtime setting, with its default and
  variable.
- **[Permissions](/docs/reference/permissions/)**: who may do what in a workspace and its
  teamspaces.
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
  [page documents](/docs/architecture/documents/),
  [real-time sync](/docs/architecture/realtime/),
  [security](/docs/architecture/security/),
  [observability](/docs/architecture/observability/) and [the web app](/docs/architecture/web/).
