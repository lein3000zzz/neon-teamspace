import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { Awareness, encodeAwarenessUpdate } from "y-protocols/awareness";
import * as Y from "yjs";
import { beforeAll, describe, expect, it } from "vitest";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS } from "@/test/pages";
import { server, syncLink } from "@/test/server";
import { awarenessMessage, fakeSync } from "@/test/sync";
import { fakeWorkspaces, people, TEAMSPACE_IDS } from "@/test/workspaces";
import { FRAGMENT, syncServer } from "./connection";

const PATH = `/w/design/p/${PAGE_IDS.roadmap}`;

/** The connection's state, as the page says it. */
async function indicator(text: string) {
  return screen.findByText(text, { selector: "output" });
}

/** The page's content, once the editor is up. */
async function content() {
  const region = await screen.findByRole("region", { name: "Page content" });
  return within(region).findByRole("textbox");
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
