# Neon Workspace docs

The documentation site, served at `/docs`. It is [Astro](https://astro.build) with
[Starlight](https://starlight.astro.build), built into static files.

## Running it

Prerequisites: [Bun](https://bun.sh). Then, in this directory:

```sh
bun install
bun run chromium   # once: the headless Chromium that draws the diagrams
bun run dev        # http://localhost:4321/docs/
```

| Script | What it does |
|---|---|
| `bun run dev` | Starts the development server, which reloads as pages change. |
| `bun run build` | Builds the site into `dist/`, then writes `csp.caddy`. Fails on a broken link or anchor; links into the generated API reference are not checked. |
| `bun run preview` | Serves `dist/` as it was built. |
| `bun run check` | Type-checks the site and its scripts, and runs the scripts' tests. |
| `bun run chromium` | Installs Playwright's headless Chromium for the diagrams. |

## Layout

| Path | What it holds |
|---|---|
| `src/content/docs/` | The pages, in Markdown. A page's path is its URL under `/docs`. |
| `src/styles/custom.css` | The Nothing palette over Starlight's theme, in light and dark. |
| `src/assets/` | The logo and the dot-grid background. |
| `astro.config.mjs` | The site: its sidebar, the diagrams, the API reference, the link check. |
| `scripts/csp.ts` | The Content-Security-Policy step after the build. |

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

The settings reference, `src/content/docs/reference/settings.md`, holds the table that
`neon settings docs` prints from the server's settings registry. For now it is copied by
hand, so a change to the registry needs the same change here. Generating the page with the
command is planned.

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
handle /docs* {
	import /etc/caddy/docs-csp.caddy
	root * /srv
	file_server
}
```

## Images

Images are served as they are, with Astro's passthrough image service. Astro's default
service resizes them with `sharp`, whose libvips binary is LGPL-licensed; the docs have no
images that need it, so it is never loaded. Astro still installs `sharp` as an optional
dependency.
