---
title: Invites and requests
description: Making invites by code or by mail, sharing their links, following their sign-ups, revoking them, and deciding on requests to join.
---

While registration is by invite, the default, people sign up with an invite an admin made.
[Accounts and registration](/docs/guides/accounts/#invites) has the rules; this page is
about the admin pages that handle them.

## Making an invite

**Invites** has a form with three fields, all optional:

- **Code**: blank lets the server make one, `inv_` and 43 more characters. A typed code has 8
  to 64 letters, digits, `-` and `_`, and any case is the same code. The form checks a typed
  code as it is typed, and says when another invite has it.
- **Sign-ups**: how many accounts it admits, 1 to 1000, 1 unless set.
- **Expires**: the last day it works. Blank takes the invite lifetime setting from now,
  a week unless changed, and it can be at most a year ahead.

The new invite's link, `/register?invite=CODE`, and its code show at once, each with a
button to copy it. With [secret keys](/docs/self-hosting/configuration/#secret-keys) set, the
server keeps a sealed copy of the code, and **Show link** brings the link back later. Without
them, this is the only time the code shows, and the page says so.

### By mail

While inviting by email is on and mail is set up, **Invite by email** mails a single-use
invite to an address, signed with the admin's name. Only that address can sign up with it,
and it counts as verified. When the mail does not go out, the page says so and shows the link
to hand over another way.

## The invites

**All invites** lists every invite, newest first, with its state, **Open**, **Used up**,
**Expired** or **Revoked**, whom it is for, how many sign-ups it admitted of how many, when
it was made and when it ends. **Who signed up** opens the list of accounts made with it.
The list never holds codes.

**Revoke** stops an open invite at once, after asking. Accounts made with it stay, and it
stays listed until the hourly cleanup removes it, 30 days after it ended.

## Requests to join

With requests to join on, people without an invite can ask for one from the sign-in page.
**Requests to join** lists those whose address confirmed the request, waiting ones first,
then those decided in the last 30 days, each with its note.

- **Approve** makes an invite bound to the address, for one sign-up, and mails it there. Its
  link and code show as for any invite, and the page says when the mail did not go out.
- **Deny**, after asking, mails no invite. The address may ask again in 30 days, and is told
  so while the denied-request mail is on.

While requests are off, the page says so, with a link to the settings that turn them on.
They need registration by invite and mail set up.
