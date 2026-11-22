import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { META_TIMEOUT_MS } from "@/api/meta";
import { ada, apiError, fakeSession, grace, signedInAs, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

describe("the session", () => {
  it("sends a visitor without one to sign in, remembering where they were going", async () => {
    server.use(signedOut);
    const { router } = renderApp("/account/tokens?tab=1");

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/login");
    expect(router.state.location.search).toEqual({ next: "/account/tokens?tab=1" });
  });

  it("goes on to that page once signed in", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(
      ...session.handlers,
      http.get("/api/v1/account/sessions", () => HttpResponse.json({ sessions: [] })),
    );
    const { user, router } = renderApp("/account/sessions");

    await user.type(await screen.findByLabelText("Email"), "Ada@Example.com");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await screen.findByRole("heading", { level: 2, name: "Sessions" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/account/sessions");
    expect(session.logins).toEqual([
      { email: "Ada@Example.com", password: "correct horse battery" },
    ]);
  });

  it("sends the user to sign in when the session ends mid-visit", async () => {
    server.use(
      http.get("/api/v1/account/sessions", () =>
        apiError(401, "unauthenticated", "the session has ended: sign in again"),
      ),
    );
    const { router } = renderApp("/account/sessions");

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(router.state.location.search).toEqual({ next: "/account/sessions" });
  });

  it("signs out from the sidebar and forgets the user", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    const { user, router, queryClient } = renderApp("/login");

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    const aside = within(await screen.findByRole("complementary"));
    expect(aside.getByText(ada.email)).toBeInTheDocument();

    await user.click(aside.getByRole("button", { name: "Sign out" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(session.signedIn).toBe(false);
    expect(router.state.location.pathname).toBe("/login");
    expect(queryClient.getQueryData(["account", "me"])).toBeNull();
    expect(queryClient.getQueryData(["account", "preferences", ada.id])).toBeUndefined();
  });

  it("keeps no password or token secret in memory once its page is done with it", async () => {
    const SECRET = "nw_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcd";
    const session = fakeSession(ada, "correct horse battery");
    server.use(
      ...session.handlers,
      http.get("/api/v1/account/tokens", () => HttpResponse.json({ tokens: [] })),
      http.post("/api/v1/account/tokens", () =>
        HttpResponse.json(
          {
            token: SECRET,
            api_token: {
              id: "0193a5b2-0000-7000-8000-0000000000a1",
              name: "CI deploys",
              prefix: "nw_AbCdEfG",
              scopes: ["read"],
              created_at: "2026-11-20T09:00:00Z",
              last_used_at: null,
              expires_at: null,
            },
          },
          { status: 201 },
        ),
      ),
    );
    const { user, queryClient } = renderApp("/login?next=/account/tokens");
    const mutations = () =>
      JSON.stringify(
        queryClient
          .getMutationCache()
          .getAll()
          .map(({ state }) => [state.variables, state.data]),
      );

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await user.type(await screen.findByLabelText("Name"), "CI deploys");
    await waitFor(() => expect(mutations()).not.toContain("correct horse battery"));

    await user.click(screen.getByRole("button", { name: "Make token" }));
    expect(await screen.findByDisplayValue(SECRET)).toBeInTheDocument();
    const aside = within(screen.getByRole("complementary"));
    await user.click(aside.getByRole("button", { name: "Sign out" }));

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    await waitFor(() => expect(queryClient.getMutationCache().getAll()).toEqual([]));
    expect(mutations()).not.toContain(SECRET);
  });

  it("lists the admin pages for admins only", async () => {
    renderApp("/");
    const nav = within(await screen.findByRole("navigation", { name: "Main" }));
    expect(nav.getByRole("link", { name: "Account" })).toBeInTheDocument();
    expect(nav.queryByRole("link", { name: "Admin" })).toBeNull();
  });

  it("shows a member the not-allowed page at an admin address, and asks nothing of the admin API", async () => {
    server.use(http.get("/api/v1/admin/users", () => HttpResponse.error()));
    renderApp("/admin");

    expect(await screen.findByRole("heading", { level: 1, name: "Not allowed" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Back to home" })).toHaveAttribute("href", "/");
  });

  it("shows the not-allowed page once an admin stops being one", async () => {
    let role: "admin" | "user" = "admin";
    server.use(
      http.get("/api/v1/account/me", () => HttpResponse.json({ ...grace, role })),
      http.get("/api/v1/admin/users", () => {
        role = "user";
        return apiError(403, "forbidden", "only an admin may do this");
      }),
    );
    renderApp("/admin");

    expect(await screen.findByRole("heading", { level: 1, name: "Not allowed" })).toBeVisible();
    const nav = within(screen.getByRole("navigation", { name: "Main" }));
    await waitFor(() => expect(nav.queryByRole("link", { name: "Admin" })).toBeNull());
  });

  it("says so when who is signed in cannot be checked, and tries again", async () => {
    server.use(http.get("/api/v1/account/me", () => HttpResponse.error(), { once: true }));
    const { user } = renderApp("/");

    expect(
      await screen.findByRole("heading", { name: "Could not check who is signed in" }),
    ).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent("Could not reach the server");
    server.use(signedInAs(ada));
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
  });

  it("reports a server that never answers who is signed in as unreachable", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
    server.use(http.get("/api/v1/account/me", () => new Promise<never>(() => {})));
    renderApp("/");

    await vi.advanceTimersByTimeAsync(META_TIMEOUT_MS);
    expect(
      await screen.findByRole("heading", { name: "Could not check who is signed in" }),
    ).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent("Could not reach the server");
  });
});
