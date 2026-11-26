---
title: Search
description: Finding pages, databases and rows by their titles, text, values and comments, the query syntax, the filters, what search reads and when, who finds what, and the API and MCP tool.
---

Search finds the pages, databases and rows of a workspace by their words, and by the words
of their comments. It looks only at what the person searching may see, so two people typing
the same query can get different answers, and nobody learns from a search that a page they
were not given exists, nor what is said about it.

## In the web app

**⌘K** on a Mac, **Ctrl+K** elsewhere, or **Search** in the sidebar opens the quick search
of the workspace the sidebar shows. The shortcut works in the editor too. Before anything is
typed it lists the pages changed last. As you type, the best eight results replace them,
each with its title and a few words of its text, the matches marked.

| Key | What it does |
|---|---|
| ↓ and ↑ | Move through the results, and from the first back up to the field. |
| Enter | Opens the result you are on. In the field, it opens the first. |
| Esc | Closes the quick search. |

The last line, **Search everything**, opens the search page with what you typed. That page
also takes filters, and reads more results with **Show more**. Its address holds the query
and every filter, so a search can be bookmarked or sent to someone, and the back button
undoes the last filter.

## The query

| You type | It finds |
|---|---|
| `roadmap q3` | Pages that hold both words, anywhere, in any order. |
| `road` | Words that start with it: `road`, `roadmap`, `roads`. Every word of two characters or more works this way. |
| `a` | The word `a` alone. One character never finds the words it starts. |
| `"launch plan"` | Those words next to each other, in that order, each whole. |
| `plan -draft` | Pages with `plan` and without `draft`. `-"old idea"` leaves out a phrase. |
| `plan OR roadmap` | Either word. `OR` must be in capitals; `or` is a word to find. |

Case does not matter, in any alphabet. Search uses Postgres's `simple` text configuration,
which keeps words as they are, without stemming, so it treats every language alike:
`running` does not find `run`, but `run` finds `running` as its start. Punctuation splits
words where Postgres's parser splits them. `e-mail` finds `e-mail`, and `3.14` finds `3.14`.
A term without a letter or a digit, such as `&` or `!`, is dropped, so nothing typed acts as
an operator of the query language. Scripts written without spaces, such as Japanese, are
read as runs of characters: `東京` finds `東京タワー` because it starts it, but a word in the
middle of such a run is not found.

Titles also match loosely, by their trigrams, so `quaterly` finds **Quarterly roadmap** and
`roadmp` finds it too. Only titles do, and only for queries with three letters or digits or
more. A word left out with `-` leaves out loose matches too.

A query has at most 200 characters and 16 terms. One with only words to leave out, such as
`-draft`, is refused, as it would match nearly everything, and so is one holding a control
character other than spaces, tabs and line breaks, which no page holds. An empty query
lists what the filters let through, the most recently changed first.

## The results

Results come best first. A match in a title counts most, then one in a row's text or URL
values, then one in a page's text, then one in its comments, and a title that is the query
itself, however short, such as `Go`, ranks above all.
More of the query's words, and more matches, rank higher. Between results that rank the
same, the one changed last comes first.

Each result has its title and a snippet of up to two passages of about thirty words around
the matches, both in runs that mark what matched. A snippet is one line, with ` · ` between
the lines it spans, such as a list's items. Both keep what reads as markup, such as the `<id>`
of `plants/<id>/moisture`, as it is written. The snippet comes from the 5,000
characters around the first place one of the query's words is, in a row's values, then the
page's text, then its comments, so on a long page it shows the first matches, not the best
ones further on. A loose title match has nothing marked.
Search reaches the best 1000 results of a query.

## Filters

| Filter | What it keeps |
|---|---|
| Teamspace | The pages of one teamspace. |
| Kind | Pages, databases, or rows. |
| Created by | What one member made. |
| Last edited by | What one member changed last. Earlier editors do not count. |
| Edited | What changed in the last day, week, month or year, its title, place, style or content. The API takes any duration, such as `12h`. |

Guests know no other member, so they cannot filter by who made or changed a page: the API
answers `403 forbidden`, and the web app offers no such filter to them.

## What search reads, and when

| What | When it changes |
|---|---|
| A page's, database's or row's title | As the title is saved. |
| A row's text and URL values | As the row's values are saved. A property that stops being text or a URL stops counting at once, and so do the values of one deleted. |
| The text of a page's or a row's content | Each time the server saves the document as a snapshot. |
| The words of a page's or a row's [comments](/docs/guides/comments-and-mentions/#search) | As each comment is saved, changed or deleted, with its thread or its own. |

Other values of a row, such as numbers, selects and dates, are not read. Nor are the files a
page links to. Of a page's content, search reads
what [plain text](/docs/architecture/documents/#the-model) gives: each block's words,
tables' cells, the names and captions of images, video, audio and files, but not what is
inside a file nor its path, and each mention as `@` and the label it was written with, so a page that mentions `@Dee Diaz` is found by `diaz`. Of a
comment, it reads the same: its text without styles or link targets, and its mentions by
their labels. It reads at most the first 100,000 characters of a page's text, of a row's
values and of its comments' words, the newest comments first. A URL in a row's value is read
whole and as its words, so `zebra` finds `https://example.com/zebra`.

Text typed in the editor is found once the server saves the document: when the last person
editing it leaves and half a minute passes, after enough changes, or when the server stops.
[Real-time sync](/docs/architecture/realtime/#saving) has the details. Titles and values are
found as soon as they are saved. Nothing waits in a queue: everything search reads is
written in the same transaction as what it comes from, as
[Data and storage](/docs/architecture/data/#search) explains.

## Who finds what

Search finds exactly the pages the person searching may [see](/docs/guides/pages/#where-a-page-lives),
out of the trash. A comment is found only through its page: whoever may see a page reads its
comments, so a search shows nobody a comment they could not read.

- Members find the pages of the teamspaces they see, their own private pages, and what is
  shared with them, with the pages under it.
- Guests find only what is shared with them.
- A private teamspace's pages are for its members, the workspace's owners and admins
  included.
- An agent or integration finds what it and its owner both see.

A result's page comes as `GET /api/v1/pages/{id}` would give it: without the page above it
when that is hidden from the caller, and, for guests, without who made or changed it.

## The API

`GET /api/v1/workspaces/{id}/search` searches with a session or a token with `read`.

| Parameter | |
|---|---|
| `query` | The query; absent or empty, the filters alone. |
| `teamspace_id`, `kind` | `kind` is `page`, `database` or `row`. |
| `created_by`, `edited_by` | A member's id. |
| `edited_within` | A duration, such as `7d` or `12h`, at most ten years. |
| `limit` | Results in a part, 1 to 50, 20 when absent. |
| `after` | The `next` of the part before. |

```json
{
  "results": [
    {
      "page": { "id": "0193a5b2-0000-7000-8000-0000000000a1", "title": "Quarterly roadmap", "kind": "page" },
      "title": [{ "text": "Quarterly ", "highlight": false }, { "text": "roadmap", "highlight": true }],
      "snippet": [{ "text": "the ", "highlight": false }, { "text": "roadmap", "highlight": true }],
      "rank": 1.62
    }
  ],
  "next": "20"
}
```

`page` is the whole page object, shortened here. `rank` orders the results of one search
and means nothing across searches. A query the server cannot run answers
`400 invalid_search`, a parameter that does not read `400 invalid_query`, and a workspace
the caller is not in `404 not_found`.

Searching holds a database connection while it runs, so the server bounds it. Each user
gets 60 searches, then 2 a second, after which a search answers `429 rate_limited` with
`Retry-After`. Half the server's database connections at most run searches at once; one that
waits 2 seconds for its turn answers `503 busy`, to try again a second later. A search that
runs longer than 5 seconds is given up and answers `503 timeout`: more words or a filter
narrow it.

[Agents over MCP](/docs/guides/agents-over-mcp/#orientation) search with the `search` tool,
which takes the same parameters and answers the same.

## Rebuilding the text

Each page's text is stored with its snapshot. A server killed before it saved one, as in a
crash, leaves the page's latest changes logged but not in its text, and a page that never
had a snapshot without any text, until the page is opened and saved again.
`neon search reindex` reads the document of every such page, its snapshot and the updates
logged after it, and stores its text when it differs; it says how many pages' text it
changed. `neon search reindex --all` reads every page, for instance after an upgrade that
reads documents' text differently. Both are safe while servers run: each page is read and
written while its row is locked, so a snapshot saved at the same moment writes its own text
after. They apply pending migrations first, as `neon serve` does.

The migrations that brought search indexed every title, every row's values, the text each
page had stored already and every comment, so a new install and an upgraded one need no
rebuild. Comments' words are kept by the database itself as comments change, and never need
rebuilding.
