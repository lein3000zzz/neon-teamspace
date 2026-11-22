---
title: Settings
description: The runtime settings on the admin page, where each value comes from, pinned settings, secrets, saving changes and the problems the server names.
---

**Settings** lists every runtime setting, in groups by the part of its key before the first
dot, such as **Accounts** for `auth.*` and **Instance** for `instance.*`. The [settings
reference](/docs/reference/settings/) describes each one. A saved change applies without a
restart.

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
several options, a number, a duration such as `30m`, `12h` or `7d`, or text. A setting saved
here has **Reset to default**, which takes it back to its default when saved.

Changes collect at the foot of the page, which counts them, until **Save changes** sends them
together, or **Discard** drops them. The server saves all of them or none. When it refuses
any, the page says nothing was saved and shows each refusal beside its setting, such as a
number out of its bounds, or requests to join turned on while registration is not by
invite; the changes stay, to fix and save again.

## Secrets

A secret setting never shows its value: the field stays empty, and says whether one is set.
Typing in it replaces the value when saved, and **Reset to default** clears it. The server
keeps secrets sealed with the [secret keys](/docs/self-hosting/configuration/#secret-keys),
and refuses to save one while none is set.
