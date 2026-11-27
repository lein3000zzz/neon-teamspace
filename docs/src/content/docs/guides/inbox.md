---
title: The inbox and notification mail
description: What comes to a person's inbox, reading, marking and archiving it in the app and the API, its live feed, and the mail of it, each notification or a digest, with the link that stops it.
---

Each person has one inbox, across every workspace they are in. It holds a notification for:

| Kind | When |
|---|---|
| `mention` | A comment, or a page's text, mentions them; see [whom a mention tells](/docs/guides/comments-and-mentions/#whom-a-mention-tells). |
| `reply` | Someone comments in a thread they wrote in or were mentioned in. |
| `comment` | Someone comments on a page they follow. |
| `assigned` | Someone else adds them to a person property of a row; once per property, and not again while that one is unread. |
| `invitation` | They are invited into a workspace; it shows while the invitation waits for an answer. |

A notification is made in the transaction of what it tells of, so a change that fails tells
nobody, and each comment tells each person at most once, by the first rule that reaches them,
as [Comments and mentions](/docs/guides/comments-and-mentions/#who-hears-of-a-comment) has
it. Nobody is told of their own change.

Each notification is checked again, whenever the inbox lists or counts it, against what its
person may see now. One about a page they lost, or a row in a database they left, is left
out, not deleted, and shows again should they get it back. The count is of those still to
read, at most 999.

## In the web app

**Inbox** in the sidebar opens it, with the number of notifications unread beside it. It
lists them newest first:

- **Unread**, **All** or **Archived** choose which;
- each says who did what on which page, in which workspace, with the start of the comment
  it is about, or for a mention in a page's text, the text of the block it is in as it was
  then. Its link opens the page there, at the thread or the block, and marks it read;
- **Mark read** or **Mark unread**, and **Archive** or **Back to the inbox**, on each;
- **Mark all read** marks every one read.

The invitations into workspaces that wait for an answer show above them, to accept or
decline there. The sidebar's count and the list follow what comes as it comes: the app holds
the inbox's live feed open while it shows.

## The API

| Route | What it does |
|---|---|
| `GET /api/v1/inbox` | Notifications, newest first: `filter`, `all` unless set, `unread` or `archived`; `workspace_id`, one workspace's alone; `limit`, 1 to 200, 50 unless set; and `before`, the `next` of the part before. |
| `GET /api/v1/inbox/count` | `{"unread"}`, of one workspace with `workspace_id`, or all. |
| `PATCH /api/v1/notifications/{id}` | Marks it read or unread, archived or not: `read`, `archived`. |
| `POST /api/v1/inbox/read` | Marks every unread one read, of one workspace with `workspace_id`, or all; answers how many. |
| `GET /api/v1/me/notifications`, `PUT` | How you want [mail of it](#notification-mail): `mail`, and whether the instance sends it now. The account's, for every workspace: a token bound to one answers `403 token_bound`. |

Each notification has its `kind`, its `workspace`, the `actor` who did it, the `page` as you
see it now, the `thread_id` and `comment_id` of a comment's, with an `excerpt` of the
comment, the `block_id` of a mention in a page's text, with an `excerpt` of the block's text
when the mention was told, or an invitation's `invitation`. A
token bound to a workspace, an agent's say, reads that workspace's inbox alone and no
invitation. Over MCP, `get_inbox`, `mark_read` and `neon://me/inbox` do the same; see
[Agents over MCP](/docs/guides/agents-over-mcp/#comments-and-the-inbox).

`/ws/inbox` is the inbox's live feed, which tells the signed-in browser that its inbox
changed, so it reads it again; [Real-time sync](/docs/architecture/realtime/#the-inbox-feed)
describes it.

## Notification mail

The inbox keeps everything; mail is for those who want to hear of it away from the app. Each
person chooses, under **Account** › **Notifications**, or with `PUT /api/v1/me/notifications`:

| Choice | `mail` | What comes |
|---|---|---|
| **Nothing** | `none` | No notification mail. |
| **Each notification** | `each` | A mail for each notification still unread a while after it came: one read meanwhile, in the app or over MCP, is not mailed. Those that come close together share a mail, which lists them, and mails go a quarter of an hour apart at the least. Unless they chose otherwise, this is everyone's. |
| **A digest** | `digest` | One mail of what came since the last and is still unread, at most once every few hours, listing at most 20 and how many more. |

Agents and integrations get no mail: their inbox is read over MCP and the API, and their
webhooks tell them as things happen.

An instance admin turns notification mail on with
[`notifications.mail`](/docs/reference/settings/), off until then, which needs
[mail](/docs/self-hosting/mail/) set up. `notifications.mail_delay`, five minutes unless set,
is how long a notification waits unread before its mail goes, and
`notifications.digest_interval`, three hours unless set, how often digests go at most. While
it is off, each person's choice is kept for when it is on.

For **Each notification**, one mail waits at a time for each person. A notification that comes
while one waits goes in it, when it too has waited its delay by then, or in the next. The next
goes fifteen minutes after the last at the soonest, so a burst of comments, forty threads
started at once say, makes one mail that lists the first twenty and counts the rest, rather
than forty mails. One that lists several reads like a digest.

Each mail is written when it goes, with only what its person may see then: a notification
about a page they lost goes unmailed, and so does one read, archived or mailed since. It says
who did what, on which page and in which workspace, with the start of the comment or the
block's text, and links to the page at the thread or block.

### Stopping it

Every notification mail ends with a link that stops them, which needs no sign-in, and
carries the headers mail clients offer their own unsubscribe button from:

```txt
List-Unsubscribe: <https://neon.example.com/api/v1/unsubscribe?token=…>
List-Unsubscribe-Post: List-Unsubscribe=One-Click
```

The link opens a page of the app, `/unsubscribe`, which stops the mail once its button is
pressed, so a mail scanner that opens links stops nothing; a mail client's button posts to the
API at once, as RFC 8058 has it. Either sets the person's choice to **Nothing**, which they
change back in their account.

The token names the person and is signed with the instance's [secret
key](/docs/self-hosting/configuration/#secret-keys), an HMAC-SHA256 with a key derived from it, so the
server keeps nothing for it. A link works for as long as the key that signed it is in the
keyring: rotating keys keeps old links working until the old key is removed. A token that
does not verify answers `400 invalid_token`, and the page says the link does not work.
