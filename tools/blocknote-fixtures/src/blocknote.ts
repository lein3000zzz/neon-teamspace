/**
 * BlockNote on the server, set up as the web app's editor is: BlockNote's default blocks and
 * the app's own, and the page's blocks in the XmlFragment the app binds. Fixtures written with
 * it are deterministic: block ids count up in document order and every Yjs document has the
 * same client id.
 */

import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { BlockNoteSchema, createBlockSpec } from "@blocknote/core";
import { ServerBlockNoteEditor } from "@blocknote/server-util";
import { getDocument } from "@blocknote/server-util/dom";
import { yXmlFragmentToProseMirrorRootNode } from "y-prosemirror";
import * as Y from "yjs";
import { databaseBlockConfig } from "../../../web/src/components/editor/custom-blocks.ts";

/** The name of the fragment that holds a page's blocks; neon-blocks has the same. */
export const FRAGMENT = "document-store";

/** The client id of every Yjs document written here, so the updates' bytes are the same on each run. */
const CLIENT_ID = 1;

export const FIXTURES = resolve(import.meta.dir, "../../../crates/blocks/tests/fixtures");

const dom = getDocument();

/**
 * The editor's schema: BlockNote's default blocks, and the app's own from the configs the web
 * app's editor uses. This tool converts blocks and never draws them, so each of the app's
 * blocks draws as an empty element here.
 */
const schema = BlockNoteSchema.create().extend({
  blockSpecs: {
    [databaseBlockConfig.type]: createBlockSpec(databaseBlockConfig, {
      render: () => ({ dom: dom.createElement("div") }),
    })(),
  },
});

/** A block of the schema, and a partial one, which leaves out what has a default. */
export type Block = typeof schema.Block;
export type PartialBlock = typeof schema.PartialBlock;

const editor = ServerBlockNoteEditor.create({ schema }, dom);

/** The names of the files in a fixture directory with an extension, without it, sorted. */
export function fixtureNames(directory: string, extension: string): string[] {
  return readdirSync(join(FIXTURES, directory))
    .filter((file) => file.endsWith(extension))
    .map((file) => file.slice(0, -extension.length))
    .sort();
}

/** Gives each block the id `block-<n>`, counting parents before their children. */
export function numberBlocks(blocks: PartialBlock[]): PartialBlock[] {
  let next = 1;
  const number = (list: PartialBlock[]): PartialBlock[] =>
    list.map((block) => {
      const id = `block-${next}`;
      next += 1;
      return { ...block, id, children: number(block.children ?? []) };
    });
  return number(blocks);
}

export function parseMarkdown(markdown: string): Promise<Block[]> {
  return editor.tryParseMarkdownToBlocks(markdown);
}

/** A Yjs update that holds the blocks, as BlockNote writes them. */
export function encode(blocks: PartialBlock[]): Uint8Array {
  const doc = new Y.Doc();
  doc.clientID = CLIENT_ID;
  // Its type asks for whole blocks, but it fills in what partial ones leave out, as
  // blocksToYDoc, which takes partial ones, does.
  editor.blocksToYXmlFragment(blocks as Block[], doc.getXmlFragment(FRAGMENT));
  return Y.encodeStateAsUpdate(doc);
}

function load(update: Uint8Array): Y.Doc {
  const doc = new Y.Doc();
  Y.applyUpdate(doc, update);
  return doc;
}

/**
 * The blocks a Yjs update holds, as BlockNote reads them, in JSON's terms. The ProseMirror
 * document is checked against the schema first: y-prosemirror drops what the schema does not
 * allow, so a reading that only succeeded would hide it.
 */
export function decode(update: Uint8Array): unknown {
  const doc = load(update);
  const fragment = doc.getXmlFragment(FRAGMENT);
  const before = Y.encodeStateAsUpdate(doc);
  yXmlFragmentToProseMirrorRootNode(fragment, editor.editor.pmSchema).check();
  const after = Y.encodeStateAsUpdate(doc);
  if (!equalBytes(before, after)) {
    throw new Error("y-prosemirror changed the document while reading it");
  }
  return JSON.parse(JSON.stringify(editor.yXmlFragmentToBlocks(fragment)));
}

/**
 * The structure of the fragment a Yjs update holds: each element's name, attributes and
 * children, and each text's runs with their formatting, with object keys sorted so two
 * writers that set the same attributes in another order compare equal.
 */
export function structure(update: Uint8Array): string {
  const fragment = load(update).getXmlFragment(FRAGMENT);
  return JSON.stringify(sortKeys(fragment.toArray().map(describe)), null, 1);
}

function describe(node: Y.XmlElement | Y.XmlText | Y.XmlHook): unknown {
  if (node instanceof Y.XmlText) {
    return { text: node.toDelta() };
  }
  if (node instanceof Y.XmlElement) {
    const attributes = Object.fromEntries(
      Object.entries(node.getAttributes()).map(([key, value]) => [
        key,
        value === undefined ? { undefined: true } : value,
      ]),
    );
    return { element: node.nodeName, attributes, children: node.toArray().map(describe) };
  }
  return { hook: true };
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortKeys);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortKeys((value as Record<string, unknown>)[key])]),
    );
  }
  return value === undefined ? { undefined: true } : value;
}

export function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((byte, index) => byte === b[index]);
}

export function toJson(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}
