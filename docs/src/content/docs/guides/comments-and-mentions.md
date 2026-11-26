---
title: Comments and mentions
description: Threads of comments on a page or on a passage of it, replies, resolving, following a page, mentions of people, agents and pages in a page's text and in comments, and whom each tells.
---

A page holds threads of comments beside its content: on the page as a whole, or on a
passage of its text that the thread stays with as the text around it changes. A comment, or
a page's text, can mention a person, an agent or another page. A mention, a reply and a
comment on a page someone follows each land in their [inbox](/docs/guides/inbox/), and by
mail when they want it; agents hear of theirs over [MCP](/docs/guides/agents-over-mcp/#comments-and-the-inbox)
and through [webhooks](/docs/guides/webhooks/#events).

Comments are rows of the database beside the page's document, never in it, so editing or
importing a page leaves its comments alone, and exporting it leaves them out.

## Who may comment

Whoever may see a page reads its comments. Whoever may **comment** on it starts threads,
replies, and resolves and reopens threads:

- those who edit the page;
- those it is shared with **can comment**, a share between viewing and editing, guests
  among them: they comment but change nothing of the page itself.

Those who only view a page read its comments and add none. A page in the trash takes no
comment. A comment is changed and deleted by its author alone, while they may still comment
on its page. An agent or an integration comments as far as both it and its owner may.
[Permissions](/docs/reference/permissions/#pages) has the table.

## Threads

A thread is a first comment and the replies after it, in order. It is either:

- **on the page**, as a whole;
- **inline**, on a passage of its text. It keeps the passage as two
  [Yjs relative positions](https://docs.yjs.dev/api/relative-positions), its start and its
  end, and the text it was made on. Text typed before the passage, after it or inside it
  moves nothing: the thread stays on the same characters, wherever they are now, in every
  editor. Once every character of the passage is deleted, the thread is **detached**: it
  stays, with the text it was made on, but has no place in the page.

The API and MCP also start a thread on the whole text of one block, by the block's id, as
[Agents over MCP](/docs/guides/agents-over-mcp/#block-ids) names blocks; that is an inline
thread over that block's text.

A thread is **resolved** once its question is settled, by anyone who may comment, and
reopened the same way; resolved threads stay, under their own heading. Deleting a thread's
last comment deletes the thread.

| Limit | |
|---|---|
| A comment | 10000 characters of markdown, without control characters but line breaks and tabs; one with another, such as NUL, answers `400 invalid_input`. |
| The text an inline thread keeps | Its first 1000 characters. |
| Threads on a page | 1000, resolved ones included; one more answers `409 too_many_threads`. |
| Comments in a thread | 500; one more answers `409 too_many_replies`. |

### Markdown

A comment is markdown limited to inline content: text in bold, italics, strikethrough and
code, links, and mentions. Block syntax, a heading or a list, stays as the text it is. The
API keeps a comment's markdown and answers with its `content` too, read as BlockNote's inline
content, as the page's text holds it.

## Mentions

A mention names a person, an agent or an integration, or a page. In markdown, in comments
and in a page's text alike, it is a link with an `@` before it and a target of its own:

```md
Can you check the dates, @[Ada Lovelace](user:0193a5b2-1c4d-7e8f-a1b2-c3d4e5f60718)?
The plan is in @[Roadmap](page:0193a5b2-0000-7000-8000-0000000000a1).
```

The name in the brackets is the label the mention was written with. The app shows a person
by the name they have now, an agent with the person it acts for, and a page as a link to it
by its title. `\@` before such a link keeps it a plain link after an `@`. In a page's
document, a mention is inline content of the type `mention`; [Page
documents](/docs/architecture/documents/#mentions) has its structure.

### Whom a mention tells

A mention tells the one it names, in their inbox, once, and never its own writer. It tells
only someone who may see the page: one who may not is told nothing, then or later, not even
once the page is shared with them. Whoever writes it hears so, with the way to share the page
first.

- **In a comment**, it tells them as the comment is saved. A comment changed later tells only
  those it mentions anew.
- **In a page's text**, it tells them once the text has settled: ten seconds after the last
  change to the page, the server saves its document and tells everyone it now mentions who
  it did not before. A mention that stays tells nobody again however the page changes; one
  removed and written again later tells again: its webhook event each time, and the inbox
  once more unless its last notification there is still unread.
- **In a new page's text**, as the API or MCP makes a page with content, it tells them as the
  page is made. Text an [import](/docs/guides/import-export/) brings tells nobody, then or
  later: it was written elsewhere, before.

A mention in a page's text is from whoever wrote it, whoever changed the page after them:
the server knows whose change added each part of the document since it was last saved, and
the mention's notification and event name that writer. Someone mentioned is told even when
they changed the page themselves before it was saved, unless every mention of them is their
own. One the server cannot tell the writer of, as of text saved before it knew, comes from
nobody named.

Someone mentioned on a page, who may see it, follows its comments from then on, unless they
chose not to.

## Who hears of a comment

A new comment tells, as it is saved and in the same transaction, each of these once, by the
first rule that reaches them, and never its writer:

1. those it **mentions**: a mention;
2. those already **in its thread**, who wrote in it or were mentioned in it: a reply;
3. those who **follow** the page: a comment.

Each is told only while they may see the page then. A comment's writer and those it
mentions follow the page from then on, unless they chose not to. **Follow** in the comments
panel, or `PUT /api/v1/pages/{id}/follow`, starts or stops it by hand, and that choice holds.

The inbox checks again, each time it lists or counts a notification, whether its person may
still see what it is about: one about a page they lost is left out, and shows again should
they get the page back. Deleting a comment deletes the notifications it made.

## In the web app

**Comments** in a page's header opens its comments in a panel beside the page, which leaves
the page usable; the button counts the open threads. Escape or the close button closes the
panel. In it:

- the field at the top comments on the page as a whole;
- **Comment on the selected text** starts a thread on the text selected in the page, as do
  the **Comment** button of the formatting toolbar and **Ctrl+Alt+M**, **⌘⌥M** on a Mac;
- **Open** and **Resolved** show the threads of each kind;
- each thread has its passage, with **Show in page**, which selects it and scrolls to it, or
  says the passage is gone; then its comments, with **Edit** and **Delete** on your own, an
  edited one marked so; then **Reply** and **Resolve**, or **Reopen**;
- **Follow** says whether you follow the page's comments, and changes it.

In the page, the text of each open inline thread is underlined and tinted, the one open in
the panel more strongly. A click on it opens its thread; so does Ctrl+Alt+M with the cursor
in it, which also moves focus to the thread. The panel and the highlights follow what others
do as they do it: the page's live connection tells of each thread that changes.

Typing `@` in a comment, or in the page, offers the people, agents and pages whose names
hold what follows it, the arrow keys moving through them and Enter or a click picking one.
Someone who cannot see the page is marked so, and picking them says that the mention will
tell them nothing, with **Share**, to those who may share the page, to share it with them
first.

A link from the inbox opens the page at what it is about: the panel at its thread, its
passage selected, or the cursor at the block a mention is in.

## The API

| Route | What it does |
|---|---|
| `GET /api/v1/pages/{id}/comments` | The page's threads, oldest first, with `may_comment` and `following` for the caller. |
| `POST /api/v1/pages/{id}/comments` | Starts a thread: `markdown`, and `anchor`, `{"start", "end", "quote"}`, for an inline one, or `block_id` for one on a block's text. |
| `POST /api/v1/threads/{id}/comments` | Replies: `markdown`. |
| `PATCH /api/v1/threads/{id}` | Resolves or reopens it: `resolved`. |
| `PATCH /api/v1/comments/{id}`, `DELETE` | Changes one's own comment's `markdown`, or deletes it. |
| `PUT /api/v1/pages/{id}/follow` | Follows the page's comments, or stops: `following`. |
| `GET /api/v1/pages/{id}/mentions?q=` | The people, agents and pages a mention on the page may name, whose names hold `q`, at most 10 of each, each person with `sees_page`. A guest gets pages alone. |

An inline thread's `anchor` answers with its `block_id`, the block its passage starts in now,
and `detached`. `start` and `end` are the relative positions in Yjs's own encoding, in
base64, as `Y.encodeRelativePosition` writes them; a client with the page's document
resolves them itself. A comment made or changed answers with its thread, its id, and
`unnotified`: those it mentions who may not see the page, and were told nothing.

The [API reference](/docs/reference/api/) has the rest: the shapes, and the codes, such as
`403 forbidden` for one who may only view the page.
