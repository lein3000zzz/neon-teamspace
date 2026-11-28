---
title: Settings
description: The server's version and updates, the runtime settings on the admin page, where each value comes from, pinned settings, secrets, saving changes and the problems the server names.
---

**Settings** opens with the server's [version and updates](#version-and-updates), then lists
every runtime setting, in groups by the part of its key before the first dot, such as
**Accounts** for `auth.*` and **Instance** for `instance.*`. The [settings
reference](/docs/reference/settings/) describes each one. A saved change applies without a
restart.

## Version and updates

The panel at the top shows the release the server runs and the newest release on GitHub,
linked to its notes, with when the server last checked and why a check failed. **Check now**
asks GitHub again. While a newer release is out, it offers **Update to** that release where
an updater is set up, and otherwise the commands that update by hand; the **Admin** entry of
the sidebar is marked meanwhile. The **Updates** group below holds its settings: whether the
server checks, whether pre-releases count, and where the updater is.
[Updating from the admin settings](/docs/self-hosting/upgrading/#updating-from-the-admin-settings)
covers an update from start to end, and [Release checks](/docs/self-hosting/upgrading/#release-checks)
what a check sends GitHub and how to turn it off.

## Where a value comes from

Each setting says where its value comes from:

| Label | The value |
|---|---|
| **Environment** | Its `NEON_SETTINGS__` variable. |
| **Config file** | Its key under `settings:` in `neon.yaml`. |
| **Saved here** | Saved from this page. |
| **Default** | Nothing sets it. |

A setting from the environment or the config file is also **Pinned**: the page shows its
value but cannot change it, and names the variable to change instead. [Pinned
settings](/docs/self-hosting/configuration/#pinned-settings) explains why.

## Changing settings

Each kind of setting has its control: **On** and **Off** for a switch, a list for one of
several options, a button for each option of a set, such as `mcp.identities`, pressed for
those it holds, a number, a duration such as `30m`, `12h` or `7d`, or text. A setting saved
here has **Reset to default**, which takes it back to its default when saved.

Changes collect at the foot of the page, which counts them, until **Save changes** sends them
together, or **Discard** drops them. The server saves all of them or none. When it refuses
any, the page says nothing was saved and shows each refusal beside its setting, such as a
number out of its bounds, or verifying addresses turned on while mail is not set up; the
changes stay, to fix and save again.

## Secrets

A secret setting never shows its value: the field stays empty, and says whether one is set.
Typing in it replaces the value when saved, and **Reset to default** clears it. The server
keeps secrets sealed with the [secret keys](/docs/self-hosting/configuration/#secret-keys),
and refuses to save one while none is set.

## Mail

The **Mail** group starts with the mail status: whether mail can go, and if not, what is
missing, such as `mail.host is not set`. Under it, as the server has seen since it started,
when the mail server last took a mail, and the last delivery that failed: which mail, when,
what the mail server answered, and whether it is tried again. [Mail](/docs/self-hosting/mail/)
explains the settings and what each failure means.

**Send a test mail**, once mail can go, queues a test mail to the signed-in admin's own
address, with the saved settings. The status then follows how it goes for a minute, without
reloading the page.
