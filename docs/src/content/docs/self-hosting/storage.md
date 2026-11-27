---
title: File storage
description: Where the server keeps attachments, on local disk or in an S3-compatible store such as SeaweedFS, how to set each up, the Compose profile that runs SeaweedFS, and backing files up with the database.
---

[Attachments](/docs/guides/attachments/), the files pages show and their cover images, live
outside Postgres. The database keeps what each file is, and its bytes go to a blob store:

| `attachments.storage` | Where files go |
|---|---|
| `local`, the default | A directory on the server's disk, `attachments` under `server.data_dir`. |
| `s3` | A bucket of an S3-compatible store: [SeaweedFS](https://github.com/seaweedfs/seaweedfs), MinIO, Garage, Ceph, Amazon S3 and others. |

The server keeps each file under a key of its own, `<workspace>/<file>` by their ids, and never
reuses one. Bytes stream to the store as they upload and from it as they are read. A change of
the settings applies from the next upload or read, without a restart, as the server opens the
store they name the first time it needs it.

Switching stores does not move the files kept in the old one. To switch, stop the server, copy
the files over with their keys, and start it with the new settings. `aws s3 sync` and
`rclone copy` copy a directory into a bucket, and back.

## Local disk

The files go in the `attachments` folder of `server.data_dir`, which is `data` in the working
directory unless set. The server makes the folder if it is missing. The Docker image sets
`NEON_SERVER__DATA_DIR` to `/var/lib/neon`, in the volume `neon_server-data`, which the user
the server runs as owns.

```yaml
server:
  data_dir: /var/lib/neon
```

A directory on local disk serves one server process. Several behind one address need a store
they share, such as an S3 bucket.

## An S3-compatible store

```yaml
settings:
  attachments:
    storage: s3
    s3:
      endpoint: http://seaweedfs:8333
      bucket: neon
      region: us-east-1
      access_key_id: neon
      path_style: true
```

| Setting | What it is |
|---|---|
| `attachments.s3.endpoint` | The store's URL, `http` or `https`, without a path. Empty for Amazon S3, whose endpoint comes from the region. |
| `attachments.s3.bucket` | The bucket. The server makes none, so it must exist, unless the store makes buckets as they are first written to, as SeaweedFS does. Needed while `storage` is `s3`. |
| `attachments.s3.region` | The bucket's region. Other stores take any, usually `us-east-1`. |
| `attachments.s3.access_key_id` | The access key that signs requests. |
| `attachments.s3.secret_access_key` | Its secret, a [secret setting](/docs/self-hosting/configuration/#secret-keys): keep it in `NEON_SETTINGS__ATTACHMENTS__S3__SECRET_ACCESS_KEY`, or save it on the admin page, which needs `secrets.keys`. |
| `attachments.s3.path_style` | `true` names the bucket in the URL's path, `http://store/bucket/key`, as SeaweedFS, MinIO and most stores want; `false` in its host, `http://bucket.store/key`, as Amazon S3 prefers. |

Requests go over HTTP/1.1, and over TLS 1.2 or later for `https` endpoints. The server checks
the store's certificate against the web PKI's roots, which it bundles. Connecting may take 5
seconds, and an answer's headers 30. A file larger than 8 MiB goes up as a multipart upload,
two parts of 8 MiB at a time, so an upload holds at most a few parts in memory. The access key
needs to read, write and delete objects in the bucket, and to make and finish multipart
uploads. It needs no other right.

Every setting that says where files go, the secret among them, changes only from a browser
session, as the mail server's settings do. A leaked API token cannot point uploads elsewhere.
Set in the environment or the file, these settings are
[pinned](/docs/self-hosting/configuration/#pinned-settings).

## SeaweedFS in the Compose stack

The Compose profile `seaweedfs` runs SeaweedFS beside the stack, with its S3 gateway on
`seaweedfs:8333` inside the stack's network and its data in the volume `neon_seaweedfs`. It
makes a bucket the first time the server writes to it, and the gateway checks requests against
the access key and secret in `deployments/.env`. Without a secret, it refuses to start.
SeaweedFS's master, volume and filer servers, which sit behind the gateway and take reads and
writes without the secret, listen only on the container's loopback, so the gateway is the one
way in, and its Iceberg and Lance servers are off.

```sh
# deployments/.env
SEAWEEDFS_ACCESS_KEY=neon
SEAWEEDFS_SECRET_KEY=...   # a long random value, such as: openssl rand -hex 32
```

```sh
# deployments/server.env
NEON_SETTINGS__ATTACHMENTS__STORAGE=s3
NEON_SETTINGS__ATTACHMENTS__S3__ENDPOINT=http://seaweedfs:8333
NEON_SETTINGS__ATTACHMENTS__S3__BUCKET=neon
NEON_SETTINGS__ATTACHMENTS__S3__ACCESS_KEY_ID=neon
NEON_SETTINGS__ATTACHMENTS__S3__SECRET_ACCESS_KEY=...   # the same secret
```

```sh
docker compose -f deployments/docker-compose.yml --profile seaweedfs up -d
```

Compose publishes no port of the gateway, so only the stack's containers reach it. SeaweedFS is
a separate program under Apache-2.0, not linked with Neon Teamspace's code.

## Backups

A backup is a `pg_dump` of the database and a copy of the files, taken together, the dump
first, so that no row names a file the copy lacks. The files go back under the same keys
before the server starts, and files deleted since the backup come back, rows and bytes.
[Backups and restore](/docs/self-hosting/backups/#attachments) has the commands: for local
disk, for SeaweedFS's bucket or its own data, and for other stores.

## What the server deletes

The hourly cleanup deletes the bytes of every file deleted, whether it went with its page,
with its workspace or once no page nor kept version had shown it for
`attachments.orphan_grace`.
[Attachments](/docs/guides/attachments/#when-files-are-deleted) says when files go. Each run
deletes them a thousand at a time until none are left, for up to ten minutes, so a workspace
of many files is gone from the store within the hour after it is deleted unless the store is
slow, and what is left then goes in the next run. Bytes the store refused to delete stay
listed for the next run.

An upload cut off while the server runs, by its client, its time limit or a refusal, leaves
nothing: the server deletes what it wrote, and aborts a multipart upload, at once. A server
that stops in the middle of an upload, killed or crashed, leaves bytes behind, which the
cleanup deletes half an hour after the upload began: on local disk, the file the blob was
being written to, beside where it would go. In an S3 bucket, a file larger than 8 MiB goes up
in parts, and the parts of an upload no one aborted stay in the store, outside any
workspace's storage, until something aborts it. Give the bucket a lifecycle rule that aborts
incomplete multipart uploads after a day, `AbortIncompleteMultipartUpload` on Amazon S3,
MinIO and the stores that take one, or list and abort them now and then with
`aws s3api list-multipart-uploads` and `aws s3api abort-multipart-upload`.

The server touches nothing else in the bucket or the directory, so they may hold other things
under other keys, though a bucket of its own keeps backups simple.
