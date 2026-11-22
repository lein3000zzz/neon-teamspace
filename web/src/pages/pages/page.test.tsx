import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { ada, apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS, samplePages } from "@/test/pages";
import { server } from "@/test/server";
import { fakeWorkspaces, TEAMSPACE_IDS } from "@/test/workspaces";

const goals = `/w/design/p/${PAGE_IDS.goals}`;

/** The page beside the sidebar, once the signed-in shell is up. */
async function main() {
  return within(await screen.findByRole("main"));
}

describe("a page's header", () => {
  it("shows the way down to the page, from its teamspace", async () => {
    fakeWorkspaces();
    fakePages();
    renderApp(goals);

    const crumbs = within(
      await (await main()).findByRole("navigation", { name: "Where this page is" }),
    );
    await waitFor(() =>
      expect(crumbs.getAllByRole("listitem").map((item) => item.textContent)).toEqual([
        "General/",
        "🗺️ Roadmap/",
        "Q1/",
        "Goals",
      ]),
    );
    expect(crumbs.getByRole("link", { name: "Q1" })).toHaveAttribute(
      "href",
      `/w/design/p/${PAGE_IDS.q1}`,
    );
    expect(crumbs.getByText("Goals")).toHaveAttribute("aria-current", "page");
    // The pages above it open in the sidebar, so it shows there.
    expect(
      await within(screen.getByRole("complementary")).findByRole("link", { name: "Goals" }),
    ).toHaveAttribute("aria-current", "page");
  });

  it("saves the title once typing pauses, showing it in the sidebar as it is typed", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);
    const title = await (await main()).findByRole("textbox", { name: "Page title" });
    const sidebar = within(screen.getByRole("complementary"));
    await sidebar.findByRole("link", { name: "Goals" });

    await user.clear(title);
    await user.type(title, "Targets");
    expect(sidebar.getByRole("link", { name: "Targets" })).toBeInTheDocument();
    expect(api.changes).toEqual([]);

    await waitFor(() =>
      expect(api.changes).toEqual([
        { method: "PATCH", path: `/api/v1/pages/${PAGE_IDS.goals}`, body: { title: "Targets" } },
      ]),
    );
    expect(api.find(PAGE_IDS.goals)?.title).toBe("Targets");
  });

  it("saves at once on leaving the title, and keeps it to one line", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);
    const title = await (await main()).findByRole("textbox", { name: "Page title" });

    await user.type(title, " 2027{Enter}");
    expect(title).toHaveValue("Goals 2027");
    expect(api.changes).toEqual([
      { method: "PATCH", path: `/api/v1/pages/${PAGE_IDS.goals}`, body: { title: "Goals 2027" } },
    ]);
  });

  it("says when the title could not be saved, and tries again", async () => {
    fakeWorkspaces();
    fakePages();
    let fail = true;
    server.use(
      http.patch("/api/v1/pages/:id", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp(goals);
    const title = await (await main()).findByRole("textbox", { name: "Page title" });

    await user.type(title, "!");
    await user.tab();
    expect(await (await main()).findByText("The title could not be saved.")).toBeInTheDocument();
    fail = false;
    await user.click((await main()).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.queryByText("The title could not be saved.")).toBeNull());
  });

  it("opens an empty page with the cursor in its title", async () => {
    fakeWorkspaces();
    fakePages({
      pages: [...samplePages(), page({ id: PAGE_IDS.blank, title: "", owner: ada.id }, "a9")],
    });
    renderApp(`/w/design/p/${PAGE_IDS.blank}`);

    const title = await (await main()).findByRole("textbox", { name: "Page title" });
    expect(title).toHaveAttribute("placeholder", "Untitled");
    await waitFor(() => expect(title).toHaveFocus());
  });

  it("sets an icon typed as text, and removes it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add icon" }));
    const picker = within(await screen.findByRole("dialog", { name: "Icon" }));
    await user.type(picker.getByLabelText("Or type a short text"), "Q4");
    await user.click(picker.getByRole("button", { name: "Use it" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ icon: "Q4" }));

    await user.click(await (await main()).findByRole("button", { name: "Change icon" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "Icon" })).getByRole("button", {
        name: "Remove icon",
      }),
    );
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ icon: null }));
    expect(await (await main()).findByRole("button", { name: "Add icon" })).toBeInTheDocument();
  });

  it("picks an emoji for the icon from the built-in set", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add icon" }));
    const picker = within(await screen.findByRole("dialog", { name: "Icon" }));
    await user.type(picker.getByLabelText("Search emoji"), "rocket");
    await user.click(await picker.findByRole("button", { name: "Rocket" }));

    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ icon: "🚀" }));
  });

  it("sets a cover from the presets, and changes it", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add cover" }));
    const picker = within(await screen.findByRole("dialog", { name: "Cover" }));
    expect(picker.getAllByRole("button")).toHaveLength(14);
    await user.click(picker.getByRole("button", { name: "Sunrise" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ cover: "sunrise" }));
    await waitFor(() =>
      expect(document.querySelector("[data-cover]")).toHaveAttribute("data-cover", "sunrise"),
    );

    await user.click((await main()).getByRole("button", { name: "Change cover" }));
    const again = within(await screen.findByRole("dialog", { name: "Cover" }));
    expect(again.getByRole("button", { name: "Sunrise" })).toHaveAttribute("aria-pressed", "true");
    await user.click(again.getByRole("button", { name: "Remove cover" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ cover: null }));
  });

  it("sets the page's font, small text and full width", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);
    const article = (await (await main()).findByRole("textbox", { name: "Page title" })).closest(
      "article",
    );

    await user.click((await main()).getByRole("button", { name: "Style" }));
    const style = within(await screen.findByRole("dialog", { name: "Style" }));
    await user.click(style.getByRole("radio", { name: /Serif/ }));
    await waitFor(() => expect(article).toHaveAttribute("data-page-font", "serif"));
    await user.click(style.getByRole("checkbox", { name: "Full width" }));
    await waitFor(() => expect(article).toHaveAttribute("data-page-width", "full"));
    await user.click(style.getByRole("checkbox", { name: "Small text" }));
    await waitFor(() => expect(article).toHaveAttribute("data-small-text"));

    expect(api.changes.map((change) => change.body)).toEqual([
      { style: { font: "serif" } },
      { style: { full_width: true } },
      { style: { small_text: true } },
    ]);
  });

  it("adds the page to the favorites and takes it out again", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add to favorites" }));
    const remove = await (await main()).findByRole("button", { name: "Remove from favorites" });
    expect(remove).toHaveAttribute("aria-pressed", "true");
    expect(api.state.favorites).toEqual([PAGE_IDS.goals]);
    await user.click(remove);
    expect(await (await main()).findByRole("button", { name: "Add to favorites" })).toBeVisible();
    expect(api.state.favorites).toEqual([]);
  });

  it("shows a viewer the page without the ways to change it", async () => {
    fakeWorkspaces({ role: "member" });
    fakePages({
      pages: [
        page(
          { id: PAGE_IDS.goals, title: "Goals", teamspace: TEAMSPACE_IDS.open, access: "view" },
          "a0",
        ),
      ],
    });
    renderApp(goals);
    const view = await main();

    expect(await view.findByRole("heading", { level: 1, name: "Goals" })).toBeInTheDocument();
    expect(view.getByText("You can view this page, but not change it.")).toBeInTheDocument();
    for (const name of ["Page title", "Add icon", "Add cover", "Style", "Share"]) {
      expect(view.queryByRole("button", { name })).toBeNull();
    }
    expect(view.queryByRole("textbox", { name: "Page title" })).toBeNull();
    expect(view.getByRole("button", { name: "Add to favorites" })).toBeInTheDocument();
  });

  it("answers not found for a page the user may not see", async () => {
    fakeWorkspaces();
    fakePages();
    renderApp("/w/design/p/0193a5b2-0000-7000-8000-0000000000ff");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Page not found" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to the workspace" })).toHaveAttribute(
      "href",
      "/w/design",
    );
  });
});

describe("a page in the trash", () => {
  it("says so to its editors, and comes back on Restore", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(`/w/design/p/${PAGE_IDS.old}`);

    const banner = within(
      (await screen.findByText("This page is in the trash.")).closest("[role=status]") ??
        document.body,
    );
    expect(screen.getByRole("heading", { level: 1, name: "Old plans" })).toBeInTheDocument();
    await user.click(banner.getByRole("button", { name: "Restore" }));

    expect(await (await main()).findByRole("textbox", { name: "Page title" })).toBeInTheDocument();
    expect(screen.queryByText("This page is in the trash.")).toBeNull();
    expect(api.changes.at(-1)?.path).toBe(`/api/v1/pages/${PAGE_IDS.old}/restore`);
  });

  it("is deleted for good after asking, and the trash opens", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user, router } = renderApp(`/w/design/p/${PAGE_IDS.old}`);

    await user.click(await (await main()).findByRole("button", { name: "Delete for good" }));
    const dialog = within(await screen.findByRole("alertdialog"));
    expect(dialog.getByText("Delete Old plans for good?")).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Delete for good" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design/trash"));
    expect(api.changes.at(-1)).toEqual({
      method: "DELETE",
      path: `/api/v1/pages/${PAGE_IDS.old}`,
      body: undefined,
    });
    expect(await screen.findByText("The trash is empty.")).toBeInTheDocument();
  });
});
