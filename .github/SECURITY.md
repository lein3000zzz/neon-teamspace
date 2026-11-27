# Security policy

Neon Teamspace holds people's accounts, passwords, sessions, API and OAuth tokens, webhook
secrets and their workspaces' pages, so a flaw in it can expose all of them. Report one
privately, so that a fix can ship before anyone else learns of it.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: on the repository's **Security** tab, **Report a
vulnerability**, or
[github.com/lein3000zzz/neon-teamspace/security/advisories/new](https://github.com/lein3000zzz/neon-teamspace/security/advisories/new).
The report, and everything said about it there, stays between you and the maintainer until an
advisory is published. Do not open an issue, a pull request or a discussion for it, as those
are public.

Say in the report:

- the version: what `neon --version` prints, or `GET /api/v1/meta` answers, or the image tag
  or commit you ran;
- how the instance runs: the Compose file of a release, a build from a clone, or something
  else, and any setting the problem needs, such as the registration mode or
  `webhooks.allow_private`;
- what an attacker needs first, such as an account, a role in a workspace, a token with some
  scopes, or nothing at all;
- the steps that show it, as requests, a script or a test, with what happened and what
  should have;
- what it gives the attacker: reading or changing what they may not, taking over an account,
  running code, or stopping the instance.

A report that names a dependency, a crate, a JS package or a base image, belongs with that
project too; report it here when Neon Teamspace's use of it is what makes it exploitable.

## Supported versions

Fixes ship in a new patch release of the newest minor version, such as 0.1.1 after 0.1.0, and
in no older line: until 1.0.0, upgrading to the newest release is how an instance gets them.
[Upgrading](../docs/src/content/docs/self-hosting/upgrading.md) covers the move. The `dev`
and `main` branches get each fix as it merges.

| Version | Fixes |
|---|---|
| The newest 0.x release | Yes |
| Earlier releases | No |
| Pre-releases, such as `0.2.0-rc.1` | No: the release that follows them gets the fix |

## What to expect

One person maintains the project, so the times below are what to expect, not promises:

1. A reply within 7 days that the report arrived, and within 14 days whether it is a
   vulnerability and how serious.
2. A fix, worked on in the advisory's private fork, with you as a collaborator if you like,
   and a patch release as soon as it is ready: within 30 days for a serious flaw, sooner when
   it is being exploited.
3. The advisory published with the release, crediting you unless you would rather not be
   named, and a CVE requested through GitHub where the flaw warrants one.

Please keep the details private until the advisory is published, or until 90 days after the
report, whichever comes first. A report that turns out not to be a vulnerability is closed
with the reason, and may become an ordinary issue.

The [security model](../docs/src/content/docs/architecture/security.md) describes how the
server keeps secrets, tells who a request is, and limits guessing and forgery. A server that
does less than that page says is a vulnerability.
