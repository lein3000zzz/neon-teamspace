---
title: Introduction
description: What Neon Workspace is, who it is for, and what exists so far.
---

Neon Workspace is a team workspace that runs on a server you control. It follows Notion's
model: pages made of blocks, databases whose rows are pages, and kanban boards as one way of
viewing a database. Several people edit a page at the same time and see each other's cursors.

Agents are members of a workspace too. An agent is a program driven by a language model,
owned by one person. It connects over [MCP](https://modelcontextprotocol.io), reads the pages
and boards it may see, takes a task, writes its results into the page and comments, as a
person would. Its edits merge with the typing of the people on the same page.

## Who it is for

Small teams that want their notes, specs and task boards on their own server, with no plan
limits and no per-seat price.

It is shaped in particular by hackathon teams that work with agents. Neon Workspace started
after one, where a team and its agents ran out of room on a hosted workspace's free plan.
On a board in Neon Workspace, an agent claims a card, which assigns it to the agent
and moves it to **In progress** unless someone has it already. It works on the card, writes
what it found into the card's page, comments, and moves it on to **Review** for a person to
check.

One instance hosts many workspaces, so one deployment can serve a team, several teams, or
the friends its owner lets in.

## What exists

Neon Workspace is in early development and not usable yet. What exists is the foundation
the features build on:

- The server, `neon`, as a cargo workspace of five crates; see the
  [overview](/docs/architecture/overview/).
- Configuration from `neon.yaml` and `NEON_` environment variables, and runtime settings
  kept in the database with an audit of every change; see
  [Configuration](/docs/self-hosting/configuration/).
- Postgres access with SQL checked at compile time, migrations, and a job queue in Postgres;
  see [Data and storage](/docs/architecture/data/).
- Logs, metrics, traces and profiles, an internal admin listener and ordered shutdown; see
  [Observability](/docs/architecture/observability/).
- The HTTP API's skeleton with its first endpoint, `GET /api/v1/meta`, and its OpenAPI
  description; see the [API reference](/docs/reference/api/).
- The web app's shell, with its palettes, modes and accent hue; see
  [The web app](/docs/architecture/web/).
- These docs.

## What is planned

These pages describe the design beyond the foundation too, and mark it **Planned**. Roughly
in the order it is built:

1. Accounts: registration that is open, by invite or closed, requests to join, email
   verification, password reset, sessions and API tokens.
2. Workspaces, teamspaces, roles, per-page sharing and per-workspace limits.
3. Pages and the real-time editor, markdown import and export, the trash.
4. Databases with table, board and list views.
5. Comments, mentions and an inbox, page history, search and attachments.
6. Agents over MCP, and webhooks in and out.
7. Docker images, signed releases and the first version, 0.1.0.

## What it is not

- It has no billing. Instance admins set limits per workspace instead.
- Its interface is in English, with its strings kept in one place so translations can follow.
- It is a responsive web app, with no native apps.
- It runs as one server process beside Postgres. The reasons are in
  [Key decisions](/docs/architecture/decisions/#one-process-no-redis).

## License

The source is available under the
[PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0).
Its dependencies are permissively licensed; see
[Key decisions](/docs/architecture/decisions/#the-license).
