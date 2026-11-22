import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { CreatedInvite, Invite } from "@/api/admin";
import type { Schemas } from "@/api/client";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const CODE = "inv_4yVd0q3oJ2pLmN8sTuVwXyZaBcDeFgHiJkLmNoPqRsT";

const open: Invite = {
  id: "0193a5b2-0000-7000-8000-0000000000c1",
  created_at: "2026-11-20T09:00:00Z",
  expires_at: "2026-11-27T09:00:00Z",
  max_uses: 5,
  uses: 2,
  status: "open",
  code_kept: true,
  signups: [
    {
      user_id: "0193a5b2-0000-7000-8000-0000000000d1",
      email: "ada@example.com",
      used_at: "2026-11-20T10:00:00Z",
    },
    { user_id: null, email: null, used_at: "2026-11-20T11:00:00Z" },
  ],
};

const revoked: Invite = {
  ...open,
  id: "0193a5b2-0000-7000-8000-0000000000c2",
  created_at: "2026-11-19T09:00:00Z",
  status: "revoked",
  uses: 0,
  signups: [],
  code_kept: false,
};

/** The invites on the server, recording what was made and revoked. */
function invites(list: Invite[], { byEmail = false, kept = true } = {}) {
  const made: Schemas["CreateInvite"][] = [];
  const revokedIds: string[] = [];
  // A copy, as revoking changes it in place.
  let current = structuredClone(list);
  server.use(
    signedInAs(grace),
    http.get("/api/v1/admin/invites", () =>
      HttpResponse.json({ invites: current, by_email: byEmail }),
    ),
    http.post("/api/v1/admin/invites", async ({ request }) => {
      const body = (await request.json()) as Schemas["CreateInvite"];
      made.push(body);
      const invite: Invite = {
        ...open,
        id: "0193a5b2-0000-7000-8000-0000000000c9",
        max_uses: body.max_uses ?? 1,
        uses: 0,
        signups: [],
        code_kept: kept,
        ...(body.email ? { email: body.email } : {}),
      };
      current = [invite, ...current];
      const created: CreatedInvite = {
        invite,
        code: body.code ?? CODE,
        ...(body.email ? { queued: true } : {}),
      };
      return HttpResponse.json(created, { status: 201 });
    }),
    http.post("/api/v1/admin/invites/check", async ({ request }) => {
      const { code } = (await request.json()) as { code: string };
      return HttpResponse.json(
        code.toLowerCase() === "spring-team" ? { problem: "another invite has this code" } : {},
      );
    }),
    http.get("/api/v1/admin/invites/:id/code", () => HttpResponse.json({ code: CODE })),
    http.post("/api/v1/admin/invites/:id/revoke", ({ params }) => {
      revokedIds.push(String(params["id"]));
      const target = current.find((invite) => invite.id === params["id"]);
      if (target) {
        target.status = "revoked";
      }
      return new HttpResponse(null, { status: 204 });
    }),
  );
  return { made, revoked: revokedIds };
}

const LINK = `${window.location.origin}/register?invite=${CODE}`;

describe("invites", () => {
  it("lists each invite with its state and who signed up", async () => {
    invites([open, revoked]);
    const { user } = renderApp("/admin/invites");

    expect(await screen.findByText("2 of 5 sign-ups")).toBeInTheDocument();
    expect(screen.getByText("Revoked")).toBeInTheDocument();
    expect(screen.getByText(/People need an invite/)).toBeInTheDocument();

    const toggle = screen.getByRole("button", { name: "Who signed up (2)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("ada@example.com")).toBeVisible();
    expect(screen.getByText("A deleted account")).toBeVisible();
  });

  it("makes an invite with a code the server makes, and shows its link once", async () => {
    const api = invites([], { kept: false });
    const { user } = renderApp("/admin/invites");

    expect(await screen.findByText("No invites yet.")).toBeInTheDocument();
    const form = within(screen.getByRole("form", { name: "Make an invite" }));
    await user.clear(form.getByLabelText("Sign-ups"));
    await user.type(form.getByLabelText("Sign-ups"), "3");
    await user.click(form.getByRole("button", { name: "Make invite" }));

    expect(await screen.findByText("Invite made: share its link")).toBeInTheDocument();
    expect(screen.getByLabelText("Invite link")).toHaveValue(LINK);
    expect(screen.getByLabelText("Invite code")).toHaveValue(CODE);
    expect(screen.getByText(/this is the only time it shows/)).toBeInTheDocument();
    expect(api.made).toEqual([{ max_uses: 3 }]);
    expect(await screen.findByText("0 of 3 sign-ups")).toBeInTheDocument();
  });

  it("checks a typed code as it is typed", async () => {
    const api = invites([]);
    const { user } = renderApp("/admin/invites");

    const code = await screen.findByLabelText("Code");
    await user.type(code, "spring-team");
    await waitFor(() => expect(code).toHaveAccessibleDescription("Another invite has this code."));
    expect(screen.getByRole("button", { name: "Make invite" })).toBeDisabled();

    await user.type(code, "-2026");
    await waitFor(() => expect(code).toHaveAccessibleDescription("Free to take."));
    await user.click(screen.getByRole("button", { name: "Make invite" }));
    await waitFor(() => expect(api.made).toEqual([{ code: "spring-team-2026", max_uses: 1 }]));
  });

  it("refuses a code of the wrong form and an out-of-range count before sending", async () => {
    const api = invites([]);
    const { user } = renderApp("/admin/invites");

    await user.type(await screen.findByLabelText("Code"), "no spaces!");
    await user.clear(screen.getByLabelText("Sign-ups"));
    await user.type(screen.getByLabelText("Sign-ups"), "1001");
    await user.click(screen.getByRole("button", { name: "Make invite" }));

    expect(screen.getByLabelText("Code")).toHaveAccessibleDescription(
      "8 to 64 letters, digits, - and _.",
    );
    expect(screen.getByLabelText("Sign-ups")).toHaveAttribute("aria-invalid", "true");
    expect(api.made).toEqual([]);
  });

  it("shows the link of an invite whose code was kept", async () => {
    invites([open, revoked]);
    const { user } = renderApp("/admin/invites");

    await user.click(await screen.findByRole("button", { name: /^Show the link of the invite/ }));
    expect(await screen.findByLabelText("Invite link")).toHaveValue(LINK);
  });

  it("revokes an open invite once confirmed", async () => {
    const api = invites([open]);
    const { user } = renderApp("/admin/invites");

    await user.click(await screen.findByRole("button", { name: /^Revoke the invite made/ }));
    const dialog = await screen.findByRole("alertdialog", { name: "Revoke this invite?" });
    await user.click(within(dialog).getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("Revoked")).toBeInTheDocument();
    expect(api.revoked).toEqual([open.id]);
    expect(screen.queryByRole("button", { name: /^Revoke the invite made/ })).toBeNull();
  });

  it("mails an invite while mail is set up", async () => {
    const api = invites([], { byEmail: true });
    const { user } = renderApp("/admin/invites");

    const form = within(await screen.findByRole("form", { name: "Invite by email" }));
    await user.type(form.getByLabelText("Invite by email"), "linus@example.com");
    await user.click(form.getByRole("button", { name: "Mail the invite" }));

    expect(await screen.findByText("Invite on its way to linus@example.com")).toBeInTheDocument();
    expect(screen.getByText(/Only linus@example.com can use it/)).toBeInTheDocument();
    expect(api.made).toEqual([{ email: "linus@example.com" }]);
  });

  it("hides inviting by email while mail is not set up", async () => {
    invites([]);
    renderApp("/admin/invites");

    await screen.findByText("No invites yet.");
    expect(screen.queryByRole("form", { name: "Invite by email" })).toBeNull();
  });

  it("says why an invite was refused", async () => {
    invites([]);
    server.use(
      http.post("/api/v1/admin/invites", () =>
        apiError(400, "invalid_input", "an invite ends after now and within a year"),
      ),
    );
    const { user } = renderApp("/admin/invites");

    await user.click(await screen.findByRole("button", { name: "Make invite" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "An invite ends after now and within a year.",
    );
    expect(screen.getByLabelText("Code")).not.toHaveAttribute("aria-invalid", "true");
  });

  it("shows a code taken since it was checked under the code", async () => {
    invites([]);
    server.use(
      http.post("/api/v1/admin/invites", () =>
        apiError(409, "invite_code_taken", "another invite has this code"),
      ),
    );
    const { user } = renderApp("/admin/invites");

    await user.type(await screen.findByLabelText("Code"), "autumn-team");
    await user.click(screen.getByRole("button", { name: "Make invite" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Code")).toHaveAccessibleDescription(
        "Another invite has this code.",
      ),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
