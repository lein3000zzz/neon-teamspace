import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { databasePages, fakeDatabase, fakeFeed } from "@/test/databases";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

/** A path of the app, with Design's pages, the database Tasks among them, and its rows. */
function setUp(path: string) {
  fakeWorkspaces();
  fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase();
  fakeFeed();
  return { ...renderApp(path), db };
}

describe("databases in the sidebar", () => {
  it("show with a database's glyph, without the way to add pages inside, and without their rows", async () => {
    setUp("/w/design");
    const sidebar = within(await screen.findByRole("complementary"));
    const tree = within(await sidebar.findByRole("list", { name: "Pages of General" }));

    const link = await tree.findByRole("link", { name: "Tasks" });
    expect(link.querySelector("svg")).toHaveClass("lucide-table-2");
    expect(tree.queryByRole("button", { name: "Add a page inside Tasks" })).toBeNull();
    expect(tree.queryByRole("link", { name: "Write docs" })).toBeNull();
    expect(tree.getByRole("button", { name: "Add a page inside Roadmap" })).toBeInTheDocument();
  });

  it("are made from a teamspace's page, and open there", async () => {
    const { user, db, router } = setUp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);

    await user.click(await screen.findByRole("button", { name: "New database" }));

    await waitFor(() => expect(db.created).toHaveLength(1));
    const made = db.created[0];
    expect(db.changes.at(-1)?.body).toEqual({ teamspace_id: TEAMSPACE_IDS.general });
    await waitFor(() =>
      expect(router.state.location.pathname).toBe(`/w/design/p/${made?.page.id ?? ""}`),
    );
  });
});
