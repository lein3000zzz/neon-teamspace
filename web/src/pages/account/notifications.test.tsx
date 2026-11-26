import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { fakeInbox } from "@/test/inbox";
import { renderApp } from "@/test/render-app";

describe("the account's notification mail", () => {
  it("shows the choice made, and saves another as it is picked", async () => {
    const api = fakeInbox({ mail: "each" });
    const { user } = renderApp("/account/notifications");

    const each = await screen.findByRole("radio", { name: "Each notification" });
    expect(each).toBeChecked();
    expect(
      screen.getByText("A mail for each notification still unread a short while after it came."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "A digest" }));

    await waitFor(() => expect(screen.getByRole("radio", { name: "A digest" })).toBeChecked());
    expect(
      screen.getByText("One mail of what is unread, at most once every hour."),
    ).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "PUT", path: "/api/v1/me/notifications", body: { mail: "digest" } },
    ]);
  });

  it("says when the instance sends no notification mail, keeping the choice for later", async () => {
    fakeInbox({ mail: "digest", mailAvailable: false });
    renderApp("/account/notifications");

    expect(
      await screen.findByText(/This instance sends no notification mail now/),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "A digest" })).toBeChecked();
  });
});
