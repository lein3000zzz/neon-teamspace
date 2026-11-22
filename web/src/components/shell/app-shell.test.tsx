import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderApp } from "@/test/render-app";

describe("app shell", () => {
  it("marks the current page in the navigation", async () => {
    renderApp("/styleguide");
    const nav = within(await screen.findByRole("navigation", { name: "Main" }));
    expect(nav.getByRole("link", { name: "Styleguide" })).toHaveAttribute("aria-current", "page");
    expect(nav.getByRole("link", { name: "Home" })).not.toHaveAttribute("aria-current");
  });

  it("opens the drawer and closes it after navigating", async () => {
    const { user, router } = renderApp("/");
    await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" });

    await user.click(screen.getByRole("button", { name: "Open navigation" }));
    const dialog = await screen.findByRole("dialog", { name: "Main" });
    const drawer = within(dialog);
    // The column's copy of the sidebar is still mounted, so the drawer's list
    // must be named by its own heading, not one with the same id behind it.
    const labelId = drawer.getByRole("list", { name: "Settings" }).getAttribute("aria-labelledby");
    expect(dialog).toContainElement(document.getElementById(labelId ?? ""));
    await user.click(drawer.getByRole("link", { name: "Account" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(router.state.location.pathname).toBe("/account");
  });

  it("closes the drawer with Escape", async () => {
    const { user } = renderApp("/");
    await user.click(await screen.findByRole("button", { name: "Open navigation" }));
    await screen.findByRole("dialog");

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Open navigation" })).toHaveFocus();
  });

  it("renders the not-found page inside the shell", async () => {
    renderApp("/no/such/page");
    expect(await screen.findByRole("heading", { level: 1, name: "Page not found" })).toBeVisible();
    expect(screen.getByText("Nothing lives at /no/such/page.")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
  });
});
