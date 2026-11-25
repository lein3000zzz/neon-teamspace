---
title: Accounts and registration
description: The first admin, who may sign up, invites, requests to join, verification, password resets, sessions, API tokens and the admin's work on accounts.
---

Accounts belong to the instance. Each has an email address, a display name, a password and
a role, `admin` or `user`. Admins manage accounts, invites, requests to join and the
runtime settings. Everyone else uses the workspaces they are in.

This page has the rules and the API. The web app's pages for them are in [Signing in and
accounts](/docs/guides/signing-in/), and the admin pages in
[Administration](/docs/admin/users/).

:::note[Mail]
Verifying sign-ups, password resets, mailed invites and requests to join all send mail, so
they work once [mail is set up](/docs/self-hosting/mail/). Until then they stay off, and
`GET /api/v1/auth/config` reports them off. The rest of this page works without mail. Each
mail is queued with the change it tells of, so a change that fails sends nothing.
:::

## The first admin

A new instance has no accounts. There are two ways to make the first admin.

Set `admin.email` and `admin.password`, or `NEON_ADMIN__EMAIL` and `NEON_ADMIN__PASSWORD`,
and start the server. It makes the admin while no admin can sign in, and logs that it did,
with the new account's id. Once an admin exists, the server ignores both values, so they can
stay in the deployment. Set both or neither. The server refuses to start with only one, or
with a password the password rules refuse. The password never reaches a log line, and
`neon config check` shows it as `[redacted]`.

Or run `neon admin create` against the database:

```sh
neon admin create --email root@example.com --display-name Root
```

It asks for the password twice at a terminal, without echoing it. When stdin is not a
terminal, it reads the password from the first line of stdin, so a script can pipe it in.
Unlike the variables, the command makes an admin even when others exist, so it also
rescues an instance whose admins all lost their passwords.

## Who may sign up

`auth.registration` decides:

| Mode | Who signs up |
|---|---|
| `open` | Anyone. |
| `invite` | People with an invite an admin made. The default. |
| `closed` | Nobody. Admins add accounts themselves with `POST /api/v1/admin/users`. |

Admins can add accounts in every mode, signed in at a browser. An account an admin makes
counts as verified.

Signing up never signs anyone in by itself. The web app's sign-up form calls
`POST /api/v1/auth/register`, then signs in with the same password, unless the address must
be verified first.

## Invites

An invite admits a number of sign-ups, 1 to 1000 and 1 unless the admin says otherwise,
until it expires or an admin revokes it. It expires after `auth.invite_ttl`, a week unless
set, or at the end the admin picks, at most a year ahead.

Admins either type the code or let the server make one:

- A typed code has 8 to 64 letters, digits, `-` and `_`. `POST /api/v1/admin/invites/check`
  says what is wrong with one as the admin types it.
- A made code is `inv_` and 43 more characters, 256 random bits.

Codes match whatever their case, so `Spring-Team` and `spring-team` are one code, and two
invites cannot share one. The database keeps the code's SHA-256. With `secrets.keys` set,
it also keeps a sealed copy, so admins can copy the code again from
`GET /api/v1/admin/invites/{id}/code`. Without a key, the code shows once, when the invite
is made. The list of invites never carries codes.

An invite bound to an address admits only that address, once. Admins make these by mailing
an invite, and the server makes them when an admin approves a request to join. The address
counts as verified when it signs up, since the mail proved it. Signing up with someone
else's bound invite fails with `invalid_invite`, and the answer does not name the address.

Revoking an invite stops it at once. It stays listed with the people who signed up with it.
Thirty days after an invite expired or was revoked, the hourly cleanup removes it.

## Requests to join

While registration is by invite and `auth.requests` is on, people without an invite can ask
for one with a note of up to 500 characters. The steps:

1. The form calls `POST /api/v1/auth/join-requests`. It answers `204` whatever comes of it,
   so nobody learns from it whether an address has an account or a request.
2. After the answer, the server mails the address a link that confirms the request. It
   works for 24 hours. Asking again mails a new link, at most one a minute and five in all.
3. Opening the link confirms the request, through `POST /api/v1/auth/join-requests/confirm`.
   Only then do admins see it, and each active admin gets a mail about it while
   `mail.notifications` is on.
4. An admin approves it, which makes an invite bound to the address, for one sign-up, and
   mails it there. While mail cannot go, the invite is made all the same, the answer's
   `queued` is false, and the admin hands the code over. Or the admin denies it. With
   `auth.request_denied_mail` on, the address is told so, and it may ask again in 30 days.

Nothing is mailed to an address with an account, one whose request waits for the admins,
or one denied in the last 30 days. An address has one open request at most, and the
instance at most 200, so the form cannot fill the table. Requests nobody confirmed go a day
after their link stopped working. Decided ones go after 30 days.

## Verifying addresses

With `auth.verify_email` on, people who sign up themselves get a link by mail and sign in
once they have opened it. The link works once, for 24 hours. Until then, signing in with the
right password answers `email_not_verified`. A wrong password answers as for anyone else.

`POST /api/v1/auth/verification` mails a new link, and answers the same whether or not the
address has an account that needs one. An account gets at most one link a minute and five a
day. An account still unverified after 7 days is removed by the hourly cleanup. Someone who
signs up again with the address before that replaces the unverified account, so nobody can
hold an address that is not theirs.

## Resetting a password

With `auth.password_reset` on, the sign-in page can ask for a link that sets a new password.
`POST /api/v1/auth/password-reset` answers the same whether or not the address has an
account, and mails nothing to a disabled one. The link works once, for an hour, with the
same limit of one a minute and five a day.

Setting the new password verifies the address, signs the account out of every session, and
mails the account that its password changed. A password the rules refuse leaves the link
working, to try again.

## Passwords

A new password has at least `auth.min_password_length` characters, 12 unless set, and at
most 1024 bytes, and cannot be only spaces. Passwords set before a change to the minimum
keep working.

Changing a password with `POST /api/v1/account/password` needs the current one and a browser
session. It signs out every other session and mails the account that its password changed.

## Sessions

Signing in opens a browser session and sets its cookie. A session signs out when unused for
`auth.session_idle_ttl`, a week unless set, and in any case after
`auth.session_absolute_ttl`, 30 days unless set, from when it started.

`GET /api/v1/account/sessions` lists a user's sessions with their address and browser.
`DELETE /api/v1/account/sessions/{id}` ends one, and `DELETE /api/v1/account/sessions` ends
all but the current one. Signing out with `POST /api/v1/auth/logout` ends the current one
and clears the cookie.

## API tokens

Programs authenticate with an API token, sent as `Authorization: Bearer nw_…`. A user makes
one at a browser with `POST /api/v1/account/tokens`, which shows its secret once. After that,
the token's first 10 characters identify it in lists. It may expire at a set time, or never.

Each token has scopes:

| Scope | What it allows |
|---|---|
| `read` | `GET` requests. Every scope brings it. |
| `write` | Every other method on the routes the user may use. |
| `admin` | The `/api/v1/admin` routes. Only an admin's token may have it. For a token bound to a workspace, that workspace's admin work instead. |

A token works across the instance unless it is bound to one workspace the user is in, with
`workspace_id` when it is made: then it works on that workspace's routes and sockets alone,
and is refused on another workspace, the account's routes but its own workspace's
favorites, and the admin routes. Its `admin` scope is for the workspace's owners and admins;
see
[tokens bound to one workspace](/docs/guides/agents/#tokens-bound-to-one-workspace). An
agent's tokens, which its owner makes, are always bound to the agent's workspace.

A token never does more than its owner's role allows. When an admin stops being one, their
tokens lose the `admin` scope at once. Tokens cannot change passwords, end sessions, or make
and revoke tokens. Those need a browser session.

An admin's token can do less than the admin at a browser. It cannot create an account with a
password, change a role, or disable or enable an account, and it cannot change the settings
marked *browser session only*. It invites people, approves requests to join and verifies
addresses instead. A token that leaked therefore cannot give its holder an account to sign
in with at a browser, nor lock an admin out.

Revoking a token with `DELETE /api/v1/account/tokens/{id}` stops it at once.

Programs that connect over OAuth, such as MCP clients, need no token copied by hand: the
person allows them on a consent page, in one workspace, as themselves or as one of their
agents. They are listed under **Connected apps**, where each is disconnected; see
[Connecting apps over OAuth](/docs/guides/oauth/).

## Managing accounts

Admins list accounts with `GET /api/v1/admin/users`, in pages of up to 200, in the order the
accounts were made. `PATCH /api/v1/admin/users/{id}` changes a role, disables or enables an
account, or marks an address verified; with an API token, only the last goes through.
Disabling an account signs it out everywhere, and its tokens stop working.

The last admin who can sign in can be neither demoted nor disabled, which answers
`last_admin`. Two admins demoting each other at the same moment cannot both succeed: each
change locks the admins first, so the second sees the first's result.

Instance admins also list, limit and delete workspaces, without seeing what is in them; see
[Workspace rules and API](/docs/guides/workspace-rules/#for-instance-admins), and the admin
page for it, [Workspaces](/docs/admin/workspaces/).

## Preferences

Each user's look of the web app, the palette, the mode, the accent hue and the interface
font, is kept with their account at `/api/v1/account/preferences`, so it follows them from
browser to browser. An `accent_hue` of `null`, the default, takes the hue of the workspace
in view.

Apart from the accent, which colours the app for them, each user is drawn in a hue of their
own wherever others see them: the one their id hashes to, or one they pin with
`hue_override` on `PATCH /api/v1/account/me`.
