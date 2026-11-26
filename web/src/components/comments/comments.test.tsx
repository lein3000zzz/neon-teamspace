import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import * as Y from "yjs";
import { beforeAll, describe, expect, it } from "vitest";
import { FRAGMENT } from "@/components/editor/connection";
import { COMMENT_IDS, comment, fakeComments, pageThread } from "@/test/comments";
import { fakePages, PAGE_IDS, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { commentsMessage, fakeSync } from "@/test/sync";
import { fakeWorkspaces, people } from "@/test/workspaces";

const PATH = `/w/design/p/${PAGE_IDS.roadmap}`;
/** The first thread the fake server makes. */
const MADE_THREAD = "0193a5b2-0000-7000-8000-0000000000e7";

/** The page's content, once the editor is up: the one field in it without a name of its own. */
async function content() {
  const region = await screen.findByRole("region", { name: "Page content" });
  return within(region).findByRole("textbox", { name: "" });
}

/** Opens the comments panel from the page's button, and gives it. */
async function openPanel(user: ReturnType<typeof renderApp>["user"], name = "Comments") {
  await user.click(await screen.findByRole("button", { name }));
  return within(await screen.findByRole("complementary", { name: "Comments" }));
}

/** A paragraph block, in the structure BlockNote keeps blocks in. */
function paragraph(id: string, parts: (string | Y.XmlElement)[]): Y.XmlElement {
  const inline = new Y.XmlElement("paragraph");
  inline.insert(
    0,
    parts.map((part) => (typeof part === "string" ? new Y.XmlText(part) : part)),
  );
  const container = new Y.XmlElement("blockContainer");
  container.setAttribute("id", id);
  container.insert(0, [inline]);
  return container;
}

/** A page whose content the server holds: one paragraph of `parts`. */
function pageWith(parts: (string | Y.XmlElement)[]) {
  const sync = fakeSync();
  const blocks = new Y.XmlElement("blockGroup");
  sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
  blocks.insert(0, [paragraph("block-1", parts)]);
  return sync;
}

/** Selects `text` where the editor shows it, as dragging over it would. */
function selectText(root: HTMLElement, text: string) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const at = node.textContent?.indexOf(text) ?? -1;
    if (at >= 0) {
      const range = document.createRange();
      range.setStart(node, at);
      range.setEnd(node, at + text.length);
      const selection = document.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      return;
    }
  }
  throw new Error(`no text "${text}" in the editor`);
}

/** Where a relative position the editor sent lands in the server's copy of the document. */
function landsAt(doc: Y.Doc, base64: string) {
  const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
  const position = Y.createAbsolutePositionFromRelativePosition(
    Y.decodeRelativePosition(bytes),
    doc,
  );
  return { text: position?.type.toString(), index: position?.index };
}

describe("a page's comments", () => {
  // The page loads the editor on demand; loaded once here, so no test waits on it.
  beforeAll(async () => {
    await import("@/components/editor/page-editor");
  });

  it("starts a thread on the page, then replies in it, and counts the open ones", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const api = fakeComments();
    const { user } = renderApp(PATH);

    const panel = await openPanel(user);
    expect(await panel.findByText("No comments on this page yet.")).toBeInTheDocument();
    await user.type(
      panel.getByRole("combobox", { name: "A comment on the page" }),
      "Ship it on Friday",
    );
    await user.click(panel.getByRole("button", { name: "Comment" }));

    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(thread.getByText("Ship it on Friday")).toBeInTheDocument();
    expect(panel.getByRole("combobox", { name: "A comment on the page" })).toHaveValue("");
    await user.click(thread.getByRole("button", { name: "Reply" }));
    await user.type(thread.getByRole("combobox", { name: "Your reply" }), "Agreed");
    await user.click(thread.getByRole("button", { name: "Reply" }));

    expect(await thread.findByText("Agreed")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `/api/v1/pages/${PAGE_IDS.roadmap}/comments`,
        body: { markdown: "Ship it on Friday" },
      },
      {
        method: "POST",
        path: `/api/v1/threads/${MADE_THREAD}/comments`,
        body: { markdown: "Agreed" },
      },
    ]);
    expect(screen.getByRole("button", { name: "Comments, 1 open" })).toBeInTheDocument();
  });

  it("closes on Escape, giving focus back to its button", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    fakeComments();
    const { user } = renderApp(PATH);

    const panel = await openPanel(user);
    expect(panel.getByRole("heading", { name: "Comments" })).toHaveFocus();
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("complementary", { name: "Comments" })).toBeNull();
    expect(screen.getByRole("button", { name: "Comments" })).toHaveFocus();
  });

  it("mentions someone from what follows @, and says who a mention cannot tell, offering to share", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const api = fakeComments({
      people: [
        { person: people.bob, sees_page: true },
        { person: people.cy, sees_page: false },
      ],
      unnotified: [people.cy],
    });
    const { user } = renderApp(PATH);

    const panel = await openPanel(user);
    const field = panel.getByRole("combobox", { name: "A comment on the page" });
    await user.type(field, "Ask @c");
    const list = within(await panel.findByRole("listbox", { name: "People and pages to mention" }));
    expect(await list.findByRole("option", { name: /Cy/ })).toHaveTextContent(
      "Cannot see this page",
    );
    expect(list.queryByRole("option", { name: /Bob/ })).toBeNull();
    await user.keyboard("{Enter}");

    expect(field).toHaveValue(`Ask @[Cy](user:${people.cy.id}) `);
    expect(panel.queryByRole("listbox")).toBeNull();
    expect(await panel.findByText(/Cy cannot see this page/)).toBeInTheDocument();
    await user.type(field, "to look");
    await user.click(panel.getByRole("button", { name: "Comment" }));

    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(thread.getByText("@Cy", { exact: false })).toHaveClass("mention");
    expect(api.changes.at(-1)?.body).toEqual({
      markdown: `Ask @[Cy](user:${people.cy.id}) to look`,
    });
    await user.click(panel.getByRole("button", { name: "Share" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Share Roadmap" }));
    expect(await dialog.findByRole("combobox", { name: "Person" })).toHaveValue(people.cy.id);
  });

  it("mentions a page, as a link to it once posted", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const launch = samplePages().filter((each) => each.id === PAGE_IDS.launch);
    fakeComments({ pages: launch });
    const { user } = renderApp(PATH);

    const panel = await openPanel(user);
    const field = panel.getByRole("combobox", { name: "A comment on the page" });
    await user.type(field, "See @lau");
    await user.click(await panel.findByRole("option", { name: /Launch/ }));
    expect(field).toHaveValue(`See @[Launch](page:${PAGE_IDS.launch}) `);
    await user.click(panel.getByRole("button", { name: "Comment" }));

    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(thread.getByRole("link", { name: "@Launch" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.launch}`,
    );
  });

  it("changes and deletes the user's own comments, and only those", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const thread = pageThread(PAGE_IDS.roadmap, "First draft");
    thread.comments.push(
      comment({
        id: COMMENT_IDS.reply,
        thread: thread.id,
        markdown: "Looks good",
        author: people.bob,
      }),
    );
    const api = fakeComments({ threads: [thread] });
    const { user } = renderApp(PATH);

    const panel = await openPanel(user, "Comments, 1 open");
    const item = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(item.queryByRole("button", { name: /^Edit: Bob/ })).toBeNull();
    await user.click(item.getByRole("button", { name: /^Edit: Ada/ }));
    const field = item.getByRole("combobox", { name: "Your comment" });
    expect(field).toHaveValue("First draft");
    await user.clear(field);
    await user.type(field, "Second draft");
    await user.click(item.getByRole("button", { name: "Save" }));

    expect(await item.findByText("Second draft")).toBeInTheDocument();
    expect(item.getByText("(edited)")).toBeInTheDocument();
    await user.click(item.getByRole("button", { name: /^Delete: Ada/ }));
    const confirm = within(
      await screen.findByRole("alertdialog", { name: "Delete this comment?" }),
    );
    await user.click(confirm.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(item.queryByText("Second draft")).toBeNull());
    expect(item.getByText("Looks good")).toBeInTheDocument();
    expect(api.changes.map((change) => `${change.method} ${change.path}`)).toEqual([
      `PATCH /api/v1/comments/${COMMENT_IDS.first}`,
      `DELETE /api/v1/comments/${COMMENT_IDS.first}`,
    ]);
  });

  it("resolves a thread into the resolved ones, and reopens it", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    fakeComments({ threads: [pageThread(PAGE_IDS.roadmap, "Needs a date")] });
    const { user } = renderApp(PATH);

    const panel = await openPanel(user, "Comments, 1 open");
    const open = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    await user.click(open.getByRole("button", { name: "Resolve" }));

    expect(await panel.findByText("No comments on this page yet.")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Comments" })).toBeInTheDocument();
    await user.click(panel.getByRole("radio", { name: "Resolved" }));
    const resolved = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(resolved.getByText(/Resolved by Ada/)).toBeInTheDocument();
    await user.click(resolved.getByRole("button", { name: "Reopen" }));

    expect(await panel.findByText("No thread here is resolved.")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Comments, 1 open" })).toBeInTheDocument();
  });

  it("follows the page's comments, and stops", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const api = fakeComments();
    const { user } = renderApp(PATH);

    const panel = await openPanel(user);
    const follow = await panel.findByRole("button", { name: "Follow" });
    expect(follow).toHaveAttribute("aria-pressed", "false");
    await user.click(follow);
    await waitFor(() => expect(follow).toHaveAttribute("aria-pressed", "true"));
    await user.click(follow);
    await waitFor(() => expect(follow).toHaveAttribute("aria-pressed", "false"));

    expect(api.changes.map((change) => change.body)).toEqual([
      { following: true },
      { following: false },
    ]);
  });

  it("lets someone who may only read the comments read them", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    fakeComments({ threads: [pageThread(PAGE_IDS.roadmap, "For the record")], mayComment: false });
    const { user } = renderApp(PATH);

    const panel = await openPanel(user, "Comments, 1 open");
    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(thread.getByText("For the record")).toBeInTheDocument();
    expect(panel.getByText("You may read these comments, not add to them.")).toBeInTheDocument();
    expect(panel.queryByRole("combobox")).toBeNull();
    expect(thread.queryByRole("button", { name: "Reply" })).toBeNull();
  });

  it("reads the comments again when the page's connection says a thread changed", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    const api = fakeComments({ threads: [pageThread(PAGE_IDS.roadmap, "Kick-off at ten")] });
    const { user } = renderApp(PATH);
    const panel = await openPanel(user, "Comments, 1 open");
    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));

    // Bob replies from his own browser.
    const only = api.state.threads[0];
    only?.comments.push(
      comment({
        id: COMMENT_IDS.reply,
        thread: COMMENT_IDS.thread,
        markdown: "Ten is fine",
        author: people.bob,
      }),
    );
    sync.send(commentsMessage(COMMENT_IDS.thread));

    expect(await thread.findByText("Ten is fine")).toBeInTheDocument();
  });

  it("opens at the thread a link names, the resolved ones shown for a resolved one", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    const resolved = pageThread(PAGE_IDS.roadmap, "Settled");
    resolved.resolved_at = "2026-11-25T09:00:00Z";
    resolved.resolved_by = people.bob.id;
    fakeComments({ threads: [resolved] });
    renderApp(`${PATH}?comment=${COMMENT_IDS.thread}`);

    const panel = within(await screen.findByRole("complementary", { name: "Comments" }));
    const thread = await panel.findByRole("article", { name: "Thread by Ada" });
    expect(thread).toHaveAttribute("data-active");
    expect(panel.getByRole("radio", { name: "Resolved" })).toBeChecked();
    expect(within(thread).getByText(/Resolved by Bob/)).toBeInTheDocument();
  });

  it("opens at the block a link names, the cursor in it", async () => {
    fakeWorkspaces();
    fakePages();
    pageWith(["Mentioned here"]);
    fakeComments();
    renderApp(`${PATH}?block=block-1`);
    const editor = await content();

    await waitFor(() => expect(editor).toHaveTextContent("Mentioned here"));
    await waitFor(() => expect(editor).toHaveFocus());
    expect(screen.queryByRole("complementary", { name: "Comments" })).toBeNull();
  });

  it("comments on the text selected, where the server can find it again, and highlights it", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = pageWith(["Kick-off on Monday"]);
    const api = fakeComments();
    const { user } = renderApp(PATH);
    const editor = await content();
    await waitFor(() => expect(editor).toHaveTextContent("Kick-off on Monday"));

    const panel = await openPanel(user);
    selectText(editor, "Monday");
    await user.click(panel.getByRole("button", { name: "Comment on the selected text" }));
    const draft = within(
      await panel.findByRole("region", { name: "A comment on the selected text" }),
    );
    expect(draft.getByText("Monday")).toBeInTheDocument();
    await user.type(
      draft.getByRole("combobox", { name: "A comment on the selected text" }),
      "Which Monday?",
    );
    await user.click(draft.getByRole("button", { name: "Comment" }));

    const thread = within(await panel.findByRole("article", { name: "Thread by Ada" }));
    expect(thread.getByText("Which Monday?")).toBeInTheDocument();
    const body = api.changes[0]?.body as {
      markdown: string;
      anchor: { start: string; end: string; quote: string };
    };
    expect(body.anchor.quote).toBe("Monday");
    expect(landsAt(sync.doc, body.anchor.start)).toEqual({ text: "Kick-off on Monday", index: 12 });
    expect(landsAt(sync.doc, body.anchor.end)).toEqual({ text: "Kick-off on Monday", index: 18 });
    await waitFor(() => {
      const highlight = editor.querySelector(`[data-thread="${MADE_THREAD}"]`);
      expect(highlight).toHaveTextContent("Monday");
      expect(highlight).toHaveClass("comment-highlight", "is-active");
    });
  });

  it("starts a thread on the text selected with Mod-Alt-M", async () => {
    fakeWorkspaces();
    fakePages();
    pageWith(["Kick-off on Monday"]);
    fakeComments();
    renderApp(PATH);
    const editor = await content();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));
    await waitFor(() => expect(editor).toHaveTextContent("Kick-off on Monday"));

    selectText(editor, "Kick-off");
    fireEvent.keyDown(editor, { key: "m", code: "KeyM", ctrlKey: true, altKey: true });

    const panel = within(await screen.findByRole("complementary", { name: "Comments" }));
    const draft = within(panel.getByRole("region", { name: "A comment on the selected text" }));
    expect(draft.getByText("Kick-off")).toBeInTheDocument();
  });
});

describe("mentions in the editor", () => {
  beforeAll(async () => {
    await import("@/components/editor/page-editor");
  });

  it("draws someone mentioned by the name they have now, and a page as a link to it", async () => {
    fakeWorkspaces();
    fakePages();
    const user = new Y.XmlElement("mention");
    user.setAttribute("kind", "user");
    user.setAttribute("target", people.bob.id);
    user.setAttribute("label", "Robert");
    const linked = new Y.XmlElement("mention");
    linked.setAttribute("kind", "page");
    linked.setAttribute("target", PAGE_IDS.launch);
    linked.setAttribute("label", "Launch");
    pageWith(["Ask ", user, " about ", linked]);
    fakeComments();
    renderApp(PATH);
    const editor = await content();

    await waitFor(() => expect(editor).toHaveTextContent("Ask @Bob about @Launch"));
    expect(within(editor).getByRole("link", { name: "@Launch" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.launch}`,
    );
  });
});
