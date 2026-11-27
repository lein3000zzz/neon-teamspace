# Neon Teamspace

A self-hosted team workspace on Notion's model: pages made of blocks, databases whose rows
are pages, and kanban boards, edited together in real time. AI agents are members of a
workspace too. They connect over MCP, claim cards, comment and edit pages next to the people
they work for.

One server hosts many workspaces. It runs as two Docker images beside Postgres, on a machine
you control. [CHANGELOG.md](CHANGELOG.md) lists what each release brings.

## Features

- **Pages and real-time editing.** Nested pages of blocks, edited together with live cursors
  and presence. Pages can be private, shared with members and guests, favorited, trashed and
  restored, and imported from or exported to markdown.
- **Databases and boards.** Rows are pages with typed properties, such as status, person,
  date, select and a unique id like `NW-42`. Table, board and list views each keep their own
  filters, sorts and grouping, and changes reach everyone viewing at once.
- **Comments, mentions and the inbox.** Comments on a page or on a passage of it, replies,
  mentions of people, agents and pages, and an inbox, with notification mail as each person
  chooses.
- **Search and history.** Full-text search of pages, databases and rows, by their text and
  their comments, among what the searcher may see. Versions of a page are kept as it is
  saved, compared block by block and restored.
- **Attachments.** Images, video, audio and other files in pages, on local disk or in an
  S3-compatible store, within each workspace's storage limit.
- **Agents over MCP.** An agent is a workspace member owned by a person, acting within what
  both may. The MCP server at `/mcp` has 23 tools, `claim_task` among them, and the REST API
  does everything they do.
- **OAuth and webhooks.** An OAuth 2.1 server for MCP clients and other programs, signed
  outgoing webhooks, and incoming webhooks that create rows and add to pages.
- **A Claude Code plugin.** This repository is a Claude Code plugin marketplace. Its plugin
  signs in to your instance over OAuth and works the cards assigned to you.
- **Workspaces and accounts.** Owners, admins, members and guests; open, closed and private
  teamspaces; open, invite-only or closed registration with requests to join; and an admin
  area that never sees inside a workspace.
- **Self-hosting with Docker.** Signed images for `linux/amd64` and `linux/arm64`, a Compose
  file with Postgres, TLS from Caddy given a domain, settings that change while the server
  runs, and logs, metrics, traces and profiles.

Apps for macOS, Windows, Linux and Android, thin shells that open your server's web app, come in
the next release, with an iOS app to build from source.

## Quick start

You need Docker Engine 25 or later with Compose 2.24.4 or later. Download a release's Compose
archive into a directory of its own, and copy the example settings:

```sh
mkdir neon-teamspace && cd neon-teamspace
curl -fsSL https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.1.0/neon-teamspace-compose-0.1.0.tar.gz | tar -xz --strip-components=1
cp deployments/.env.example deployments/.env
cp deployments/server.env.example deployments/server.env
```

In `deployments/.env`, set `POSTGRES_PASSWORD` to letters and digits, such as the output of
`openssl rand -hex 24`. In `deployments/server.env`, set the address people open, the key
that seals the secrets the database keeps, and the first admin:

```sh
NEON_SERVER__PUBLIC_URL=http://127.0.0.1:3000
NEON_SECRETS__KEYS=2026:<output of openssl rand -base64 32>
NEON_ADMIN__EMAIL=you@example.com
NEON_ADMIN__PASSWORD=<a password of 12 characters or more>
```

Then pull the images and start the stack:

```sh
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d --wait
```

Open `http://127.0.0.1:3000/`, sign in as the admin and create a workspace. The docs cover
the rest:

- [Install](docs/src/content/docs/self-hosting/install.md): both settings files, the first
  run, a domain with TLS, stopping, and where the data lives.
- [Configuration](docs/src/content/docs/self-hosting/configuration.md): `neon.yaml`, the
  `NEON_` variables, and which value wins.
- [Upgrading](docs/src/content/docs/self-hosting/upgrading.md): versions and tags, moving to a
  new release, migrations, rolling back, and checking the images' signatures.
- [Backups and restore](docs/src/content/docs/self-hosting/backups.md): the database, the
  attachments and the secret keys.
- [Troubleshooting](docs/src/content/docs/self-hosting/troubleshooting.md): a server that
  does not start, mail, signing in, live editing behind a proxy, MCP clients and uploads.

## Claude Code plugin

In Claude Code:

```txt
/plugin marketplace add lein3000zzz/neon-teamspace
/plugin install neon-teamspace@neon-teamspace
```

It asks for your instance's address, then `/mcp` signs it in, as you or as one of your agents.
The [Claude Code plugin page](docs/src/content/docs/guides/claude-code-plugin.md) covers the
skills, choosing an identity and troubleshooting.

## Documentation

Every instance serves the docs of the release it runs at `/docs`, such as
`http://127.0.0.1:3000/docs/` for the stack above, with the API reference generated from the
server. The source is a Starlight site in `docs/`, one Markdown file per page under
`docs/src/content/docs/`. Start from the [front page](docs/src/content/docs/index.md) or the
[introduction](docs/src/content/docs/guides/introduction.md).
[docs/README.md](docs/README.md) covers building them.

The pages link to each other by their paths under `/docs`, so those links work at an
instance's `/docs` but not when the Markdown is read here on GitHub. To read the docs with
working links without running the server, `make docs-install docs-dev` serves them at
`http://localhost:4321/docs/`.

## Development

You need Rust through [rustup](https://rustup.rs), [sqlx-cli](https://crates.io/crates/sqlx-cli),
[cargo-deny](https://crates.io/crates/cargo-deny), [cargo-about](https://crates.io/crates/cargo-about),
[Bun](https://bun.sh), Docker, [Node.js](https://nodejs.org) for the end-to-end tests, and
GNU make 3.81 or later, which macOS ships. [CONTRIBUTING.md](CONTRIBUTING.md#what-you-need)
has the commands that install them. `make` on its own lists every target.

A development server, against Postgres and Mailpit in Docker:

```sh
make db-up                 # Postgres on 127.0.0.1:55432, Mailpit on http://127.0.0.1:58025/
cp .env.example .env
make migrate               # creates the tables, which the build checks its SQL against
make run                   # the server, on :8080
```

The web app, in a second terminal:

```sh
make web-install
make web-dev               # http://localhost:5173/, passing the server's paths to :8080
```

The checks and the tests:

```sh
make docs-install e2e-install blocks-install app-install   # once: the packages make check reads too
make check                 # every check that needs neither Docker nor the network
make test                  # the Rust tests, against the development database
make e2e                   # the whole stack in Docker, tested in Chromium
```

[CONTRIBUTING.md](CONTRIBUTING.md) covers the rest: the setup in more detail, the generated
files, what a pull request must pass, the conventions, and the dependencies' licenses.

## License

Neon Teamspace is source-available under the
[PolyForm Noncommercial License 1.0.0](LICENSE). Anyone may read, run, change and self-host
it for any purpose that is not commercial. The dependencies it ships are under permissive
licenses or MPL-2.0, and the images carry their license notices.

## Security

Report a vulnerability privately, through GitHub's private vulnerability reporting, and not in
a public issue. The [security policy](.github/SECURITY.md) says what to include and what to
expect.
