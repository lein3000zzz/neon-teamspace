import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { ada, apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { blockStorage } from "@/test/storage";
import { design, fakeWorkspaces, studio, studioInvitation, summaryOf } from "@/test/workspaces";

describe("the start page", () => {
  it("opens the workspace used last", async () => {
    fakeWorkspaces({ workspaces: [summaryOf(design()), studio] });
    window.localStorage.setItem(`neon.workspace.${ada.id}`, studio.id);
    const { router } = renderApp("/");

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio"));
    expect(await screen.findByRole("heading", { level: 1, name: "Studio" })).toBeVisible();
  });

  it("opens the first workspace when the last one is gone, and remembers the one opened", async () => {
    fakeWorkspaces({ workspaces: [summaryOf(design()), studio] });
    window.localStorage.setItem(`neon.workspace.${ada.id}`, "0193a5b2-0000-7000-8000-0000000000ff");
    const { router } = renderApp("/");

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design"));
    await waitFor(() =>
      expect(window.localStorage.getItem(`neon.workspace.${ada.id}`)).toBe(design().id),
    );
  });

  it("says when the workspaces cannot load, and tries again", async () => {
    fakeWorkspaces();
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user, router } = renderApp("/");

    const main = within(await screen.findByRole("main"));
    expect(await main.findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click(main.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design"));
  });

  it("still opens a workspace where the browser refuses storage", async () => {
    fakeWorkspaces();
    blockStorage();
    const { router } = renderApp("/");
    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design"));
  });

  it("offers to create a workspace to someone in none", async () => {
    fakeWorkspaces({ workspaces: [] });
    const { user, router } = renderApp("/");

    expect(
      await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    expect(screen.getByText("No invitations wait for you.")).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "Create a workspace" }));
    expect(router.state.location.pathname).toBe("/workspaces/new");
  });

  it("says how to get in where only instance admins create workspaces", async () => {
    fakeWorkspaces({ workspaces: [], canCreate: false });
    renderApp("/");

    expect(
      await screen.findByText("On this instance, only instance admins create workspaces."),
    ).toBeInTheDocument();
    expect(screen.getByText(/ask its owner or an admin to invite/)).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Create a workspace" })).toBeNull();
  });

  it("lists the invitations waiting, and opens the workspace once one is accepted", async () => {
    const api = fakeWorkspaces({ workspaces: [], received: [studioInvitation] });
    const { user, router } = renderApp("/");

    const card = within((await screen.findByText("Studio")).closest("li") ?? document.body);
    expect(card.getByText("Join as a member")).toBeInTheDocument();
    expect(card.getByText("Invited by Bob")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Accept: Studio" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio"));
    expect(api.changes.map((change) => change.path)).toEqual([
      `/api/v1/account/invitations/${studioInvitation.id}/accept`,
    ]);
  });
});
