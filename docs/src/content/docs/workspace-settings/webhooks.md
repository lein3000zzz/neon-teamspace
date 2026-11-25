---
title: Webhooks
description: The workspace's outgoing webhooks, registering one, its test event, secret, log and switch, and its incoming webhooks, with their URLs, signing secrets and requests.
---

**Webhooks**, at `/w/{slug}/settings/webhooks`, lists the workspace's webhooks. Everyone but
guests gets the tab. What webhooks send and take, and how to check their signatures, is in
the [Webhooks](/docs/guides/webhooks/) guide.

## Outgoing webhooks

The workspace's owners and admins see every outgoing webhook of the workspace; everyone else
sees their own and their agents'. An agent's card under [Agents](/docs/workspace-settings/agents/)
links here with **Webhooks**, showing that agent's alone, with the form ready to register one
for it; **Show every webhook** shows them all again.

### Register a webhook

- **For**: whose events it tells of. Owners and admins choose anyone in the workspace;
  everyone else themselves or one of their own agents.
- **URL**: `http` or `https`. A host on the server's own network is refused, unless the
  instance allows private addresses.
- **Events**: assigned to a row, a status changed on a row they are assigned to, or both.
- **Also for their own changes**: off, nobody is told of what they did themselves.

The new webhook's signing secret shows once, with a button to copy it. A server without
`secrets.keys` refuses, and the form says so.

### Each webhook

Each card shows whose it is, its URL, **On** or **Off**, its events, and who registered it.
One that is off says why: turned off by someone, or after 5 deliveries in a row failed, with
the last failure. One whose last deliveries failed says how many. After a rotation, the card
says until when the previous secret signs too.

- **Send test event** sends one now and says how it went: its status and time, or why it
  failed.
- **Delivery log** shows its last attempts: how each went, its status, how long it took, why
  it failed or was skipped, and the start of the answer.
- **Turn off** and **Turn on**; turning it on clears its failures.
- **Edit** changes its URL, events and whether it hears the identity's own changes.
- **Rotate secret**, after a question, shows the new secret once; the old one signs beside it
  for a day.
- **Delete**, after a question.

## Incoming webhooks

The workspace's owners and admins make and manage them; everyone else is told that they do.

### Make an incoming webhook

- **Name**: its integration's name, shown as the name · via its maker where it writes.
- **Target**: a database or page of the teamspaces, one the maker may edit.
- **Take only signed requests**: makes a signing secret now.

Its URL shows once, with its signing secret if it has one, each with a button to copy it. A
workspace at its agent limit refuses, and the form says so.

### Each incoming webhook

Each card shows its name, its target, **On** or **Off**, **Signed** or **Unsigned**, its
integration's status and owner. A suspended integration says why it refuses what is sent.

- **Requests** shows its last requests: how each was answered, its status, action, client
  address and time.
- **Turn off** and **Turn on**.
- **Rename** renames it and its integration.
- **New URL**, after a question, shows the new URL once; the old one stops working at once.
- **Require signatures**, or **Rotate signing secret** once it has one, after a question,
  shows the new secret once; one it had still verifies for a day.
- **Stop checking signatures**, after a question.
- **Delete**, after a question, deletes its integration too.
