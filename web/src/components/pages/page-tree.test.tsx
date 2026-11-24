import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { ada, apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS } from "@/test/pages";
import { server } from "@/test/server";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

/** The sidebar's column, once the signed-in shell is up. */
async function sidebar() {
  return within(await screen.findByRole("complementary"));
}

/** The titles of a tree's pages, in the order they show, without their icons. */
function titles(list: HTMLElement) {
  return within(list)
    .getAllByRole("link")
    .map((link) => link.lastElementChild?.textContent);
}

/** The row of a page in the sidebar: the line its link, buttons and menu sit on. */
async function row(title: string) {
  const link = await (await sidebar()).findByRole("link", { name: title });
  const line = link.closest("[data-row]");
  if (!(line instanceof HTMLElement)) {
    throw new Error(`no row for ${title}`);
  }
  return line;
}

describe("the page trees in the sidebar", () => {
  it("shows each teamspace's pages, the pages under one once it is opened", async () => {
    fakeWorkspaces();
    fakePages();
    const { user } = renderApp("/w/design");
    const side = await sidebar();

    const general = await side.findByRole("list", { name: "Pages of General" });
    expect(titles(general)).toEqual(["Roadmap", "Launch"]);
    expect(side.queryByRole("link", { name: "Q1" })).toBeNull();
    expect(side.queryByRole("link", { name: "Old plans" })).toBeNull();

    const expand = side.getByRole("button", { name: "Expand Roadmap" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    await user.click(expand);
    expect(side.getByRole("link", { name: "Q1" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.q1}`,
    );
    await user.click(side.getByRole("button", { name: "Collapse Roadmap" }));
    expect(side.queryByRole("link", { name: "Q1" })).toBeNull();

    // A teamspace folds its pages away too.
    await user.click(side.getByRole("button", { name: "Collapse General" }));
    expect(side.queryByRole("list", { name: "Pages of General" })).toBeNull();
  });

  it("lists the user's private pages, and the pages shared with them", async () => {
    fakeWorkspaces();
    fakePages({
      shared: [
        {
          id: PAGE_IDS.goals,
          kind: "page",
          title: "Goals",
          teamspace_id: TEAMSPACE_IDS.private,
          child_count: 0,
          access: "view",
        },
      ],
    });
    renderApp("/w/design");
    const side = await sidebar();

    expect(titles(await side.findByRole("list", { name: "Private" }))).toEqual(["Notes"]);
    expect(titles(await side.findByRole("list", { name: "Shared with you" }))).toEqual(["Goals"]);
  });

  it("gives a guest only the pages shared with them", async () => {
    fakeWorkspaces({ role: "guest" });
    fakePages({
      shared: [
        { id: PAGE_IDS.q1, kind: "page", title: "Q1", child_count: 1, access: "view" },
        {
          id: PAGE_IDS.goals,
          kind: "page",
          title: "Goals",
          parent_id: PAGE_IDS.q1,
          child_count: 0,
          access: "view",
        },
      ],
    });
    const { user } = renderApp("/w/design");
    const side = await sidebar();

    const shared = await side.findByRole("list", { name: "Shared with you" });
    expect(titles(shared)).toEqual(["Q1"]);
    expect(side.queryByText("Private")).toBeNull();
    expect(side.queryByRole("button", { name: "Add a private page" })).toBeNull();
    expect(side.queryByRole("button", { name: "Add a page inside Q1" })).toBeNull();

    await user.click(side.getByRole("button", { name: "Expand Q1" }));
    expect(titles(shared)).toEqual(["Q1", "Goals"]);
    await user.click(side.getByRole("button", { name: "More for Q1" }));
    const menu = within(await screen.findByRole("menu"));
    expect(menu.getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      "Add to favorites",
      "Export as markdown",
      "Export with subpages",
    ]);
  });

  it("makes a page at the top of a teamspace and opens it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user, router } = renderApp("/w/design");
    const side = await sidebar();

    await user.click(await side.findByRole("button", { name: "Add a page to General" }));

    await waitFor(() =>
      expect(router.state.location.pathname).toBe(
        "/w/design/p/0193a5b2-0000-7000-8000-0000000000b1",
      ),
    );
    expect(api.changes).toContainEqual({
      method: "POST",
      path: "/api/v1/workspaces/0193a5b2-0000-7000-8000-00000000a001/pages",
      body: { teamspace_id: TEAMSPACE_IDS.general },
    });
    const general = await side.findByRole("list", { name: "Pages of General" });
    await waitFor(() => expect(titles(general)).toEqual(["Roadmap", "Launch", "Untitled"]));
  });

  it("makes a page inside another, which opens to show it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(
      within(await row("Launch")).getByRole("button", { name: "Add a page inside Launch" }),
    );

    expect(await (await sidebar()).findByRole("button", { name: "Collapse Launch" })).toBeVisible();
    expect(api.changes.at(-1)?.body).toEqual({ parent_id: PAGE_IDS.launch });
  });

  it("renames a page in place from its menu", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const field = (await sidebar()).getByRole("textbox", { name: "New title for Launch" });
    expect(field).toHaveFocus();
    await user.clear(field);
    await user.type(field, "Go-live{Enter}");

    expect(await (await sidebar()).findByRole("link", { name: "Go-live" })).toHaveFocus();
    expect(api.changes).toContainEqual({
      method: "PATCH",
      path: `/api/v1/pages/${PAGE_IDS.launch}`,
      body: { title: "Go-live" },
    });
  });

  it("puts the title back, and says why, when a rename is refused", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.patch("/api/v1/pages/:id", () =>
        apiError(400, "invalid_input", "the title is too long"),
      ),
    );
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const field = (await sidebar()).getByRole("textbox", { name: "New title for Launch" });
    await user.clear(field);
    await user.type(field, "Liftoff{Enter}");

    const side = await sidebar();
    expect(await side.findByText("The title is too long.")).toBeInTheDocument();
    expect(await side.findByRole("link", { name: "Launch" })).toBeInTheDocument();
    expect(side.queryByRole("link", { name: "Liftoff" })).toBeNull();
  });

  it("leaves the title as it was on Escape", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    await user.type((await sidebar()).getByRole("textbox"), " soon{Escape}");

    expect(await (await sidebar()).findByRole("link", { name: "Launch" })).toBeInTheDocument();
    expect(api.changes.filter((change) => change.method === "PATCH")).toEqual([]);
  });

  it("moves a page up among its siblings from the menu, the keyboard's way to reorder", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    const menu = within(await screen.findByRole("menu"));
    expect(menu.queryByRole("menuitem", { name: "Move down" })).toBeNull();
    await user.click(menu.getByRole("menuitem", { name: "Move up" }));

    const general = (await sidebar()).getByRole("list", { name: "Pages of General" });
    await waitFor(() => expect(titles(general)).toEqual(["Launch", "Roadmap"]));
    expect(api.changes.at(-1)?.body).toEqual({
      teamspace_id: TEAMSPACE_IDS.general,
      placement: { before: PAGE_IDS.roadmap },
    });
  });

  it("moves a page anywhere the user edits through Move to", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Move Launch" }));
    const places = dialog.getByRole("list", { name: "Places" });
    // Not into itself, nor where it is already.
    expect(within(places).queryByRole("button", { name: /^Launch/ })).toBeNull();
    expect(within(places).queryByRole("button", { name: /^General/ })).toBeNull();
    await user.type(dialog.getByLabelText("Find a place"), "not");
    await user.click(within(places).getByRole("button", { name: "Notes, In Private pages" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.changes.at(-1)).toEqual({
      method: "POST",
      path: `/api/v1/pages/${PAGE_IDS.launch}/move`,
      body: { parent_id: PAGE_IDS.notes, placement: "last" },
    });
    expect(await (await sidebar()).findByText("Moved Launch.")).toBeInTheDocument();
  });

  it("says why a move was refused", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(http.post("/api/v1/pages/:id/move", () => apiError(409, "too_deep", "too deep")));
    const { user } = renderApp("/w/design");

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Move Launch" }));
    await user.click(dialog.getByRole("button", { name: /^Roadmap/ }));

    expect(await dialog.findByText(/more than 100 levels deep/)).toBeInTheDocument();
  });

  it("moves a dragged page into the page it is dropped on", async () => {
    fakeWorkspaces();
    const api = fakePages();
    renderApp("/w/design");
    const launch = await row("Launch");
    const roadmap = await row("Roadmap");

    fireEvent.dragStart(launch, { dataTransfer: new FakeDataTransfer() });
    fireEvent.dragOver(roadmap, { dataTransfer: new FakeDataTransfer() });
    expect(roadmap).toHaveAttribute("data-drop", "inside");
    fireEvent.drop(roadmap, { dataTransfer: new FakeDataTransfer() });

    await waitFor(() =>
      expect(api.changes.at(-1)).toEqual({
        method: "POST",
        path: `/api/v1/pages/${PAGE_IDS.launch}/move`,
        body: { parent_id: PAGE_IDS.roadmap, placement: "last" },
      }),
    );
    expect(
      await (await sidebar()).findByRole("button", { name: "Collapse Roadmap" }),
    ).toBeVisible();
  });

  it("takes a page dropped on a section's heading to the top of it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");
    await user.click(await (await sidebar()).findByRole("button", { name: "Expand Roadmap" }));
    const q1 = await row("Q1");

    fireEvent.dragStart(q1, { dataTransfer: new FakeDataTransfer() });
    const heading = (await sidebar()).getByText("Private").closest("[data-row]");
    if (!(heading instanceof HTMLElement)) {
      throw new Error("no heading");
    }
    fireEvent.dragOver(heading, { dataTransfer: new FakeDataTransfer() });
    fireEvent.drop(heading, { dataTransfer: new FakeDataTransfer() });

    await waitFor(() =>
      expect(api.changes.at(-1)?.body).toEqual({ private: true, placement: "last" }),
    );
    expect(titles((await sidebar()).getByRole("list", { name: "Private" }))).toContain("Q1");
  });

  it("never drops a page under itself", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");
    await user.click(await (await sidebar()).findByRole("button", { name: "Expand Roadmap" }));
    const roadmap = await row("Roadmap");
    const q1 = await row("Q1");

    fireEvent.dragStart(roadmap, { dataTransfer: new FakeDataTransfer() });
    fireEvent.dragOver(q1, { dataTransfer: new FakeDataTransfer() });
    fireEvent.drop(q1, { dataTransfer: new FakeDataTransfer() });

    expect(q1).not.toHaveAttribute("data-drop");
    expect(api.changes).toEqual([]);
  });

  it("favorites a page from its menu, and lists it under Favorites", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");
    const side = await sidebar();
    expect(side.queryByRole("list", { name: "Favorites" })).toBeNull();

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Add to favorites" }));

    const favorites = await side.findByRole("list", { name: "Favorites" });
    expect(titles(favorites)).toEqual(["Launch"]);
    expect(api.state.favorites).toEqual([PAGE_IDS.launch]);
  });

  it("moves a page to the trash, and brings it back on Undo", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design");
    const side = await sidebar();

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to the trash" }));

    expect(await side.findByText("Launch is in the trash.")).toBeInTheDocument();
    await waitFor(() => expect(side.queryByRole("link", { name: "Launch" })).toBeNull());
    await user.click(side.getByRole("button", { name: "Undo" }));
    expect(await side.findByRole("link", { name: "Launch" })).toBeInTheDocument();
    // The row the page was trashed from is gone; the notice still hears the answer.
    await waitFor(() => expect(side.queryByText("Launch is in the trash.")).toBeNull());
    expect(api.changes.map((change) => change.path)).toEqual([
      `/api/v1/pages/${PAGE_IDS.launch}/trash`,
      `/api/v1/pages/${PAGE_IDS.launch}/restore`,
    ]);
  });

  it("says why an Undo of a trash failed", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.post("/api/v1/pages/:id/restore", () =>
        apiError(409, "conflict", "the page above it is in the trash"),
      ),
    );
    const { user } = renderApp("/w/design");
    const side = await sidebar();

    await user.click(within(await row("Launch")).getByRole("button", { name: "More for Launch" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to the trash" }));
    await waitFor(() => expect(side.queryByRole("link", { name: "Launch" })).toBeNull());
    await user.click(await side.findByRole("button", { name: "Undo" }));

    expect(await side.findByText("The page above it is in the trash.")).toBeInTheDocument();
    expect(side.queryByText("Launch is in the trash.")).toBeNull();
  });

  it("offers a viewer no way to change a page, only to mark and export it", async () => {
    fakeWorkspaces({ role: "member" });
    fakePages({
      pages: [
        page(
          {
            id: PAGE_IDS.roadmap,
            title: "Roadmap",
            teamspace: TEAMSPACE_IDS.general,
            access: "view",
          },
          "a0",
        ),
        page({ id: PAGE_IDS.notes, title: "Notes", owner: ada.id }, "a0"),
      ],
    });
    const { user } = renderApp("/w/design");
    const roadmap = await row("Roadmap");

    expect(roadmap).toHaveAttribute("draggable", "false");
    expect(within(roadmap).queryByRole("button", { name: "Add a page inside Roadmap" })).toBeNull();
    await user.click(within(roadmap).getByRole("button", { name: "More for Roadmap" }));
    const menu = within(await screen.findByRole("menu"));
    expect(menu.getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      "Add to favorites",
      "Export as markdown",
      "Export with subpages",
    ]);
  });
});

/** Enough of DataTransfer for a drag inside the page, which jsdom does not have. */
class FakeDataTransfer {
  dropEffect = "none";
  effectAllowed = "all";
  readonly data = new Map<string, string>();
  setData(type: string, value: string) {
    this.data.set(type, value);
  }
  getData(type: string) {
    return this.data.get(type) ?? "";
  }
}
