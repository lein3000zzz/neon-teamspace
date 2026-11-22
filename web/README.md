# web

The browser UI of Neon Teamspace. It is a single-page app built with React 19, Vite, TanStack
Router and Query, Tailwind CSS v4 and shadcn/ui components. The `neon` server and this app share
one origin, so the UI calls `/api/v1/...` directly and session cookies need no CORS setup. The
mobile and desktop shells load the same UI, which is why every screen works from 360px wide with
touch input.

## Scripts

Run them with `bun run <script>` from this directory, after `bun install`. From the repository's
root, `make web-install`, `make web-dev`, `make web-check` and `make web-build` do the same for
`bun install`, `dev`, `check` and `build`.

| Script         | What it does                                                                                                        |
| -------------- | ------------------------------------------------------------------------------------------------------------------- |
| `dev`          | Vite dev server. It proxies `/api`, `/ws`, `/mcp`, `/hooks` and `/.well-known` to the server on `127.0.0.1:8080`.   |
| `build`        | Type-checks, then writes the production bundle to `dist/`, with `THIRD_PARTY_NOTICES.txt` beside it.                |
| `preview`      | Serves `dist/` locally.                                                                                             |
| `gen:api`      | Regenerates `src/api/schema.d.ts` from `../api/openapi.json`. Commit the result.                                    |
| `check:api`    | Fails when `schema.d.ts` is out of date with the spec.                                                              |
| `lint`         | oxlint, here and in `../scripts`. Warnings fail too.                                                                |
| `format`       | prettier, here and in `../scripts`. It also sorts Tailwind classes.                                                 |
| `format:check` | prettier without writing.                                                                                           |
| `test`         | vitest with jsdom, Testing Library and MSW, then the tests of `../scripts` with Bun.                                |
| `licenses`     | Fails when any installed package, dev dependencies included, has a license outside the allowlist in `../deny.toml`. |
| `check`        | `tsc -b`, lint, format check, API check and tests, in that order.                                                   |

TypeScript stays on 6.x. `openapi-typescript` builds its output with the TypeScript compiler
API, and the 7.x native compiler does not ship that API.

The dev proxy forwards each of those paths and everything below it, but not a longer name
(`/apiary` stays with Vite). Like Caddy in production, it keeps the browser's `Host` header.

## Licenses

The license check is `../scripts/licenses.ts`, which the docs use too. It reads the allowlist
from `../deny.toml`, so Rust and JS dependencies follow one policy. Only a package that never
ships may sit outside it, used while building or never loaded: it needs a build-only exception
in `../scripts/licenses/policy.ts` with its exact license and a reason, and the check fails on
an exception nothing uses any more. A package whose `package.json` misstates its license gets
a clarification there instead, for that one version. Fonts are checked on their own. Every
directory under `public/` that holds font files must carry its license file, and the SIL Open
Font License 1.1 is accepted for fonts only. `bunfig.toml` pins the hoisted `node_modules`
layout that the check walks.

`../scripts` has no packages of its own, so this app's tools check it: `tsc -b` type-checks it
through `tsconfig.node.json`, and `lint`, `format` and `test` cover it as well, with the
configuration in `../scripts`. Its `package.json` only marks the scripts as ES modules, for the
Vite config, which imports some of them.

The minifier strips the license comments from the bundle, so the build writes
`dist/THIRD_PARTY_NOTICES.txt` instead (`scripts/licenses/notices.ts`, on helpers it shares
with the docs in `../scripts/licenses/notices.ts`). It holds the license
text of every package whose code or stylesheet ends up in `dist/`, and of every bundled font,
and it is served like any other static file. The build fails when a bundled package ships no
license text. In that case, copy the text from the package's source repository into
`scripts/licenses/supplied/` and list it in `SUPPLIED`.

## Theming

Four attributes on `<html>` drive the whole theme:

- `data-palette` is `nothing`, `soft` or `warm`.
- `data-mode` is the resolved mode, `light` or `dark`. The "system" choice resolves here and
  follows `prefers-color-scheme` live.
- `data-ui-font` is `default` (the system sans stack) or `mono` (iA Writer Mono, bundled in
  `public/fonts`).
- the `--accent-h` custom property holds the accent hue, 0 to 359.

`src/styles/palettes.css` sets raw tokens per palette and mode, then derives the shadcn
variables (`--background`, `--primary`, `--ring`, ...) from them. `src/styles/index.css` maps
those into Tailwind's `@theme`, so utilities such as `bg-primary` or `text-muted-foreground`
follow the theme. The default Tailwind colours and shadows are removed on purpose. A colour
that is not a token cannot creep in.

The accent is `oklch(var(--accent-l) var(--accent-c) var(--accent-h))`. The user picks only the
hue. Lightness and chroma are fixed per palette and mode, chosen so accent text on the page and
dark or light ink on an accent fill stay at 4.5:1 or better at every hue. Dark modes use a
brighter, more saturated accent and give hue-carrying elements a glow (`shadow-glow`). The glow
is zero in light modes. Anything that is not a focus ring, a primary action, a presence dot or
the active item stays flat.

An element can carry another entity's hue instead of the user's accent. Set `data-hue` and
`--accent-h` on it (see `hueScope` in `src/components/nodes/hue.ts`). Inside it, `--neon`,
`--neon-soft` and `--glow` re-derive from that hue, so the `bg-neon`, `text-neon`,
`border-neon` and `shadow-glow` utilities show the entity's colour. `--primary` and `--ring`
stay the user's accent, so primary actions and focus rings keep it. Use the neon utilities for
anything that should take the entity's hue. `hueFor(id)` in `src/lib/hue.ts` computes an
entity's hue from its id with FNV-1a, the same way as lein3000.dev. `src/lib/hue-vectors.json`
holds test vectors that any other implementation must match.

Preferences are kept with the account, at `/api/v1/account/preferences`: signing in loads them
into the theme store (`src/components/theme/preferences-sync.tsx`), and the appearance page saves
each change there once a slider drag stops. `localStorage` under `neon.theme` is this browser's
copy of them, and `public/theme.js` applies it before first paint. `src/theme/preferences.ts` holds the same rules in TypeScript, and
`boot-script.test.ts` checks that the two agree. Where the browser refuses storage, as it does
with site data blocked, both fall back to the defaults. Where storage is full, the stored theme
still loads. Either way, new choices apply for the rest of the visit without being saved.

## Signing in and the API

Every call to the server goes through `src/api`: `client.ts` is the typed `openapi-fetch` client,
and `auth.ts`, `account.ts`, `admin.ts`, `workspaces.ts`, `teamspaces.ts` and `pages.ts` hold
the TanStack Query queries and mutations of each part, so a change to an endpoint has one place to land. The session is the server's `HttpOnly`
cookie, sent because the app and the API share an origin; the same origin satisfies the server's
check of `Origin` on requests that change something.

The routes hang off two layouts. `_app` is the shell, for everything that needs someone signed
in: its `beforeLoad` asks `GET /api/v1/account/me` and sends a visitor without a session to
`/login?next=…`. `_auth` is the centred card of the pages for signing in and up, the mailed links
and requests to join. `createQueryClient` watches every failed request: a 401 `unauthenticated`
means the session ended, which forgets the user and leads back to sign in, and a 403 `forbidden`
asks again who is signed in, so a role taken away shows at once. Answers the server refused on
purpose, any 4xx, are not retried. A mutation can carry a password, a token's secret or an
invite code, so it leaves the cache as soon as no component shows it, and signing in or out
clears every one along with the previous user's queries (`forgetPrivateData` in
`src/api/account.ts`).

Search values stay strings (`src/router.tsx`), and the one-time token of a mailed link, or an
invite link's code, is taken out of the address as its page opens (`useLinkSecret` in
`src/lib/links.ts`).

## Workspaces

A workspace's pages are under `/w/$slug` (`src/routes/_app/w`). Its layout finds the slug in the
user's list of workspaces and hands the workspace down with `useWorkspace()`; the sidebar's
switcher and teamspace list read the same list through `useCurrentWorkspace()`
(`src/components/workspaces/workspace-context.tsx`). `/` opens the workspace used last in this
browser, kept per account in `localStorage` (`src/lib/last-workspace.ts`).

Each workspace's queries sit under one key, `keys.workspace(id)`, so a change to its members
refreshes all of them. `createQueryClient` also reads the workspaces again after a 403, as a
role may have changed, and the list after a workspace's data answers 404, as the user may have
been removed. `src/lib/workspace-rules.ts` repeats the server's permission rules only to decide
which controls a role sees; the server's answer still decides, and a refusal shows like any
other error.

A workspace invite link, `/workspace-invite/$token`, moves its token into the tab's
`sessionStorage` and out of the address, so it survives the way through signing in or up; the
start page sends the tab back to it while it is held. Leaving or deleting a workspace, or
changing its slug, moves the page on from the mutation's own `onSuccess`, before the workspace
leaves the cache: a page whose workspace is gone unmounts, and with it its `mutate()` callbacks.

## Pages and the editor

A page is at `/w/$slug/p/$pageId` (`src/pages/pages/page.tsx`) and the trash at
`/w/$slug/trash`. `src/api/pages.ts` holds their calls, under the workspace's key: the trees
(`keys.pageTrees`), each page and what hangs under it (`keys.pages`), the trash and the
favorites. The sidebar's trees are in `src/components/pages`; which pages are open, which
sections folded and which page is being dragged live in `PageNavProvider`, shared by the
column and the drawer. A title typed in a page's header is written into the cached trees
as it is typed (`showPage`) and saved once typing pauses.

The editor (`src/components/editor`) is BlockNote with its shadcn variant, at exactly the
versions `tools/blocknote-fixtures` pins, with no XL package. `React.lazy` loads it, so its
chunk only comes with a page. It binds the page's Yjs document, the `document-store`
fragment the server's `neon-blocks` reads, through the stock `y-websocket` provider on
`/ws/pages/{id}` of this origin. `connection.ts` turns the provider's events into the states
the page shows: it decides from a close code whether to try again (`afterClose`), reads the
server's auth messages, which turn the editor read-only and back, and cleans others'
awareness states before the editor reads them. `cursor.ts` draws others' cursors in their
hue, and `paste.ts` refuses a paste larger than the server takes. `lib0`, the encoding
library y-websocket's messages are read with, is a dependency for those message handlers.
`src/styles/editor.css`
puts BlockNote on the theme's tokens; `index.css` has Tailwind generate the utilities its
shadcn components use. BlockNote also asks for `interactive-widget=resizes-content` in the
viewport meta tag, which `index.html` has, so a phone's keyboard resizes the page rather
than covering it.

The tests talk to an MSW fake of the page routes (`src/test/pages.ts`) and, for the editor,
to a fake of the sync endpoint speaking the y-websocket protocol (`src/test/sync.ts`).

## Content-Security-Policy

The app needs this policy:

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'self'
```

Scripts need nothing inline: `public/theme.js` is a same-origin file, not an inline script.
Styles need `'unsafe-inline'`. The Radix dialog behind the narrow-screen drawer locks page
scrolling by injecting a `<style>` element, which `style-src 'self'` blocks, and the page behind
the drawer then keeps scrolling. A nonce would avoid that, but only with HTML rendered per
response. The styles React and Radix set on elements, such as an entity's hue or the slider
thumb's position, go through the DOM's style object, which a policy does not restrict. Images,
fonts and API calls fall under `default-src 'self'`: the build inlines no assets as `data:`
URIs, and the API shares the app's origin.
