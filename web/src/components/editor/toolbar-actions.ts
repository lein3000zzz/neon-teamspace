import {
  insertOrUpdateBlockForSlashMenu,
  type Block,
  type BlockNoteEditor,
  type BlockSchema,
  type InlineContentSchema,
  type PartialBlock,
  type StyleSchema,
  type Styles,
} from "@blocknote/core";
import {
  DEFAULT_LINK_PROTOCOL,
  FilePanelExtension,
  SuggestionMenu,
  VALID_LINK_PROTOCOLS,
} from "@blocknote/core/extensions";
import { TextSelection, type Command } from "prosemirror-state";
import { yUndoPluginKey } from "y-prosemirror";
import type { UndoManager } from "yjs";

/** The page's editor, as the toolbar works on it, whatever its schema. */
export type Editor = BlockNoteEditor<BlockSchema, InlineContentSchema, StyleSchema>;

type AnyBlock = Block<BlockSchema, InlineContentSchema, StyleSchema>;

type AnyPartialBlock = PartialBlock<BlockSchema, InlineContentSchema, StyleSchema>;

/**
 * A change to a block, by its type and props. Under a schema of any blocks,
 * the editor's types give props no values at all, so the change goes through
 * as one the editor takes; the editor checks it against the page's schema.
 */
function change(update: { type?: string; props: Record<string, unknown> }): AnyPartialBlock {
  return update as unknown as AnyPartialBlock;
}

/** The block types the toolbar turns blocks into, in its order. */
export const BLOCK_TYPES = [
  { key: "paragraph", type: "paragraph" },
  { key: "heading1", type: "heading", level: 1 },
  { key: "heading2", type: "heading", level: 2 },
  { key: "heading3", type: "heading", level: 3 },
  { key: "bulletList", type: "bulletListItem" },
  { key: "numberedList", type: "numberedListItem" },
  { key: "checkList", type: "checkListItem" },
  { key: "quote", type: "quote" },
  { key: "code", type: "codeBlock" },
] as const;

export type BlockTypeKey = (typeof BLOCK_TYPES)[number]["key"];

/** The inline styles the toolbar toggles. */
export const MARKS = ["bold", "italic", "strike", "code"] as const;

export type Mark = (typeof MARKS)[number];

/**
 * Turns an inline style on for the text selected, or for what is typed next,
 * or off where it is on. The page's schema has all four; under a schema of any
 * styles, the editor's types give them no values, as for props.
 */
export function toggleMark(editor: Editor, mark: Mark) {
  editor.toggleStyles({ [mark]: true } as unknown as Styles<StyleSchema>);
}

/** The blocks the selection covers, or the one the caret is in. */
export function currentBlocks(editor: Editor): AnyBlock[] {
  return editor.getSelection()?.blocks ?? [editor.getTextCursorPosition().block];
}

/** A toggle heading is not one of the toolbar's headings: picking one makes it a plain heading. */
function toggleable(block: AnyBlock): boolean {
  return (block.props as { isToggleable?: boolean }).isToggleable === true;
}

/** Which of the toolbar's types a block is; `null` for any other, such as a toggle list. */
export function blockTypeOf(block: AnyBlock): BlockTypeKey | null {
  const level = (block.props as { level?: number }).level;
  const found = BLOCK_TYPES.find(
    (each) =>
      each.type === block.type &&
      (!("level" in each) || (each.level === level && !toggleable(block))),
  );
  return found?.key ?? null;
}

/** Whether blocks can change type: only those with text, not an image, a table or a database. */
export function turnable(blocks: readonly AnyBlock[]): boolean {
  return blocks.every((block) => Array.isArray(block.content));
}

/** Turns the blocks into a type, in one step of undo. */
export function turnInto(editor: Editor, key: BlockTypeKey) {
  const target = BLOCK_TYPES.find((each) => each.key === key);
  if (target === undefined) {
    return;
  }
  const heading = editor.schema.blockSchema["heading"]?.propSchema ?? {};
  const props: Record<string, number | boolean> =
    "level" in target
      ? { level: target.level, ...("isToggleable" in heading ? { isToggleable: false } : {}) }
      : {};
  editor.transact(() => {
    for (const block of currentBlocks(editor)) {
      editor.updateBlock(block, change({ type: target.type, props }));
    }
  });
}

/** Whether the blocks have somewhere to move: up or down past a sibling, or out of their parent. */
export function movable(
  editor: Editor,
  blocks: readonly AnyBlock[],
): { up: boolean; down: boolean } {
  const first = blocks[0];
  const last = blocks.at(-1);
  if (first === undefined || last === undefined) {
    return { up: false, down: false };
  }
  return {
    up: editor.getPrevBlock(first) !== undefined || editor.getParentBlock(first) !== undefined,
    down: editor.getNextBlock(last) !== undefined || editor.getParentBlock(last) !== undefined,
  };
}

/** Moves the blocks up or down past their neighbour, and scrolls to keep them in sight. */
export function move(editor: Editor, direction: "up" | "down") {
  if (direction === "up") {
    editor.moveBlocksUp();
  } else {
    editor.moveBlocksDown();
  }
  editor.transact((tr) => {
    tr.scrollIntoView();
  });
}

/** A block as new, for a copy: its content and its children's, without the ids. */
function copyOf(block: AnyBlock): AnyPartialBlock {
  return {
    type: block.type,
    props: block.props,
    content: block.content,
    children: block.children.map(copyOf),
  } as AnyPartialBlock;
}

/** Puts a copy of the blocks after them, and the caret at the end of the copy's first. */
export function duplicate(editor: Editor) {
  const blocks = currentBlocks(editor);
  const last = blocks.at(-1);
  if (last === undefined) {
    return;
  }
  editor.transact(() => {
    const [copy] = editor.insertBlocks(blocks.map(copyOf), last, "after");
    if (copy !== undefined) {
      editor.setTextCursorPosition(copy, "end");
    }
  });
}

/** Deletes the blocks. */
export function remove(editor: Editor) {
  editor.removeBlocks(currentBlocks(editor));
}

/** Whether text can be typed where the caret is: in a block of inline content, and not code. */
function typesText(block: AnyBlock): boolean {
  return Array.isArray(block.content) && block.type !== "codeBlock";
}

/**
 * A block of a type with no text, such as a divider or an image: in place of
 * the block the caret is in while that is empty, as the slash menu puts one,
 * and after it otherwise. The caret goes on to the next block that takes text.
 */
export function insertBlock(editor: Editor, type: string): AnyBlock {
  const current = editor.getTextCursorPosition().block;
  if (Array.isArray(current.content)) {
    return insertOrUpdateBlockForSlashMenu(editor, { type });
  }
  // A selected image, say, which the slash menu's helper cannot stand in.
  const [made] = editor.insertBlocks([{ type }], current, "after") as [AnyBlock];
  return made;
}

/**
 * An image or another file, picked on the device: a block for it where the
 * caret is, then the file uploaded into it as the editor uploads any. What the
 * server refuses leaves the block empty, to pick another, as the editor's own
 * file panel does.
 */
export async function insertUpload(editor: Editor, type: "image" | "file", file: File) {
  const upload = editor.uploadFile;
  if (upload === undefined) {
    return;
  }
  const block = insertBlock(editor, type);
  try {
    const uploaded = await upload(file, block.id);
    if (editor.getBlock(block.id) === undefined) {
      return;
    }
    editor.updateBlock(
      block.id,
      typeof uploaded === "string" ? { props: { name: file.name, url: uploaded } } : uploaded,
    );
  } catch {
    // The editor's upload hook says why; the empty block stays, to try again from.
  }
}

/**
 * Opens the menu a character opens when typed, `/` for blocks and `@` for
 * mentions, at the caret, which leaves any text selected in place. Where text
 * cannot be typed, as in code or on a selected image, the menu opens in a new
 * empty block after the caret's.
 */
export function openMenu(editor: Editor, trigger: "/" | "@") {
  const current = editor.getTextCursorPosition().block;
  if (typesText(current)) {
    editor.transact((tr) => {
      tr.setSelection(TextSelection.create(tr.doc, tr.selection.to));
    });
  } else {
    const [paragraph] = editor.insertBlocks([{ type: "paragraph" }], current, "after");
    if (paragraph !== undefined) {
      editor.setTextCursorPosition(paragraph, "start");
    }
  }
  editor.getExtension(SuggestionMenu)?.openSuggestionMenu(trigger, {
    deleteTriggerCharacter: true,
  });
}

/** The image, video, audio or other file the caret is on, alone; `null` for any other block. */
export function fileBlockOf(editor: Editor): AnyBlock | null {
  const blocks = currentBlocks(editor);
  const [block] = blocks;
  if (blocks.length !== 1 || block === undefined) {
    return null;
  }
  const props = editor.schema.blockSchema[block.type]?.propSchema ?? {};
  return "url" in props ? block : null;
}

/** Whether a block has a caption to write, as images, videos, audio and files do. */
export function captioned(editor: Editor, block: AnyBlock): boolean {
  return "caption" in (editor.schema.blockSchema[block.type]?.propSchema ?? {});
}

/** Sets a file block's caption; an empty one takes it away. */
export function setCaption(editor: Editor, block: AnyBlock, caption: string) {
  if (editor.getBlock(block.id) !== undefined) {
    editor.updateBlock(block.id, change({ props: { caption: caption.trim() } }));
  }
}

/** Opens the editor's own panel for a file block, to upload another file or link one instead. */
export function replaceFile(editor: Editor, block: AnyBlock) {
  editor.getExtension(FilePanelExtension)?.showMenu(block.id);
}

type History = { undoCommand: Command; redoCommand: Command };

/** The editor's history: the shared document's own undo, or ProseMirror's without one. */
function historyOf(editor: Editor): History | undefined {
  return (editor.getExtension("yUndo") ?? editor.getExtension("history")) as History | undefined;
}

/** Whether there is anything to undo and to redo. */
export function historyState(editor: Editor): { undo: boolean; redo: boolean } {
  const history = historyOf(editor);
  return history === undefined
    ? { undo: false, redo: false }
    : { undo: editor.canExec(history.undoCommand), redo: editor.canExec(history.redoCommand) };
}

/** What the shared document's undo manager tells of its stacks as they change. */
const STACK_EVENTS = ["stack-item-added", "stack-item-popped", "stack-cleared"] as const;

/** The shared document's undo manager, where the editor has one. */
function undoManagerOf(editor: Editor): UndoManager | undefined {
  const state = yUndoPluginKey.getState(editor.prosemirrorState) as
    { undoManager?: UndoManager } | undefined;
  return state?.undoManager;
}

/**
 * Calls `changed` whenever there may be more or less to undo or redo: as the
 * document changes, and as the shared document's undo manager adds to its
 * stacks or takes from them. An undo or a redo reaches the editor before the
 * manager moves what it undid to the other stack, so the editor's own change
 * comes too early to tell.
 */
export function onHistoryChange(editor: Editor, changed: () => void): () => void {
  let manager: UndoManager | undefined;
  const follow = () => {
    const current = undoManagerOf(editor);
    if (current === manager) {
      return;
    }
    for (const event of STACK_EVENTS) {
      manager?.off(event, changed);
      current?.on(event, changed);
    }
    manager = current;
  };
  follow();
  const stop = editor.onChange(() => {
    follow();
    changed();
  });
  return () => {
    stop();
    for (const event of STACK_EVENTS) {
      manager?.off(event, changed);
    }
  };
}

/** Whether the selection holds text, for a mark, a link or a comment to go on. */
export function selectsText(editor: Editor): boolean {
  const { from, to, empty } = editor.prosemirrorState.selection;
  return !empty && editor.prosemirrorState.doc.textBetween(from, to, " ", " ").trim() !== "";
}

/**
 * An address as typed, with the default scheme when it starts with none the
 * editor links to, as a bare domain does, or one it does not, such as
 * `javascript:`.
 */
export function linkAddress(typed: string): string {
  const address = typed.trim();
  return VALID_LINK_PROTOCOLS.some((protocol) => address.toLowerCase().startsWith(`${protocol}:`))
    ? address
    : `${DEFAULT_LINK_PROTOCOL}://${address}`;
}
