---
title: Workspaces and teamspaces
description: The workspace switcher, creating a workspace, joining one by invitation or invite link, and teamspaces in the web app.
---

A workspace is a team's space on the instance, and one account can be in several, with a
role in each: owner, admin, member or guest. This page is about the web app's pages for
them. [Workspace rules and API](/docs/guides/workspace-rules/) has the rules behind them and
the API they call, and [Permissions](/docs/reference/permissions/) the table of who may do
what.

The app shows each role only the controls it can use: a member sees no invite form, and an
admin no way to change an owner. The server decides in the end, and when it refuses
something, because a role changed since the page loaded, the page says so and reads the
workspace again.

## Where the app opens

Every workspace has its pages under `/w/` and its slug, such as `/w/design`. Signing in, or
opening `/`, leads to the workspace used last in this browser, while the account is still in
it, or else to the first of its workspaces by name. Each account has its own last
workspace, so people sharing a browser each come back to theirs.

An account in no workspace gets a page that says so, with the ways in:

- **Create a workspace**, while the instance lets everyone create them, which is the
  default. Where the `workspaces.creation` setting leaves it to instance admins, the page
  says so instead.
- How to be invited: ask a workspace's owner or an admin to invite the address the account
  signs in with, or for an invite link.
- The invitations waiting for the account, to accept or decline.

A workspace address the account is not in, or no longer, shows **Workspace not found**, as
for one that does not exist.

## The switcher

The top of the sidebar shows the workspace in view, with its mark: its icon, or the first
letter of its name, in its hue. It also shows the role the account has there. Pressing it
opens a menu of every workspace the account is in, each with its role, the current one
checked. Under them are **Workspace settings** and, while allowed, **Create a workspace**.
On pages outside any workspace, such as the account pages, the sidebar keeps showing the
workspace used last.

**Create a workspace**, at `/workspaces/new`, asks for a name, and optionally an icon and an
address. Blank, the address comes from the name. The new workspace comes with a teamspace
named General, and its creator owns both.

## Teamspaces

Under the switcher, the sidebar lists the teamspaces the account is in, each in its own
hue; a lock marks private ones. Guests are in no teamspace, so they get no list.

**Browse teamspaces**, at `/w/{slug}/teamspaces`, lists every teamspace the account sees:

| Access | On the page |
|---|---|
| Default | General, which everyone but guests is in, and stays in. |
| Open | **Join**, at once, and **Leave** from the teamspace's page. |
| Closed | Whom to ask to be added: its owners, by name. A workspace owner or admin can join it. |
| Private | Not listed, unless the account is in it. |

Each teamspace has a page of its own, `/w/{slug}/teamspaces/{id}`, with its access, its
members and the way in or out. Its pages will live there too, **planned**. A private
teamspace answers **Teamspace not found** to anyone outside it, the workspace's owners and
admins included. They see its name in the [settings](/docs/workspace-settings/teamspaces/),
where they can delete it, or add themselves to manage it, which its members then see.

## Invitations

An owner or admin invites an existing account by its address, from the
[members settings](/docs/workspace-settings/members/). The invited account sees the
invitation as soon as it signs in:

- the sidebar shows **Invitations** with the number waiting;
- `/account/invitations`, a tab of the account pages, lists each with the workspace, the
  role it offers, who sent it and when it ends;
- an account in no workspace sees them on the page it opens on.

**Accept** joins with the invitation's role, and **Decline**, after asking, drops it. A
workspace at its member limit takes nobody in, and accepting says so: the invitation stays,
to accept once there is room. No mail is sent for workspace invitations yet; the list is how
they arrive.

## Invite links

An invite link, `/workspace-invite/` followed by its token, admits anyone with an account on
the instance who opens it, with the link's role, until its uses run out or it ends. Opening
it takes the token out of the address, which then reads `/workspace-invite`. The page keeps
the token for that browser tab alone, in its session storage, until the link is used.

- Signed out, the page asks to sign in, and comes back to the link afterwards. Signing up
  first works too, in the same tab, where the instance takes sign-ups: the new account
  comes back to the link once it is signed in. Once the link has shown what it admits to,
  the app opens as usual from `/` again; `/workspace-invite` still shows the link while
  the tab keeps it.
- Signed in, the page shows the workspace the link is for, by name and mark, the role it
  admits with and when it ends. **Join** joins it with one press and opens it.
- An account in the workspace already gets the way into it instead.
- A link used up, past its end or revoked says it does not work.

Where the browser refuses session storage, the token stays in the address while the person
signs in, as it must for the way back.
