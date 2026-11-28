import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { beforeAll, describe, expect, it } from "vitest";
import { ada, apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { fakePages, page, PAGE_IDS, samplePages, UPLOADED_IDS } from "@/test/pages";
import { server } from "@/test/server";
import { fakeWorkspaces, scout, TEAMSPACE_IDS } from "@/test/workspaces";

const goals = `/w/design/p/${PAGE_IDS.goals}`;

/** The page beside the sidebar, once the signed-in shell is up. */
async function main() {
  return within(await screen.findByRole("main"));
}

// The page loads the editor on demand; loaded once here, so no test waits on it. Left to the
// first page a test opens, the import would run on into the tests after it.
beforeAll(async () => {
  await import("@/components/editor/page-editor");
});

describe("a page's header", () => {
  it("names who made the page and who changed it, an agent with whom it acts for", async () => {
    fakeWorkspaces({ agents: [scout] });
    fakePages({
      pages: samplePages().map((entry) =>
        entry.id === PAGE_IDS.goals ? Object.assign(entry, { created_by: scout.id }) : entry,
      ),
    });
    renderApp(goals);

    const byline = within(
      await (await main()).findByRole("region", { name: "Who made and last changed this page" }),
    );
    await waitFor(() =>
      expect(byline.getAllByRole("definition").map((item) => item.textContent)).toEqual([
        "ScoutAgent· via Ada",
        "Ada",
      ]),
    );
    expect(byline.getByText("Created by")).toBeInTheDocument();
    expect(byline.getByText("Last edited by")).toBeInTheDocument();
  });

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
    expect(picker.getAllByRole("button")).toHaveLength(15);
    await user.click(picker.getByRole("button", { name: "Sunrise" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ cover: "sunrise" }));
    await waitFor(() =>
      expect(document.querySelector("[data-cover]")).toHaveAttribute("data-cover", "sunrise"),
    );

    await user.click((await main()).getByRole("button", { name: "Change cover" }));
    const again = within(await screen.findByRole("dialog", { name: "Cover" }));
    expect(again.getByRole("button", { name: "Sunrise" })).toHaveAttribute("aria-pressed", "true");
    await user.click(again.getByRole("button", { name: "Remove cover" }));
    await waitFor(() =>
      expect(api.changes.at(-1)?.body).toEqual({ cover: null, cover_image: null }),
    );
  });

  it("uploads an image as the cover, which a preset then takes the place of", async () => {
    fakeWorkspaces();
    const api = fakePages();
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add cover" }));
    const picker = within(await screen.findByRole("dialog", { name: "Cover" }));
    expect(picker.getByLabelText("An image for the cover")).toHaveAttribute(
      "accept",
      "image/png,image/jpeg,image/gif,image/webp,image/avif,image/svg+xml",
    );
    await user.upload(
      picker.getByLabelText("An image for the cover"),
      new File([new Uint8Array([137, 80, 78, 71])], "beach.png", { type: "image/png" }),
    );
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ cover_image: UPLOADED_IDS[0] }));
    expect(api.uploads).toEqual([{ page: PAGE_IDS.goals, kind: "image" }]);
    await waitFor(() =>
      expect(document.querySelector("[data-cover-image]")).toHaveAttribute(
        "data-cover-image",
        UPLOADED_IDS[0],
      ),
    );
    expect(
      (document.querySelector("[data-cover-image]") as HTMLElement).style.backgroundImage,
    ).toContain(`/api/v1/files/${UPLOADED_IDS[0]}`);

    await user.click((await main()).getByRole("button", { name: "Change cover" }));
    const again = within(await screen.findByRole("dialog", { name: "Cover" }));
    await user.click(again.getByRole("button", { name: "Dusk" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ cover: "dusk" }));
    await waitFor(() =>
      expect(document.querySelector("[data-cover]")).toHaveAttribute("data-cover", "dusk"),
    );
  });

  it("says why a cover was not uploaded", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.post("/api/v1/pages/:id/files", () =>
        apiError(409, "storage_limit", "this workspace has reached its storage limit of 10 bytes"),
      ),
    );
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add cover" }));
    const picker = within(await screen.findByRole("dialog", { name: "Cover" }));
    await user.upload(
      picker.getByLabelText("An image for the cover"),
      new File([new Uint8Array([1])], "big.png", { type: "image/png" }),
    );
    const alert = await (await main()).findByRole("alert");
    expect(alert).toHaveTextContent("The cover could not be uploaded.");
    expect(alert).toHaveTextContent("This workspace has reached its storage limit of 10 bytes.");

    await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(alert).not.toBeInTheDocument());
  });

  it("forgets why a cover was not uploaded once a preset takes its place", async () => {
    fakeWorkspaces();
    fakePages();
    server.use(
      http.post("/api/v1/pages/:id/files", () =>
        apiError(409, "storage_limit", "this workspace has reached its storage limit of 10 bytes"),
      ),
    );
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Add cover" }));
    await user.upload(
      within(await screen.findByRole("dialog", { name: "Cover" })).getByLabelText(
        "An image for the cover",
      ),
      new File([new Uint8Array([1])], "big.png", { type: "image/png" }),
    );
    await (await main()).findByRole("alert");

    await user.click((await main()).getByRole("button", { name: "Add cover" }));
    const picker = within(await screen.findByRole("dialog", { name: "Cover" }));
    await user.click(picker.getByRole("button", { name: "Dusk" }));
    await waitFor(() =>
      expect(document.querySelector("[data-cover]")).toHaveAttribute("data-cover", "dusk"),
    );
    expect((await main()).queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says a cover is uploading in place of a cover it replaces", async () => {
    fakeWorkspaces();
    fakePages({
      pages: samplePages().map((entry) =>
        entry.id === PAGE_IDS.goals ? Object.assign(entry, { cover: "sunrise" }) : entry,
      ),
    });
    let arrive: (() => void) | undefined;
    const arrived = new Promise<void>((resolve) => {
      arrive = resolve;
    });
    server.use(
      http.post("/api/v1/pages/:id/files", async () => {
        await arrived;
        return apiError(413, "file_too_large", "files take at most 1 KiB");
      }),
    );
    const { user } = renderApp(goals);

    await user.click(await (await main()).findByRole("button", { name: "Change cover" }));
    await user.upload(
      within(await screen.findByRole("dialog", { name: "Cover" })).getByLabelText(
        "An image for the cover",
      ),
      new File([new Uint8Array([1])], "beach.png", { type: "image/png" }),
    );
    const uploading = await (await main()).findByRole("button", { name: "Uploading the cover…" });
    expect(uploading).toBeDisabled();

    arrive?.();
    expect(await (await main()).findByRole("button", { name: "Change cover" })).toBeEnabled();
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
