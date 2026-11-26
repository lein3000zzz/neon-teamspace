---
title: Attachments
description: Images, video, audio and other files in pages and as covers, what each block takes, the limits on uploads and storage, how files are served and kept safe, and when they are deleted.
---

A page holds files in its image, video, audio and file blocks, and may have an image of its own
as its cover. Each one is uploaded to the page, and the server keeps it in the
[store the instance uses](/docs/self-hosting/storage/), a directory on its disk or a bucket of
an S3-compatible store. A file belongs to the page it was uploaded to. Whoever sees the page
sees its files.

## In the editor

| How | What it does |
|---|---|
| Paste an image, such as a screenshot | Adds an image block where the cursor is, and uploads the image into it. |
| Drop files onto the page | Adds a block for each where it lands: an image, video or audio block for those, a file block for anything else. |
| **Upload** in an image, video, audio or file block's panel | Uploads the file you pick into that block. |
| **Embed** in the same panel | Shows a file from another site by its URL, without uploading it. |

While a file uploads, its block shows a spinner. Everyone with the page open sees the block at
once, and the file in it when the upload ends. When the server refuses a file, its block stays
empty and the editor says why, naming the type, the size or the full workspace.

Images, video and audio show in their blocks and play in place. A file block shows the file's
name, and clicking it downloads the file.

### Covers

**Add cover**, or **Change cover** on a page with one, offers the theme's presets and
**Upload an image**. The editor uploads the image to the page like any other file, and the page
shows it across its top in place of a preset. Picking a preset later replaces it, and
**Remove cover** takes either kind off. A cover image counts against the workspace's storage
like the page's other files.

## What each block takes

The server tells what a file is from its bytes alone, never from its name or the type the
browser says it is. Each kind of block takes the types it can show:

| Block | Types |
|---|---|
| Image, and covers | PNG, JPEG, GIF, WebP, AVIF, SVG |
| Video | MP4, WebM, Ogg, QuickTime |
| Audio | MP3, WAV, Ogg, FLAC, AAC, M4A, and WebM and MP4 recordings |
| File | Any file |

The server refuses a file of another type with `415 unsupported_type`, which names the type it
found. It reads an image's width and height from its header as it uploads.

## Limits

| Limit | Value | Set by |
|---|---|---|
| One file | 50 MiB | [`attachments.max_file_bytes`](/docs/reference/settings/), from 1 KiB to 5 GiB |
| A workspace's files together | No limit | [`workspaces.limits.storage_bytes`](/docs/reference/settings/), and an instance admin's limit for the workspace |
| Uploads by one person or agent | 60 at once, then one more a second | |
| How long one upload may take | 15 minutes | |

An upload streams to the store as it comes, so the server never holds a whole file in memory.
It stops as soon as the bytes pass the largest file, whether or not the client declared their
size. A client that declares its upload's size, as browsers do, gets the refusal of one too
large before the server reads any of it. The form's file must start within its first 64 KiB,
after its boundary and part headers, so a form that never gets to its file stops there.

A workspace at its storage limit refuses uploads, and imports that bring files. Nothing in it
is removed, and its pages stay editable. An upload that would take the workspace past its
limit fails once its size is known, and the server keeps nothing of it. The workspace's
settings show the storage it uses against its limit, and the instance admins' page shows it
beside the other limits.

## How files are served

Each file is read at `/api/v1/files/{id}`, the path its block keeps. The server checks every
request against the file's page, as it would a read of the page. Members of its teamspace,
people and guests it is shared with, and agents whose owners may see it too, read the file.
Anyone else gets `404 not_found`, the same answer as for a file that does not exist. A file of
a page in the trash shows to those who could edit the page, as the page does.

The answer keeps the file in a sandbox of its own:

- `Content-Type` is the type its bytes said, and `X-Content-Type-Options: nosniff` keeps
  browsers to it.
- `Content-Disposition: inline` for images, video and audio that run nothing; `attachment`,
  a download, for everything else, PDFs, HTML and SVG among them.
- `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`. A file a
  browser opens as a page anyway runs no script, and it is no page of the app's origin.
- `Cross-Origin-Resource-Policy: same-origin`, so other sites cannot embed it.
- `Cache-Control: private, no-cache` with an `ETag`, the file's SHA-256. Only the browser that
  read the file may keep it, and it asks again each time, so the rules answer every read. The
  answer is `304`, without the bytes, while the file is the one the browser kept.
- `Accept-Ranges: bytes`. Video and audio players ask for a part at a time as they seek, and
  get `206` with that part.

SVG counts as an image. Its block shows it with `<img>`, where browsers run none of its script.
Opened at its own URL, it downloads.

## In markdown and over MCP

Markdown writes an image block as `![name](url)` and the others as links in the
[extension's](/docs/architecture/documents/#the-extension) `:::file`, `:::video` and
`:::audio`. A file kept on the server is linked by its path, `/api/v1/files/{id}`:

```markdown
![Team photo](/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1)

:::file [Quarterly report.pdf](/api/v1/files/0193a5b2-0000-7000-8000-0000000000f3)
```

An agent's [`read_page`](/docs/guides/agents-over-mcp/#pages) shows files the same way, and
the agent reads one with `GET` on the server it connected to, with the same token. MCP could
carry a file's bytes only as text inside a tool's arguments, held whole in memory, so there
is no tool to upload one. Agents upload through the API instead, with the same token, and put
the answer's `url` in the markdown they write.

## The API

`POST /api/v1/pages/{id}/files?kind=image|video|audio|file` takes a `multipart/form-data` form
with one part, `file`, whose name the file keeps. It needs edit access to the page, and the page
out of the trash. It answers `201` with the file:

```sh
curl -X POST \
  -H "Authorization: Bearer $NEON_TOKEN" \
  -F "file=@diagram.png" \
  "https://neon.example.com/api/v1/pages/$PAGE/files?kind=image"
```

```json
{
  "id": "0193a5b2-0000-7000-8000-0000000000f1",
  "page_id": "0193a5b2-0000-7000-8000-0000000000a3",
  "url": "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1",
  "kind": "image",
  "name": "diagram.png",
  "content_type": "image/png",
  "size": 48213,
  "sha256": "…",
  "width": 1280,
  "height": 720,
  "created_at": "2026-11-26T12:00:00Z"
}
```

`PATCH /api/v1/pages/{id}` with `{"cover_image": "<file id>"}` makes an image of the page its
cover, and `null` takes it off. `GET /api/v1/files/{id}` reads a file, with `Range`,
`If-None-Match` and `If-Range` as above.

| Status | Code | When |
|---|---|---|
| `400` | `invalid_input` | The form's first part is not `file`, or a cover image that is not an image of the page. |
| `400` | `invalid_body` | A form that cannot be read, or stopped short. |
| `403` | `forbidden` | The caller views the page and does not edit it, or it is in the trash. |
| `404` | `not_found` | No such page or file, or none the caller sees. |
| `409` | `storage_limit` | The workspace has no room for the file. |
| `413` | `file_too_large` | Past `attachments.max_file_bytes`. |
| `413` | `payload_too_large` | A form whose file does not start within its first 64 KiB. |
| `415` | `unsupported_type` | A type the kind does not take. |
| `416` | `range_not_satisfiable` | A `Range` that starts past the file's end. |
| `429` | `rate_limited` | Too many uploads; wait for `Retry-After`. |
| `503` | `storage_unavailable` | The file store did not answer. |

## When files are deleted

- **With their page.** A page's files go to the trash with it, readable by those who could
  edit it, and come back when it is restored. When the page leaves the trash for good, by hand
  or once [`pages.trash_retention`](/docs/reference/settings/) is over, its files go with it.
- **With their workspace.** Deleting a workspace deletes all its files.
- **Once nothing shows them.** The hourly cleanup keeps every file that any page's document or
  cover shows. It marks a file nothing shows, and deletes it once it has stayed that way for
  [`attachments.orphan_grace`](/docs/reference/settings/), 30 days unless set. A file shown
  again within that time, after an undo, a paste of its block or a restored older version of
  a page, stays and starts over. An older version than that may show a file deleted since, and
  its block then stays empty.

A file belongs to the page it was uploaded to. A block copied into another page shows the file
to those who may see the page it came from, and the cleanup keeps the file while the copy shows
it. The file still goes when its own page goes.

The cleanup deletes a file's bytes from the store after its row, however the row went. An
upload cut short by a server that stopped mid-way leaves bytes behind, and the cleanup deletes
those too, half an hour after the upload began.

## Import and export

An exported zip holds the files its pages show, those the exporter may read, in each page's
folder, and the pages link to them by relative paths. Importing the zip brings them back as
files of the new pages. [Import and export](/docs/guides/import-export/#files) has the details.
