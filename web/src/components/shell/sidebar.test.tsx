import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server, testUpdates } from "@/test/server";
import { design, fakeWorkspaces, studio, summaryOf, teamspaces } from "@/test/workspaces";

/** The sidebar's column, once the signed-in shell is up. */
async function column() {
  return within(await screen.findByRole("complementary"));
}

describe("the sidebar in a workspace", () => {
  it("switches workspaces from the switcher, which shows each with the role", async () => {
    fakeWorkspaces({ workspaces: [summaryOf(design()), studio] });
    const { user, router } = renderApp("/w/design");
    const sidebar = await column();

    const trigger = await sidebar.findByRole("button", {
      name: "Design, owner: switch workspace",
    });
    await user.click(trigger);
    const menu = within(await screen.findByRole("menu"));
    const current = menu.getByRole("menuitem", { name: "Design, Owner, current" });
    expect(current).toHaveAttribute("aria-current", "page");
    expect(menu.getByRole("menuitem", { name: "Studio, Member" })).toBeInTheDocument();
    expect(menu.getByRole("menuitem", { name: "Workspace settings" })).toBeInTheDocument();
    await user.click(menu.getByRole("menuitem", { name: /Studio/ }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio"));
    expect(
      await sidebar.findByRole("button", { name: "Studio, member: switch workspace" }),
    ).toBeInTheDocument();
  });

  it("offers creating a workspace only where the instance lets the user", async () => {
    fakeWorkspaces({ canCreate: false });
    const { user } = renderApp("/w/design");
    const sidebar = await column();

    await user.click(
      await sidebar.findByRole("button", { name: "Design, owner: switch workspace" }),
    );
    const menu = within(await screen.findByRole("menu"));
    expect(menu.queryByRole("menuitem", { name: "Create a workspace" })).toBeNull();
  });

  it("lists the teamspaces the user is in, marking private ones, and the way to the others", async () => {
    fakeWorkspaces({
      teamspaces: [teamspaces.general, teamspaces.open, teamspaces.closed, teamspaces.private],
    });
    renderApp("/w/design");
    const sidebar = await column();

    const list = within(await sidebar.findByRole("list", { name: "Teamspaces" }));
    expect(await list.findByRole("link", { name: "General" })).toBeInTheDocument();
    expect(list.getByRole("link", { name: "Secret lab, private" })).toBeInTheDocument();
    expect(list.queryByRole("link", { name: "Open studio" })).toBeNull();
    expect(list.getByRole("link", { name: "Browse teamspaces" })).toHaveAttribute(
      "href",
      "/w/design/teamspaces",
    );
  });

  it("lists no teamspace for a guest", async () => {
    fakeWorkspaces({ role: "guest" });
    renderApp("/w/design");
    const sidebar = await column();

    await sidebar.findByRole("button", { name: "Design, guest: switch workspace" });
    expect(sidebar.queryByRole("list", { name: "Teamspaces" })).toBeNull();
  });

  it("counts what is unread in the inbox", async () => {
    fakeWorkspaces();
    server.use(http.get("/api/v1/inbox/count", () => HttpResponse.json({ unread: 3 })));
    renderApp("/w/design");
    const sidebar = await column();

    expect(await sidebar.findByRole("link", { name: "Inbox, 3 unread" })).toHaveAttribute(
      "href",
      "/inbox",
    );
  });

  it("names the inbox alone while nothing in it is unread", async () => {
    fakeWorkspaces();
    renderApp("/w/design");
    const sidebar = await column();

    expect(await sidebar.findByRole("link", { name: "Inbox" })).toHaveAttribute("href", "/inbox");
  });

  it("says when the workspaces could not be loaded, and tries again", async () => {
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces", () =>
        fail
          ? apiError(500, "internal", "internal error")
          : HttpResponse.json({ workspaces: [summaryOf(design())], can_create: true }),
      ),
    );
    const { user } = renderApp("/account");
    const sidebar = await column();

    expect(await sidebar.findByText("Could not load your workspaces.")).toBeInTheDocument();
    fail = false;
    await user.click(sidebar.getByRole("button", { name: "Retry" }));
    expect(
      await sidebar.findByRole("button", { name: "Design, owner: switch workspace" }),
    ).toBeInTheDocument();
  });

  it("shows the workspace used last on pages outside any workspace", async () => {
    fakeWorkspaces({ workspaces: [summaryOf(design()), studio] });
    renderApp("/account");
    const sidebar = await column();

    expect(
      await sidebar.findByRole("button", { name: "Design, owner: switch workspace" }),
    ).toBeInTheDocument();
    expect(sidebar.getByRole("link", { name: "Workspace settings" })).toHaveAttribute(
      "href",
      "/w/design/settings",
    );
  });
});

describe("the Admin entry", () => {
  it("is marked for an instance admin while a newer release is out", async () => {
    server.use(
      signedInAs(grace),
      http.get("/api/v1/admin/updates", () =>
        HttpResponse.json({
          ...testUpdates,
          latest: {
            version: "0.2.0",
            prerelease: false,
            url: "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0",
          },
          newer: true,
        }),
      ),
    );
    renderApp("/account");
    const sidebar = await column();

    expect(
      await sidebar.findByRole("link", { name: "Admin, an update is available" }),
    ).toHaveAttribute("href", "/admin");
  });

  it("is plain while the server runs the newest release", async () => {
    server.use(signedInAs(grace));
    renderApp("/account");
    const sidebar = await column();

    expect(await sidebar.findByRole("link", { name: "Admin" })).toBeInTheDocument();
    expect(sidebar.queryByRole("link", { name: /an update is available/ })).toBeNull();
  });

  it("is not there for anyone else, who never asks about updates", async () => {
    let asked = 0;
    server.use(
      http.get("/api/v1/admin/updates", () => {
        asked += 1;
        return HttpResponse.json(testUpdates);
      }),
    );
    renderApp("/account");
    const sidebar = await column();

    expect(await sidebar.findByRole("link", { name: "Account" })).toBeInTheDocument();
    expect(sidebar.queryByRole("link", { name: /Admin/ })).toBeNull();
    expect(asked).toBe(0);
  });
});
