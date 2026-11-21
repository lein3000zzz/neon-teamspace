# web

The browser UI of Neon Workspace. It is a single-page app built with React 19, Vite, TanStack
Router and Query, Tailwind CSS v4 and shadcn/ui components. The `neon` server and this app share
one origin, so the UI calls `/api/v1/...` directly and session cookies need no CORS setup. The
mobile and desktop shells load the same UI, which is why every screen works from 360px wide with
touch input.

## Scripts

Run them with `bun run <script>` from this directory, after `bun install`.

| Script         | What it does                                                                                                        |
| -------------- | ------------------------------------------------------------------------------------------------------------------- |
| `dev`          | Vite dev server. It proxies `/api`, `/ws`, `/mcp`, `/hooks` and `/.well-known` to the server on `127.0.0.1:8080`.   |
| `build`        | Type-checks, then writes the production bundle to `dist/`, with `THIRD_PARTY_NOTICES.txt` beside it.                |
| `preview`      | Serves `dist/` locally.                                                                                             |
| `gen:api`      | Regenerates `src/api/schema.d.ts` from `../api/openapi.json`. Commit the result.                                    |
| `check:api`    | Fails when `schema.d.ts` is out of date with the spec.                                                              |
| `lint`         | oxlint. Warnings fail too.                                                                                          |
| `format`       | prettier, which also sorts Tailwind classes.                                                                        |
| `format:check` | prettier without writing.                                                                                           |
| `test`         | vitest with jsdom, Testing Library and MSW.                                                                         |
| `licenses`     | Fails when any installed package, dev dependencies included, has a license outside the allowlist in `../deny.toml`. |
| `check`        | `tsc -b`, lint, format check, API check and tests, in that order.                                                   |

TypeScript stays on 6.x. `openapi-typescript` builds its output with the TypeScript compiler
API, and the 7.x native compiler does not ship that API.

The dev proxy forwards each of those paths and everything below it, but not a longer name
(`/apiary` stays with Vite). Like Caddy in production, it keeps the browser's `Host` header.

## Licenses

The license check reads the allowlist from `../deny.toml`, so Rust and JS dependencies follow
one policy. Only data may sit outside it: such a package needs an entry in `EXCEPTIONS` in
`scripts/licenses.ts` with its exact license and a reason, and the check fails on an exception
nothing uses any more. Fonts are checked on their own. Every directory under `public/` that
holds font files must carry its license file, and the SIL Open Font License 1.1 is accepted for
fonts only. `bunfig.toml` pins the hoisted `node_modules` layout that the check walks.

The minifier strips the license comments from the bundle, so the build writes
`dist/THIRD_PARTY_NOTICES.txt` instead (`scripts/licenses/notices.ts`). It holds the license
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

Preferences live in `localStorage` under `neon.theme`. `public/theme.js` applies them before
first paint. `src/theme/preferences.ts` holds the same rules in TypeScript, and
`boot-script.test.ts` checks that the two agree. Where the browser refuses storage, as it does
with site data blocked, both fall back to the defaults. Where storage is full, the stored theme
still loads. Either way, new choices apply for the rest of the visit without being saved.

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
