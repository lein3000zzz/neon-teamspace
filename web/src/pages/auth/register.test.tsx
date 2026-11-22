import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { Schemas } from "@/api/client";
import { ada, apiError, authConfig, fakeSession, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

type User = ReturnType<typeof renderApp>["user"];

/** The server's sign-up, recording what it was sent. */
function registration(answer: Partial<Schemas["Registered"]> = {}) {
  const bodies: Schemas["RegisterRequest"][] = [];
  server.use(
    http.post("/api/v1/auth/register", async ({ request }) => {
      bodies.push((await request.json()) as Schemas["RegisterRequest"]);
      const registered: Schemas["Registered"] = { user: ada, must_verify: false, ...answer };
      return HttpResponse.json(registered, { status: 201 });
    }),
  );
  return bodies;
}

/** Types into each field in turn, as a person would, then submits. */
async function fillForm(user: User, fields: Record<string, string>) {
  await Object.entries(fields).reduce(async (previous, [label, value]) => {
    await previous;
    await user.type(await screen.findByLabelText(label), value);
  }, Promise.resolve());
  await user.click(screen.getByRole("button", { name: "Create account" }));
}

describe("creating an account", () => {
  it("explains that sign-ups are closed, with no form", async () => {
    server.use(signedOut, authConfig({ registration: "closed" }));
    renderApp("/register");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Sign-ups are closed" }),
    ).toBeVisible();
    expect(screen.getByText(/Admins add the accounts/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Email")).toBeNull();
    expect(screen.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/login");
  });

  it("signs straight in after an open sign-up", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers, authConfig({ registration: "open" }));
    const bodies = registration();
    const { user, router } = renderApp("/register");

    await fillForm(user, {
      Email: "ada@example.com",
      "Display name": "Ada Lovelace",
      Password: "correct horse battery",
    });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Nothing here yet" }),
    ).toBeVisible();
    expect(router.state.location.pathname).toBe("/");
    expect(bodies).toEqual([
      { email: "ada@example.com", password: "correct horse battery", display_name: "Ada Lovelace" },
    ]);
    expect(session.logins).toEqual([
      { email: "ada@example.com", password: "correct horse battery" },
    ]);
    expect(screen.queryByLabelText("Invite code")).toBeNull();
  });

  it("takes the code from an invite link, out of the address, and fills in its bound address", async () => {
    const session = fakeSession(ada, "correct horse battery");
    let looked: unknown;
    server.use(
      ...session.handlers,
      http.post("/api/v1/auth/invite", async ({ request }) => {
        looked = await request.json();
        return HttpResponse.json({ email: "ada@example.com" });
      }),
    );
    const bodies = registration();
    const { user, router } = renderApp("/register?invite=Spring-Team-2026");

    const email = await screen.findByLabelText("Email");
    expect(await screen.findByDisplayValue("ada@example.com")).toBe(email);
    expect(email).toHaveAttribute("readonly");
    expect(email).toHaveAccessibleDescription("The invite is for this address alone.");
    expect(screen.getByLabelText("Invite code")).toHaveValue("Spring-Team-2026");
    expect(router.state.location.search).toEqual({});
    expect(router.state.location.href).toBe("/register");
    expect(looked).toEqual({ code: "Spring-Team-2026" });

    await fillForm(user, { Password: "correct horse battery" });
    expect(
      await screen.findByRole("heading", { level: 1, name: "Nothing here yet" }),
    ).toBeVisible();
    expect(bodies[0]).toEqual({
      email: "ada@example.com",
      password: "correct horse battery",
      invite_code: "Spring-Team-2026",
    });
  });

  it("keeps a numeric code as typed", async () => {
    server.use(
      signedOut,
      http.post("/api/v1/auth/invite", () => HttpResponse.json({})),
    );
    renderApp("/register?invite=12345678");

    expect(await screen.findByLabelText("Invite code")).toHaveValue("12345678");
  });

  it("says when the invite of a link does not work", async () => {
    server.use(
      signedOut,
      http.post("/api/v1/auth/invite", () =>
        apiError(400, "invalid_invite", "invalid or expired invite"),
      ),
    );
    renderApp("/register?invite=used-up-code");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("This invite does not work");
    expect(alert).toHaveTextContent("Invalid or expired invite.");
  });

  it("asks for a code by invite and offers to ask for one when requests are on", async () => {
    server.use(signedOut, authConfig({ requests: true }));
    const bodies = registration();
    const { user } = renderApp("/register");

    expect(await screen.findByText(/you need a code from an admin/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Ask for one" })).toHaveAttribute("href", "/join");
    await fillForm(user, { Email: "ada@example.com", Password: "correct horse battery" });

    expect(screen.getByLabelText("Invite code")).toHaveAccessibleDescription("Fill this in.");
    expect(screen.getByLabelText("Invite code")).toHaveFocus();
    expect(bodies).toEqual([]);
  });

  it("holds passwords to the minimum the server sets", async () => {
    server.use(signedOut, authConfig({ registration: "open", min_password_length: 16 }));
    const bodies = registration();
    const { user } = renderApp("/register");

    expect(await screen.findByText("At least 16 characters.")).toBeInTheDocument();
    await fillForm(user, { Email: "ada@example.com", Password: "fifteen chars.." });

    expect(screen.getByLabelText("Password")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByLabelText("Password")).toHaveAccessibleDescription(
      "At least 16 characters.",
    );
    expect(bodies).toEqual([]);
  });

  it("puts the server's refusal on the field it is about", async () => {
    server.use(
      signedOut,
      authConfig({ registration: "open" }),
      http.post("/api/v1/auth/register", () =>
        apiError(409, "email_taken", "an account has this address already"),
      ),
    );
    const { user } = renderApp("/register");

    await fillForm(user, { Email: "ada@example.com", Password: "correct horse battery" });

    expect(await screen.findByText("An account has this address already.")).toBeInTheDocument();
    expect(screen.getByLabelText("Email")).toHaveAttribute("aria-invalid", "true");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("waits for the mailed link when addresses are verified", async () => {
    let resent = 0;
    server.use(
      signedOut,
      authConfig({ registration: "open", verify_email: true }),
      http.post("/api/v1/auth/verification", () => {
        resent += 1;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    registration({ must_verify: true });
    const { user } = renderApp("/register");

    await fillForm(user, { Email: "ada@example.com", Password: "correct horse battery" });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Check your inbox" }),
    ).toBeVisible();
    expect(screen.getByText(/A link to verify ada@example.com is on its way/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Send the link again" }));
    expect(await screen.findByText(/a new link is on its way/)).toBeInTheDocument();
    expect(resent).toBe(1);
  });

  it("sends to the sign-in form when signing in right after fails", async () => {
    server.use(
      signedOut,
      authConfig({ registration: "open" }),
      http.post("/api/v1/auth/login", () =>
        apiError(429, "rate_limited", "too many attempts: try again in 2m", { retryAfter: 90 }),
      ),
    );
    registration();
    const { user } = renderApp("/register");

    await fillForm(user, { Email: "ada@example.com", Password: "correct horse battery" });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Your account is ready" }),
    ).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent("Try again in 2 minutes.");
    expect(screen.getByRole("link", { name: "Sign in" })).toHaveAttribute("href", "/login");
  });

  it("reports an unreachable server while reading the settings, and retries", async () => {
    server.use(
      signedOut,
      http.get("/api/v1/auth/config", () => HttpResponse.error(), { once: true }),
    );
    const { user } = renderApp("/register");

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not reach the server");
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByLabelText("Invite code")).toBeInTheDocument();
  });
});
