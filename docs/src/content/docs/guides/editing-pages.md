---
title: Editing pages
description: The page trees in the sidebar, making, moving and sharing pages, a page's icon, cover, title and style, the live editor, and the trash, in the web app.
---

This page is about the web app's pages for pages: the trees in the sidebar, a page's header,
its content in the live editor, sharing and the trash. [Pages](/docs/guides/pages/) has the
rules behind them and the API they call, and [Permissions](/docs/reference/permissions/#pages)
the table of who may do what.

The app shows each person only the controls that work for them: someone who views a page
gets no way to rename it, and a guest no way to share. The server decides in the end. When
it refuses something, because access changed since the page loaded, the app says so and
reads the page again.

## The sidebar

Under the workspace switcher, the sidebar lists a workspace's pages in sections:

| Section | What it holds |
|---|---|
| **Favorites** | The pages marked as favorites, in your order. Shown while there are any. |
| **Teamspaces** | Each teamspace you are in, with its pages under it as a tree. |
| **Private** | Your private pages, which nobody else sees. Not for guests, who have none. |
| **Shared with you** | Pages shared with you from places you do not otherwise reach, with the pages under them. Shown while there are any. For a guest, it is everything. |

**Trash**, at the top with **Home**, opens the [trash](#the-trash). On a phone the sidebar is
a drawer, opened from the button at the top left.

Each page is a row: the arrow before it opens and closes the pages under it, the title opens
the page, and the row's buttons add a page inside it and open its menu. A teamspace's row
folds its tree away the same way. On a computer the buttons show as the pointer passes over
a row or the keyboard reaches it; on a touch screen they always show. Opening a page opens
the pages above it in the tree, so it shows there.

## Making a page

| Where | Makes |
|---|---|
| **+** on a teamspace's row | A page at the top of that teamspace. |
| **+** on a page's row, or **Add a page inside** in its menu | A page inside that page. |
| **+** beside **Private** | One of your private pages. |
| **New private page** on the workspace's home | The same. |
| **New page** on a teamspace's page | A page at the top of the teamspace. |

A new page is Untitled and goes last among its siblings. It opens with the cursor in its
title, ready to type.

## A page's menu

| Item | What it does | Who sees it |
|---|---|---|
| **Rename** | Turns the row's title into a field: Enter or leaving it saves, Escape puts the old title back. | Those who edit the page. |
| **Add a page inside** | Makes a page inside it. | Those who edit it. |
| **Add to favorites**, **Remove from favorites** | Marks it, or takes the mark away. | Everyone who sees it. |
| **Share…** | Opens [sharing](#sharing). | Those who edit it, but not guests. |
| **Move to…** | Opens the list of places it can go. | Those who edit it. |
| **Move up**, **Move down** | Swaps it with the sibling before or after it. | Those who edit it, while there is such a sibling. |
| **Move to the trash** | Moves it, with the pages under it, to the [trash](#the-trash). | Those who edit it. |

The page's own header has the same menu, but for renaming in place and the moves up and
down, which only make sense in the tree.

## Moving pages

With a mouse, drag a page's row and drop it:

- on the top or bottom edge of another row, to place it before or after that page;
- on the middle of a row, to put it inside that page, last;
- on a teamspace's row or on the **Private** heading, to put it at the top of that space, last.

Where it would land lights up as it passes. A page never drops under itself or one of its
own subpages, and only lands where you may add pages.

**Move to…** does the same from the keyboard or on a touch screen. It lists the top of each
teamspace you are in, your private pages, and every page you edit, but the page itself and
the pages under it. Type to narrow the list, then pick a place. **Move up** and **Move down**
reorder a page among its siblings. A move takes the pages under the page along, and they take
their access from where they land.

## A page's header

Above the page, the way down to it: its teamspace, or **Private**, then each page above it
you can see. A guest's starts at the page shared with them. Beside it are the state of the
[live connection](#the-live-connection), the favorite star, **Share**, the **Style** menu
and the page's menu.

| Part | What it does |
|---|---|
| **Cover** | **Add cover** picks one of 14 presets drawn from the theme: the accent, eight hues and four gradients. Each follows the reader's palette and mode, so it suits both light and dark. **Change cover** picks another or removes it. |
| **Icon** | **Add icon** opens a picker of emoji, by category or searched by name, which come with the app, so nothing is fetched elsewhere. A short text works too, up to 16 characters. The icon also shows in the sidebar and the way down to the page. |
| **Title** | Typed in place. The sidebar shows it as it is typed, and it is saved once typing pauses, or as you leave it. Enter goes on to the page's content. A page without a title shows as Untitled. |
| **Style** | The page's font, Default, Serif or Mono, as in Notion; small text, a step down; and full width, which lets the page take the whole width beside the sidebar. It holds for everyone who opens the page. |

Someone who only views a page sees its header without the ways to change it, and a line
that says they can view it.

## The editor

A page's content is blocks, edited with [BlockNote](https://www.blocknotejs.org/):
paragraphs, headings, lists, to-dos, toggles, quotes, code, tables and more. Type `/` for the
menu of blocks, or the usual markdown shortcuts, such as `#` and a space for a heading or
`-` and a space for a list. Select text for its formatting toolbar. The handle beside a block
drags it or opens its menu.

Everyone with the page open edits the same document at once. Each change reaches the
others as it is made, and each person's cursor shows to the others in that person's own
colour, with their name while they type. The people with the page open show as dots beside
the connection's state.

### The live connection

| State | What it means |
|---|---|
| **Connecting** | The page's content is on its way. Nothing can be typed until it arrived, so the first keystrokes never land in an empty copy. |
| **Connected** | In sync: what you type is saved, and others see it at once. |
| **Read-only** | In sync, and you can view the page but not change it. Others' changes still show as they make them. |
| **Reconnecting** | The connection dropped, or the server asked the browser to slow down because it fell behind or sent too much, and it is tried again, after a pause in the second case. Changes made meanwhile are kept in the browser and sent once it is back. |
| **Disconnected** | The server ended the connection for a reason trying again would not cure. The editor stops taking input, a notice says why, and the app reads the page again. **Reconnect** opens the page afresh, from what the server saved. |

When your access changes while the page is open, the editor follows at once, without
reconnecting: someone whose share now only lets them view gets a read-only editor and
header, and one who may edit again gets them back.

**Disconnected** comes with one of these:

| Notice | Why | What was typed and not yet saved |
|---|---|---|
| Your session ended | You signed out elsewhere, or your session expired or was revoked. The app takes you to sign in. | Lost. |
| You can no longer see this page | It went to the trash, a share was removed, or you left its teamspace. The page then shows as not found. | Lost. |
| You may only view this page now | You typed while your access was changing to view. | Lost: the server refused it. |
| The last change is too large | One change was larger than the server takes at once, 4 MiB, or would make the page larger than it keeps, 16 MiB. | Lost, with that change. |
| An error | The server could not read what the browser sent. | Lost. |

A paste that would be larger than the server takes is refused before it reaches the page,
with a notice; paste it in parts, or [import it](/docs/guides/import-export/) as a markdown
file. The server's own limits are in [Real-time sync](/docs/architecture/realtime/#limits).

The editor connects to `/ws/pages/{id}` on the same origin as the app, over `wss:` when the
app is served over HTTPS. Browser tabs never sync with each other directly: every change
goes through the server, which checks what each person may do. The names and colours of
others' cursors come from the server, which says who each connection is, so nobody shows
under another person's name.

## Sharing

**Share**, in a page's header or its menu, opens the page's shares:

- **Share with someone** picks a person in the workspace and what the share gives, **Can
  view** or **Can edit**, and adds it.
- Each share on the page itself has its access, which changes on the spot, and **Remove**.
- Shares set on a page above hold here too. They are listed after the page's own, with the
  page they come from, and change on that page.
- Guests are marked, as a share is all they reach in the workspace.

A share holds for the page and every page under it, and only ever adds to what someone may
do. A guest finds what is shared with them under **Shared with you**.

## The trash

**Move to the trash** takes a page out of the trees with every page under it. The sidebar
says so, with **Undo** to bring it back at once. Opened while it is in the trash, a page
says so too, to those who edit it, with **Restore** and **Delete for good**.

The **Trash** page lists the pages in the trash you could edit, latest first. A page that
went with the pages under it stands for them, and restoring it brings them all back. **Delete
for good** asks first, then deletes the page and everything under it, for everyone. After 30
days in the trash, or the time `pages.trash_retention` sets, the server deletes a page
itself.

## On a phone

Everything above works on a phone. The sidebar is a drawer, every row's buttons show, and
each control is a full touch target. Pages do not drag on a touch screen: **Move to…**,
**Move up** and **Move down** do the same. The editor works with the phone's keyboard, but
its block handles and menus assume a mouse; a toolbar above the keyboard for editing on a
phone is **planned**.
