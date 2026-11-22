---
title: Import and export
description: Importing markdown files and zips of them as pages, exporting a page, its subpages or a teamspace as markdown, the limits on both, and the API.
---

Pages come in as markdown and go out as markdown. An import takes one `.md` file, or a zip
of them whose folders become pages with subpages. An export gives one page as a `.md` file,
or a page with the pages under it, or a whole teamspace, as a zip laid out the same way. A
zip that one export writes imports back as the same pages with the same markdown.

:::note[What is built]
Both work through the API. The web app's buttons for them are **planned**.
:::

The markdown is GitHub's, with a small extension for toggles and media blocks, as
[Page documents](/docs/architecture/documents/#markdown) lists. Some things a page holds have
no markdown, such as colours, and an export leaves them out; that page lists them too.

## Importing

`POST /api/v1/workspaces/{id}/pages/import` takes a `multipart/form-data` form with one part,
`file`. Its name says what it is: `.md` or `.markdown` for one page, `.zip` for a zip of them.
The query string names where the pages go, exactly one of:

| Query | Where the pages go |
|---|---|
| `parent_id=<page>` | Under that page of the workspace, which the caller edits. |
| `teamspace_id=<teamspace>` | At the top of that teamspace, which the caller is in. |
| `private=true` | At the top of the caller's private pages. Guests have none. |

The rules are the same as for making a page there, and the server checks them before it reads
the file. The new pages go after the pages already there, and the caller edits them.

```sh
curl -X POST \
  -H "Authorization: Bearer $NEON_TOKEN" \
  -F "file=@notes.zip" \
  "https://neon.example.com/api/v1/workspaces/$WORKSPACE/pages/import?teamspace_id=$TEAMSPACE"
```

The answer, `201`, lists the pages made at the place named, in order, and `count`, the number
made with the pages under them. `skipped` lists the files of a zip left out, each with its
`path` and `reason`.

### How a zip becomes pages

- Each markdown file is a page.
- Each folder that holds a markdown file, at any depth, is a page too, with the pages of its
  files and folders under it.
- A folder and a markdown file of the same name beside it are one page: `Roadmap/` and
  `Roadmap.md` make a page with the content of `Roadmap.md` and the pages of `Roadmap/` under
  it. A folder without such a file is a page without content.
- Siblings keep the order of the zip.
- A zip that wraps everything in one folder makes one page of it, with the rest under it.

A page's title is the file's first heading, when the file starts with one, and that heading
leaves the content. Otherwise the title is the file's name without its extension, or the
folder's name. A block of YAML front matter at the very top, `---` and `key: value` lines up to
the next `---`, is left out, as note apps and site generators write it.

Each page's content becomes the first snapshot of its document, with fresh block ids, and its
text the page's search text. Nothing is made unless all of it can be: an import that breaks a
rule or a limit makes no page at all.

### What a zip leaves out

| `reason` | The file |
|---|---|
| `attachment` | Not a markdown file, such as an image or a PDF. Attachments are **planned**. |
| `hidden` | Its name, or a folder's it is in, starts with a dot, such as `.obsidian/`. |
| `link` | A symbolic link. |
| `encrypted` | Encrypted with a password. |
| `unsupported_compression` | Compressed with a method other than deflate, such as bzip2 or zstd. Files stored without compression import. |
| `not_text` | Not UTF-8 text. |
| `duplicate` | A second markdown file for one page, such as `Notes.markdown` beside `Notes.md`. |

The files archivers add for their own use, `__MACOSX/`, `.DS_Store`, `._*`, `Thumbs.db` and
`desktop.ini`, are left out without being listed.

## Exporting

| Route | What it answers |
|---|---|
| `GET /api/v1/pages/{id}/export` | The page as a markdown file, named after its title. |
| `GET /api/v1/pages/{id}/export?children=true` | A zip of the page and every page under it. |
| `GET /api/v1/teamspaces/{id}/export` | A zip of the teamspace's pages, named after it. |

Each page's file starts with its title as a level 1 heading, then its content. In a zip, a
page's file sits beside a folder of the same name that holds the files of the pages under it:

```txt
Roadmap.md
Roadmap/
  Q1 Goals.md
  Q1 Goals/
    Hiring.md
Welcome.md
```

Anyone who sees a page may export it. Whoever sees a page sees every page under it, so the
zip holds all of them. A teamspace exports for everyone who may list its pages, which leaves
out guests and those outside a private teamspace. Pages in the trash stay out. A page in the
trash exports for those who could edit it, with the pages that went to the trash with it.

The content is what the page holds at that moment. For a page open in someone's editor it
comes from the live document, edits not yet saved included.

File names are the titles, with the characters Windows, macOS or Linux refuse as spaces,
without dots or spaces at the ends, and cut to 100 bytes. A page without a title is
`Untitled`. Siblings whose names would be the same, ignoring case, get ` (2)`, ` (3)` and on.
The headings keep the titles as they are.

The answers carry `Content-Disposition: attachment` with the file's name, and
`Cache-Control: no-store`.

## Round trips

Exporting a zip and importing it gives the same pages in the same order, with the same
titles, and exporting those gives the same files. Icons, covers, styles and shares do not go
into markdown, and what markdown loses of the content is listed on
[Page documents](/docs/architecture/documents/#what-markdown-loses). Two more things change:

- A page without a title comes back titled `Untitled`, after its file.
- A page without a title whose content starts with a heading comes back with that heading as
  its title.

## Limits

| Limit | Value |
|---|---|
| The file uploaded | 32 MiB |
| A markdown file, alone or unpacked from a zip | 4 MiB |
| What a zip's markdown files unpack to, together | 64 MiB |
| Files and folders a zip lists, those left out included | 2000 |
| How many folders deep a file is | 32 |
| Pages one import makes | 1000 |
| Pages one export holds | 5000 |
| Markdown one export holds, before it is zipped | 128 MiB |
| How long an import or export takes, after the upload | 20 seconds |
| Imports and exports running at once on the server | 4 |

The server reads a zip in its memory and never writes its files to disk. It refuses a zip
whose entry would land outside the zip, with `..` or a path from the root of a disk, and it
counts a zip's entries before it parses its directory, so a zip that lists too many is refused
before it takes any memory. The server does not trust the size a zip states for a file. It
stops reading the file at its limit. A page also holds at most 16 MiB of document, as the
[live documents](/docs/architecture/realtime/#limits) do.

Each person may import 10 times, then once more every 30 seconds, and export the same, counted
apart. Past that the answer is `429 rate_limited` with `Retry-After`.

## Errors

Every error has the API's usual shape, `{"error": {"code", "message"}}`, and the message names
the file or limit at fault.

| Status | Code | When |
|---|---|---|
| `400` | `invalid_input` | No place or several, no `file` or several parts, a file not named `.md`, `.markdown` or `.zip`, a markdown file that is not UTF-8, a zip that cannot be read or holds no markdown file, or an entry whose path leads outside the zip. |
| `400` | `invalid_body` | A body that is not a form. |
| `403` | `forbidden` | The caller may not add pages there: they view the parent and do not edit it, are not in the teamspace, or are a guest. Or `insufficient_scope`, a token without `write`, to import. |
| `404` | `not_found` | No such workspace, teamspace or page, or none the caller sees. |
| `409` | `too_deep` | The pages would nest deeper than 100 levels. |
| `413` | `too_large` | Past one of the limits above. A body larger than the largest upload gets `payload_too_large`. |
| `429` | `rate_limited` | Too many imports or exports. |
| `503` | `busy` | Four imports and exports are running already. Try again after `Retry-After`. |
| `503` | `timeout` | It took longer than 20 seconds. Import or export less at once. |
