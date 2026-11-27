---
title: Backups and restore
description: What to back up, the database with pg_dump, attachments on local disk or in SeaweedFS, and the secret keys; taking a consistent backup while the server runs, restoring onto a new machine, testing the restore, and what a partial restore loses.
---

An instance keeps what it knows in three places, and a backup needs all three:

| What | Where, in the Docker stack | Without it |
|---|---|---|
| The database | The volume `neon_postgres` | Nothing is left: accounts, workspaces, pages and their history, databases, comments, webhooks and the settings saved on the admin page are all in it. |
| Attachments' bytes | The volume `neon_server-data`, or the bucket of an S3-compatible store, such as the `seaweedfs` profile's volume `neon_seaweedfs` | Every image and file answers `404 file_missing`; the pages that show them keep their place. |
| The secret keys | `NEON_SECRETS__KEYS` in `deployments/server.env` | The database restores, but the secrets it keeps sealed no longer open; see [below](#what-a-partial-restore-loses). |

Caddy's certificates, in `neon_caddy-data`, need no backup: Caddy gets new ones for
`NEON_DOMAIN` as it starts, within the certificate authority's rate limits. The
`observability` profile's volume holds only telemetry.

The commands below run from the directory that holds `deployments/`, for the stack the
[install page](/docs/self-hosting/install/) starts, whose Compose project is `neon`. Another
project name changes the volumes' prefix, and the network's, `neon_default`:
`docker volume ls` lists them.

## The database

`pg_dump`, in the stack's Postgres container, writes the database to a file on the host in
its custom format: compressed, and read by `pg_restore`, which restores it in one
transaction.

```sh
mkdir -p backup
docker compose -f deployments/docker-compose.yml exec -T postgres \
  pg_dump -U neon -d neon --format=custom > backup/neon.dump
```

`-T` runs the command without a terminal, which would turn the dump's line feeds into
carriage returns and line feeds and break it.

The dump reads the database as one snapshot, as it was when the dump began, whatever is
written while it runs. So it runs while the server runs and people work. It blocks no reads
or writes, only changes to the schema: an upgrade's migrations wait for a dump that is
running. The edits of pages open in browsers are in it too, up to a quarter of a second
before it began, as the server logs each update it applies within that.

To check that a dump is readable, list what it holds:

```sh
docker compose -f deployments/docker-compose.yml exec -T postgres pg_restore --list < backup/neon.dump | head
```

The dump holds the database `neon` and nothing outside it: not Postgres's users, as the
stack makes `neon` itself on a new volume, and not the secret keys.

## Attachments

### On local disk

Attachments on local disk, the default, are in the server's data directory, the volume
`neon_server-data`. A container with `tar` copies it whole, with each file's owner: the
server runs as user 65532, and must still be able to write there after a restore.

```sh
docker run --rm -v neon_server-data:/data:ro -v "$PWD/backup":/backup busybox:1.37 \
  tar czf /backup/server-data.tar.gz -C /data .
```

Run from source, the directory is `server.data_dir`, `data` in the working directory unless
set; `rsync -a` or `tar` copies it.

### In SeaweedFS

With the `seaweedfs` profile, there are two ways.

**A copy of the bucket**, through its S3 gateway, while everything runs. The gateway is not
published, so the copy runs on the stack's network, here with
[rclone](https://rclone.org) and the access key and secret from `deployments/.env`. The
access key is `neon` when `.env` leaves it blank, as the Compose file has it:

```sh
set -a; . deployments/.env; set +a
RCLONE_S3_ACCESS_KEY_ID="${SEAWEEDFS_ACCESS_KEY:-neon}" RCLONE_S3_SECRET_ACCESS_KEY="$SEAWEEDFS_SECRET_KEY" \
docker run --rm --network neon_default -v "$PWD/backup":/backup \
  -e RCLONE_S3_PROVIDER=SeaweedFS -e RCLONE_S3_ENDPOINT=http://seaweedfs:8333 \
  -e RCLONE_S3_ACCESS_KEY_ID -e RCLONE_S3_SECRET_ACCESS_KEY \
  rclone/rclone:1.76.0 sync :s3:neon /backup/bucket
```

The secret reaches the container through the environment, so it never shows in the command
lines that `ps` lists. `backup/bucket/` then holds a file per attachment, under the key the
server gave it, `<workspace>/<file>`.

**SeaweedFS's own data**, the volume `neon_seaweedfs`: its master's state, its volume
server's `.dat` and `.idx` files, which hold the bytes, and its filer's metadata, which maps
the bucket's keys to them. They change together, so copy them while SeaweedFS is stopped.
Meanwhile uploads and file reads fail, and the rest of the instance works.

```sh
docker compose -f deployments/docker-compose.yml stop seaweedfs
docker run --rm -v neon_seaweedfs:/data:ro -v "$PWD/backup":/backup busybox:1.37 \
  tar czf /backup/seaweedfs.tar.gz -C /data .
docker compose -f deployments/docker-compose.yml --profile seaweedfs start seaweedfs
```

A bucket of another store is backed up with that store's own means, such as versioning,
replication or snapshots, or with `rclone sync` or `aws s3 sync` to another place.

### Dump first, then the files

Take the dump before the copy of the files. A file uploaded between the two is then in the
copy without a row, which does no harm: the server never reads it, nor deletes what it does
not know. In the other order, a row could name a file the copy lacks. A file deleted between
the two comes back as a row without bytes, which answers `404 file_missing` until the cleanup
deletes it, once no page shows it. For a backup without that gap, stop the server for both,
with `docker compose -f deployments/docker-compose.yml stop server`, and start it again after.

## The secret keys

`secrets.keys` seals the secrets the database keeps, each bound to its row:

- the copies of invite codes and of workspace invite links, which admins show again;
- the secret settings saved on the admin page, `mail.password` and
  `attachments.s3.secret_access_key`;
- mail waiting in the job queue, as it carries links that work once;
- the signing secrets of outgoing webhooks, and of incoming webhooks that check signatures.

The keys also sign the links in notification mail that stop it.

Back up `deployments/server.env`, which holds `NEON_SECRETS__KEYS`, and `deployments/.env`,
with the backups but not beside them: whoever has a dump and its keys opens every sealed
secret, so keep them where one leak does not give away both, such as the files in a
password manager. `deployments/.env` holds the database's password and SeaweedFS's keys. A
restore needs neither as it was, as a new volume takes the password `.env` names, but a copy
saves choosing new ones. After a [rotation](/docs/self-hosting/configuration/#secret-keys),
keep the old key in the list, and in the backups, while values it sealed remain.

## Restoring onto a new machine

Restore with the release the backup was taken with, then upgrade if need be, as
[Upgrading](/docs/self-hosting/upgrading/) describes. A newer release migrates the database
forward as it starts, and the database cannot then go back without the backup.

1. Install Docker, and get `deployments/` of that release: its Compose archive, unpacked as
   [Install](/docs/self-hosting/install/#starting-it) shows, or a clone at its tag. Put the
   backed-up `.env` and `server.env` back in it, and check that `NEON_VERSION` is blank or
   names the release.
2. Create the stack's containers and empty volumes without starting them, then start
   Postgres alone. The server must not start before the database is restored: it would
   apply the migrations to the empty database, and `pg_restore` would then fail on the
   tables they made.

   ```sh
   docker compose -f deployments/docker-compose.yml up --no-start
   docker compose -f deployments/docker-compose.yml up -d --wait postgres
   ```

3. Restore the dump in one transaction, which stops at the first error and leaves the
   database empty:

   ```sh
   docker compose -f deployments/docker-compose.yml exec -T postgres \
     pg_restore -U neon -d neon --single-transaction --exit-on-error < backup/neon.dump
   ```

4. Put the files back. From local disk:

   ```sh
   docker run --rm -v neon_server-data:/data -v "$PWD/backup":/backup:ro busybox:1.37 \
     tar xzf /backup/server-data.tar.gz -C /data
   ```

   From SeaweedFS's own data, create its container and volume, extract the copy into the
   volume, and start it:

   ```sh
   docker compose -f deployments/docker-compose.yml --profile seaweedfs up --no-start seaweedfs
   docker run --rm -v neon_seaweedfs:/data -v "$PWD/backup":/backup:ro busybox:1.37 \
     tar xzf /backup/seaweedfs.tar.gz -C /data
   docker compose -f deployments/docker-compose.yml --profile seaweedfs up -d --wait seaweedfs
   ```

   From a copy of the bucket, start SeaweedFS empty, with the last of those commands, and run
   the `rclone` command of [the backup](#in-seaweedfs) with `copy /backup/bucket :s3:neon` in
   place of `sync :s3:neon /backup/bucket`.
5. Start the rest, with the profiles the instance runs with:

   ```sh
   docker compose -f deployments/docker-compose.yml up -d --wait
   ```

6. Check it. `docker compose -f deployments/docker-compose.yml logs server` says
   `an admin can sign in already; admin.email and admin.password are not used`, and never
   `saved value ignored`. Sign in, open a page that shows an image, and send a test event
   from a webhook's settings: the receiver should find it signed with the secret it has.

People who were signed in stay signed in, as their sessions are in the database and their
browsers keep the cookies. Point the domain's DNS at the new machine last.

### Over an instance that runs

To go back to a backup on the same machine, after a failed upgrade say, stop the server,
replace the database with an empty one, and restore into that. A restore over the old
database would leave behind the tables newer migrations made.

```sh
docker compose -f deployments/docker-compose.yml stop server
docker compose -f deployments/docker-compose.yml exec -T postgres dropdb -U neon --force neon
docker compose -f deployments/docker-compose.yml exec -T postgres createdb -U neon neon
docker compose -f deployments/docker-compose.yml exec -T postgres \
  pg_restore -U neon -d neon --single-transaction --exit-on-error < backup/neon.dump
docker compose -f deployments/docker-compose.yml up -d --wait
```

The files need no restore when they are newer than the dump: the files uploaded since stay
in the store, unread.

## Testing the restore

A backup is known to work once it has been restored. The procedure above was run end to end
on a throwaway stack of images built from the 0.1.0 code, with a page that shows an image,
an outgoing webhook to a receiver on the stack's network, and an incoming webhook that checks
signatures. The stack was backed up, then deleted with its volumes and its directory, and
restored into a new directory, as onto a new machine. So that it could run beside a live
instance, it had a project name of its own, and other ports in its `.env`; its volumes were
`neon-drill_postgres` and so on. The commands, run from `old/` and then `new/`, each holding
a copy of `deployments/`:

```sh
export COMPOSE_PROJECT_NAME=neon-drill

# The backup, with the old stack running.
docker compose -f deployments/docker-compose.yml exec -T postgres \
  pg_dump -U neon -d neon --format=custom > ../backup/neon.dump
docker run --rm -v neon-drill_server-data:/data:ro -v "$PWD/../backup":/backup busybox:1.37 \
  tar czf /backup/server-data.tar.gz -C /data .
cp deployments/.env ../secrets/stack.env
cp deployments/server.env ../secrets/server.env

# The old machine, gone.
docker compose -f deployments/docker-compose.yml down --volumes
cd .. && rm -rf old

# The new machine: a fresh copy of deployments/, with the secrets put back.
cd new
cp ../secrets/stack.env deployments/.env
cp ../secrets/server.env deployments/server.env
docker compose -f deployments/docker-compose.yml up --no-start
docker compose -f deployments/docker-compose.yml up -d --wait postgres
docker compose -f deployments/docker-compose.yml exec -T postgres \
  pg_restore -U neon -d neon --single-transaction --exit-on-error < ../backup/neon.dump
docker run --rm -v neon-drill_server-data:/data -v "$PWD/../backup":/backup:ro busybox:1.37 \
  tar xzf /backup/server-data.tar.gz -C /data
docker compose -f deployments/docker-compose.yml up -d --wait
```

Then, through the API:

- the session cookie from before the backup still answered `200` on `/api/v1/account/me`;
- the page's markdown named the image, and `GET` on the image answered `200` with the
  SHA-256 of the file uploaded;
- the outgoing webhook's test event was `delivered`, and the `Neon-Webhook-Signature` the
  receiver got matched the secret shown when the webhook was registered;
- a request to the incoming webhook, signed with its secret, answered
  `200 {"ok":true,"action":"append"}`.

Both ways of backing up SeaweedFS were restored the same way, each into an empty store, and
the file read back with its SHA-256; so was the restore over an instance that runs.

## What a partial restore loses

| Restored | What happens |
|---|---|
| The database without the files | Each attachment answers `404 file_missing`, and the server logs `a file's bytes are missing from the blob store`. Pages keep their text, and new uploads work. |
| The files without the database | Nothing comes back: the files are named by ids only the database holds. |
| The database without its keys, or with others | The server starts, and logs a warning for each secret setting saved on the admin page, such as `mail.password: saved value ignored: it was sealed with key "2026", which secrets.keys no longer lists`. Outgoing webhooks fail: a test event answers `500`, and deliveries fail as `internal` until five in a row turn the webhook off. Incoming webhooks that check signatures answer `500`. For both, the server logs `a webhook's secret does not open: it was sealed with key "2026", which secrets.keys no longer lists`. Invite codes and invite links can no longer be shown again, mail waiting in the queue is dropped, and the links that stop notification mail no longer work. |
| An older database than the instance had | Everything since the dump is gone: pages, rows, comments, accounts. Files uploaded since stay in the store, unread. Sessions and API tokens revoked since the dump work again, so revoke them again. Jobs that were queued when the dump began run again, so some mail and webhook deliveries may go out twice; receivers drop the events whose `id` they have seen. |

Putting the right keys back mends the third case: every sealed value opens again. A key
lost for good loses what it sealed. Save the secret settings again, make new invites and
invite links, and give each webhook a new secret. Rotating an outgoing webhook's secret once
is not enough. For 24 hours the server also tries to sign with the secret from before the
rotation, and that one does not open, so deliveries still fail. Rotate it twice, or register
the webhook again. An incoming webhook gets a new secret from **Stop checking
signatures** followed by **Require signatures**.
