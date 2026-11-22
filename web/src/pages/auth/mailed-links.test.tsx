import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { apiError, authConfig, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

/** A fresh answer each time: a response's body can be read only once. */
const deadLink = () =>
  apiError(
    400,
    "invalid_link",
    "this link does not work any more: it was used already, or it is too old",
  );

describe("verifying an address", () => {
  it("verifies once, then opens the sign-in form with the address filled in", async () => {
    const tokens: unknown[] = [];
    server.use(
      signedOut,
      http.post("/api/v1/auth/verify-email", async ({ request }) => {
        tokens.push(await request.json());
        return HttpResponse.json({ email: "ada@example.com" });
      }),
    );
    const { router } = renderApp("/verify?token=tok_verify");

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Your address is verified.");
    expect(screen.getByLabelText("Email")).toHaveValue("ada@example.com");
    expect(tokens).toEqual([{ token: "tok_verify" }]);
    expect(router.state.location.href).toBe("/login");
    expect(router.history.location.href).not.toContain("tok_verify");
  });

  it("offers a new link when this one does not work", async () => {
    let resent: unknown;
    server.use(
      signedOut,
      http.post("/api/v1/auth/verify-email", deadLink),
      http.post("/api/v1/auth/verification", async ({ request }) => {
        resent = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user, router } = renderApp("/verify?token=tok_old");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link does not work" }),
    ).toBeVisible();
    expect(router.state.location.href).toBe("/verify");
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.click(screen.getByRole("button", { name: "Send the link again" }));

    expect(await screen.findByText(/a new link is on its way/)).toBeInTheDocument();
    expect(resent).toEqual({ email: "ada@example.com" });
  });

  it("says when mail is not set up to send a new link", async () => {
    server.use(
      signedOut,
      http.post("/api/v1/auth/verify-email", deadLink),
      http.post("/api/v1/auth/verification", () =>
        apiError(409, "mail_not_set_up", "mail is not set up"),
      ),
    );
    const { user } = renderApp("/verify?token=tok_old");

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    await user.click(screen.getByRole("button", { name: "Send the link again" }));
    expect(await screen.findByText("Mail is not set up.")).toBeInTheDocument();
  });
});

describe("forgetting a password", () => {
  it("says resets are off while they are", async () => {
    server.use(signedOut);
    renderApp("/forgot");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Password resets are off" }),
    ).toBeVisible();
  });

  it("answers the same whatever the address", async () => {
    let body: unknown;
    server.use(
      signedOut,
      authConfig({ password_reset: true }),
      http.post("/api/v1/auth/password-reset", async ({ request }) => {
        body = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user } = renderApp("/forgot");

    await user.type(await screen.findByLabelText("Email"), "nobody@example.com");
    await user.click(screen.getByRole("button", { name: "Send the link" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Check your inbox" }),
    ).toBeVisible();
    expect(screen.getByText(/If this address has an account/)).toBeInTheDocument();
    expect(body).toEqual({ email: "nobody@example.com" });
  });

  it("carries the address over from the sign-in form", async () => {
    server.use(signedOut, authConfig({ password_reset: true }));
    const { user } = renderApp("/login");

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    await user.click(await screen.findByRole("link", { name: "Forgot your password?" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Forgot your password?" }),
    ).toBeVisible();
    expect(screen.getByLabelText("Email")).toHaveValue("ada@example.com");
  });
});

describe("setting a new password", () => {
  it("sets it with the token, then opens the sign-in form", async () => {
    let body: unknown;
    server.use(
      signedOut,
      authConfig({ password_reset: true }),
      http.post("/api/v1/auth/password-reset/confirm", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ email: "ada@example.com" });
      }),
    );
    const { user, router } = renderApp("/reset?token=tok_reset");

    const password = await screen.findByLabelText("New password");
    expect(router.state.location.href).toBe("/reset");
    await user.type(password, "a much longer passphrase");
    await user.type(screen.getByLabelText("Repeat the new password"), "a much longer passphrase");
    await user.click(screen.getByRole("button", { name: "Set the password" }));

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Your password is set");
    expect(screen.getByLabelText("Email")).toHaveValue("ada@example.com");
    expect(body).toEqual({ token: "tok_reset", password: "a much longer passphrase" });
  });

  it("checks the two passwords agree and meet the minimum", async () => {
    let sent = false;
    server.use(
      signedOut,
      http.post("/api/v1/auth/password-reset/confirm", () => {
        sent = true;
        return HttpResponse.json({ email: "ada@example.com" });
      }),
    );
    const { user } = renderApp("/reset?token=tok_reset");

    await user.type(await screen.findByLabelText("New password"), "short");
    await user.type(screen.getByLabelText("Repeat the new password"), "shorter");
    await user.click(screen.getByRole("button", { name: "Set the password" }));

    expect(screen.getByLabelText("New password")).toHaveAccessibleDescription(
      "At least 12 characters.",
    );
    expect(screen.getByLabelText("Repeat the new password")).toHaveAccessibleDescription(
      "The two passwords differ.",
    );
    expect(sent).toBe(false);
  });

  it("offers a new link when the link is spent", async () => {
    server.use(signedOut, http.post("/api/v1/auth/password-reset/confirm", deadLink));
    const { user } = renderApp("/reset?token=tok_old");

    await user.type(await screen.findByLabelText("New password"), "a much longer passphrase");
    await user.type(screen.getByLabelText("Repeat the new password"), "a much longer passphrase");
    await user.click(screen.getByRole("button", { name: "Set the password" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("This link does not work any more");
    expect(screen.getByRole("link", { name: "Ask for a new link" })).toHaveAttribute(
      "href",
      "/forgot",
    );
    expect(screen.getByRole("button", { name: "Set the password" })).toBeDisabled();
  });

  it("says when the address carries no token", async () => {
    server.use(signedOut);
    renderApp("/reset");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link is incomplete" }),
    ).toBeVisible();
  });
});
