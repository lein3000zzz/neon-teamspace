import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { apiError, authConfig, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const SENT = /If this address can ask, a link to confirm the request is on its way/;

describe("asking for an invite", () => {
  it("says requests are off while they are", async () => {
    server.use(signedOut);
    renderApp("/join");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Requests to join are off" }),
    ).toBeVisible();
    expect(screen.queryByLabelText("Email")).toBeNull();
  });

  it("says the same afterwards whatever the address, naming none", async () => {
    let body: unknown;
    server.use(
      signedOut,
      authConfig({ requests: true }),
      http.post("/api/v1/auth/join-requests", async ({ request }) => {
        body = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { user } = renderApp("/join");

    await user.type(await screen.findByLabelText("Email"), " ada@example.com ");
    await user.type(screen.getByLabelText("Note for the admins"), "  From the hackathon team. ");
    await user.click(screen.getByRole("button", { name: "Ask for an invite" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Check your inbox" }),
    ).toBeVisible();
    expect(screen.getByText(SENT)).toBeInTheDocument();
    expect(screen.queryByText(/ada@example.com/)).toBeNull();
    expect(body).toEqual({ email: "ada@example.com", note: "From the hackathon team." });
  });

  it("counts the note's characters and refuses more than 500", async () => {
    server.use(signedOut, authConfig({ requests: true }));
    const { user } = renderApp("/join");

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    const note = screen.getByLabelText("Note for the admins");
    await user.click(note);
    await user.paste("é".repeat(501));
    expect(screen.getByText("Optional. 501 of 500 characters.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Ask for an invite" }));

    expect(note).toHaveAccessibleDescription(
      "At most 500 characters. Optional. 501 of 500 characters.",
    );
    expect(note).toHaveFocus();
  });

  it("gives the wait when rate limited", async () => {
    server.use(
      signedOut,
      authConfig({ requests: true }),
      http.post("/api/v1/auth/join-requests", () =>
        apiError(429, "rate_limited", "too many attempts: try again in 45s", { retryAfter: 45 }),
      ),
    );
    const { user } = renderApp("/join");

    await user.type(await screen.findByLabelText("Email"), "ada@example.com");
    await user.click(screen.getByRole("button", { name: "Ask for an invite" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Try again in 45 seconds.");
  });
});

describe("confirming a request to join", () => {
  it("confirms once, with the token out of the address", async () => {
    const tokens: unknown[] = [];
    server.use(
      signedOut,
      http.post("/api/v1/auth/join-requests/confirm", async ({ request }) => {
        tokens.push(await request.json());
        return new HttpResponse(null, { status: 204 });
      }),
    );
    const { router } = renderApp("/join/confirm?token=tok_confirm");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Your request is with the admins" }),
    ).toBeVisible();
    expect(tokens).toEqual([{ token: "tok_confirm" }]);
    expect(router.state.location.href).toBe("/join/confirm");
  });

  it("says when the link does not work any more", async () => {
    server.use(
      signedOut,
      http.post("/api/v1/auth/join-requests/confirm", () =>
        apiError(
          400,
          "invalid_link",
          "this link does not work any more: it was used already, or it is too old",
        ),
      ),
    );
    renderApp("/join/confirm?token=tok_old");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link does not work" }),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Ask again" })).toHaveAttribute("href", "/join");
  });

  it("says when the address carries no token", async () => {
    server.use(signedOut);
    renderApp("/join/confirm");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link is incomplete" }),
    ).toBeVisible();
  });
});
