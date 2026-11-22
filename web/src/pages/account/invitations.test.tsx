import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { fakeWorkspaces, studioInvitation } from "@/test/workspaces";

async function invitationCard() {
  const item = (await screen.findByText("Studio", { selector: "span" })).closest("li");
  if (item === null) {
    throw new Error("no invitation card");
  }
  return within(item);
}

describe("the account's invitations", () => {
  it("accepts an invitation, with the way into the workspace", async () => {
    const api = fakeWorkspaces({ received: [studioInvitation] });
    const { user } = renderApp("/account/invitations");

    const card = await invitationCard();
    expect(card.getByText("Join as a member")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Accept: Studio" }));

    expect(await screen.findByText("You joined Studio.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open Studio" })).toHaveAttribute("href", "/w/studio");
    expect(await screen.findByText("No invitations wait for you.")).toBeInTheDocument();
    expect(api.changes.map((change) => change.path)).toEqual([
      `/api/v1/account/invitations/${studioInvitation.id}/accept`,
    ]);
  });

  it("declines an invitation once confirmed", async () => {
    const api = fakeWorkspaces({ received: [studioInvitation] });
    const { user } = renderApp("/account/invitations");

    const card = await invitationCard();
    await user.click(card.getByRole("button", { name: "Decline: Studio" }));
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Decline the invitation into Studio?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Decline" }));

    expect(await screen.findByText("Declined the invitation into Studio.")).toBeInTheDocument();
    expect(api.changes.map((change) => change.path)).toEqual([
      `/api/v1/account/invitations/${studioInvitation.id}/decline`,
    ]);
  });

  it("lets an invitation that ended since the list was read go, saying so", async () => {
    const api = fakeWorkspaces({ received: [studioInvitation] });
    const { user } = renderApp("/account/invitations");
    const card = await invitationCard();

    // Meanwhile, Ada joined Studio by an invite link, which ended the invitation.
    api.state.received = [];
    server.use(
      http.post("/api/v1/account/invitations/:id/accept", () =>
        apiError(404, "not_found", "no such invitation"),
      ),
    );
    await user.click(card.getByRole("button", { name: "Accept: Studio" }));

    expect(
      await screen.findByText(/The invitation into Studio is no longer open/),
    ).toBeInTheDocument();
    expect(await screen.findByText("No invitations wait for you.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says a full workspace takes nobody yet, and keeps the invitation", async () => {
    fakeWorkspaces({ received: [studioInvitation] });
    server.use(
      http.post("/api/v1/account/invitations/:id/accept", () =>
        apiError(409, "member_limit", "the workspace is at its member limit"),
      ),
    );
    const { user } = renderApp("/account/invitations");

    const card = await invitationCard();
    await user.click(card.getByRole("button", { name: "Accept: Studio" }));
    expect(await card.findByRole("alert")).toHaveTextContent(/The invitation stays/);
    expect(card.getByRole("button", { name: "Accept: Studio" })).toBeEnabled();
  });

  it("says when the invitations cannot load, and tries again", async () => {
    fakeWorkspaces({ received: [studioInvitation] });
    let fail = true;
    server.use(
      http.get("/api/v1/account/invitations", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp("/account/invitations");

    const main = within(await screen.findByRole("main"));
    expect(await main.findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click(main.getByRole("button", { name: "Try again" }));
    expect(await invitationCard()).toBeTruthy();
  });

  it("is a tab of the account pages", async () => {
    fakeWorkspaces();
    const { user, router } = renderApp("/account");

    const tabs = within(await screen.findByRole("navigation", { name: "Account" }));
    await user.click(tabs.getByRole("link", { name: "Invitations" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/account/invitations"));
    expect(await screen.findByText("No invitations wait for you.")).toBeInTheDocument();
  });
});
