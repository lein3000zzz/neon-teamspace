import { screen, waitFor, within } from "@testing-library/react";
import { delay, http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { keys } from "@/api/keys";
import type { InviteLink, SentInvitation } from "@/api/workspaces";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { design, fakeWorkspaces, LINK_TOKEN, people, scout, summaryOf } from "@/test/workspaces";

const ID = design().id;
const base = `/api/v1/workspaces/${ID}`;

/** A member's card, by their address. */
async function memberCard(email: string) {
  const region = within(await screen.findByRole("region", { name: "Members" }));
  const item = (await region.findByText(email)).closest("li");
  if (item === null) {
    throw new Error(`no card for ${email}`);
  }
  return within(item);
}

const pending: SentInvitation = {
  id: "0193a5b2-0000-7000-8000-00000000d001",
  invitee: {
    id: "0193a5b2-0000-7000-8000-00000000c00d",
    kind: "human",
    display_name: "Dee",
    email: "dee@example.com",
    hue: 40,
  },
  role: "member",
  created_at: "2026-11-20T09:00:00Z",
  expires_at: "2026-12-04T09:00:00Z",
};

const keptLink: InviteLink = {
  id: "0193a5b2-0000-7000-8000-00000000e001",
  role: "guest",
  max_uses: 5,
  uses: 2,
  status: "open",
  created_at: "2026-11-20T09:00:00Z",
  expires_at: "2026-11-27T09:00:00Z",
  token_kept: true,
};

describe("inviting people", () => {
  it("invites an account by address, with a role", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite by email" }));
    await user.type(form.getByLabelText("Email"), " dee@example.com ");
    await user.selectOptions(form.getByLabelText("Role"), "admin");
    expect(form.getByLabelText("Role")).toHaveAccessibleDescription(/Admins manage members/);
    await user.click(form.getByRole("button", { name: "Invite" }));

    expect(await form.findByText("Invited dee@example.com.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `${base}/invitations`,
        body: { email: "dee@example.com", role: "admin" },
      },
    ]);
    const waiting = within(await screen.findByRole("region", { name: "Pending invitations" }));
    expect(await waiting.findByText("dee@example.com")).toBeInTheDocument();
  });

  it("explains that an address without an account cannot be invited", async () => {
    fakeWorkspaces();
    server.use(
      http.post("/api/v1/workspaces/:id/invitations", () =>
        apiError(404, "no_account", "no account can sign in with this address"),
      ),
    );
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite by email" }));
    await user.type(form.getByLabelText("Email"), "nobody@example.com");
    await user.click(form.getByRole("button", { name: "Invite" }));

    await waitFor(() =>
      expect(form.getByLabelText("Email")).toHaveAccessibleDescription(
        /never create accounts: ask an instance admin for an account/,
      ),
    );
  });

  it("says when the workspace is full, and when to wait before the next lookup", async () => {
    fakeWorkspaces();
    let answer = apiError(409, "member_limit", "the workspace is at its member limit");
    server.use(http.post("/api/v1/workspaces/:id/invitations", () => answer));
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite by email" }));
    await user.type(form.getByLabelText("Email"), "dee@example.com");
    await user.click(form.getByRole("button", { name: "Invite" }));
    expect(await form.findByRole("alert")).toHaveTextContent(/at its member limit/);

    answer = apiError(429, "rate_limited", "too many lookups", { retryAfter: 30 });
    await user.click(form.getByRole("button", { name: "Invite" }));
    expect(
      await form.findByText("Too many attempts. Try again in 30 seconds."),
    ).toBeInTheDocument();
  });

  it("revokes a pending invitation once confirmed", async () => {
    const api = fakeWorkspaces({ invitations: [pending] });
    const { user } = renderApp("/w/design/settings/members");

    await user.click(
      await screen.findByRole("button", { name: "Revoke the invitation of dee@example.com" }),
    );
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Revoke the invitation of dee@example.com?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("No invitations wait for an answer.")).toBeInTheDocument();
    expect(api.changes.map((change) => change.path)).toEqual([
      `${base}/invitations/${pending.id}/revoke`,
    ]);
  });
});

describe("invite links", () => {
  it("makes a link and shows its address once", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite links" }));
    await user.selectOptions(form.getByLabelText("Joins as"), "guest");
    await user.clear(form.getByLabelText("People"));
    await user.type(form.getByLabelText("People"), "3");
    await user.click(form.getByRole("button", { name: "Make link" }));

    expect(await screen.findByText("Invite link made: share it")).toBeInTheDocument();
    expect(screen.getByLabelText("Invite link")).toHaveValue(
      `${window.location.origin}/workspace-invite/${LINK_TOKEN}`,
    );
    expect(screen.getByText(/this is the only time it shows/)).toBeInTheDocument();
    expect(api.changes[0]?.body).toEqual({ role: "guest", max_uses: 3 });
    expect(await screen.findByText("0 of 3 joined")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByLabelText("Invite link")).toBeNull();
    expect(screen.getByRole("form", { name: "Invite links" })).toBeInTheDocument();
  });

  it("refuses a count out of range before sending", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite links" }));
    await user.clear(form.getByLabelText("People"));
    await user.type(form.getByLabelText("People"), "1001");
    await user.click(form.getByRole("button", { name: "Make link" }));

    expect(form.getByLabelText("People")).toHaveAccessibleDescription(
      expect.stringContaining("A whole number from 1 to 1000."),
    );
    expect(api.changes).toEqual([]);
  });

  it("takes a new link's address off the page once the link is revoked", async () => {
    fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const form = within(await screen.findByRole("form", { name: "Invite links" }));
    await user.click(form.getByRole("button", { name: "Make link" }));
    expect(await screen.findByText("Invite link made: share it")).toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: /^Revoke the invite link made/ }));
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Revoke this invite link?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("Revoked")).toBeInTheDocument();
    expect(screen.queryByText("Invite link made: share it")).toBeNull();
    expect(screen.queryByLabelText("Invite link")).toBeNull();
    expect(screen.getByRole("form", { name: "Invite links" })).toBeInTheDocument();
  });

  it("lists links with their uses, copies a kept one again and revokes it", async () => {
    const api = fakeWorkspaces({ links: [keptLink] });
    const { user } = renderApp("/w/design/settings/members");

    expect(await screen.findByText("2 of 5 joined")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Show the invite link made/ }));
    expect(await screen.findByLabelText("Invite link")).toHaveValue(
      `${window.location.origin}/workspace-invite/${LINK_TOKEN}`,
    );

    await user.click(screen.getByRole("button", { name: /^Revoke the invite link made/ }));
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Revoke this invite link?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Revoke" }));

    expect(await screen.findByText("Revoked")).toBeInTheDocument();
    // A revoked link's address admits nobody, so it is no longer there to copy.
    expect(screen.queryByLabelText("Invite link")).toBeNull();
    expect(api.changes.map((change) => change.path)).toEqual([
      `${base}/invite-links/${keptLink.id}/revoke`,
    ]);
  });
});

describe("the members", () => {
  it("changes a role once confirmed", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const cy = await memberCard("cy@example.com");
    await user.selectOptions(cy.getByRole("combobox", { name: "Role of Cy" }), "admin");
    const dialog = within(await screen.findByRole("alertdialog", { name: "Make Cy an admin?" }));
    await user.click(dialog.getByRole("button", { name: "Change role" }));

    expect(await screen.findByText("Cy is an admin now.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "PATCH", path: `${base}/members/${people.cy.id}`, body: { role: "admin" } },
    ]);
    await waitFor(() =>
      expect(cy.getByRole("combobox", { name: "Role of Cy" })).toHaveValue("admin"),
    );
  });

  it("explains why the last owner keeps the role", async () => {
    fakeWorkspaces();
    server.use(
      http.patch("/api/v1/workspaces/:id/members/:user_id", () =>
        apiError(409, "last_owner", "a workspace keeps at least one owner"),
      ),
    );
    const { user } = renderApp("/w/design/settings/members");

    const ada = await memberCard("ada@example.com");
    expect(ada.getByText("You")).toBeInTheDocument();
    await user.selectOptions(ada.getByRole("combobox", { name: "Role of Ada" }), "member");
    const dialog = within(await screen.findByRole("alertdialog", { name: "Make Ada a member?" }));
    expect(dialog.getByText(/This is you/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Change role" }));

    expect(await ada.findByRole("alert")).toHaveTextContent(/keeps at least one owner/);
  });

  it("removes a member, and transfers ownership, each once confirmed", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/members");

    const gus = await memberCard("gus@example.com");
    await user.click(gus.getByRole("button", { name: "Remove Gus" }));
    await user.click(
      within(
        await screen.findByRole("alertdialog", { name: "Remove Gus from the workspace?" }),
      ).getByRole("button", { name: "Remove" }),
    );
    expect(await screen.findByText("Gus is removed.")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("gus@example.com")).toBeNull());

    const bob = await memberCard("bob@example.com");
    await user.click(bob.getByRole("button", { name: "Transfer ownership to Bob" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Make Bob the owner?" }));
    expect(dialog.getByText(/They become an owner, and you an admin/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Transfer ownership" }));

    expect(await screen.findByText("Bob owns the workspace now.")).toBeInTheDocument();
    expect(api.changes.map((change) => `${change.method} ${change.path}`)).toEqual([
      `DELETE ${base}/members/${people.gus.id}`,
      `POST ${base}/transfer`,
    ]);
    // Ada is an admin now, who cannot change an owner.
    await waitFor(() => expect(bob.queryByRole("combobox", { name: "Role of Bob" })).toBeNull());
  });

  it("shows an admin no control over owners", async () => {
    fakeWorkspaces({ role: "admin" });
    renderApp("/w/design/settings/members");

    const bob = await memberCard("bob@example.com");
    expect(bob.getByText("Owner")).toBeInTheDocument();
    expect(bob.queryByRole("combobox")).toBeNull();
    expect(bob.queryByRole("button", { name: "Remove Bob" })).toBeNull();
    const cy = await memberCard("cy@example.com");
    expect(cy.getByRole("combobox", { name: "Role of Cy" })).not.toHaveTextContent("Owner");
    expect(cy.getByRole("button", { name: "Remove Cy" })).toBeInTheDocument();
    // An admin may step down, but not leave by removing themselves.
    const ada = await memberCard("ada@example.com");
    expect(ada.getByRole("combobox", { name: "Role of Ada" })).toBeInTheDocument();
    expect(ada.queryByRole("button", { name: "Remove Ada" })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Transfer ownership/ })).toBeNull();
  });

  it("shows a member the list, and nothing to manage", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp("/w/design/settings/members");

    await memberCard("bob@example.com");
    expect(screen.queryByRole("form", { name: "Invite by email" })).toBeNull();
    expect(screen.queryByRole("form", { name: "Invite links" })).toBeNull();
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.queryByRole("button", { name: /^Remove/ })).toBeNull();
  });

  it("says when the list cannot load, and tries again", async () => {
    fakeWorkspaces({ role: "member" });
    let fail = true;
    server.use(
      http.get("/api/v1/workspaces/:id/members", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp("/w/design/settings/members");

    const region = within(await screen.findByRole("region", { name: "Members" }));
    expect(await region.findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click(region.getByRole("button", { name: "Try again" }));
    expect(await region.findByText("bob@example.com")).toBeInTheDocument();
  });
});

/**
 * Ada manages Design on the Members tab when its invitations and invite links
 * start to answer 403. A cap stops the refusals after 50, so a loop shows as a
 * count rather than as a test that never ends.
 */
async function refuseManagers(options: { demoted: boolean }) {
  const api = fakeWorkspaces({ role: "admin" });
  const { queryClient } = renderApp("/w/design/settings/members");
  expect(await screen.findByRole("form", { name: "Invite by email" })).toBeInTheDocument();
  await screen.findByText("No invitations wait for an answer.");

  if (options.demoted) {
    api.state.workspaces = [summaryOf(design("member"))];
    api.state.workspace = design("member");
  }
  const counts = { refused: 0, lists: 0 };
  const refuse = (empty: object) => async () => {
    counts.refused += 1;
    await delay(5);
    return counts.refused > 50
      ? HttpResponse.json(empty)
      : apiError(403, "forbidden", "only the workspace's owners and admins may do this");
  };
  server.use(
    http.get("/api/v1/workspaces", async () => {
      counts.lists += 1;
      // Slower than the refusals, as a list of many workspaces can be.
      await delay(30);
      return HttpResponse.json({ workspaces: api.state.workspaces, can_create: true });
    }),
    http.get("/api/v1/workspaces/:id/invitations", refuse({ invitations: [] })),
    http.get("/api/v1/workspaces/:id/invite-links", refuse({ links: [] })),
  );
  // What the window regaining focus does to the invitations, once they are stale.
  void queryClient.invalidateQueries({ queryKey: keys.sentInvitations(ID) });
  return counts;
}

describe("a role taken away while the page is open", () => {
  it("reads the role again after a refusal, and shows a demoted admin the member's page", async () => {
    const counts = await refuseManagers({ demoted: true });

    await waitFor(() => expect(screen.queryByRole("form", { name: "Invite by email" })).toBeNull());
    await memberCard("bob@example.com");
    await delay(300);
    expect(counts.refused).toBeLessThanOrEqual(4);
    expect(counts.lists).toBeLessThanOrEqual(2);
  });

  it("asks the refused routes no more while the role stays the same", async () => {
    const counts = await refuseManagers({ demoted: false });

    const pendingRegion = within(
      await screen.findByRole("region", { name: "Pending invitations" }),
    );
    expect(await pendingRegion.findByRole("alert")).toBeInTheDocument();
    await delay(300);
    expect(counts.refused).toBeLessThanOrEqual(4);
    expect(counts.lists).toBeLessThanOrEqual(2);
  });
});

describe("agents among the members", () => {
  it("shows an agent with whom it acts for, suspended, and no role to change", async () => {
    fakeWorkspaces({ agents: [{ ...scout, status: "suspended" }] });
    renderApp("/w/design/settings/members");

    const region = within(await screen.findByRole("region", { name: "Members" }));
    const card = within((await region.findByText("Scout")).closest("li") as HTMLElement);
    expect(card.getByText("Agent")).toBeInTheDocument();
    expect(card.getByText("· via Ada")).toBeInTheDocument();
    expect(card.getByText("Suspended")).toBeInTheDocument();
    expect(card.queryByRole("combobox")).toBeNull();
    expect(card.queryByRole("button", { name: /Remove/ })).toBeNull();
    expect(card.queryByRole("button", { name: /Transfer/ })).toBeNull();
  });
});
