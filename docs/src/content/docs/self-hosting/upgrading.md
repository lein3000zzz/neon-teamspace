---
title: Upgrading
description: How releases are numbered and tagged, their notes, moving a Compose install to a new release from its Compose archive, a clone or the admin settings, the server's release checks and what they send GitHub, how the server migrates the database as it starts and what a failed migration leaves, rolling back with a backup, one-click updates with the updater, and checking the images.
---

## Releases

A release is a Git tag, `v` and a version such as `v0.1.0`, and the images built from it. It
has one version for the server image, the web image, the updater image, the Compose file and
the [Claude Code plugin](/docs/guides/claude-code-plugin/). The server says its version in
`GET /api/v1/meta` and in `neon --version`.

Versions follow [semantic versioning](https://semver.org). Until 1.0.0, a new minor version,
such as 0.2.0 after 0.1.x, may change the API, the settings or the behaviour in a way that
asks something of whoever runs or calls the server, and its notes say what; a patch version,
such as 0.1.1, fixes what its minor version shipped. A pre-release, such as `0.2.0-rc.1`,
comes before the release of its number.

Each release publishes its images, `ghcr.io/lein3000zzz/neon-teamspace-server`,
`ghcr.io/lein3000zzz/neon-teamspace-web` and `ghcr.io/lein3000zzz/neon-teamspace-updater`,
which the [updater](#one-click-updates) runs, for `linux/amd64` and `linux/arm64`, under
three tags:

| Tag | Moves |
|---|---|
| `0.1.0`, the version | Never: it names one release. |
| `0.1`, the minor version | To each patch release of 0.1 as it is published. |
| `latest` | To each release as it is published. |

From 1.0.0 on, the major version, such as `1`, is a tag too, and moves to each release of 1.
A pre-release, such as `v0.2.0-rc.1`, gets its own version as its tag alone, and never moves
`latest` or the others. Its GitHub release is marked as a pre-release. A release candidate,
`-rc.` and a number, rehearses the release of its version on the same commit, so its Compose
archive is that release's: it runs the candidate's images only with `NEON_VERSION` set to the
candidate's version, such as `0.2.0-rc.1`.

The Compose file runs the images of its own release, which its `image` lines name, unless
`NEON_VERSION` in `deployments/.env` names another. Set to `0.1`, it follows patch releases
as they are pulled; set to `latest`, it follows every release, and each `pull` may then take
the instance to a release whose migrations cannot be undone. On an instance whose data
matters, leave it unset, or name a full version: the instance then changes only when you
upgrade it, and the backup you take first matches what runs.

## The release notes

Read them before an upgrade, for every release between the one running and the new one.
They are in two places:

- `CHANGELOG.md` at the repository's root, by area, with a section for each release. A
  change that needs something of you, a setting renamed, a route that answers differently, a
  step to run after the upgrade, is called out there under **Breaking changes**, with what to
  do.
- The release's own page among the repository's
  [releases](https://github.com/lein3000zzz/neon-teamspace/releases). Its notes name the
  images, their platforms and their tags, say how to check their signatures, and link the
  install page, this page and the changelog; the pull requests merged since the full release
  before it come after, as GitHub lists them. The release's Compose archive is attached to it.

## Upgrading a Compose install

1. Read the notes of every release between the one running and the new one.
2. [Back up](/docs/self-hosting/backups/) the database, the files and the secrets. Migrations
   only go forward, so this backup is the way back.
3. Get `deployments/` of the new release, as its Compose file may change with it, and start
   it: from its [Compose archive](#with-the-compose-archive), or
   [built from a clone](#built-from-a-clone).
4. Check `GET /api/v1/meta` for the new version, and the server's log for warnings, as
   [What happens](#what-happens) shows.

### With the Compose archive

In the directory the [install](/docs/self-hosting/install/#starting-it) unpacked the archive
in, unpack the new release's over it, then pull and start its images:

```sh
curl -fsSL https://github.com/lein3000zzz/neon-teamspace/releases/download/v0.1.0/neon-teamspace-compose-0.1.0.tar.gz | tar -xz --strip-components=1
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d --wait
```

With the GitHub CLI, signed in, `gh release download v0.1.0 --repo lein3000zzz/neon-teamspace --pattern 'neon-teamspace-compose-*.tar.gz' --clobber`
fetches the archive instead, for `tar -xzf neon-teamspace-compose-0.1.0.tar.gz --strip-components=1`
to unpack.

Unpacking replaces the Compose file and the two example files, and keeps `deployments/.env`
and `deployments/server.env`, which the archive does not hold. A release may add settings, so
compare the new examples with the files made from the old ones:

```sh
diff deployments/.env.example deployments/.env
diff deployments/server.env.example deployments/server.env
```

With `NEON_VERSION` set in `deployments/.env`, set it to the new release too, or leave it
blank to run the archive's own.

### Built from a clone

Check out the release's tag, then build and start both images from it:

```sh
git fetch --tags
git checkout v0.1.0
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml up -d --build --wait
```

## Updating from the admin settings

**Admin > Settings** opens with **Version and updates**: the release the server runs, the
newest release its last [release check](#release-checks) saw, linked to its notes, when it
checked, and why the last check failed if it did. **Check now** asks GitHub again. While a
newer release is out, the **Admin** entry of the sidebar carries a mark for instance admins.
A development build, as `cargo run` and `make run` make, is never offered an update: it is
updated from its code.

Without an updater, the panel gives the commands that update by hand, for the release it
found: set `NEON_VERSION` to it in `deployments/.env`, then, in the directory that holds
`deployments/`, pull its images and restart the stack with them:

```sh
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d --wait
```

These keep the Compose file you have. A release that changes it says so in its notes; then
get the new one from the release's [Compose archive](#with-the-compose-archive) instead.

With the updater set up, as [One-click updates](#one-click-updates) describes, the panel
offers **Update to** the release instead. After a confirmation, the server asks the updater
to update the stack, and the updater pulls the release's images and restarts the server and
the web container with them, as [What happens](#what-happens) describes; the new server
migrates the database as it starts. The page follows the update with the end of the
updater's log, waits through the restart, and reloads once `GET /api/v1/meta` answers with
the new version, so it runs the new release's app. For a release candidate, that is the
version it rehearses: 0.3.0 for 0.3.0-rc.1. An update that fails before the server
and web are recreated, as when a pull or the signature check fails, leaves the stack as it
was, and the panel shows the updater's error and its log. One that fails later, as when the
new server does not start, leaves `deployments/.env` naming the new release while no server
answers, and the page waits for it. After ten minutes without the server back on the new
release, the page stops waiting and says so; the server's log and the updater's then say what
happened, and [When an update fails](#when-an-update-fails) says what to do.

The server asks for no other update than this: a release newer than its own that its last
check saw on GitHub, never an older release and never another tag, so the release check must
be on. Only an instance admin at a browser may update; an API token with the `admin` scope
reads the status and checks, but cannot update. The server logs each update it asked for,
`the updater was asked to update the stack to a release`, with the admin's id and the
version. When the updater's answer does not say whether it started, as when none comes
within 10 seconds, the server logs a warning instead, ending `the update may be under way`,
with the admin's id, the version and why. Read the notes and take a
[backup](/docs/self-hosting/backups/) first, as for any upgrade: the panel cannot undo a
migration either.

A release candidate's server states the version it rehearses: on 0.3.0-rc.1,
`GET /api/v1/meta` and the panel say 0.3.0. So the server of an instance on a candidate
offers neither a later candidate of that version nor its release; update to those by hand,
as [Upgrading a Compose install](#upgrading-a-compose-install) describes. A later version,
such as 0.3.1, is offered as usual.

The same is in the API, for instance admins: `GET /api/v1/admin/updates` for the status,
`POST /api/v1/admin/updates/check` to check now, and `POST /api/v1/admin/updates/apply`, with
the release's version in its `version` field, to update, from a browser session. The
[API reference](/docs/reference/api/) has their answers and error codes.

## Release checks

While `updates.check` is on, as it is unless set, the server asks GitHub for the releases of
Neon Teamspace as it starts and then once a day, an hour after a check that failed, and
whenever an admin presses **Check now**. It keeps what it found in memory, so a restart asks
again. Of the releases, the newest by semantic versioning counts: never a draft, and a
pre-release only while `updates.prereleases` is on.

A check is one request, without credentials, to GitHub's API:
`GET https://api.github.com/repos/lein3000zzz/neon-teamspace/releases?per_page=30`, the 30
latest releases, with the `User-Agent` `NeonTeamspace/` and the server's version, such as
`NeonTeamspace/0.1.0`, and the `Accept` and `X-GitHub-Api-Version` headers GitHub asks for.
It sends nothing else: no account, no instance name, no address of the instance, no token.
GitHub sees the request come from the server's address, with that version. The server reads
at most 4 MiB of the answer, and gives up after 20 seconds. GitHub allows an address 60 such
requests an hour; a check refused for it fails with GitHub's status, and the next tries
again an hour later.

To turn the checks off, set `updates.check` off in **Admin > Settings**, or pin it with
`NEON_SETTINGS__UPDATES__CHECK=false` in `deployments/server.env`. Off, the server never asks
GitHub, forgets what it found, and offers no update from the admin settings; the panel says
the check is off, and updating works [by hand](#upgrading-a-compose-install) as before.
Where the server cannot reach GitHub, the config file's `updates.github_api_url` names a
mirror of its API instead, as [Configuration](/docs/self-hosting/configuration/#the-config-file)
describes.

## What happens

Compose replaces the containers whose image or settings changed. The old server stops as
[Stopping](/docs/self-hosting/install/#stopping) describes. The new one migrates the
database, as [below](#migrations), and logs `database migrations applied` when it is done,
then takes requests; `/readyz` answers `503` until then. Meanwhile Caddy answers `502`, and
open pages show **Reconnecting** and keep what is typed until the server is back. The new web
container starts once the new server is healthy. MCP sessions live in the server's memory,
so MCP clients start new ones. A server that was killed rather than stopped may leave pages
whose latest text search does not read yet; `neon search reindex` stores it, as
[Search](/docs/guides/search/#rebuilding-the-text) explains.

Then check that every service is healthy and that the new release runs:

```sh
docker compose -f deployments/docker-compose.yml ps
docker compose -f deployments/docker-compose.yml exec server neon --version
curl -s http://127.0.0.1:3000/api/v1/meta
```

```json
{"name":"Neon Teamspace","version":"0.1.0"}
```

The foot of the app's sidebar shows the same version.

## Migrations

The server applies pending migrations as it starts, before it takes requests, unless
`database.auto_migrate` is `false`; `neon admin create` and `neon search reindex` do too,
before their own work. `neon migrate` applies them whatever the setting says, and exits.
The migrations are built into the binary. The server:

1. takes a Postgres advisory lock, so two servers that start at once do not both migrate;
2. checks the migrations the database lists in `_sqlx_migrations` against its own: each must
   be one it has, unchanged;
3. applies each pending one in order, each in a transaction of its own with its row in
   `_sqlx_migrations`, so a migration is applied whole or not at all.

While it migrates, the admin listener answers `/livez` with `200` and `/readyz` with `503`,
so the container is not healthy yet; the Compose file gives it two minutes to start before
failed checks count. Migrations are never changed once released: a change to the schema is a
new migration.

A migration that fails rolls back alone: the database stays as the migrations before it left
it, those of the same upgrade included. The server logs `stopping after a failure` with the
error, prints it, and exits with status 1, such as:

```txt
Error: applying database migrations

Caused by:
    0: while executing migration 20261121165322: error returned from database: relation "jobs" already exists at line 1488
    1: error returned from database: relation "jobs" already exists at line 1488
    2: relation "jobs" already exists at line 1488
```

Docker starts it again, as the Compose file says `restart: unless-stopped`, and each start
tries the same migration, so the server keeps restarting and never turns healthy:
`up --wait` stops with `dependency failed to start: container neon-server-1 is unhealthy`, and
`docker compose -f deployments/docker-compose.yml logs server` shows the error. Restore the
backup and go back to the release before, as [below](#rolling-back), then report the error.

To migrate apart from the start, to watch it say, set
`NEON_DATABASE__AUTO_MIGRATE=false` in `deployments/server.env`, pull the new images, and
run the migrations in a one-off container of the new server before starting it:

```sh
docker compose -f deployments/docker-compose.yml run --rm --no-deps server migrate
docker compose -f deployments/docker-compose.yml up -d --wait
```

A server with `auto_migrate` off never checks the database: it must not start on a database
older or newer than its own migrations.

## Rolling back

A server refuses a database that a newer release migrated, naming the migration it does not
know:

```txt
Error: applying database migrations

Caused by:
    migration 20270101000000 was previously applied but is missing in the resolved migrations
```

So rolling back is a restore of the backup taken before the upgrade. First point the stack
at the older release: unpack its Compose archive over the directory again, or check out its
tag in a clone, and set `NEON_VERSION` back if it names the newer one. Setting `NEON_VERSION`
to the older release runs its images too, with the newer Compose file. Then restore the
backup [over the instance](/docs/self-hosting/backups/#over-an-instance-that-runs), whose
last command starts the older release's images; built from a clone, start them with
`docker-compose.build.yml` and `--build` instead, as [above](#built-from-a-clone).
Everything changed since the backup is lost. An upgrade without migrations can go back by its
image alone, but the notes do not promise which releases have none: take the backup either
way.

## One-click updates

The Compose file's `updater` profile runs the updater, `neon-updater`, beside the stack, so
that an instance admin can update the instance from **Admin > Settings**. The server checks
for new releases either way, and shows a newer one there with its notes and the commands that
update by hand, as [above](#updating-from-the-admin-settings). With the updater running,
**Update** does it instead: the server asks the updater, which pulls the release's server,
web and updater images, checks their signatures, keeps the release in `deployments/.env`,
recreates the server and web, then updates itself.

It updates a stack that runs the published images, as installed from a Compose archive. A
stack built from a clone with `docker-compose.build.yml` runs images of its own, which the
updater can neither pull nor check. With the signature check on, it refuses to update such a
stack; with it off, it reports the update done while recreating nothing, and the panel says
at once that the server still runs the release it ran. Update a stack built from a clone by
checking out the new tag, as in [Built from a clone](#built-from-a-clone).

0.1.0 has no updater. The update to 0.2.0, the first release with one, is by hand, and the
profile can be turned on from there.

### Turning it on

The server and the updater share a token of 32 characters or more, which the updater asks
for with every request but its health check. Make one:

```sh
openssl rand -hex 32
```

Put it in `deployments/.env`, with the profile:

```sh
COMPOSE_PROFILES=updater
NEON_UPDATER_TOKEN=<the output of openssl rand -hex 32>
```

Compose gives the token to the updater, and to the server as its `updates.updater_token`
setting, so the one value sets both. `COMPOSE_PROFILES` turns the profiles it lists on for
every `docker compose` command of the stack, so that a later `up` or `down` takes the updater
with the rest. A `--profile` on a command replaces the variable rather than adding to it:
list every profile the instance runs with, such as `COMPOSE_PROFILES=updater,mail`, and leave
`--profile` off the stack's commands. Then pull its image and start it:

```sh
docker compose -f deployments/docker-compose.yml pull
docker compose -f deployments/docker-compose.yml up -d --wait
```

**Admin > Settings** then shows the updater under **Version and updates**, whether or not a
newer release is out: ready, with its version, or why the server cannot reach it.

The updater listens on `updater:8080`, reachable from the server alone: the two share a
network of their own, which nothing else in the stack is on and which leads nowhere outside.
The server reaches it at `http://updater:8080`, the default of the server's
`updates.updater_url` setting, and it is published on no port of the machine. It reads these
variables, of which the Compose file passes the token and `NEON_UPDATER_VERIFY` from
`deployments/.env`; a Compose file of your own beside it can set the others:

| Variable | Default | What it sets |
|---|---|---|
| `NEON_UPDATER_TOKEN` | none | The token. The updater refuses to start without one of 32 characters or more. |
| `NEON_UPDATER_ADDR` | `0.0.0.0:8080` | The address and port it listens on. |
| `NEON_UPDATER_SERVICES` | `server,web` | The Compose services an update updates, separated by commas. |
| `NEON_UPDATER_PROJECT` | its own container's | The Compose project of the stack it updates. |
| `NEON_UPDATER_IMAGE` | its own container's | The image each update runs in. |
| `NEON_UPDATER_VERIFY` | `true` | Whether an update checks the images' signatures, as [below](#the-signature-check). |

### What it may do

The updater holds the Docker socket, `/var/run/docker.sock`, and whoever controls the Docker
daemon controls the machine as root. Whoever takes over the updater itself can do what root
can. Whoever holds the token can have it update the stack and nothing else, as below: to the
updater's own release or a newer one, signed by this repository's release workflow. With the
signature check off, that is whatever the registry holds under such a tag, so whoever can
push there can then do what root can too. The updater is built to do one thing with the
socket:

- It takes a release's version and nothing else: no image, command or path. A version is
  `major.minor.patch`, with a pre-release after a hyphen, such as `0.2.1` or `0.3.0-rc.1`.
  Anything else is refused with `400`, and so is a release older than the updater's own: it
  never goes back, so the token only moves the stack forward. Its own release is taken, so
  that a failed update can be tried again. Going back is done by hand, as
  [Rolling back](#rolling-back) describes.
- Every request but `/healthz` needs the token, which it compares in constant time, and a
  wrong or missing one gets `401`. It runs one update at a time, and refuses another while
  one runs with `409`.
- It updates the services `NEON_UPDATER_SERVICES` names, then itself, to the release's images
  as the Compose file names them, and nothing else: Postgres, the other profiles' services,
  the volumes and the Compose file stay as they are.
- It starts as root only to look at the socket's group, then serves as an unprivileged user
  in that group, without capabilities, where the group may use the socket, as Docker Engine's
  `root:docker` socket lets it.

Keep the token as you keep `POSTGRES_PASSWORD`: `deployments/.env` should be readable by the
machine's administrators alone. Leave the profile off to keep the socket out of the stack.

### What an update does

Each update runs in a container of its own, `neon-update`, started from the updater's image,
so that it carries on while the updater is replaced. The container sees the socket, and the
stack's Compose files and env files at the paths the stack was started with: their
directories read-only, the env files writable. It takes the stack's variables from those env
files alone, never from the shell or service manager that started the stack. A variable set
only there, such as `NEON_DOMAIN`, `NEON_HTTP_PORT`, `NEON_HTTPS_PORT` or
`NEON_UPDATER_TOKEN`, is unset for the update, and the recreated services fall back to the
Compose file's default for it. Keep every variable the stack needs in `deployments/.env`.

The update:

1. pulls the server's, web's and updater's images of the release;
2. checks each image's signature, as [below](#the-signature-check), and stops if one does
   not check out, the updater's included;
3. keeps the release in the last env file the stack was started with, `deployments/.env`, as
   `NEON_VERSION='0.2.1'` for 0.2.1, in place: the file's other lines and comments stay as
   they are, and a line is added at its end if none sets the variable;
4. recreates the server and web from the images it checked, without pulling again, and
   without touching the services they depend on;
5. recreates the updater the same way, last, from the image it checked with the others. Only
   this step may fail softly: the updater then stays as it was, and the update is done.

The new server migrates the database as it starts, as [What happens](#what-happens)
describes. The updater takes no backup: [take one](/docs/self-hosting/backups/) first, as for
any upgrade.

The updater does not replace the Compose file, which stays the one of the release it was
unpacked from, now running a newer release's images. A release whose Compose file changes
says so in its notes; update to it by hand, [with its Compose archive](#with-the-compose-archive).

### The signature check

Before it recreates anything, the update takes each image's digest as Docker pulled it, and
checks with cosign that the release workflow signed that digest for the release's tag: the
signing certificate's identity must be
`https://github.com/lein3000zzz/neon-teamspace/.github/workflows/release.yml@refs/tags/v`
followed by the version, and its issuer `https://token.actions.githubusercontent.com`. It is
the check [Checking the images](#checking-the-images) does by hand, held to the one tag. An
image without such a signature, or one cosign cannot check, as when the registry or Sigstore
cannot be reached, stops the update before anything changes.

Images built outside this repository's release workflow and pushed to a registry carry no
such signature. For them, set `NEON_UPDATER_VERIFY=false` in `deployments/.env` and start the
updater again with `docker compose -f deployments/docker-compose.yml up -d`. Updates then
recreate the services from whatever the registry holds under the version's tag, unchecked,
and the update's container runs without a network. Leave the check on for the published
images.

### When an update fails

**Admin > Settings** shows the error and the end of the update's output, and the updater's
log holds each line of it:

```sh
docker compose -f deployments/docker-compose.yml logs updater
```

What to do depends on how far it went:

- **Before anything was recreated**, as when a pull failed or an image did not check out:
  nothing changed, `deployments/.env` included, and the stack runs as before. Fix the cause,
  such as the network or the registry, and press **Update** again. An image that did not
  check out, the updater's included, is one to keep off the machine: do not pull and start
  the release by hand after it, which would run that image unchecked.
- **While the server and web started**: `deployments/.env` names the new release, and the new
  server may be failing to start, such as on a migration, as [Migrations](#migrations) shows.
  `docker compose -f deployments/docker-compose.yml ps` and the server's log say which. To go
  back, [roll back](#rolling-back): set `NEON_VERSION` in `deployments/.env` back to the
  release before, and restore the backup if the new server migrated the database.
- **The updater's own restart**: the server and web run the new release, and the updater the
  old one, which keeps working. Its new image was pulled and checked with the others, so
  `docker compose -f deployments/docker-compose.yml up -d --wait` brings it to the new
  release.

The updater replaces itself as each update ends, and the new one knows of no update before
it: its status says `idle`. `GET /api/v1/meta` and the foot of the sidebar say which release
runs.

An update cut off halfway, as by the machine restarting, can leave its container behind, and
the next update then fails at once, as the name `neon-update` is taken.
`docker rm -f neon-update` removes it.

## Checking the images

Each image carries a software bill of materials, which lists the packages in it, the crates
the `neon` binary links among them in the server image, and the provenance of its build,
which records the commit and the workflow that built it:

```sh
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .SBOM }}'
docker buildx imagetools inspect ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 --format '{{ json .Provenance }}'
```

Each release's images are signed with [cosign](https://docs.sigstore.dev/cosign/)'s keyless
signing: the release workflow signs each image, and each platform's image in it, with its
own GitHub identity, which the signature's certificate and Sigstore's public transparency log
record with the repository and the commit. To check one:

```sh
cosign verify ghcr.io/lein3000zzz/neon-teamspace-server:0.1.0 \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/lein3000zzz/neon-teamspace/\.github/workflows/release\.yml@refs/tags/v'
```

The command needs cosign 3, or cosign 2.6 or later with `--new-bundle-format`. The workflow
writes each signature as a Sigstore bundle, and as GHCR has no OCI referrers API, cosign
stores the bundle under a tag of the image's package named `sha256-` and the signed image's
digest. Older cosign, and tools that read only cosign's older `.sig` tags, find no signature.
