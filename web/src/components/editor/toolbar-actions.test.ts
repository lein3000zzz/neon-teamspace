import { BlockNoteEditor, type PartialBlock } from "@blocknote/core";
import { SuggestionMenu } from "@blocknote/core/extensions";
import { TextSelection } from "prosemirror-state";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  blockTypeOf,
  currentBlocks,
  duplicate,
  fileBlockOf,
  historyState,
  insertBlock,
  linkAddress,
  move,
  movable,
  openMenu,
  remove,
  selectsText,
  setCaption,
  toggleMark,
  turnable,
  turnInto,
  type Editor,
} from "./toolbar-actions";

let mounted: Editor[] = [];

// The suggestion menu keeps where its trigger is as a DOMRect's JSON, which jsdom's lack.
beforeEach(() => {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect());
});

afterEach(() => {
  for (const editor of mounted) {
    editor.unmount();
  }
  mounted = [];
  document.body.replaceChildren();
});

/** An editor of BlockNote's own blocks holding `blocks`, mounted in the page. */
function editorWith(blocks: PartialBlock[]): Editor {
  const editor = BlockNoteEditor.create({ initialContent: blocks }) as unknown as Editor;
  const element = document.createElement("div");
  document.body.append(element);
  editor.mount(element);
  mounted.push(editor);
  return editor;
}

/** The blocks' types and text, in order, nested ones after their parent with a ">" each. */
function outline(editor: Editor): string[] {
  const lines: string[] = [];
  const walk = (blocks: Editor["document"], depth: number) => {
    for (const block of blocks) {
      const content: unknown = block.content;
      const text = Array.isArray(content)
        ? content.map((item: { text?: string }) => item.text ?? "").join("")
        : "";
      lines.push(`${">".repeat(depth)}${block.type}${text === "" ? "" : `: ${text}`}`);
      walk(block.children, depth + 1);
    }
  };
  walk(editor.document, 0);
  return lines;
}

/** The block of that id, which the test put there. */
function blockOf(editor: Editor, id: string) {
  const block = editor.getBlock(id);
  if (block === undefined) {
    throw new Error(`no block ${id}`);
  }
  return block;
}

/** The editor's suggestion menus, which the page's editor always has. */
function menuOf(editor: Editor) {
  const menu = editor.getExtension(SuggestionMenu);
  if (menu === undefined) {
    throw new Error("no suggestion menus");
  }
  return menu;
}

/** Selects the text from `from` to `to` in the block of that id. */
function selectIn(editor: Editor, id: string, from: number, to: number) {
  editor.setTextCursorPosition(id, "start");
  editor.transact((tr) => {
    const start = tr.selection.from;
    tr.setSelection(TextSelection.create(tr.doc, start + from, start + to));
  });
}

const THREE: PartialBlock[] = [
  { id: "a", type: "paragraph", content: "Alpha" },
  { id: "b", type: "paragraph", content: "Bravo" },
  { id: "c", type: "paragraph", content: "Charlie" },
];

describe("the editing toolbar's actions", () => {
  it("turns the caret's block into each type, a heading by its level", () => {
    const editor = editorWith(THREE);
    editor.setTextCursorPosition("b", "end");

    turnInto(editor, "heading2");
    expect(editor.getBlock("b")).toMatchObject({ type: "heading", props: { level: 2 } });
    expect(blockTypeOf(blockOf(editor, "b"))).toBe("heading2");

    turnInto(editor, "checkList");
    expect(editor.getBlock("b")?.type).toBe("checkListItem");
    expect(blockTypeOf(blockOf(editor, "b"))).toBe("checkList");

    turnInto(editor, "code");
    expect(editor.getBlock("b")?.type).toBe("codeBlock");
    // The text stays through every change.
    expect(outline(editor)).toEqual(["paragraph: Alpha", "codeBlock: Bravo", "paragraph: Charlie"]);
  });

  it("turns every block selected at once, in one step of undo", () => {
    const editor = editorWith(THREE);
    editor.setSelection("a", "b");

    turnInto(editor, "bulletList");
    expect(outline(editor)).toEqual([
      "bulletListItem: Alpha",
      "bulletListItem: Bravo",
      "paragraph: Charlie",
    ]);
    editor.undo();
    expect(outline(editor)).toEqual(["paragraph: Alpha", "paragraph: Bravo", "paragraph: Charlie"]);
  });

  it("names no type for a toggle heading, and changes none of a block without text", () => {
    const editor = editorWith([
      { id: "t", type: "heading", props: { level: 1, isToggleable: true }, content: "Folded" },
      { id: "d", type: "divider" },
    ]);
    expect(blockTypeOf(blockOf(editor, "t"))).toBeNull();
    expect(turnable([blockOf(editor, "t")])).toBe(true);
    expect(turnable([blockOf(editor, "d")])).toBe(false);

    editor.setTextCursorPosition("t", "end");
    turnInto(editor, "heading1");
    // Picked from the toolbar, a heading is a plain one.
    expect(editor.getBlock("t")?.props).toMatchObject({ level: 1, isToggleable: false });
  });

  it("moves a block up and down past its neighbours, and says where it can go", () => {
    const editor = editorWith(THREE);
    editor.setTextCursorPosition("a", "end");
    expect(movable(editor, currentBlocks(editor))).toEqual({ up: false, down: true });

    move(editor, "down");
    expect(outline(editor)).toEqual(["paragraph: Bravo", "paragraph: Alpha", "paragraph: Charlie"]);
    move(editor, "down");
    expect(outline(editor)).toEqual(["paragraph: Bravo", "paragraph: Charlie", "paragraph: Alpha"]);
    expect(movable(editor, currentBlocks(editor))).toEqual({ up: true, down: false });

    move(editor, "up");
    expect(outline(editor)).toEqual(["paragraph: Bravo", "paragraph: Alpha", "paragraph: Charlie"]);
    // The caret stays in the block it moved.
    expect(editor.getTextCursorPosition().block.id).toBe("a");
  });

  it("moves a nested block out past its parent's end", () => {
    const editor = editorWith([
      {
        id: "p",
        type: "paragraph",
        content: "Parent",
        children: [{ id: "k", type: "paragraph", content: "Kid" }],
      },
    ]);
    editor.setTextCursorPosition("k", "end");
    expect(movable(editor, currentBlocks(editor))).toEqual({ up: true, down: true });
  });

  it("duplicates the blocks after them, children too, with the caret in the copy", () => {
    const editor = editorWith([
      {
        id: "p",
        type: "bulletListItem",
        content: "Parent",
        children: [{ id: "k", type: "paragraph", content: "Kid" }],
      },
      { id: "z", type: "paragraph", content: "After" },
    ]);
    editor.setTextCursorPosition("p", "end");

    duplicate(editor);
    expect(outline(editor)).toEqual([
      "bulletListItem: Parent",
      ">paragraph: Kid",
      "bulletListItem: Parent",
      ">paragraph: Kid",
      "paragraph: After",
    ]);
    const copy = editor.document[1];
    expect(copy?.id).not.toBe("p");
    expect(editor.getTextCursorPosition().block.id).toBe(copy?.id);
  });

  it("deletes the blocks selected", () => {
    const editor = editorWith(THREE);
    editor.setSelection("a", "b");
    remove(editor);
    expect(outline(editor)).toEqual(["paragraph: Charlie"]);
  });

  it("puts a divider in place of an empty block, after one with text, and after a block without", () => {
    const editor = editorWith([
      { id: "e", type: "paragraph" },
      { id: "t", type: "paragraph", content: "Text" },
      { id: "i", type: "image", props: { url: "/files/a.png" } },
    ]);
    editor.setTextCursorPosition("e");
    insertBlock(editor, "divider");
    expect(outline(editor).slice(0, 2)).toEqual(["divider", "paragraph: Text"]);

    editor.setTextCursorPosition("t", "end");
    insertBlock(editor, "divider");
    expect(outline(editor).slice(0, 3)).toEqual(["divider", "paragraph: Text", "divider"]);

    editor.setTextCursorPosition("i");
    insertBlock(editor, "divider");
    expect(outline(editor).slice(3, 5)).toEqual(["image", "divider"]);
  });

  it("opens the slash and mention menus at the caret, keeping the text selected", () => {
    const editor = editorWith(THREE);
    const menu = menuOf(editor);
    menu.addSuggestionMenu({ triggerCharacter: "/" });
    menu.addSuggestionMenu({ triggerCharacter: "@" });
    selectIn(editor, "b", 0, 5);

    openMenu(editor, "@");
    expect(menu.store.state).toMatchObject({ triggerCharacter: "@", show: true });
    // The selection was not typed over: the trigger goes after it.
    expect(outline(editor)[1]).toBe("paragraph: Bravo@");
  });

  it("opens the slash menu in a new block after one that takes no text", () => {
    const editor = editorWith([
      { id: "c", type: "codeBlock", content: "let x = 1;" },
      { id: "z", type: "paragraph", content: "After" },
    ]);
    const menu = menuOf(editor);
    menu.addSuggestionMenu({ triggerCharacter: "/" });
    editor.setTextCursorPosition("c", "end");

    openMenu(editor, "/");
    expect(outline(editor)).toEqual(["codeBlock: let x = 1;", "paragraph: /", "paragraph: After"]);
    expect(menu.store.state).toMatchObject({ triggerCharacter: "/", show: true });
  });

  it("marks the text selected, and says whether text is selected", () => {
    const editor = editorWith(THREE);
    editor.setTextCursorPosition("a", "end");
    expect(selectsText(editor)).toBe(false);

    selectIn(editor, "a", 0, 5);
    expect(selectsText(editor)).toBe(true);
    toggleMark(editor, "bold");
    toggleMark(editor, "strike");
    expect(editor.getBlock("a")?.content).toEqual([
      { type: "text", text: "Alpha", styles: { bold: true, strike: true } },
    ]);
    toggleMark(editor, "bold");
    expect(editor.getActiveStyles()).toEqual({ strike: true });
  });

  it("knows when there is something to undo and redo", () => {
    const editor = editorWith(THREE);
    expect(historyState(editor)).toEqual({ undo: false, redo: false });
    editor.setTextCursorPosition("a", "end");
    turnInto(editor, "quote");
    expect(historyState(editor)).toEqual({ undo: true, redo: false });
    editor.undo();
    expect(historyState(editor)).toEqual({ undo: false, redo: true });
  });

  it("finds the file block the caret is on, and writes its caption", () => {
    const editor = editorWith([
      { id: "t", type: "paragraph", content: "Text" },
      { id: "i", type: "image", props: { url: "/files/a.png" } },
    ]);
    editor.setTextCursorPosition("t");
    expect(fileBlockOf(editor)).toBeNull();

    editor.setTextCursorPosition("i");
    const image = fileBlockOf(editor);
    expect(image?.id).toBe("i");
    setCaption(editor, blockOf(editor, "i"), "  The launch  ");
    expect(editor.getBlock("i")?.props).toMatchObject({ caption: "The launch" });
  });

  it("links an address as typed, with https where it names no scheme a link may have", () => {
    expect(linkAddress(" example.com/a ")).toBe("https://example.com/a");
    expect(linkAddress("http://example.com")).toBe("http://example.com");
    expect(linkAddress("mailto:ada@example.com")).toBe("mailto:ada@example.com");
    expect(linkAddress("HTTPS://example.com")).toBe("HTTPS://example.com");
    // A scheme that runs script, or a domain that only starts like a scheme, is a host.
    expect(linkAddress("javascript:alert(1)")).toBe("https://javascript:alert(1)");
    expect(linkAddress("httpbin.org")).toBe("https://httpbin.org");
  });
});
