/**
 * The fixtures BlockNote writes, from the corpus in crates/blocks/tests/fixtures:
 *
 * - each markdown/<name>.md, parsed by BlockNote's markdown parser, and each
 *   blocks/<name>.json, blocks in BlockNote's JSON for what markdown cannot carry;
 * - become blocknote/<name>.yjs, the Yjs update BlockNote writes for them, and
 *   blocknote/<name>.json, the blocks BlockNote reads back from that update.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  FIXTURES,
  decode,
  encode,
  fixtureNames,
  numberBlocks,
  parseMarkdown,
  toJson,
  type PartialBlock,
} from "./blocknote.ts";

export interface Fixture {
  name: string;
  update: Uint8Array;
  json: string;
}

async function corpus(): Promise<{ name: string; blocks: PartialBlock[] }[]> {
  const markdown = await Promise.all(
    fixtureNames("markdown", ".md").map(async (name) => ({
      name,
      blocks: await parseMarkdown(readFileSync(join(FIXTURES, "markdown", `${name}.md`), "utf8")),
    })),
  );
  const json = fixtureNames("blocks", ".json").map((name) => ({
    name,
    blocks: JSON.parse(
      readFileSync(join(FIXTURES, "blocks", `${name}.json`), "utf8"),
    ) as PartialBlock[],
  }));
  const names = [...markdown, ...json].map((entry) => entry.name);
  const twice = names.filter((name, index) => names.indexOf(name) !== index);
  if (twice.length > 0) {
    throw new Error(`markdown/ and blocks/ both have ${twice.join(", ")}`);
  }
  return [...markdown, ...json];
}

/** Every fixture BlockNote writes, sorted by name. */
export async function blockNoteFixtures(): Promise<Fixture[]> {
  const fixtures = (await corpus()).map(({ name, blocks }) => {
    const update = encode(numberBlocks(blocks));
    return { name, update, json: toJson(decode(update)) };
  });
  return fixtures.sort((a, b) => a.name.localeCompare(b.name));
}
