import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { ApiToken } from "@/api/account";
import type { Schemas } from "@/api/client";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const SECRET = "nw_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcd";

const deploy: ApiToken = {
  id: "0193a5b2-0000-7000-8000-0000000000a1",
  name: "Deploy script",
  prefix: "nw_AbCdEfG",
  scopes: ["read", "write"],
  created_at: "2026-11-20T09:00:00Z",
  last_used_at: null,
  expires_at: null,
};

/** The account's tokens on the server, recording what was made and revoked. */
function tokens(initial: ApiToken[] = []) {
  let list = initial;
  const made: Schemas["CreateApiToken"][] = [];
  const revoked: string[] = [];
  server.use(
    http.get("/api/v1/account/tokens", () => HttpResponse.json({ tokens: list })),
    http.post("/api/v1/account/tokens", async ({ request }) => {
      const body = (await request.json()) as Schemas["CreateApiToken"];
      made.push(body);
      const token: ApiToken = { ...deploy, name: body.name, scopes: body.scopes };
      list = [...list, token];
      return HttpResponse.json({ token: SECRET, api_token: token }, { status: 201 });
    }),
    http.delete("/api/v1/account/tokens/:id", ({ params }) => {
      revoked.push(String(params["id"]));
      list = list.filter((token) => token.id !== params["id"]);
      return new HttpResponse(null, { status: 204 });
    }),
  );
  return { made, revoked };
}

describe("API tokens", () => {
  it("makes a token and shows its secret once", async () => {
    const api = tokens();
    const { user } = renderApp("/account/tokens");

    expect(await screen.findByText("No tokens yet.")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Name"), "CI deploys");
    await user.click(screen.getByRole("checkbox", { name: "Write" }));
    await user.click(screen.getByRole("button", { name: "Make token" }));

    expect(await screen.findByText("Copy the token “CI deploys” now")).toBeInTheDocument();
    expect(screen.getByDisplayValue(SECRET)).toHaveAttribute("readonly");
    expect(api.made).toEqual([{ name: "CI deploys", scopes: ["read", "write"] }]);

    await user.click(screen.getByRole("button", { name: /^Copy/ }));
    expect(await navigator.clipboard.readText()).toBe(SECRET);
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByDisplayValue(SECRET)).toBeNull();
    expect(screen.getByLabelText("Name")).toBeInTheDocument();
  });

  it("offers the admin scope to admins only", async () => {
    tokens();
    renderApp("/account/tokens");
    await screen.findByLabelText("Name");
    expect(screen.queryByRole("checkbox", { name: "Admin" })).toBeNull();
  });

  it("lets an admin's token have the admin scope", async () => {
    server.use(signedInAs(grace));
    tokens();
    renderApp("/account/tokens");
    expect(await screen.findByRole("checkbox", { name: "Admin" })).not.toBeChecked();
  });

  it("checks the form before sending it", async () => {
    const api = tokens();
    const { user } = renderApp("/account/tokens");

    await user.click(await screen.findByRole("checkbox", { name: "Read" }));
    await user.click(screen.getByRole("button", { name: "Make token" }));

    expect(screen.getByLabelText("Name")).toHaveAccessibleDescription("Fill this in.");
    expect(screen.getByText("Pick at least one.")).toBeInTheDocument();
    expect(api.made).toEqual([]);
  });

  it("sends an end date as the end of that day", async () => {
    const api = tokens();
    const { user } = renderApp("/account/tokens");

    await user.type(await screen.findByLabelText("Name"), "Short-lived");
    await user.type(screen.getByLabelText("Expires"), "2099-01-31");
    await user.click(screen.getByRole("button", { name: "Make token" }));

    await waitFor(() => expect(api.made).toHaveLength(1));
    expect(api.made[0]?.expires_at).toBe(new Date("2099-01-31T23:59:59").toISOString());
  });

  it("revokes a token once confirmed", async () => {
    const api = tokens([deploy]);
    const { user } = renderApp("/account/tokens");

    await user.click(await screen.findByRole("button", { name: "Revoke Deploy script" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Revoke “Deploy script”?" });
    await user.click(within(dialog).getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("No tokens yet.")).toBeInTheDocument();
    expect(api.revoked).toEqual([deploy.id]);
  });

  it("keeps the token when the question is cancelled", async () => {
    const api = tokens([deploy]);
    const { user } = renderApp("/account/tokens");

    await user.click(await screen.findByRole("button", { name: "Revoke Deploy script" }));
    await user.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.getByText("Deploy script")).toBeInTheDocument();
    expect(api.revoked).toEqual([]);
  });

  it("says why a token was refused", async () => {
    tokens();
    server.use(
      http.post("/api/v1/account/tokens", () =>
        apiError(403, "forbidden", "only a browser session may make tokens"),
      ),
    );
    const { user } = renderApp("/account/tokens");

    await user.type(await screen.findByLabelText("Name"), "Any");
    await user.click(screen.getByRole("button", { name: "Make token" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("You are not allowed to do this.");
  });
});
