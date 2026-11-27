import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import * as Y from "yjs";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { fakeComments } from "@/test/comments";
import { fakePages, page, PAGE_IDS } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeSync } from "@/test/sync";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";
import { COARSE_QUERY } from "@/lib/media";
import { FRAGMENT } from "./connection";

const PATH = `/w/design/p/${PAGE_IDS.roadmap}`;

/** A screen whose main pointer is a finger, or a mouse. */
function pointer(kind: "touch" | "mouse") {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: kind === "touch" && query === COARSE_QUERY,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

/** A paragraph block, in the structure BlockNote keeps blocks in. */
function paragraph(id: string, text: string): Y.XmlElement {
  const inline = new Y.XmlElement("paragraph");
  inline.insert(0, [new Y.XmlText(text)]);
  const container = new Y.XmlElement("blockContainer");
  container.setAttribute("id", id);
  container.insert(0, [inline]);
  return container;
}

/** A page whose content the server holds: a paragraph of each text. */
function pageWith(...texts: string[]) {
  const sync = fakeSync();
  const group = new Y.XmlElement("blockGroup");
  sync.doc.getXmlFragment(FRAGMENT).insert(0, [group]);
  group.insert(
    0,
    texts.map((text, index) => paragraph(`block-${index + 1}`, text)),
  );
  return sync;
}

/** The page's content, once it is editable. */
async function editable() {
  const region = await screen.findByRole("region", { name: "Page content" });
  const content = await within(region).findByRole("textbox", { name: "" });
  await waitFor(() => expect(content).toHaveAttribute("contenteditable", "true"));
  return content;
}

/**
 * Selects `text` where the editor shows it, as a long press and the handles
 * would, and tells the editor, as the browser does.
 */
function selectText(root: HTMLElement, text: string) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const at = node.textContent?.indexOf(text) ?? -1;
    if (at >= 0) {
      const range = document.createRange();
      range.setStart(node, at);
      range.setEnd(node, at + text.length);
      document.getSelection()?.removeAllRanges();
      document.getSelection()?.addRange(range);
      act(() => {
        document.dispatchEvent(new Event("selectionchange"));
      });
      return;
    }
  }
  throw new Error(`no text "${text}" in the editor`);
}

/** The editing toolbar, while it shows. */
function toolbar() {
  return within(screen.getByRole("toolbar", { name: "Editing tools" }));
}

/** The editing toolbar, shown or not: hidden, it has no name to find it by. */
function hiddenBar() {
  const bar = screen.getByRole("toolbar", { hidden: true });
  expect(bar).toHaveAttribute("aria-label", "Editing tools");
  return bar;
}

/** The blocks of the page's content, by their content's tag and text. */
function blocks(content: HTMLElement): string[] {
  return [...content.querySelectorAll(".bn-block-content")].map((block) => {
    const inline = block.querySelector(".bn-inline-content");
    const tag = inline?.tagName.toLowerCase() ?? block.getAttribute("data-content-type");
    return `${tag}: ${block.textContent ?? ""}`;
  });
}

describe("the editing toolbar of a touch screen", () => {
  // The page loads the editor on demand; loaded once here, so no test waits on it.
  beforeAll(async () => {
    await import("./page-editor");
  });

  it("shows while the content has focus, and goes with it", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    renderApp(PATH);
    const content = await editable();

    const bar = hiddenBar();
    expect(bar).not.toBeVisible();
    act(() => content.focus());
    await waitFor(() => expect(bar).toBeVisible());
    expect(toolbar().getByRole("button", { name: "Block type: Text" })).toBeInTheDocument();

    act(() => content.blur());
    await waitFor(() => expect(bar).not.toBeVisible());
  });

  it("is not there for a mouse, which has the formatting toolbar", async () => {
    pointer("mouse");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    renderApp(PATH);
    const content = await editable();
    act(() => content.focus());

    expect(screen.queryByRole("toolbar", { hidden: true })).toBeNull();
  });

  it("does not show for someone who only views the page", async () => {
    pointer("touch");
    fakeWorkspaces({ role: "member" });
    fakePages({
      pages: [
        page(
          { id: PAGE_IDS.roadmap, title: "Roadmap", teamspace: TEAMSPACE_IDS.open, access: "view" },
          "a0",
        ),
      ],
    });
    pageWith("Kick-off on Monday");
    renderApp(PATH);
    const region = await screen.findByRole("region", { name: "Page content" });
    const content = await within(region).findByRole("textbox", { name: "" });
    expect(await screen.findByText("Read-only", { selector: "output" })).toBeInTheDocument();
    act(() => content.focus());

    expect(hiddenBar()).not.toBeVisible();
  });

  it("formats what is typed next, and changes, moves, inserts and undoes blocks", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday", "Retro on Friday");
    const { user } = renderApp(PATH);
    const content = await editable();
    await user.click(content);
    await waitFor(() => toolbar());

    // Bold, pressed, holds for what is typed next; the focus stays in the content.
    await user.click(toolbar().getByRole("button", { name: "Bold" }));
    expect(toolbar().getByRole("button", { name: "Bold" })).toHaveAttribute("aria-pressed", "true");
    expect(content).toHaveFocus();
    await user.keyboard("Agenda ");
    expect(content.querySelector("strong")).toHaveTextContent("Agenda");

    // The block's type, from its row, which goes back to the main one.
    await user.click(toolbar().getByRole("button", { name: "Block type: Text" }));
    await user.click(toolbar().getByRole("button", { name: "Heading 2" }));
    expect(blocks(content)[0]).toBe("h2: Agenda Kick-off on Monday");
    expect(toolbar().getByRole("button", { name: "Block type: Heading 2" })).toBeInTheDocument();

    // The block's menu moves it down, past the next, and back up.
    await user.click(toolbar().getByRole("button", { name: "Block menu" }));
    const menu = within(toolbar().getByRole("group", { name: "Block menu" }));
    expect(menu.getByRole("button", { name: "Move up" })).toBeDisabled();
    await user.click(menu.getByRole("button", { name: "Move down" }));
    expect(blocks(content)).toEqual(["p: Retro on Friday", "h2: Agenda Kick-off on Monday"]);
    expect(menu.getByRole("button", { name: "Move down" })).toBeDisabled();
    await user.click(menu.getByRole("button", { name: "Move up" }));
    expect(blocks(content)).toEqual(["h2: Agenda Kick-off on Monday", "p: Retro on Friday"]);

    // Duplicated, then the copy deleted.
    await user.click(menu.getByRole("button", { name: "Duplicate" }));
    expect(blocks(content)).toEqual([
      "h2: Agenda Kick-off on Monday",
      "h2: Agenda Kick-off on Monday",
      "p: Retro on Friday",
    ]);
    await user.click(toolbar().getByRole("button", { name: "Block menu" }));
    await user.click(toolbar().getByRole("button", { name: "Delete" }));
    expect(blocks(content)).toEqual(["h2: Agenda Kick-off on Monday", "p: Retro on Friday"]);

    // With the copy gone, the caret is in the block after it: a divider goes after that, and
    // a block to type in after the divider.
    await user.click(toolbar().getByRole("button", { name: "Insert" }));
    await user.click(toolbar().getByRole("button", { name: "Divider" }));
    expect(blocks(content)).toEqual([
      "h2: Agenda Kick-off on Monday",
      "p: Retro on Friday",
      "divider: ",
      "p: ",
    ]);
    // Undone, with the changes made just before it, which the history takes as one; and back.
    await user.click(toolbar().getByRole("button", { name: "Undo" }));
    expect(blocks(content)).not.toContain("divider: ");
    await user.click(toolbar().getByRole("button", { name: "Redo" }));
    expect(blocks(content)).toEqual([
      "h2: Agenda Kick-off on Monday",
      "p: Retro on Friday",
      "divider: ",
      "p: ",
    ]);

    // Done: the keyboard goes, and the toolbar with it.
    await user.click(toolbar().getByRole("button", { name: "Done editing" }));
    expect(content).not.toHaveFocus();
    await waitFor(() =>
      expect(screen.queryByRole("toolbar", { name: "Editing tools" })).toBeNull(),
    );
  });

  it("offers Redo as soon as Undo took something back, and Undo again once Redo put it", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    const { user } = renderApp(PATH);
    const content = await editable();
    await user.click(content);
    await waitFor(() => toolbar());
    const undo = toolbar().getByRole("button", { name: "Undo" });
    const redo = toolbar().getByRole("button", { name: "Redo" });
    expect(undo).toBeDisabled();
    await user.keyboard("Agenda ");
    expect(undo).toBeEnabled();

    // A tap and nothing else, no selection changed after it: the history alone says what
    // there is.
    act(() => {
      fireEvent.click(undo);
    });
    expect(content).toHaveTextContent(/^Kick-off on Monday$/);
    expect(redo).toBeEnabled();
    expect(undo).toBeDisabled();

    act(() => {
      fireEvent.click(redo);
    });
    expect(content).toHaveTextContent("Agenda Kick-off on Monday");
    expect(undo).toBeEnabled();
    expect(redo).toBeDisabled();
  });

  it("indents a block under the one before, and back out", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday", "Retro on Friday");
    const { user } = renderApp(PATH);
    const content = await editable();
    act(() => content.focus());
    selectText(content, "Friday");
    await waitFor(() => expect(toolbar().getByRole("button", { name: "Indent" })).toBeEnabled());

    expect(toolbar().getByRole("button", { name: "Outdent" })).toBeDisabled();
    await user.click(toolbar().getByRole("button", { name: "Indent" }));
    expect(content.querySelector(".bn-block-group .bn-block-group")).toHaveTextContent(
      "Retro on Friday",
    );
    await user.click(toolbar().getByRole("button", { name: "Outdent" }));
    expect(content.querySelector(".bn-block-group .bn-block-group")).toBeNull();
  });

  it("links the text selected, from a field of its own", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    const { user } = renderApp(PATH);
    const content = await editable();
    act(() => content.focus());
    await waitFor(() => toolbar());
    expect(toolbar().getByRole("button", { name: "Link" })).toBeDisabled();

    selectText(content, "Monday");
    await waitFor(() => expect(toolbar().getByRole("button", { name: "Link" })).toBeEnabled());
    await user.click(toolbar().getByRole("button", { name: "Link" }));
    const field = toolbar().getByRole("textbox", { name: "Link address" });
    expect(field).toHaveFocus();
    // Focus in the toolbar's own field keeps it there.
    await user.type(field, "example.com/monday{Enter}");

    expect(content.querySelector("a")).toHaveAttribute("href", "https://example.com/monday");
    expect(content.querySelector("a")).toHaveTextContent("Monday");
    expect(content).toHaveFocus();
    expect(toolbar().getByRole("button", { name: "Link" })).toHaveAttribute("aria-pressed", "true");
  });

  it("gives the focus back to the content as Back leaves the link's field", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    const { user } = renderApp(PATH);
    const content = await editable();
    act(() => content.focus());
    selectText(content, "Monday");
    await waitFor(() => expect(toolbar().getByRole("button", { name: "Link" })).toBeEnabled());
    await user.click(toolbar().getByRole("button", { name: "Link" }));
    expect(toolbar().getByRole("textbox", { name: "Link address" })).toHaveFocus();

    await user.click(toolbar().getByRole("button", { name: "Back" }));
    expect(content).toHaveFocus();
    expect(toolbar().getByRole("button", { name: "Link" })).toBeInTheDocument();
    expect(content.querySelector("a")).toBeNull();
  });

  it("puts the keyboard away from the link's field too", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    const { user } = renderApp(PATH);
    const content = await editable();
    act(() => content.focus());
    selectText(content, "Monday");
    await waitFor(() => expect(toolbar().getByRole("button", { name: "Link" })).toBeEnabled());
    await user.click(toolbar().getByRole("button", { name: "Link" }));
    const field = toolbar().getByRole("textbox", { name: "Link address" });
    expect(field).toHaveFocus();

    await user.click(toolbar().getByRole("button", { name: "Done editing" }));
    expect(field).not.toHaveFocus();
    expect(content).not.toHaveFocus();
    await waitFor(() =>
      expect(screen.queryByRole("toolbar", { name: "Editing tools" })).toBeNull(),
    );
  });

  it("starts a comment on the text selected", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    fakeComments();
    const { user } = renderApp(PATH);
    const content = await editable();
    act(() => content.focus());
    await waitFor(() => toolbar());
    expect(toolbar().getByRole("button", { name: "Comment" })).toBeDisabled();

    selectText(content, "Kick-off");
    await waitFor(() => expect(toolbar().getByRole("button", { name: "Comment" })).toBeEnabled());
    await user.click(toolbar().getByRole("button", { name: "Comment" }));

    const panel = within(await screen.findByRole("complementary", { name: "Comments" }));
    const draft = within(panel.getByRole("region", { name: "A comment on the selected text" }));
    expect(draft.getByText("Kick-off")).toBeInTheDocument();
  });

  it("starts again from its main row once it shows again", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    pageWith("Kick-off on Monday");
    const { user } = renderApp(PATH);
    const content = await editable();
    await user.click(content);
    await waitFor(() => toolbar());
    await user.click(toolbar().getByRole("button", { name: "Insert" }));
    expect(toolbar().getByRole("group", { name: "Insert" })).toBeInTheDocument();

    act(() => content.blur());
    await waitFor(() =>
      expect(screen.queryByRole("toolbar", { name: "Editing tools" })).toBeNull(),
    );
    act(() => content.focus());
    await waitFor(() => toolbar());
    expect(toolbar().queryByRole("group", { name: "Insert" })).toBeNull();
    expect(toolbar().getByRole("button", { name: "Insert" })).toBeInTheDocument();
  });
});
