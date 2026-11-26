import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { signedOut } from "@/test/fixtures";
import { fakeInbox, UNSUBSCRIBE_TOKEN } from "@/test/inbox";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

describe("the link that stops notification mail", () => {
  it("stops it with a click, without signing in, and keeps the token out of the address", async () => {
    server.use(signedOut);
    const api = fakeInbox();
    const { user, router } = renderApp(`/unsubscribe?token=${UNSUBSCRIBE_TOKEN}`);

    expect(
      await screen.findByRole("heading", { level: 1, name: "Stop notification mail" }),
    ).toBeVisible();
    expect(router.history.location.href).not.toContain(UNSUBSCRIBE_TOKEN);
    // A scanner that only opens the link stops nothing.
    expect(api.changes).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Stop the mail" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "No more notification mail" }),
    ).toBeVisible();
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `/api/v1/unsubscribe?token=${UNSUBSCRIBE_TOKEN}`,
        body: undefined,
      },
    ]);
    expect(api.state.mail).toBe("none");
  });

  it("says so when the link is not one the server signed", async () => {
    server.use(signedOut);
    const api = fakeInbox();
    api.state.tokenValid = false;
    const { user } = renderApp(`/unsubscribe?token=${UNSUBSCRIBE_TOKEN}`);

    await user.click(await screen.findByRole("button", { name: "Stop the mail" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link does not work" }),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Notification settings" })).toHaveAttribute(
      "href",
      "/account/notifications",
    );
  });

  it("says when the link lost its token", async () => {
    server.use(signedOut);
    renderApp("/unsubscribe");

    expect(
      await screen.findByRole("heading", { level: 1, name: "This link is incomplete" }),
    ).toBeVisible();
  });
});
