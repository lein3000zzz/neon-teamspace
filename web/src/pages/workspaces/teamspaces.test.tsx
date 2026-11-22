import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { fakeWorkspaces, TEAMSPACE_IDS, teamspaces } from "@/test/workspaces";

/** The page beside the sidebar, once the signed-in shell is up. */
async function page() {
  return within(await screen.findByRole("main"));
}

/** The card of one teamspace in a list, found by its name. */
async function card(name: string) {
  const heading = await (await page()).findByText(name, { selector: "span" });
  const item = heading.closest("li");
  if (item === null) {
    throw new Error(`no card for ${name}`);
  }
  return within(item);
}

describe("a workspace's pages", () => {
  it("answers not found for a workspace the user is not in", async () => {
    fakeWorkspaces();
    renderApp("/w/elsewhere/settings");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Workspace not found" }),
    ).toBeVisible();
    expect(screen.getByText(/You are in no workspace at \/w\/elsewhere/)).toBeInTheDocument();
  });

  it("opens on the teamspaces the user is in, and the ways onward", async () => {
    fakeWorkspaces();
    renderApp("/w/design");

    expect(await (await page()).findByRole("link", { name: /General/ })).toHaveAttribute(
      "href",
      `/w/design/teamspaces/${TEAMSPACE_IDS.general}`,
    );
    expect((await page()).getByRole("link", { name: "Invite people" })).toHaveAttribute(
      "href",
      "/w/design/settings/members",
    );
    expect((await page()).queryByRole("link", { name: /Open studio/ })).toBeNull();
  });

  it("says when its teamspaces cannot load, and tries again", async () => {
    fakeWorkspaces();
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces/:id/teamspaces", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp("/w/design");

    expect(await (await page()).findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click((await page()).getByRole("button", { name: "Try again" }));
    expect(await (await page()).findByRole("link", { name: /General/ })).toBeInTheDocument();
  });

  it("tells a guest only shared pages show, with no teamspaces", async () => {
    fakeWorkspaces({ role: "guest" });
    renderApp("/w/design");

    expect(await screen.findByText(/As a guest, you see the pages shared with you/)).toBeVisible();
    expect((await page()).queryByRole("link", { name: "Browse teamspaces" })).toBeNull();
    expect((await page()).queryByRole("link", { name: "Invite people" })).toBeNull();
  });
});

describe("browsing teamspaces", () => {
  it("joins an open teamspace at once", async () => {
    const api = fakeWorkspaces({ role: "member" });
    const { user } = renderApp("/w/design/teamspaces");

    const open = await card("Open studio");
    expect(open.getByText("You are not in it")).toBeInTheDocument();
    await user.click(open.getByRole("button", { name: "Join Open studio" }));

    expect(await screen.findByText("You joined Open studio.")).toBeInTheDocument();
    expect(api.changes.map((change) => change.path)).toEqual([
      `/api/v1/workspaces/${teamspaces.open.workspace_id}/teamspaces/${TEAMSPACE_IDS.open}/join`,
    ]);
    await waitFor(() => expect(open.getByText("You are in it")).toBeInTheDocument());
    expect(open.queryByRole("button", { name: "Join Open studio" })).toBeNull();
  });

  it("names the owners to ask for a closed teamspace", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp("/w/design/teamspaces");

    const closed = await card("Closed room");
    expect(await closed.findByText("Closed: ask Bob to add you.")).toBeInTheDocument();
    expect(closed.queryByRole("button", { name: /^Join/ })).toBeNull();
  });

  it("lets a workspace admin join a closed teamspace", async () => {
    fakeWorkspaces({ role: "admin" });
    renderApp("/w/design/teamspaces");

    const closed = await card("Closed room");
    expect(closed.getByRole("button", { name: "Join Closed room" })).toBeInTheDocument();
  });

  it("says when the teamspaces cannot load, and tries again", async () => {
    fakeWorkspaces({ role: "member" });
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces/:id/teamspaces", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp("/w/design/teamspaces");

    expect(await (await page()).findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click((await page()).getByRole("button", { name: "Try again" }));
    expect(await card("Open studio")).toBeTruthy();
  });

  it("shows a failed join where it happened", async () => {
    fakeWorkspaces({ role: "member" });
    server.use(
      http.post("/api/v1/workspaces/:id/teamspaces/:teamspace_id/join", () =>
        apiError(403, "forbidden", "a teamspace owner adds who joins this teamspace"),
      ),
    );
    const { user } = renderApp("/w/design/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Join Open studio" }));
    expect(await open.findByText("You are not allowed to do this.")).toBeInTheDocument();
  });
});

describe("a teamspace's page", () => {
  it("shows what it is and who is in it", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.closed}`);

    expect(await screen.findByRole("heading", { level: 1, name: "Closed room" })).toBeVisible();
    expect(screen.getByText("You are not in it")).toBeInTheDocument();
    expect(await (await page()).findByText("bob@example.com")).toBeInTheDocument();
    expect((await page()).queryByRole("combobox", { name: /Teamspace role of/ })).toBeNull();
    expect(await (await page()).findByText("Closed: ask Bob to add you.")).toBeInTheDocument();
  });

  it("answers not found for a private teamspace the user is not in", async () => {
    fakeWorkspaces({
      role: "member",
      teamspaces: [teamspaces.general, { ...teamspaces.private, role: null }],
    });
    renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.private}`);

    expect(
      await screen.findByRole("heading", { level: 1, name: "Teamspace not found" }),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "All teamspaces" })).toHaveAttribute(
      "href",
      "/w/design/teamspaces",
    );
  });

  it("says when the teamspace cannot load for another reason, and tries again", async () => {
    fakeWorkspaces({ role: "member" });
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces/:id/teamspaces/:teamspace_id", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.closed}`);

    expect(await (await page()).findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 1, name: "Teamspace not found" })).toBeNull();
    fail = false;
    await user.click((await page()).getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Closed room" })).toBeVisible();
  });

  it("leaves a teamspace once asked, but never the default one", async () => {
    const api = fakeWorkspaces({
      role: "member",
      teamspaces: [teamspaces.general, { ...teamspaces.open, role: "member", member_count: 2 }],
    });
    const { user } = renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.open}`);

    await user.click(await (await page()).findByRole("button", { name: "Leave" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Leave Open studio?" }));
    await user.click(dialog.getByRole("button", { name: "Leave" }));

    await waitFor(() => expect(api.changes.at(-1)?.path).toMatch(/\/leave$/));
    expect(
      await (await page()).findByRole("button", { name: "Join Open studio" }),
    ).toBeInTheDocument();
  });

  it("offers no leaving the default teamspace", async () => {
    fakeWorkspaces({ role: "member", teamspaces: [{ ...teamspaces.general, role: "member" }] });
    renderApp(`/w/design/teamspaces/${TEAMSPACE_IDS.general}`);

    expect(await screen.findByRole("heading", { level: 1, name: "General" })).toBeVisible();
    expect(screen.getByText(/Everyone in the workspace but guests is in General/)).toBeVisible();
    expect((await page()).queryByRole("button", { name: "Leave" })).toBeNull();
  });
});
