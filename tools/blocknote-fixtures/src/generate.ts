/**
 * Rewrites crates/blocks/tests/fixtures/blocknote from the corpus. `make blocks-fixtures`
 * runs it, then has the Rust tests rewrite rust-out from the result.
 */

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FIXTURES } from "./blocknote.ts";
import { blockNoteFixtures } from "./fixtures.ts";

const directory = join(FIXTURES, "blocknote");
const fixtures = await blockNoteFixtures();
rmSync(directory, { recursive: true, force: true });
mkdirSync(directory, { recursive: true });
for (const { name, update, json } of fixtures) {
  writeFileSync(join(directory, `${name}.yjs`), update);
  writeFileSync(join(directory, `${name}.json`), json);
}
console.log(`wrote ${fixtures.length} BlockNote fixtures to ${directory}`);
