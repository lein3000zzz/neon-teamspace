import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { ada, apiError, authConfig, fakeSession, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

async function fill(user: ReturnType<typeof renderApp>["user"], email: string, password: string) {
  await user.type(await screen.findByLabelText("Email"), email);
  await user.type(screen.getByLabelText("Password"), password);
  await user.click(screen.getByRole("button", { name: "Sign in" }));
}

describe("signing in", () => {
  it("signs in and goes home", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    const { user, router } = renderApp("/login");

    await fill(user, "ada@example.com", "correct horse battery");

    expect(
      await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    expect(router.state.location.pathname).toBe("/");
  });

  it("says why signing in failed, without saying which half was wrong", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    const { user, router } = renderApp("/login");

    await fill(user, "ada@example.com", "wrong password");

    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid email or password.");
    expect(router.state.location.pathname).toBe("/login");
  });

  it("checks the form before sending it", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    const { user } = renderApp("/login");

    await user.type(await screen.findByLabelText("Email"), "not an address");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(screen.getByLabelText("Email")).toHaveAccessibleDescription(
      "Enter an address like name@example.com.",
    );
    expect(screen.getByLabelText("Email")).toHaveFocus();
    expect(screen.getByLabelText("Password")).toHaveAttribute("aria-invalid", "true");
    expect(session.logins).toEqual([]);
  });

  it("gives the wait from Retry-After when rate limited", async () => {
    server.use(
      signedOut,
      http.post("/api/v1/auth/login", () =>
        apiError(429, "rate_limited", "too many attempts: try again in 30s", { retryAfter: 30 }),
      ),
    );
    const { user } = renderApp("/login");

    await fill(user, "ada@example.com", "anything at all");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Too many attempts. Try again in 30 seconds.",
    );
  });

  it("offers a new link to an address still to verify", async () => {
    let resent: unknown;
    server.use(
      signedOut,
      http.post("/api/v1/auth/login", () =>
        apiError(403, "email_not_verified", "verify your email first: open the link mailed to you"),
      ),
      http.post("/api/v1/auth/verification", async ({ request }) => {
        resent = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user } = renderApp("/login");

    await fill(user, "ada@example.com", "correct horse battery");
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Verify your email first");
    await user.click(within(alert).getByRole("button", { name: "Send the link again" }));

    expect(await screen.findByText(/a new link is on its way/)).toBeInTheDocument();
    expect(resent).toEqual({ email: "ada@example.com" });
  });

  it("links the other forms as the settings allow", async () => {
    server.use(
      signedOut,
      authConfig({ registration: "invite", requests: true, password_reset: true }),
    );
    renderApp("/login");

    expect(await screen.findByRole("link", { name: "Create one" })).toHaveAttribute(
      "href",
      "/register",
    );
    expect(screen.getByRole("link", { name: "Ask for one" })).toHaveAttribute("href", "/join");
    expect(screen.getByRole("link", { name: "Forgot your password?" })).toHaveAttribute(
      "href",
      "/forgot",
    );
  });

  it("hides the sign-up, request and reset links while they are off", async () => {
    server.use(signedOut, authConfig({ registration: "closed" }));
    const { queryClient } = renderApp("/login");

    await screen.findByRole("button", { name: "Sign in" });
    await waitFor(() => expect(queryClient.getQueryData(["auth", "config"])).toBeDefined());
    expect(screen.queryByRole("link", { name: "Create one" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Ask for one" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Forgot your password?" })).toBeNull();
  });

  it("never follows a return path to another site", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    const { user, router } = renderApp("/login?next=%2F%2Fevil.example%2Fsteal");

    await fill(user, "ada@example.com", "correct horse battery");

    expect(
      await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    expect(router.state.location.href).toBe("/");
  });

  it("moves a visitor who is signed in already on", async () => {
    const { router } = renderApp("/login?next=%2Faccount");

    expect(await screen.findByRole("heading", { level: 1, name: "Account" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/account");
  });
});
