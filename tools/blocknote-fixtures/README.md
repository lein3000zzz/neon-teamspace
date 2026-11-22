# blocknote-fixtures

BlockNote on the server, through `@blocknote/server-util`, to test `crates/blocks` against the
editor itself. It writes the fixtures `neon-blocks` reads, and checks that BlockNote loads what
`neon-blocks` writes. [Page documents](../../docs/src/content/docs/architecture/documents.md)
describes the document model and the fixtures.

Its editor has the web app's schema: BlockNote's default blocks, and the app's own made from
the configs in `../../web/src/components/editor/custom-blocks.ts`. That module imports nothing,
so this project imports it by its path and needs none of the web app's packages.

The BlockNote packages are pinned to exact versions, and the web app's editor takes the same
ones. A new BlockNote version changes the fixtures, so its update runs `make blocks-fixtures`
and commits the result.

| Script     | What it does                                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `generate` | Rewrites `crates/blocks/tests/fixtures/blocknote` from the corpus beside it.                                                         |
| `verify`   | Fails when `blocknote/` is not what BlockNote writes now, or when an update in `rust-out/` does not load in BlockNote as its blocks. |
| `check`    | `tsc`, the Prettier check, then `verify`.                                                                                            |
| `format`   | Prettier.                                                                                                                            |
| `licenses` | The license check of `../../scripts/licenses.ts` for this project's packages.                                                        |

From the repository's root, `make blocks-install` installs the packages, `make
blocks-fixtures` rewrites both fixture directories, and `make blocks-check` runs the Rust
tests and `check`. `bunfig.toml` pins the hoisted `node_modules` layout the license check walks.
