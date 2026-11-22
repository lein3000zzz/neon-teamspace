---
title: Users
description: Who sees the admin pages, and the accounts on the instance, with adding one, changing roles, disabling and enabling, and verifying addresses.
---

The admin pages are at `/admin`, behind **Admin** in the sidebar, which only admins see.
Anyone else who opens one of their addresses gets a page that says it is not allowed, and
the app asks the server nothing for it. The server refuses those requests in any case:
the menu decides nothing. When an admin stops being one, the admin pages say so as soon as
they next ask the server for anything.

They have four parts: **Users**, **Invites** and **Requests to join**, described in
[Invites and requests](/docs/admin/invites/), and [Settings](/docs/admin/settings/). Each
has its own address: `/admin`, `/admin/invites`, `/admin/join-requests`, which the mail
telling admins of a new request links to, and `/admin/settings`.

## The accounts

**Users** lists every account, in the order they were made, 50 at a time with **Load more**
for the rest. Each shows its display name and address, and marks what sets it apart: an
admin, the signed-in admin's own account (**You**), a disabled account, an address not
verified yet.

## Adding an account

**Add an account** makes one with an address, a password and a role, and an optional
display name. It works whatever the registration mode, `closed` included, and the account
counts as verified. The password follows the instance's rules, so hand it over safely, and
the person can change it from their account page.

## Roles and disabling

Each account has these, each asked again before it is done:

- **Make admin**: the account can manage every account, invite and setting.
- **Make member**: it loses the admin pages, and its API tokens lose the `admin` scope.
- **Disable**: it is signed out everywhere, its API tokens stop working, and it cannot sign
  in until enabled again.
- **Enable**: it can sign in again.

The last admin who can sign in can be neither made a member nor disabled. The page then says
nothing changed, because this is the last active admin, and that another account must be
made an admin first. An admin's own account has the same buttons, with a warning: making
yourself a member closes the admin pages, and disabling yourself signs you out.

## Verifying addresses

An account whose address is not verified has two more buttons. **Mark verified** takes the
admin's word for it, for when the mail does not arrive. **Send a verification link** queues
a new link, at most one a minute and five a day, and says so, with what is missing, when the
instance has no mail set up.
