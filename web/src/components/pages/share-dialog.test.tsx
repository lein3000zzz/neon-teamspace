import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Share } from "@/api/pages";
import { renderApp } from "@/test/render-app";
import { fakePages, PAGE_IDS } from "@/test/pages";
import { fakeWorkspaces, people } from "@/test/workspaces";

const CREATED = "2026-11-20T09:00:00Z";

function share(person: Share["user"], access: Share["access"], pageId: string): Share {
  return {
    user: person,
    access,
    page_id: pageId,
    inherited: false,
    created_at: CREATED,
    updated_at: CREATED,
  };
}

/** Opens the share dialog of Q1, under Roadmap. */
async function openShares() {
  const rendered = renderApp(`/w/design/p/${PAGE_IDS.q1}`);
  const main = within(await screen.findByRole("main"));
  await rendered.user.click(await main.findByRole("button", { name: "Share" }));
  const dialog = within(await screen.findByRole("dialog", { name: "Share Q1" }));
  return { ...rendered, dialog };
}

describe("sharing a page", () => {
  it("lists the page's own shares, then those from above it, guests marked", async () => {
    fakeWorkspaces();
    fakePages({
      shares: {
        [PAGE_IDS.q1]: [share(people.gus, "can_view", PAGE_IDS.q1)],
        [PAGE_IDS.roadmap]: [share(people.cy, "can_edit", PAGE_IDS.roadmap)],
      },
    });
    const { dialog } = await openShares();

    const list = within(await dialog.findByRole("region", { name: "People with a share" }));
    const gus = within((await list.findByText("Gus")).closest("li") ?? document.body);
    expect(gus.getByText("Guest")).toBeInTheDocument();
    expect(gus.getByRole("combobox", { name: "Access of Gus" })).toHaveValue("can_view");
    const cy = within(list.getByText("Cy").closest("li") ?? document.body);
    expect(cy.getByText("Can edit · From Roadmap", { exact: false })).toBeInTheDocument();
    expect(cy.queryByRole("button")).toBeNull();
    expect(cy.queryByText("Guest")).toBeNull();
  });

  it("shares with someone in the workspace, to view", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user, dialog } = await openShares();

    const form = within(dialog.getByRole("form", { name: "Share with someone" }));
    const person = form.getByLabelText("Person");
    // Never yourself.
    expect(within(person).queryByRole("option", { name: "Ada" })).toBeNull();
    await user.selectOptions(person, people.gus.id);
    expect(form.getByLabelText("Access")).toHaveValue("can_view");
    await user.click(form.getByRole("button", { name: "Share" }));

    expect(await dialog.findByText("Shared with Gus.")).toBeInTheDocument();
    expect(api.changes.at(-1)).toEqual({
      method: "POST",
      path: `/api/v1/pages/${PAGE_IDS.q1}/shares`,
      body: { user_id: people.gus.id, access: "can_view" },
    });
    expect(await dialog.findByRole("combobox", { name: "Access of Gus" })).toBeInTheDocument();
  });

  it("asks for a person before sharing", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user, dialog } = await openShares();

    await user.click(
      within(dialog.getByRole("form", { name: "Share with someone" })).getByRole("button", {
        name: "Share",
      }),
    );
    expect(dialog.getByLabelText("Person")).toHaveAttribute("aria-invalid", "true");
    expect(api.changes).toEqual([]);
  });

  it("changes what a share gives, and removes it", async () => {
    fakeWorkspaces();
    const api = fakePages({
      shares: { [PAGE_IDS.q1]: [share(people.bob, "can_view", PAGE_IDS.q1)] },
    });
    const { user, dialog } = await openShares();

    await user.selectOptions(
      await dialog.findByRole("combobox", { name: "Access of Bob" }),
      "can_edit",
    );
    await waitFor(() =>
      expect(api.changes.at(-1)).toEqual({
        method: "PATCH",
        path: `/api/v1/pages/${PAGE_IDS.q1}/shares/${people.bob.id}`,
        body: { access: "can_edit" },
      }),
    );

    await user.click(dialog.getByRole("button", { name: "Remove the share of Bob" }));
    expect(await dialog.findByText("Removed the share of Bob.")).toBeInTheDocument();
    expect(await dialog.findByText("Nobody has a share on this page yet.")).toBeInTheDocument();
  });

  it("offers guests no way to share", async () => {
    fakeWorkspaces({ role: "guest" });
    fakePages();
    renderApp(`/w/design/p/${PAGE_IDS.q1}`);
    const main = within(await screen.findByRole("main"));

    await main.findByRole("textbox", { name: "Page title" });
    expect(main.queryByRole("button", { name: "Share" })).toBeNull();
  });
});
