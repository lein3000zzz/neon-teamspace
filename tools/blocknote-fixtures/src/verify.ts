/**
 * Checks the fixtures both ways, and exits 1 on any difference:
 *
 * - blocknote/ is what BlockNote writes for the corpus now, so the Rust tests that read it
 *   test against this BlockNote version;
 * - every update in rust-out/, which neon-blocks wrote, loads in BlockNote without a change
 *   and reads as the blocks in the JSON beside it, and has the very structure BlockNote
 *   gives the same blocks.
 */

import { existsSync, readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import { join } from "node:path";
import type { PartialBlock } from "@blocknote/core";
import { FIXTURES, decode, encode, equalBytes, fixtureNames, structure } from "./blocknote.ts";
import { blockNoteFixtures } from "./fixtures.ts";

const problems: string[] = [];

const expected = await blockNoteFixtures();
const committed = fixtureNames("blocknote", ".yjs");
const names = expected.map((fixture) => fixture.name);
for (const name of committed.filter((name) => !names.includes(name))) {
  problems.push(`blocknote/${name}.yjs has no source in the corpus`);
}
for (const { name, update, json } of expected) {
  const updatePath = join(FIXTURES, "blocknote", `${name}.yjs`);
  const jsonPath = join(FIXTURES, "blocknote", `${name}.json`);
  if (!existsSync(updatePath) || !equalBytes(readFileSync(updatePath), update)) {
    problems.push(`blocknote/${name}.yjs is not what BlockNote writes`);
  }
  if (!existsSync(jsonPath) || readFileSync(jsonPath, "utf8") !== json) {
    problems.push(`blocknote/${name}.json is not what BlockNote reads`);
  }
}

const rustOut = fixtureNames("rust-out", ".yjs");
if (rustOut.length === 0) {
  problems.push("rust-out/ has no updates");
}
for (const name of rustOut) {
  const update = new Uint8Array(readFileSync(join(FIXTURES, "rust-out", `${name}.yjs`)));
  const blocks = JSON.parse(
    readFileSync(join(FIXTURES, "rust-out", `${name}.json`), "utf8"),
  ) as PartialBlock[];
  let read: unknown;
  try {
    read = decode(update);
  } catch (error) {
    problems.push(`rust-out/${name}.yjs does not load in BlockNote: ${String(error)}`);
    continue;
  }
  if (!isDeepStrictEqual(read, blocks)) {
    problems.push(
      `rust-out/${name}.yjs reads as other blocks than rust-out/${name}.json:\n${JSON.stringify(read, null, 2)}`,
    );
  }
  if (structure(update) !== structure(encode(blocks))) {
    problems.push(
      `rust-out/${name}.yjs differs in structure from BlockNote's update for the same blocks:\n` +
        `neon-blocks: ${structure(update)}\nBlockNote: ${structure(encode(blocks))}`,
    );
  }
}

if (problems.length > 0) {
  for (const problem of problems) {
    console.error(problem);
  }
  console.error("\nRun make blocks-fixtures to rewrite the fixtures, then look at the difference.");
  process.exit(1);
}
console.log(
  `${expected.length} BlockNote fixtures are current; ${rustOut.length} updates from neon-blocks load in BlockNote as expected`,
);
