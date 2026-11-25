import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { Awareness, encodeAwarenessUpdate } from "y-protocols/awareness";
import * as Y from "yjs";
import { beforeAll, describe, expect, it } from "vitest";
import { databasePages, DB, fakeDatabase, fakeFeed } from "@/test/databases";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS, samplePages } from "@/test/pages";
import { server, syncLink } from "@/test/server";
import { authMessage, awarenessMessage, fakeSync } from "@/test/sync";
import { fakeWorkspaces, people, TEAMSPACE_IDS } from "@/test/workspaces";
import { afterClose, FRAGMENT, syncServer } from "./connection";
import { collaborator } from "./cursor";

const PATH = `/w/design/p/${PAGE_IDS.roadmap}`;
/** A database no database is: deleted, or never shared with the user. */
const NO_DATABASE = "0193a5b2-0000-7000-8000-0000000000d9";

/** The connection's state, as the page says it. */
async function indicator(text: string, timeout = 1000) {
  return screen.findByText(text, { selector: "output" }, { timeout });
}

/** The page's content, once the editor is up: the one field in it without a name of its own. */
async function content() {
  const region = await screen.findByRole("region", { name: "Page content" });
  return within(region).findByRole("textbox", { name: "" });
}

describe("the live editor", () => {
  // The page loads the editor on demand; loaded once here, so no test waits on it.
  beforeAll(async () => {
    await import("./page-editor");
  });

  it("connects to the page's document on this origin, and edits once it is in sync", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    renderApp(PATH);

    expect(await indicator("Connected")).toBeInTheDocument();
    expect(sync.urls).toEqual([`ws://localhost:3000/ws/pages/${PAGE_IDS.roadmap}`]);
    await waitFor(async () => expect(await content()).toHaveAttribute("contenteditable", "true"));
  });

  it("takes no input before the document came from the server", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync({ answers: false });
    renderApp(PATH);

    expect(await indicator("Connecting")).toBeInTheDocument();
    expect(await content()).toHaveAttribute("contenteditable", "false");
  });

  it("shows what the server holds, and what others change as they change it", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    blocks.insert(0, [paragraph("block-1", "Kick-off on Monday")]);
    renderApp(PATH);

    await waitFor(async () => expect(await content()).toHaveTextContent("Kick-off on Monday"));
    blocks.insert(1, [paragraph("block-2", "Retro on Friday")]);
    await waitFor(async () => expect(await content()).toHaveTextContent("Retro on Friday"));
  });

  it("shows a database block's database in the page, in its views", async () => {
    fakeWorkspaces();
    fakePages();
    fakeDatabase();
    fakeFeed();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    blocks.insert(0, [database("block-1", DB.tasks), database("block-2", NO_DATABASE)]);
    renderApp(PATH);

    const region = within(await screen.findByRole("region", { name: "Page content" }));
    const embedded = within(await region.findByRole("region", { name: "Database Tasks" }));
    expect(await embedded.findByRole("tab", { name: "Table", selected: true })).toBeInTheDocument();
    expect(await embedded.findByRole("rowheader", { name: /Write docs/ })).toBeInTheDocument();
    expect(region.getByRole("textbox", { name: "Database title" })).toHaveValue("Tasks");
    // A database the user may not see, or that is gone, says so where it was.
    expect(await region.findByText("This database is not available here.")).toBeInTheDocument();
  });

  it("keeps the view picked in an embedded database in its block", async () => {
    fakeWorkspaces();
    fakePages();
    fakeDatabase();
    fakeFeed();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    blocks.insert(0, [database("block-1", DB.tasks)]);
    const { user } = renderApp(PATH);

    const embedded = within(await screen.findByRole("region", { name: "Database Tasks" }));
    await waitFor(async () => expect(await content()).toHaveAttribute("contenteditable", "true"));
    await user.click(await embedded.findByRole("tab", { name: "Board" }));

    expect(await embedded.findByRole("region", { name: "In progress" })).toBeInTheDocument();
    await waitFor(() => expect(blockAttribute(blocks, 0, "viewId")).toBe(DB.board));
  });

  it("names an embedded database as its title is typed, in the sidebar too, and saves it", async () => {
    fakeWorkspaces();
    const pages = fakePages({ pages: databasePages(samplePages()) });
    fakeDatabase();
    fakeFeed();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    blocks.insert(0, [database("block-1", DB.tasks)]);
    const { user } = renderApp(PATH);

    const sidebar = within(await screen.findByRole("complementary"));
    expect(await sidebar.findByRole("link", { name: "Tasks" })).toBeInTheDocument();
    const title = await screen.findByRole("textbox", { name: "Database title" });
    await user.clear(title);
    await user.type(title, "Sprint tasks");

    expect(sidebar.getByRole("link", { name: "Sprint tasks" })).toBeInTheDocument();
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(pages.changes.at(-1)).toEqual({
        method: "PATCH",
        path: `/api/v1/pages/${DB.tasks}`,
        body: { title: "Sprint tasks" },
      }),
    );
    expect(sidebar.getByRole("link", { name: "Sprint tasks" })).toBeInTheDocument();
  });

  it("makes a database for a block that names none, inside the page", async () => {
    fakeWorkspaces();
    fakePages();
    const db = fakeDatabase();
    fakeFeed();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    blocks.insert(0, [database("block-1", "")]);
    const { user } = renderApp(PATH);

    const region = within(await screen.findByRole("region", { name: "Page content" }));
    expect(await region.findByText("No database chosen yet.")).toBeInTheDocument();
    await waitFor(async () => expect(await content()).toHaveAttribute("contenteditable", "true"));
    await user.click(region.getByRole("button", { name: "Make a database here" }));

    expect(await region.findByRole("region", { name: "Database Untitled" })).toBeInTheDocument();
    expect(db.changes).toEqual([
      {
        method: "POST",
        path: "/api/v1/workspaces/0193a5b2-0000-7000-8000-00000000a001/databases",
        body: { parent_id: PAGE_IDS.roadmap, inline: true },
      },
    ]);
    await waitFor(() =>
      expect(blockAttribute(blocks, 0, "databaseId")).toBe(db.created[0]?.page.id),
    );
  });

  it("says when the connection dropped, and when it is back", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    renderApp(PATH);
    await indicator("Connected");

    // A close the server means as passing: tried again at once.
    sync.close(4500, "restarting");
    expect(await indicator("Reconnecting")).toBeInTheDocument();
    expect(await indicator("Connected")).toBeInTheDocument();
  });

  it("tries again after a pause when the server says the client fell behind, keeping what was typed", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    const { user } = renderApp(PATH);
    await indicator("Connected");
    const editor = await content();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));

    sync.close(4408, "the connection fell too far behind");
    expect(await indicator("Reconnecting")).toBeInTheDocument();
    // Still editable: what is typed now is kept, and sent once the connection is back.
    expect(editor).toHaveAttribute("contenteditable", "true");
    await user.click(editor);
    await user.keyboard("Typed meanwhile");
    expect(sync.urls).toHaveLength(1);

    expect(await indicator("Connected", 5000)).toBeInTheDocument();
    expect(sync.urls).toHaveLength(2);
    await waitFor(() =>
      expect(sync.doc.getXmlFragment(FRAGMENT).toString()).toContain("Typed meanwhile"),
    );
  });

  it("takes no input once the server ended the connection for good", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    renderApp(PATH);
    await indicator("Connected");
    const editor = await content();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));

    sync.close(4401, "your session or token ended: sign in again");

    expect(await indicator("Disconnected")).toBeInTheDocument();
    expect(await screen.findByText(/Your session ended/)).toBeInTheDocument();
    expect(await content()).toHaveAttribute("contenteditable", "false");
  });

  it("does not send a change the server refused as too large again", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    const { user } = renderApp(PATH);
    await indicator("Connected");

    sync.close(1009, "the message is too large");

    expect(await indicator("Disconnected")).toBeInTheDocument();
    expect(await screen.findByText(/too large for the server to save/)).toBeInTheDocument();
    expect(await content()).toHaveAttribute("contenteditable", "false");
    await new Promise((resolve) => setTimeout(resolve, 600));
    expect(sync.urls).toHaveLength(1);

    // Reconnecting starts again from what the server holds.
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(await indicator("Connected")).toBeInTheDocument();
    expect(sync.urls).toHaveLength(2);
  });

  it("refuses a paste larger than the server takes, and says so", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    renderApp(PATH);
    await indicator("Connected");
    const editor = await content();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));
    const huge = "A line of pasted text.\n".repeat(200_000);

    fireEvent.paste(editor, {
      clipboardData: {
        types: ["text/plain"],
        getData: (type: string) => (type === "text/plain" ? huge : ""),
      },
    });

    expect(await screen.findByText(/too much to paste at once/)).toBeInTheDocument();
    expect(sync.doc.getXmlFragment(FRAGMENT).toString()).not.toContain("A line of pasted text");
  });

  it("turns read-only, and back, as the server says access changed, reading the page again", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    let reads = 0;
    server.use(
      http.get(`/api/v1/pages/${PAGE_IDS.roadmap}`, () => {
        reads += 1;
      }),
    );
    renderApp(PATH);
    await indicator("Connected");
    const editor = await content();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));
    const before = reads;

    sync.send(authMessage("read-only"));
    expect(await indicator("Read-only")).toBeInTheDocument();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "false"));
    await waitFor(() => expect(reads).toBeGreaterThan(before));

    sync.send(authMessage());
    expect(await indicator("Connected")).toBeInTheDocument();
    await waitFor(() => expect(editor).toHaveAttribute("contenteditable", "true"));
    // One connection all along: access changes in place.
    expect(sync.urls).toHaveLength(1);
  });

  it("stops for good when the server closes the page, reads it again, and reconnects on asking", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    let reads = 0;
    server.use(
      http.get(`/api/v1/pages/${PAGE_IDS.roadmap}`, () => {
        reads += 1;
      }),
    );
    const { user } = renderApp(PATH);
    await indicator("Connected");
    const before = reads;

    sync.close(4403, "access changed");
    expect(await indicator("Disconnected")).toBeInTheDocument();
    await waitFor(() => expect(reads).toBeGreaterThan(before));

    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(await indicator("Connected")).toBeInTheDocument();
    expect(sync.urls).toHaveLength(2);
  });

  it("is read-only for someone who only views the page", async () => {
    fakeWorkspaces({ role: "member" });
    fakePages({
      pages: [
        page(
          { id: PAGE_IDS.roadmap, title: "Roadmap", teamspace: TEAMSPACE_IDS.open, access: "view" },
          "a0",
        ),
      ],
    });
    fakeSync();
    renderApp(PATH);

    expect(await indicator("Read-only")).toBeInTheDocument();
    expect(await content()).toHaveAttribute("contenteditable", "false");
    expect(screen.queryByRole("textbox", { name: "Page title" })).toBeNull();
    expect(screen.getByRole("heading", { level: 1, name: "Roadmap" })).toBeInTheDocument();
  });

  it("names the others who have the page open", async () => {
    fakeWorkspaces();
    fakePages();
    fakeSync();
    // Bob, in another browser, as the server passes his presence on.
    const theirs = new Awareness(new Y.Doc());
    theirs.setLocalStateField("user", { id: people.bob.id, name: "Bob", hue: people.bob.hue });
    server.use(
      syncLink.addEventListener("connection", ({ client }) => {
        client.send(awarenessMessage(encodeAwarenessUpdate(theirs, [theirs.clientID])));
      }),
    );
    renderApp(PATH);

    expect(await screen.findByText("Also here: Bob")).toBeInTheDocument();
  });

  it("draws another's selection in a colour made from their hue, whatever their state says", async () => {
    fakeWorkspaces();
    fakePages();
    const sync = fakeSync();
    const blocks = new Y.XmlElement("blockGroup");
    sync.doc.getXmlFragment(FRAGMENT).insert(0, [blocks]);
    const line = paragraph("block-1", "Kick-off on Monday");
    blocks.insert(0, [line]);
    const inline = line.firstChild;
    const text = inline instanceof Y.XmlElement ? inline.firstChild : null;
    if (!(text instanceof Y.XmlText)) {
      throw new Error("no text");
    }
    // Bob's client, or one claiming to be his, with a colour that is not one.
    const theirs = new Awareness(new Y.Doc());
    theirs.setLocalState({
      user: {
        id: people.bob.id,
        name: "Bob",
        hue: people.bob.hue,
        color:
          "#000000; position: fixed; inset: 0; z-index: 2147483647; background-image: url(data:image/svg+xml,x); x:",
      },
      cursor: {
        anchor: Y.relativePositionToJSON(Y.createRelativePositionFromTypeIndex(text, 0)),
        head: Y.relativePositionToJSON(Y.createRelativePositionFromTypeIndex(text, 8)),
      },
    });
    server.use(
      syncLink.addEventListener("connection", ({ client }) => {
        client.send(awarenessMessage(encodeAwarenessUpdate(theirs, [theirs.clientID])));
      }),
    );
    renderApp(PATH);
    // Bob's caret, with his name, sits after "Kick-off".
    await waitFor(async () => expect(await content()).toHaveTextContent(/Kick-off.*on Monday/));

    const region = await screen.findByRole("region", { name: "Page content" });
    const selection = await waitFor(() => {
      const found = region.querySelector(".ProseMirror-yjs-selection");
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    expect(selection.getAttribute("style")).not.toMatch(/position|inset|z-index|image/);
    const tint = collaborator(people.bob.id, "Bob", people.bob.hue).color;
    const probe = document.createElement("span");
    probe.style.backgroundColor = `${tint}70`;
    expect(selection.style.backgroundColor).toBe(probe.style.backgroundColor);
  });
});

describe("what follows a close from the server", () => {
  it("tries again at once, later, or not, by its code", () => {
    expect(afterClose(1006)).toBe("again");
    expect(afterClose(1012)).toBe("again");
    expect(afterClose(1013)).toBe("again");
    expect(afterClose(4422)).toBe("again");
    expect(afterClose(4500)).toBe("again");
    expect(afterClose(4408)).toBe("later");
    expect(afterClose(4429)).toBe("later");
    expect(afterClose(4401)).toBe("signedOut");
    expect(afterClose(4403)).toBe("readOnly");
    expect(afterClose(4404)).toBe("accessLost");
    expect(afterClose(1009)).toBe("tooLarge");
    expect(afterClose(4413)).toBe("tooLarge");
    expect(afterClose(1007)).toBe("failed");
    expect(afterClose(4400)).toBe("failed");
  });
});

describe("the address of the live connection", () => {
  it("follows the page's scheme and host", () => {
    expect(syncServer({ protocol: "https:", host: "neon.example.com" })).toBe(
      "wss://neon.example.com/ws/pages",
    );
    expect(syncServer({ protocol: "http:", host: "127.0.0.1:3000" })).toBe(
      "ws://127.0.0.1:3000/ws/pages",
    );
  });
});

/** A paragraph block, in the structure BlockNote keeps blocks in. */
function paragraph(id: string, text: string): Y.XmlElement {
  const inline = new Y.XmlElement("paragraph");
  inline.insert(0, [new Y.XmlText(text)]);
  const container = new Y.XmlElement("blockContainer");
  container.setAttribute("id", id);
  container.insert(0, [inline]);
  return container;
}

/** An attribute of a block's content, as the server's copy of the document holds it. */
function blockAttribute(blocks: Y.XmlElement, index: number, name: string): unknown {
  const container = blocks.get(index);
  const block = container instanceof Y.XmlElement ? container.get(0) : undefined;
  return block instanceof Y.XmlElement ? block.getAttribute(name) : undefined;
}

/** A database block, in the structure BlockNote keeps blocks in. */
function database(id: string, databaseId: string): Y.XmlElement {
  const element = new Y.XmlElement("database");
  element.setAttribute("databaseId", databaseId);
  const container = new Y.XmlElement("blockContainer");
  container.setAttribute("id", id);
  container.insert(0, [element]);
  return container;
}
