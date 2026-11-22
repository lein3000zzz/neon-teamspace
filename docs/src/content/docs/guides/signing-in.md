---
title: Signing in and accounts
description: Signing in and up in the web app, invite links, asking to join, mailed links, and the account pages for the profile, password, sessions, API tokens and appearance.
---

Every page of the app needs someone signed in. Opening one without a session leads to the
sign-in page, and signing in leads back to the page that was asked for. The pages for
signing in and up are the only ones open to anyone. [Accounts and
registration](/docs/guides/accounts/) describes the rules behind them, and the API they call.

:::note[Mail]
The pages that need mail, password resets, asking to join and verifying an address, say
they are off until an admin sets [mail](/docs/self-hosting/mail/) up. The rest of this page
works without mail.
:::

## Signing in

`/login` asks for the email address and the password. A wrong password and an unknown
address get the same answer, `Invalid email or password`, so the form never tells anyone
whether an address has an account. A disabled account, or one whose address still needs
verifying, is only named once the password is right.

Too many attempts in a row are refused for a while. The form then says how long to wait,
from the server's `Retry-After`. Every form on these pages does the same.

The session lasts as the instance's settings say, a week unused and 30 days at most unless
an admin changed them. When it ends, by its time or because it was signed out from another
browser, the next page asks to sign in again, and then goes back to where the person was.
**Sign out**, at the foot of the sidebar, ends it at once.

## Creating an account

`/register` shows what the instance's registration mode allows:

| Mode | The page |
|---|---|
| `open` | A form: address, password, and an optional display name. |
| `invite` | The same form, with an invite code from an admin. |
| `closed` | A note that admins add the accounts, and no form. |

The password needs the minimum the instance sets, 12 characters unless an admin changed it,
and the form says so before it sends anything. Once the account exists, the form signs in
with it straight away, unless the instance verifies addresses first; then it asks to open
the link it mailed, and offers to send it again.

### Invite links

An admin shares an invite as a link, `/register?invite=CODE`. Opening it fills in the code,
and takes the code out of the address bar and the browser's history. An invite mailed to an
address works for that address alone, which the form fills in and keeps.

### Asking to join

While the instance takes requests to join, the sign-in and sign-up pages link to `/join`.
It asks for an address and a note for the admins. Whatever the address, the page then says
the same thing: if this address can ask, a link to confirm is on its way. Opening that link
confirms the request, and only then do the admins see it. If they approve it, the invite
arrives by mail.

## Mailed links

Links in mail carry a one-time token: `/verify?token=…` verifies an address,
`/reset?token=…` sets a new password, `/join/confirm?token=…` confirms a request to join.
Each page takes the token out of the address as it opens, so it stays out of the browser's
history. A link works once; an old or used one says so, and offers a new one.

`/forgot` asks for a link to set a new password. Like the request to join, it says the same
thing whether or not the address has an account. `/reset` without a token asks the same,
as the mail saying a password changed links there for anyone who did not change it. Setting
the new password signs the account out everywhere, then opens the sign-in form with the
address filled in.

## The account pages

**Account**, in the sidebar, opens `/account`, in four parts:

- **Profile**: the address, the role, and the display name, which can change. A blank name
  goes back to the part of the address before the `@`. Under it, the password: the current
  one and a new one, twice. Changing it signs out every other session.
- **Sessions**: each browser signed in, with its address and when it was last active. Any
  of them can be signed out from here, this one included, or every other one at once.
- **API tokens**: tokens for scripts and tools, each with a name, its scopes and an optional
  end date. A new token's secret shows once, with a button to copy it; after that only its
  first characters identify it. Revoking a token stops it at once.
- **Appearance**: the palette, the mode, the accent hue and the interface font. A change
  shows at once, is kept in this browser for the next first paint, and is saved to the
  account, so signing in on another browser brings the same look.

Admins also see **Admin** in the sidebar, which [Administration](/docs/admin/users/)
describes.
