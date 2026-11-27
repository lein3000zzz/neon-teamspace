# Changelog

What changes in each release of Neon Teamspace, for the people who run it. Versions follow
[semantic versioning](https://semver.org); until 1.0.0, a minor version may change the API or
the settings in a way that needs something of you, and says so under **Breaking changes**.
[Upgrading](docs/src/content/docs/self-hosting/upgrading.md) covers moving an instance from
one release to the next. The numbers are the pull requests that made each change.

## 0.1.0

The first release: a self-hosted team workspace with pages edited together in real time,
databases and boards, comments and an inbox, search, history and attachments, and agents
that work beside people over MCP, OAuth, the REST API and webhooks.

### Breaking changes

None: there is no earlier release to upgrade from. This release sets the shapes later
releases will be measured against, and calls out when they change:

- The REST API under `/api/v1`, described by `api/openapi.json` and served at
  `/api/v1/openapi.json`. Every error is `{"error": {"code", "message"}}`, and `code` is the
  stable part.
- Token prefixes: `nw_` for API tokens, `nwo_` and `nwr_` for OAuth access and refresh
  tokens, `nwh_` in incoming webhooks' URLs and `whsec_` for webhook signing secrets.
- Version 1 of the outgoing webhook payload and of the incoming webhook payload, and the
  `Neon-Webhook-Signature` scheme, `v1=` and an HMAC-SHA256.
- The MCP server's 23 tools and its `neon://` resources.
- The configuration keys, their `NEON_` variables, and the runtime settings the settings
  reference lists.

### Running it

- Two images for `linux/amd64` and `linux/arm64`: `ghcr.io/lein3000zzz/neon-teamspace-server`,
  the `neon` binary on distroless as an unprivileged user, and
  `ghcr.io/lein3000zzz/neon-teamspace-web`, Caddy with the app and the docs, which passes
  the server's paths on to it on one origin and gets its own TLS certificate given a domain.
  Release images carry an SBOM and build provenance, and are built only after the
  end-to-end tests pass. (#8, #9, #31)
- `deployments/docker-compose.yml` runs them with Postgres 18, pinned by `NEON_VERSION`, with
  the stack's settings in `deployments/.env` and the server's in `deployments/server.env`.
  Profiles add Mailpit (`mail`), Grafana's otel-lgtm with a dashboard of the server's metrics
  (`observability`) and SeaweedFS for attachments (`seaweedfs`). (#8, #41)
- The `neon` command line: `serve`, `migrate`, `admin create`, `search reindex`, `openapi`,
  `settings docs`, `config check` and `health`, the containers' health check. (#3, #8, #11, #40)
- Configuration from `neon.yaml` and `NEON_` variables, and runtime settings that admins
  change while the server runs, kept in Postgres with an audit of every change. A setting
  the environment or the file sets is pinned. (#3)
- Migrations apply as the server starts, under an advisory lock, each in its own
  transaction; `database.auto_migrate: false` leaves them to `neon migrate`. (#3)
- Secrets the database keeps, invite codes' copies, secret settings, queued mail and
  webhooks' signing secrets, are sealed with the keys of `secrets.keys`, which rotate. (#11)
- Logs as JSON or pretty text with trace ids, OpenTelemetry traces over OTLP/HTTP, and CPU
  and heap profiles to pull or push; Prometheus metrics, pulled profiles, `/livez` and
  `/readyz` on an internal admin listener; and an ordered shutdown on SIGTERM. (#2, #6)
- A Postgres job queue for mail, webhook deliveries and notification mail, queued in the same
  transaction as the change that causes them and retried with backoff. (#3, #11, #30, #39)
- Docs at `/docs`, with the API reference generated from the server, and pages on
  installing, configuration, mail, file storage, observability, backups and restore,
  upgrading and troubleshooting. (#5, #7)

### Accounts and administration

- Registration that is open, by invite (the default) or closed; invites with uses, an
  expiry and an optional address; requests to join that admins approve or deny. (#11, #12)
- Email verification, password resets and password changes; sessions in an HttpOnly cookie,
  `__Host-neon_session` over HTTPS; API tokens with read, write and admin scopes. (#11, #12)
- The first admin from `NEON_ADMIN__EMAIL` and `NEON_ADMIN__PASSWORD`, or
  `neon admin create`; the last active admin cannot be removed. (#11)
- Mail through an SMTP relay of your choice, with STARTTLS or TLS 1.2 and up, a test mail
  and a mail status on the admin page. (#11, #12)
- Rate limits on sign-ins, sign-ups, mailed links and requests to join, by client address,
  believed from `X-Forwarded-For` only from `server.trusted_proxies`. (#11)
- Admin pages for users, invites, requests to join, settings, mail and workspaces with their
  limits. Instance admins never see inside a workspace. (#12, #17)

### Workspaces

- Many workspaces per instance, with owner, admin, member and guest roles, a switcher, and
  `workspaces.creation`, which can leave making them to instance admins. (#17)
- Teamspaces: a default one everyone is in, and open, closed and private ones, each with its
  owners and members. (#17)
- Invitations to existing accounts, accepted or declined in the inbox, and invite links
  with uses and an expiry. (#17, #39)
- Limits per workspace on members, agents and integrations, and storage, with defaults in
  the settings and overrides by instance admins; unlimited unless set. (#17, #27, #41)
- A hue for each person, workspace, teamspace, agent and integration, worked out from its
  id. (#17)

### Pages

- Nested pages in teamspaces, private pages, and pages shared with a person to view, comment
  on or edit; moving, favorites, icons, covers, fonts, the trash with restore. (#18, #39)
- The BlockNote editor, edited together in real time over WebSocket with cursors and
  presence, saved as a log of updates and snapshots; connections close or turn read-only as
  access changes. (#18)
- Markdown import, of `.md` files and zips of them, and export of pages and teamspaces. (#18)
- Page history: versions kept as pages are saved, thinned with age, compared block by block
  and restored through the live document. (#40)
- Full-text search of pages, databases, rows and comments, with Postgres's `simple`
  configuration and `pg_trgm`, among what the searcher may see. (#40)
- Comments on whole pages and on passages, replies, resolving, mentions of people, agents and
  pages, and an inbox, live in the app. Notification mail once an admin turns
  `notifications.mail` on: a mail for each notification left unread, with those close
  together in one, or a digest, as each person chooses. (#39)
- Attachments: images, video, audio and other files in pages, and image covers, kept on local
  disk or in an S3-compatible store, within each workspace's storage limit. (#41)

### Databases and boards

- Databases as full pages or inline in a page, whose rows are pages with typed properties:
  title, text, number, select, multi-select, status, date, person, checkbox, URL, a unique id
  such as `NW-42`, and created and edited times and people. (#23, #25)
- Table, board and list views, each with its filters, sorts, grouping and visible
  properties; changes reach everyone viewing at once. (#23, #25)

### Agents, MCP and webhooks

- Agents as members of a workspace, owned by a person, acting within what both may, shown
  as `agent · via Owner`, with tokens bound to one workspace; integrations the same way for
  incoming webhooks. `mcp.identities` says who MCP clients may act as. (#27)
- An OAuth 2.1 server for MCP clients and other programs: dynamic client registration, PKCE,
  a consent page that chooses the workspace, the identity and the scopes, rotating refresh
  tokens with reuse detection, revocation and a list of connected apps. (#28)
- The MCP server at `/mcp` over Streamable HTTP, with 23 tools, `claim_task` among them,
  resources with subscriptions, and every action also in the REST API. (#29, #33, #39, #40)
- Outgoing webhooks, signed and retried, for assignments, status changes, mentions and
  replies; incoming webhooks that make and change rows, add to a page or comment on it, with
  optional signatures. (#30, #33, #39)
- A Claude Code plugin, installed from this repository, that connects Claude Code to an
  instance and works the cards assigned to you. (#32)

### Fixes made before the release

- Edits from MCP and the API that replace a range or a whole page need the revision they
  read, so a stale read never removes a teammate's blocks; and further fixes from two audits
  of agents, OAuth, MCP and webhooks. (#33)
- The web app fits database pages on phones, keeps table columns at their saved widths, opens
  a comment thread's link without selecting the passage, and two dozen other fixes. (#24,
  #38, #44)
