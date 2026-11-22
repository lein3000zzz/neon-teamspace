# Neon Teamspace docs

The documentation site, served at `/docs`. It is [Astro](https://astro.build) with
[Starlight](https://starlight.astro.build), built into static files.

## Running it

Prerequisites: [Bun](https://bun.sh). Then, in this directory:

```sh
bun install
bun run chromium   # once: the headless Chromium that draws the diagrams
bun run dev        # http://localhost:4321/docs/
```

From the repository's root, `make docs-install` does the first two and `make docs-dev` the
third. `make docs-build` and `make docs-check` run the build and the check below.

| Script | What it does |
|---|---|
| `bun run dev` | Starts the development server, which reloads as pages change. |
| `bun run build` | Builds the site into `dist/`, with `THIRD_PARTY_NOTICES.txt`, then writes `csp.caddy`. Fails on a broken link or anchor; links into the generated API reference are not checked. |
| `bun run preview` | Serves `dist/` as it was built. |
| `bun run check` | Type-checks the site and its scripts, and runs the scripts' tests. |
| `bun run chromium` | Installs Playwright's headless Chromium for the diagrams. |
| `bun run licenses` | Fails when any installed package, dev dependencies included, has a license outside the allowlist in `../deny.toml`, unless `../scripts/licenses/policy.ts` lists it as a build-only exception. |

## Layout

| Path | What it holds |
|---|---|
| `src/content/docs/` | The pages, in Markdown. A page's path is its URL under `/docs`. |
| `src/styles/custom.css` | The Nothing palette over Starlight's theme, in light and dark. |
| `src/assets/` | The logo and the dot-grid background. |
| `astro.config.mjs` | The site: its sidebar, the diagrams, the API reference, the link check. |
| `scripts/csp.ts` | The Content-Security-Policy step after the build. |
| `scripts/lib/notices.ts` | The third-party notices the build writes; see [Licenses](#licenses). |

New pages go into the sidebar in `astro.config.mjs`.

## Diagrams

Diagrams are Mermaid code blocks (` ```mermaid `). `rehype-mermaid` draws each into inline
SVG while the site builds, so pages carry no diagram script. It renders them in a headless
Chromium through Playwright, which must be installed before `dev` or `build`:

```sh
bun run chromium
```

On Linux, Chromium also needs system libraries. In a container or on CI, install them with
the browser:

```sh
bun --bun playwright install --with-deps --only-shell chromium
```

The diagrams are drawn in black and white and inverted in dark mode by `custom.css`.

## The API reference

The pages under Reference → API are generated from `../api/openapi.json` by
[`starlight-openapi`](https://github.com/HiDeoo/starlight-openapi) on each build. The spec
is written by the server (`neon openapi --out api/openapi.json`, from the repository's
root), so the docs need no change when the API does: rebuild them.

The settings reference, `src/content/docs/reference/settings.md`, is the output of
`neon settings docs`, generated from the server's settings registry. It is not edited by
hand: after a change to the registry, `make docs-gen`, from the repository's root, rewrites
it. `make docs-check` and a server test fail while the committed page differs from the
registry's.

## The Content-Security-Policy

Starlight puts a few small scripts inline: the theme picker, the sidebar's state. After
`astro build`, `scripts/csp.ts` reads every page in `dist/`, hashes each inline script with
SHA-256, and writes `csp.caddy`, a Caddy snippet with one directive:

```caddy
header Content-Security-Policy "default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-…' …; …"
```

The policy allows scripts from the docs' own origin, those inline scripts by their hashes and
no others, and WebAssembly for search (Pagefind), which `'wasm-unsafe-eval'` permits without
permitting `eval()` of JavaScript. The build fails if a page has an inline event handler,
such as `onclick`, which no hash can allow.

`csp.caddy` is a build output and is not committed. The web image copies it next to its
Caddyfile, as `/etc/caddy/docs-csp.caddy`, and imports it where it serves `/docs`, so the
policy always matches the pages it was built with:

```caddy
@docs path /docs /docs/*
handle @docs {
	import /etc/caddy/docs-csp.caddy
	root * /srv
	file_server
}
```

`web/Dockerfile` builds the docs, checks the Caddyfile with the snippet in place, and fails
the image's build when either is wrong. The app's policy is set first and the snippet's
replaces it, so it must stay a plain `header` directive, which Caddy applies at once.

## Images

Images are served as they are, with Astro's passthrough image service. Astro's default
service resizes them with `sharp`, whose libvips binary is LGPL-licensed; the docs have no
images that need it, so it is never loaded. Astro still installs `sharp` as an optional
dependency.

## Licenses

`bun run licenses` checks every installed package against the allow list in `../deny.toml`,
the one cargo-deny enforces for the crates. Two packages are outside it, and the check
allows them as build-only exceptions, by name and exact license, in
`../scripts/licenses/policy.ts`:

- `@img/sharp-libvips-*` (LGPL-3.0-or-later), above, which is never loaded.
- `@fortawesome/fontawesome-free` (CC-BY-4.0 AND OFL-1.1 AND MIT). `rehype-mermaid` draws the
  diagrams through `mermaid-isomorphic`, which adds Font Awesome's stylesheet to the page in
  the headless Chromium. The diagrams reach `dist/` as SVG, and none of Font Awesome does.

`khroma` 2.1.0, which Mermaid uses, declares no license in its `package.json`; a clarification
in the same file records the MIT License of its license file, for that version only. The
check fails on an exception or a clarification nothing installed needs any more.
`bunfig.toml` pins the hoisted `node_modules` layout that it walks.

The minifier strips the license comments from what ships, so the build writes
`dist/THIRD_PARTY_NOTICES.txt`, served at `/docs/THIRD_PARTY_NOTICES.txt`, with the license
text of every package whose code or styles reach `dist/`. `scripts/lib/notices.ts` finds
them in two ways: a Vite plugin records the packages with modules in the browser's scripts or
in the stylesheets, and `OUTSIDE_VITE` names those whose files get there another way, such as
Pagefind's search files and Expressive Code's `ec.*` files. `SUPPLIED` adds texts that a
package does not publish, copied from its source repository into `scripts/licenses/`, for
one version each: `@pagefind/default-ui` has none of its own and has Svelte's runtime
compiled in. The build fails when a shipped package has no license text, or when one of the
lists names a package it no longer needs.
