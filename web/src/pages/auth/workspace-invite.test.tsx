import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { InviteLinkPreview } from "@/api/workspaces";
import { keys } from "@/api/keys";
import { ada, apiError, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { fakeWorkspaces, LINK_TOKEN, studio, studioInvitation } from "@/test/workspaces";

const preview: InviteLinkPreview = {
  workspace: { id: studio.id, name: "Studio", slug: "studio", icon: "🎨", hue: studio.hue },
  role: "guest",
  expires_at: "2026-11-27T09:00:00Z",
};

/** The link routes, recording each token sent. */
function linkRoutes(answer: InviteLinkPreview | Response = preview) {
  const sent: { path: string; token: string }[] = [];
  server.use(
    http.post("/api/v1/workspace-invite-links/preview", async ({ request }) => {
      sent.push({ path: "preview", token: ((await request.json()) as { token: string }).token });
      return answer instanceof Response ? answer : HttpResponse.json(answer);
    }),
    http.post("/api/v1/workspace-invite-links/join", async ({ request }) => {
      sent.push({ path: "join", token: ((await request.json()) as { token: string }).token });
      return HttpResponse.json({ ...studio, role: "guest" });
    }),
  );
  return sent;
}

describe("a workspace invite link", () => {
  it("takes the token out of the address, and asks a visitor to sign in first", async () => {
    server.use(signedOut);
    const sent = linkRoutes();
    const { router } = renderApp(`/workspace-invite/${LINK_TOKEN}`);

    expect(
      await screen.findByRole("heading", { level: 1, name: "Join a workspace" }),
    ).toBeVisible();
    await waitFor(() => expect(router.state.location.pathname).toBe("/workspace-invite"));
    expect(window.sessionStorage.getItem("neon.workspace-invite")).toBe(LINK_TOKEN);
    expect(await screen.findByRole("link", { name: "Sign in to join" })).toHaveAttribute(
      "href",
      "/login?next=%2Fworkspace-invite",
    );
    expect(sent).toEqual([]);
  });

  it("shows what it admits to, and joins with one press", async () => {
    fakeWorkspaces({ workspaces: [] });
    const sent = linkRoutes();
    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    const { user, router } = renderApp("/workspace-invite");

    expect(await screen.findByRole("heading", { level: 1, name: "Studio" })).toBeVisible();
    expect(screen.getByText(/This link invites you to join as a guest/)).toBeInTheDocument();
    expect(screen.getByText("Signed in as ada@example.com.")).toBeInTheDocument();
    server.use(
      http.get("/api/v1/workspaces", () =>
        HttpResponse.json({ workspaces: [{ ...studio, role: "guest" }], can_create: true }),
      ),
    );
    await user.click(screen.getByRole("button", { name: "Join Studio" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio"));
    expect(sent).toEqual([
      { path: "preview", token: LINK_TOKEN },
      { path: "join", token: LINK_TOKEN },
    ]);
    expect(window.sessionStorage.getItem("neon.workspace-invite")).toBeNull();
  });

  it("ends the invitation into the same workspace that waited, once joined by the link", async () => {
    const api = fakeWorkspaces({ received: [studioInvitation] });
    linkRoutes();
    server.use(
      http.post("/api/v1/workspace-invite-links/join", () => {
        // As the server does: joining ends the invitation that waited.
        api.state.received = [];
        api.state.workspaces = [...api.state.workspaces, studio];
        return HttpResponse.json(studio);
      }),
    );
    const { user, router } = renderApp("/w/design");
    const sidebar = within(await screen.findByRole("complementary"));
    expect(await sidebar.findByRole("link", { name: "Invitations, 1 pending" })).toBeVisible();

    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    await router.navigate({ to: "/workspace-invite" });
    await user.click(await screen.findByRole("button", { name: "Join Studio" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio"));
    const after = within(await screen.findByRole("complementary"));
    await waitFor(() => expect(after.queryByRole("link", { name: /^Invitations/ })).toBeNull());
  });

  it("sends someone already in the workspace there instead", async () => {
    linkRoutes({ ...preview, member_role: "member" });
    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    renderApp("/workspace-invite");

    expect(await screen.findByText("You are in Studio already, as a member.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Studio" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Join Studio" })).toBeNull();
  });

  it("names no role for someone who joined in between", async () => {
    linkRoutes();
    server.use(
      http.post("/api/v1/workspace-invite-links/join", () =>
        apiError(409, "already_member", "you are in this workspace already"),
      ),
    );
    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    const { user } = renderApp("/workspace-invite");

    await user.click(await screen.findByRole("button", { name: "Join Studio" }));
    expect(await screen.findByText("You are in Studio already.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Studio" })).toBeInTheDocument();
  });

  it("says when the link no longer works, and forgets it", async () => {
    linkRoutes(apiError(400, "invalid_invite_link", "this invite link does not work"));
    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    renderApp("/workspace-invite");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This invite link does not work" }),
    ).toBeVisible();
    await waitFor(() => expect(window.sessionStorage.getItem("neon.workspace-invite")).toBeNull());
  });

  it("says when the workspace is full", async () => {
    linkRoutes();
    server.use(
      http.post("/api/v1/workspace-invite-links/join", () =>
        apiError(409, "member_limit", "the workspace is at its member limit"),
      ),
    );
    window.sessionStorage.setItem("neon.workspace-invite", LINK_TOKEN);
    const { user } = renderApp("/workspace-invite");

    await user.click(await screen.findByRole("button", { name: "Join Studio" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/at its member limit/);
  });

  it("asks for the link again when this tab holds no token", async () => {
    renderApp("/workspace-invite");
    expect(
      await screen.findByRole("heading", { level: 1, name: "This link is incomplete" }),
    ).toBeVisible();
  });

  it("goes back to the link once signed up, from the app's start", async () => {
    let signedIn = false;
    server.use(
      http.get("/api/v1/account/me", () =>
        signedIn ? HttpResponse.json(ada) : apiError(401, "unauthenticated", "sign in to continue"),
      ),
    );
    fakeWorkspaces();
    const sent = linkRoutes();
    const { router, queryClient } = renderApp(`/workspace-invite/${LINK_TOKEN}`);
    expect(await screen.findByRole("link", { name: "Sign in to join" })).toBeVisible();

    // Signing up in the same tab: the sign-up page sends the new account to the app's start.
    await router.navigate({ to: "/register" });
    expect(await screen.findByRole("heading", { level: 1, name: /Create/ })).toBeVisible();
    signedIn = true;
    await queryClient.invalidateQueries({ queryKey: keys.me });

    await waitFor(() => expect(router.state.location.pathname).toBe("/workspace-invite"));
    expect(await screen.findByRole("button", { name: "Join Studio" })).toBeInTheDocument();
    expect(sent).toEqual([{ path: "preview", token: LINK_TOKEN }]);
  });

  it("lets someone who looked at the link and went on use the app's start", async () => {
    fakeWorkspaces();
    const sent = linkRoutes();
    const { router } = renderApp(`/workspace-invite/${LINK_TOKEN}`);
    expect(await screen.findByRole("button", { name: "Join Studio" })).toBeVisible();

    // Not joining, they go to the app's start by its address, and again later.
    await router.navigate({ to: "/" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design"));
    await router.navigate({ to: "/" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/w/design"));
    expect(sent).toEqual([{ path: "preview", token: LINK_TOKEN }]);
    // The link still works from its own page while the tab holds it.
    await router.navigate({ to: "/workspace-invite" });
    expect(await screen.findByRole("button", { name: "Join Studio" })).toBeVisible();
  });
});
