# Changelog

What each release of Neon Teamspace changes. The server and web images, the Compose file and
the Claude Code plugin share one version, the cargo workspace's. Before 1.0, a minor release
such as 0.2.0 may need its admins to do something as they upgrade, and its section here says
what; [Upgrading](docs/src/content/docs/self-hosting/upgrade.md) covers the steps every
upgrade takes.

Each section is a release, newest first, dated the day it was tagged. Between releases, the
next one's section may say `Unreleased` instead of a date.

## 0.1.0 - 2026-11-27

The first release.

### Added

- Pages made of blocks, nested in teamspaces, private or shared with members and guests, with
  the trash and favorites, edited together in real time over WebSocket, as Yjs documents.
- Databases with typed properties, table, board and list views, filters, sorts and grouping,
  row pages and inline databases, changes reaching everyone at once.
- Accounts with three registration modes, invites, requests to join, verification, password
  resets, sessions and API tokens, and the admin pages for accounts, invites, workspaces and
  runtime settings.
- Workspaces with roles, invitations and invite links, teamspaces, limits and hues.
- Agents and integrations as members of a workspace, acting for their owners with tokens
  bound to one workspace.
- An OAuth 2.1 authorization server, for MCP clients and other programs.
- The MCP server at `/mcp`, with tools, resources and subscriptions, claiming tasks among
  them, and the same over the REST API.
- Outgoing webhooks, signed and retried, and incoming webhooks that make rows and add to
  pages.
- Comments on pages and on passages of them, mentions, an inbox, and notification mail.
- Page history, compared block by block and restored through the live document.
- Full-text search of pages, databases, rows and comments.
- Attachments in pages and as covers, on local disk or in an S3-compatible store.
- Import of markdown files and zips of them, and export of pages and teamspaces as markdown.
- The Claude Code plugin, which connects Claude Code to an instance over OAuth and works the
  cards assigned to you.
- The server and web images for `linux/amd64` and `linux/arm64`, with an SBOM and build
  provenance, and the Compose file that runs them with Postgres, attached to the release as
  its Compose archive.
- The documentation, served by the instance at `/docs`, with the API reference.

### Upgrading

Instances run from images built before this release, from the repository's `main` or `dev`
branch, upgrade as any release does: the server applies the migrations since as it starts.
The Compose file now runs the images of the release it belongs to when `NEON_VERSION` is
unset, where it ran `latest` before.
