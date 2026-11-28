# Changelog

What changes in each release of Neon Teamspace, for the people who run it. The server and web
images, the Compose file, the desktop and mobile apps and the Claude Code plugin share one
version, the cargo workspace's.
Versions follow [semantic versioning](https://semver.org); until 1.0.0, a minor version may
change the API or the settings in a way that needs something of you, and says so under
**Breaking changes**. [Upgrading](docs/src/content/docs/self-hosting/upgrading.md) covers
moving an instance from one release to the next. The numbers are the pull requests that made
each change.

Each section is a release, newest first, dated the day it was tagged. Between releases, the
next one's section may say `Unreleased` instead of a date.

## 0.2.0 - 2026-11-28

Desktop and Android apps that open your own server, editing on phones and tablets, and
updates from inside the product. The server now tells its admins when a new release is out
and, with the new updater beside it, moves the stack to that release in one click. This
release also fixes a crash that anyone who could edit a page could cause, so upgrade soon.

### Breaking changes

None. No route, setting or environment variable changed its meaning or went away. The
Compose file only gained the updater service, which runs only once its profile is on, a
network the server shares with it, and three variables that may stay blank. Two things
behave differently and need nothing of you:

- The server asks GitHub's API for this repository's releases as it starts and once a day,
  sending its version in the `User-Agent` and nothing else. To stop it, set `updates.check`
  off in **Admin > Settings**, or pin it with `NEON_SETTINGS__UPDATES__CHECK=false` in
  `deployments/server.env`. (#68)
- An MCP `initialize` whose body cannot be read gets `400 invalid_body` where it got
  `413 payload_too_large`. A body over the 2 MiB limit still gets 413, so only requests cut
  short see the change, which is listed under **Fixed**. (#65)

### Upgrading from 0.1.0

0.1.0 has no updater, so this one upgrade is by hand. Take a backup, unpack the 0.2.0 Compose
archive over the install's directory, then pull and start the images, as
[From 0.1.0 to 0.2.0](docs/src/content/docs/self-hosting/upgrading.md#from-010-to-020) shows.
Use the new archive rather than setting `NEON_VERSION` alone, since the 0.1.0 Compose file
has no updater service. `deployments/.env` and `deployments/server.env` need no change for
the upgrade. If `NEON_VERSION` names `0.1.0`, set it to `0.2.0` or leave it blank.

To update from **Admin > Settings** after that, turn on the `updater` profile. Put a token of
32 characters or more, such as the output of `openssl rand -hex 32`, in `deployments/.env` as
`NEON_UPDATER_TOKEN`, add `COMPOSE_PROFILES=updater`, then pull and run `up -d --wait` again.
[One-click updates](docs/src/content/docs/self-hosting/upgrading.md#one-click-updates) has the
steps, and says what the updater can do with the Docker socket it holds. Without it, the
panel shows the commands that update by hand.

### Added

- **Desktop and mobile apps.** Apps for macOS on Apple silicon and Intel, Windows, Linux and
  Android are attached to each release, and an iOS and iPadOS app builds from source. Each is
  a thin shell around the system's web view that opens the web app of a server you pick.
  Its one screen of its own, the server picker, checks that a Neon Teamspace server answers
  at the address, keeps a list of servers and reopens the last one. The window stays on the
  chosen server's origin, and no page of a server can call the picker's commands. The server
  needs no setting for them. The [Apps guide](docs/src/content/docs/guides/apps.md) covers
  installing, checking a download and connecting. (#63)
- **App updates.** The desktop and Android apps look for a newer release a few seconds after
  they start and once a day. Installed from the `.dmg`, the Windows setup or the AppImage, a
  desktop app downloads the new release, checks its signature against the key it carries,
  installs it and restarts. **Check for Updates…** is in the app menu on macOS and under
  **File** on Windows and Linux. A `.deb` or `.rpm` install belongs to the package manager,
  so it opens the release page instead. The Android app offers the new signed APK to
  download in the browser, and its picker has **Check for updates**. The iOS app does not
  update itself. (#68)
- **Editing on phones and tablets.** On a touch screen, a toolbar above the on-screen keyboard
  takes the place of the block handles and the floating formatting toolbar. It sets block
  types and formatting, adds links, mentions, files and dividers, indents, moves, duplicates
  and deletes blocks, and has comments, undo and redo. The back gesture closes the drawer,
  the comments or a side panel before it leaves the page, and every control is a touch
  target of 44 pixels or more.
  [On a phone or a tablet](docs/src/content/docs/guides/editing-pages.md#on-a-phone-or-a-tablet)
  has the rest. (#62)
- **Release checks.** **Admin > Settings** opens with **Version and updates**. It shows the
  release the server runs, the newest one its last check saw on GitHub with a link to its
  notes, when it checked, why a check failed, and the updater's state. **Check now** asks
  again, and the **Admin** entry of the sidebar carries a mark for instance admins while a
  newer release is out. Instance admins have the same in the API: `GET /api/v1/admin/updates`,
  `POST /api/v1/admin/updates/check`, and `POST /api/v1/admin/updates/apply`, which takes a
  browser session. (#68)
- **One-click updates.** With the Compose file's `updater` profile on, **Update to** in that
  panel asks the updater, `neon-updater`, to update the stack. It pulls the release's server,
  web and updater images and checks each image's cosign signature, which must come from this
  repository's release workflow at the release's tag. Then it writes the release into
  `NEON_VERSION` in `deployments/.env`, recreates the server and web, and updates itself
  last. A failed pull or signature check changes nothing. The server asks only for a release
  newer than its own that its last check saw, the updater refuses to go back, and one update
  runs at a time. The page follows the updater's log and reloads once the server answers with
  the new version. `NEON_UPDATER_VERIFY=false` turns the signature check off, for images you
  build and push yourself. The updater keeps the Compose file you have, so a release whose
  Compose file changes says so in its notes, and you upgrade to it by hand. (#68)
- **Settings.** Four runtime settings: `updates.check`, on by default; `updates.prereleases`,
  off; `updates.updater_url`, `http://updater:8080`; and `updates.updater_token`, a secret,
  empty. The config file's `updates.github_api_url` points the release check at a mirror of
  GitHub's API. In the Compose stack, `updates.updater_token` comes from `NEON_UPDATER_TOKEN`
  in `deployments/.env`. The [settings reference](docs/src/content/docs/reference/settings.md)
  describes each. (#68)
- `GET /api/v1/meta` also answers `product`, always `neon-teamspace`, which the apps check
  before they open a server. (#63)

### Changed

- **The Compose file.** A new `updater` service, under the `updater` profile, runs only once
  you turn the profile on. It holds the Docker socket, drops every capability but `SETUID`
  and `SETGID`, and is published on no port. It sits on a new network, `updater`, which the
  server alone joins beside the stack's own. That network is internal, with no way out of
  the machine. The server takes `updates.updater_token` from `NEON_UPDATER_TOKEN`.
  `deployments/.env.example` adds `COMPOSE_PROFILES`, `NEON_UPDATER_TOKEN` and
  `NEON_UPDATER_VERIFY`, and `NEON_VERSION` now picks the updater's image too. (#68)
- **Release assets.** Beside the Compose archive, each release attaches the desktop
  installers: a `.dmg` for Apple silicon and one for Intel, the Windows setup `.exe`, and the
  Linux `.deb`, `.rpm` and AppImage. It also attaches the Android APK, signed with the
  project's key; the files the desktop apps update themselves from, each with its `.sig`
  signature; and `latest.json`, which those apps read. `SHA256SUMS` lists the checksum of
  everything attached. The apps build from the tag without the Actions cache, once the
  end-to-end tests pass, and their files carry build provenance that
  `gh attestation verify` checks. (#63, #68)
- **The updater image.** `ghcr.io/lein3000zzz/neon-teamspace-updater`, for `linux/amd64` and
  `linux/arm64`, holds `neon-updater`, the docker CLI with its Compose plugin, and cosign,
  with their license notices. Each release builds, signs and attests it beside the server
  and web images, under the same tags. (#68)
- Loading, reading and compacting a page's document, keeping and comparing its versions, and
  building a database query's rows run on threads where blocking is allowed, no longer on
  the async workers that serve requests. Marking many notifications at once takes two
  queries instead of two per notification. (#65)
- Dependency updates: the Rust build image, `hyper`, `clap` and five of the web app's
  packages. CI builds the Rust code once for its tests and end-to-end tests, and two flaky
  tests wait for the editor; releases still build every image from scratch. (#51, #60, #61,
  #66, #67)

### Fixed

- **Security: editing a page could crash the server.** In 0.1.0, anyone who could edit a
  page, guests and agents included, could make the server abort. A table cell in an update
  sent over the page's WebSocket could claim a span of up to about 4 billion columns, and
  reading the document then tried to allocate up to 64 GiB, which killed the process. The
  saved page did the same on each later read, so the server went down again every time
  someone opened it. Reading a table now allocates in proportion to what the document holds,
  and tables read as before, up to the 1,000-column limit. (#65)
- An MCP `initialize` whose body cannot be read to its end, such as one cut short by a
  dropped connection, gets `400 invalid_body`, as the incoming webhooks' route answers it,
  instead of `413 payload_too_large`. Such requests no longer count as too large in the logs
  and the request metrics. (#65)
- An outgoing webhook delivery given up on, by its own timeout or its job's, now ends its
  connection. One to a receiver that had stopped reading stayed open for good. (#65)
- The server waits for the MCP sessions' background tasks as it shuts down. They could
  outlive the MCP server and still use the database while the server closed it. (#65)
- A trash cursor whose time overflows is refused as invalid. In release builds it wrapped
  into another time. (#65)
- Error reports in the logs, traces and failed jobs show each cause once. A database or
  document error, or a telemetry start-up failure, repeated its cause, up to three times for
  a page whose document did not load. (#65)
- A workspace cleanup run that fails at several steps logs each failed step with its error,
  in the order they ran. Only the first was logged. (#65)
- An app that cannot start writes why to its log before it quits, a fix made before the
  apps' first release. (#65)
- Typing a time such as `15:00` no longer opens the emoji menu, and on iPhones with a Dynamic
  Island the hidden skip link no longer shows in the status bar. (#62)

## 0.1.0 - 2026-11-28

The first release: a self-hosted team workspace with pages edited together in real time,
databases and boards, comments and an inbox, search, history and attachments, and agents
that work beside people over MCP, OAuth, the REST API and webhooks.

### Breaking changes

None: there is no earlier release to upgrade from. An instance built from the repository's
`main` or `dev` branch before this release upgrades to it as to any release, as the server
applies the migrations it lacks as it starts. One thing differs for such an instance: while
`NEON_VERSION` is unset, the Compose file now runs the images of the release it belongs to,
where it ran `latest` before.

This release sets the shapes later releases will be measured against, and calls out when
they change:

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
  (#8)
- Releases from a `v*` tag. The workflow checks that the tag and every file that states the
  version agree, runs the end-to-end tests on the release build, then pushes both images
  with OCI labels, an SBOM that lists the crates the server links, and build provenance,
  tagged `0.1.0`, `0.1` and `latest`. The GitHub release then names the images and attaches
  the Compose archive: the Compose file with its example settings. Images are signed with
  cosign's keyless signing, and are built from the tagged commit alone, never from a cache.
  A `vX.Y.Z-rc.N` tag rehearses a release, publishing only its own image tag. (#9, #31, #54)
- A security policy (`.github/SECURITY.md`) for reporting vulnerabilities privately, and the
  license notices of everything that ships: the crates, the npm packages with their `NOTICE`
  files, shadcn/ui's code copied into the app, and Caddy with the Go modules it links. (#54)
- `deployments/docker-compose.yml` runs them with Postgres 18: the images of its own release,
  unless `NEON_VERSION` names another, with the stack's settings in `deployments/.env` and the
  server's in `deployments/server.env`. Profiles add Mailpit (`mail`), Grafana's otel-lgtm
  with a dashboard of the server's metrics (`observability`) and SeaweedFS for attachments
  (`seaweedfs`). (#8, #41)
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
  an IPv6 one by its /64, believed from `X-Forwarded-For` only from `server.trusted_proxies`.
  (#11)
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
- Fixes from a security audit of the whole product. Every limit by client counts an IPv6
  address by its /64, the server checks as many passwords at once as it has processors and
  answers `503 busy` past a 2-second wait, and a password reset revokes the account's API
  tokens and connected apps too. A removed or demoted admin's pending invitations and invite
  links end with their role. Large content is counted before it is written, and writes into
  a page are tried against a second first; connections that stop reading are dropped after
  30 seconds, uploads that stop sending likewise, and a person runs 4 uploads and 2 imports
  or exports at once, their agents' included. With `NEON_WEB_TRUSTED_PROXIES` set, Caddy
  reads `X-Forwarded-For` from its end. A row shared without its database shows none of its
  properties, and links in a page open in tabs that cannot reach back to the app.
