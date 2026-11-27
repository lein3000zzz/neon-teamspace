import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { attachmentName } from "@/api/pages";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { fakePages, PAGE_IDS } from "@/test/pages";
import { server } from "@/test/server";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

/** The sidebar's column, once the signed-in shell is up. */
async function sidebar() {
  return within(await screen.findByRole("complementary"));
}

/** The row of a page in the sidebar. */
async function row(title: string) {
  const link = await (await sidebar()).findByRole("link", { name: title });
  const line = link.closest("[data-row]");
  if (!(line instanceof HTMLElement)) {
    throw new Error(`no row for ${title}`);
  }
  return within(line);
}

/** The files the app saved as downloads, with their names and content. */
const saved: { name: string; file: Blob }[] = [];

beforeEach(() => {
  saved.length = 0;
  // jsdom neither makes addresses for files nor follows links: the download is caught here.
  const files = new Map<string, Blob>();
  vi.spyOn(URL, "createObjectURL").mockImplementation((file) => {
    const address = `blob:test/${files.size}`;
    files.set(address, file as Blob);
    return address;
  });
  vi.spyOn(URL, "revokeObjectURL").mockImplementation((address) => {
    files.delete(address);
  });
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function click(
    this: HTMLAnchorElement,
  ) {
    const file = files.get(this.getAttribute("href") ?? "");
    if (file !== undefined) {
      saved.push({ name: this.download, file });
    }
  });
});

describe("importing markdown", () => {
  it("imports a file into a page from its menu, and opens what it made", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click((await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Import into this page…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Import into Launch" }));
    await user.upload(
      dialog.getByLabelText("File"),
      new File(["# Checklist\n\n- [ ] Book the room\n"], "Checklist.md", {
        type: "text/markdown",
      }),
    );
    await user.click(dialog.getByRole("button", { name: "Import" }));

    expect(await dialog.findByText("Imported 1 page.")).toBeInTheDocument();
    expect(api.imports).toEqual([
      {
        query: { parent_id: PAGE_IDS.launch },
        text: "# Checklist\n\n- [ ] Book the room\n",
      },
    ]);
    await user.click(dialog.getByRole("link", { name: "Open Checklist" }));
    expect(await screen.findByRole("textbox", { name: "Page title" })).toHaveValue("Checklist");
    expect(screen.queryByRole("dialog")).toBeNull();
    // The tree was read again: the new page is under Launch, which opens to show it.
    expect(
      await (await row("Checklist")).findByRole("button", { name: "More for Checklist" }),
    ).toBeVisible();
    expect((await row("Launch")).getByRole("button", { name: "Collapse Launch" })).toBeVisible();
  });

  it("imports a zip at the top of a teamspace, and says what it left out", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);

    await user.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Import into General" }));
    await user.upload(
      dialog.getByLabelText("File"),
      new File(["PK"], "Notes.zip", { type: "application/zip" }),
    );
    await user.click(dialog.getByRole("button", { name: "Import" }));

    expect(await dialog.findByText("1 file of the zip was left out:")).toBeInTheDocument();
    expect(dialog.getByText("Roadmap/logo.png")).toBeInTheDocument();
    expect(dialog.getByText(/not markdown, and no page shows it/)).toBeInTheDocument();
    expect(api.imports.map((entry) => entry.query)).toEqual([
      { teamspace_id: TEAMSPACE_IDS.general },
    ]);
    await user.click(dialog.getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("imports into the user's private pages from the workspace's home", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(await screen.findByRole("button", { name: "Import markdown" }));
    const dialog = within(
      await screen.findByRole("dialog", { name: "Import into your private pages" }),
    );
    await user.upload(
      dialog.getByLabelText("File"),
      new File(["# Diary\n\nHello"], "Diary.markdown", { type: "text/markdown" }),
    );
    await user.click(dialog.getByRole("button", { name: "Import" }));

    expect(await dialog.findByText("Imported 1 page.")).toBeInTheDocument();
    expect(api.imports.map((entry) => entry.query)).toEqual([{ private: "true" }]);
    await user.click(dialog.getByRole("button", { name: "Done" }));
    const side = await sidebar();
    expect(
      await within(side.getByRole("list", { name: "Private" })).findByRole("link", {
        name: "Diary",
      }),
    ).toBeInTheDocument();
  });

  it("checks the file's kind and size before sending it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);
    await user.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Import into General" }));
    const field = dialog.getByLabelText("File");

    await user.click(dialog.getByRole("button", { name: "Import" }));
    expect(await dialog.findByText("Choose a file to import.")).toBeInTheDocument();

    // The field's accept list only suggests: a file of another kind can still be picked.
    await userEvent
      .setup({ applyAccept: false })
      .upload(field, new File(["x"], "logo.png", { type: "image/png" }));
    await user.click(dialog.getByRole("button", { name: "Import" }));
    expect(await dialog.findByText("Choose a .md, .markdown or .zip file.")).toBeInTheDocument();

    const huge = new File(["x"], "Everything.zip");
    Object.defineProperty(huge, "size", { value: 33 * 1024 * 1024 });
    await user.upload(field, huge);
    await user.click(dialog.getByRole("button", { name: "Import" }));
    expect(
      await dialog.findByText("That file is 33 MiB: an import takes up to 32 MiB."),
    ).toBeInTheDocument();
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(api.imports).toEqual([]);
  });

  it("says why the server refused an import", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.post("/api/v1/workspaces/:id/pages/import", () =>
        apiError(400, "invalid_input", "the zip holds no markdown file"),
      ),
    );
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);
    await user.click(await screen.findByRole("button", { name: "Import" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Import into General" }));

    await user.upload(dialog.getByLabelText("File"), new File(["PK"], "Empty.zip"));
    await user.click(dialog.getByRole("button", { name: "Import" }));

    expect(await dialog.findByText("The zip holds no markdown file.")).toBeInTheDocument();
  });

  it("offers importing only where the user may add pages", async () => {
    fakeWorkspaces({ role: "member" });
    fakePages();
    renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.open}`);

    // Not in the open teamspace: its pages export, but nothing imports there.
    expect(await screen.findByRole("heading", { name: "Pages" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Import" })).toBeNull();
  });
});

describe("exporting markdown", () => {
  it("saves a page as markdown, or with its subpages as a zip, under the server's names", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click((await row("Roadmap")).getByRole("button", { name: "More for Roadmap" }));
    await user.click(await screen.findByRole("menuitem", { name: "Export as markdown" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]?.name).toBe("Roadmap.md");
    expect(await saved[0]?.file.text()).toBe("# Roadmap\n");

    await user.click((await row("Roadmap")).getByRole("button", { name: "More for Roadmap" }));
    await user.click(await screen.findByRole("menuitem", { name: "Export with subpages" }));
    await waitFor(() => expect(saved).toHaveLength(2));
    expect(saved[1]?.name).toBe("Roadmap.zip");
    expect(api.exports).toEqual([
      `/api/v1/pages/${PAGE_IDS.roadmap}/export`,
      `/api/v1/pages/${PAGE_IDS.roadmap}/export?children=true`,
    ]);
    // The notice that it is on its way goes once it is saved.
    await waitFor(() => expect(screen.queryByText("Exporting Roadmap…")).toBeNull());
  });

  it("says why an export failed", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.get("/api/v1/pages/:id/export", () =>
        apiError(400, "invalid_input", "the page holds nothing to export"),
      ),
    );
    const { user } = renderApp("/w/design");

    await user.click((await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Export as markdown" }));

    expect(
      await (await sidebar()).findByText("The page holds nothing to export."),
    ).toBeInTheDocument();
    expect(saved).toEqual([]);
  });

  it("saves a whole teamspace as a zip from its page", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);

    await user.click(await screen.findByRole("button", { name: "Export" }));

    await waitFor(() => expect(saved.map((file) => file.name)).toEqual(["General.zip"]));
    expect(api.exports).toEqual([`/api/v1/teamspaces/${TEAMSPACE_IDS.general}/export`]);
  });
});

describe("the name of an exported file", () => {
  it("is the full name RFC 6266 gives, or else the plain one", () => {
    expect(
      attachmentName(
        "attachment; filename=\"Q1 Planung f_r 2027.zip\"; filename*=UTF-8''Q1%20Planung%20f%C3%BCr%202027.zip",
      ),
    ).toBe("Q1 Planung für 2027.zip");
    expect(attachmentName('attachment; filename="Roadmap.md"')).toBe("Roadmap.md");
    expect(attachmentName("attachment; filename*=UTF-8''%E0%A4%A; filename=\"Odd.md\"")).toBe(
      "Odd.md",
    );
    expect(attachmentName("attachment")).toBeUndefined();
    expect(attachmentName(null)).toBeUndefined();
  });
});
