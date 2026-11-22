import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS, samplePages } from "@/test/pages";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

async function trash() {
  return within(await screen.findByRole("list", { name: "Trash" }));
}

describe("the trash", () => {
  it("lists the pages in it, and brings one back", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp("/w/design/trash");

    const list = await trash();
    expect(list.getByRole("link", { name: "Old plans" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.old}`,
    );
    await user.click(list.getByRole("button", { name: "Restore Old plans" }));

    expect(await screen.findByText("Old plans is back.")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("The trash is empty.")).toBeInTheDocument());
    expect(api.changes.at(-1)?.path).toBe(`/api/v1/pages/${PAGE_IDS.old}/restore`);
    // Back in its teamspace's tree.
    expect(
      await within(screen.getByRole("complementary")).findByRole("link", { name: "Old plans" }),
    ).toBeInTheDocument();
  });

  it("deletes a page for good after asking", async () => {
    fakeWorkspaces();
    const api = fakePages({
      pages: [
        ...samplePages(),
        page(
          { id: PAGE_IDS.blank, title: "Drafts", teamspace: TEAMSPACE_IDS.general, trashed: true },
          "a3",
        ),
      ],
    });
    const { user } = renderApp("/w/design/trash");

    await user.click((await trash()).getByRole("button", { name: "Delete Old plans for good" }));
    const dialog = within(await screen.findByRole("alertdialog"));
    expect(dialog.getByText(/every page under it/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(api.changes).toEqual([]);

    await user.click((await trash()).getByRole("button", { name: "Delete Old plans for good" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: "Delete for good",
      }),
    );

    expect(await screen.findByText("Old plans is deleted.")).toBeInTheDocument();
    await waitFor(async () =>
      expect((await trash()).queryByRole("link", { name: "Old plans" })).toBeNull(),
    );
    expect((await trash()).getByRole("link", { name: "Drafts" })).toBeInTheDocument();
  });

  it("says when it is empty", async () => {
    fakeWorkspaces();
    fakePages({ pages: samplePages().filter((entry) => entry.id !== PAGE_IDS.old) });
    renderApp("/w/design/trash");

    expect(await screen.findByText("The trash is empty.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Trash" })).toBeInTheDocument();
  });
});
