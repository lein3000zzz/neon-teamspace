import { screen, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { Schemas } from "@/api/client";
import { ada, apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

async function renderProfile() {
  const app = renderApp("/account");
  await screen.findByRole("heading", { level: 2, name: "Profile" });
  return app;
}

describe("the profile", () => {
  it("shows the account's address, role and verification", async () => {
    await renderProfile();

    const profile = within(screen.getByRole("region", { name: "Profile" }));
    expect(profile.getByText(ada.email)).toBeInTheDocument();
    expect(profile.getByText("Verified")).toBeInTheDocument();
    expect(profile.getByText("Member")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Account" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Profile" })).toHaveAttribute("aria-current", "page");
  });

  it("renames the account, and the sidebar follows", async () => {
    let body: unknown;
    server.use(
      http.patch("/api/v1/account/me", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ...ada, display_name: "Ada Lovelace" });
      }),
    );
    const { user } = await renderProfile();

    const name = screen.getByLabelText("Display name");
    await user.clear(name);
    await user.type(name, "  Ada Lovelace ");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Saved.")).toBeInTheDocument();
    expect(body).toEqual({ display_name: "Ada Lovelace" });
    expect(within(screen.getByRole("complementary")).getByText("Ada Lovelace")).toBeInTheDocument();
  });

  it("refuses a name over 100 characters before sending it", async () => {
    const { user } = await renderProfile();

    const name = screen.getByLabelText("Display name");
    await user.clear(name);
    await user.click(name);
    await user.paste("a".repeat(101));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(name).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("At most 100 characters, on one line.")).toBeInTheDocument();
  });
});

/** Fills in the password form and submits it. */
async function change(
  user: ReturnType<typeof renderApp>["user"],
  current: string,
  next: string,
  repeat = next,
) {
  await user.type(screen.getByLabelText("Current password"), current);
  await user.type(screen.getByLabelText("New password"), next);
  await user.type(screen.getByLabelText("Repeat the new password"), repeat);
  await user.click(screen.getByRole("button", { name: "Change password" }));
}

describe("changing the password", () => {
  it("changes it and says the other sessions are signed out", async () => {
    let body: Schemas["ChangePassword"] | undefined;
    server.use(
      http.post("/api/v1/account/password", async ({ request }) => {
        body = (await request.json()) as Schemas["ChangePassword"];
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user } = await renderProfile();

    await change(user, "correct horse battery", "an even longer passphrase");

    expect(await screen.findByText(/Password changed/)).toBeInTheDocument();
    expect(body).toEqual({
      current_password: "correct horse battery",
      new_password: "an even longer passphrase",
    });
    expect(screen.getByLabelText("Current password")).toHaveValue("");
  });

  it("marks a wrong current password without signing out", async () => {
    server.use(
      http.post("/api/v1/account/password", () =>
        apiError(401, "invalid_credentials", "invalid email or password"),
      ),
    );
    const { user, router } = await renderProfile();

    await change(user, "not my password", "an even longer passphrase");

    expect(await screen.findByText("That is not your current password.")).toBeInTheDocument();
    expect(screen.getByLabelText("Current password")).toHaveAttribute("aria-invalid", "true");
    expect(router.state.location.pathname).toBe("/account");
  });

  it("checks the new password against the server's minimum", async () => {
    const { user } = await renderProfile();

    await change(user, "correct horse battery", "too short");

    expect(screen.getByLabelText("New password")).toHaveAccessibleDescription(
      "At least 12 characters.",
    );
  });

  it("puts the server's refusal of the new password on its field", async () => {
    server.use(
      http.post("/api/v1/account/password", () =>
        apiError(400, "weak_password", "a password cannot be only spaces"),
      ),
    );
    const { user } = await renderProfile();

    await change(user, "correct horse battery", "a password that passes here");

    expect(await screen.findByText("A password cannot be only spaces.")).toBeInTheDocument();
    expect(screen.getByLabelText("New password")).toHaveAttribute("aria-invalid", "true");
  });

  it("gives the wait when rate limited", async () => {
    server.use(
      http.post("/api/v1/account/password", () =>
        apiError(429, "rate_limited", "too many attempts: try again in 1h", { retryAfter: 3600 }),
      ),
    );
    const { user } = await renderProfile();

    await change(user, "correct horse battery", "an even longer passphrase");

    expect(await screen.findByRole("alert")).toHaveTextContent("Try again in 1 hour.");
  });

  it("checks the two new passwords agree", async () => {
    const { user } = await renderProfile();

    await change(user, "correct horse battery", "an even longer passphrase", "something else");

    expect(screen.getByText("The two passwords differ.")).toBeInTheDocument();
  });
});
