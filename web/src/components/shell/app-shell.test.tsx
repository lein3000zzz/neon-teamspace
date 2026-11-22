import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { keys } from "@/api/keys";
import { defaultPreferences } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { design, fakeWorkspaces, studio, summaryOf } from "@/test/workspaces";
import { FALLBACK_ACCENT_HUE, FOLLOWED_HUE_KEY } from "@/theme/preferences";

const accent = () => document.documentElement.style.getPropertyValue("--accent-h");

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

describe("the accent", () => {
  it("is the hue of the workspace in view for someone with none of their own", async () => {
    const api = fakeWorkspaces({ workspaces: [summaryOf(design()), studio] });
    const { router, queryClient } = renderApp("/w/design");

    await waitFor(() => expect(accent()).toBe(String(design().hue)));
    await router.navigate({ to: "/w/$slug", params: { slug: "studio" } });
    await waitFor(() => expect(accent()).toBe(String(studio.hue)));
    expect(window.localStorage.getItem(FOLLOWED_HUE_KEY)).toBe(String(studio.hue));

    // In no workspace any more, the app is drawn in the fallback blue again.
    api.state.workspaces = [];
    await router.navigate({ to: "/account" });
    await queryClient.invalidateQueries({ queryKey: keys.workspaces });
    await waitFor(() => expect(accent()).toBe(String(FALLBACK_ACCENT_HUE)));
  });

  it("stays the user's own where they chose one", async () => {
    fakeWorkspaces();
    server.use(
      http.get("/api/v1/account/preferences", () =>
        HttpResponse.json({ ...defaultPreferences, accent_hue: 25 }),
      ),
    );
    renderApp("/w/design");

    await waitFor(() => expect(accent()).toBe("25"));
    expect(window.localStorage.getItem(FOLLOWED_HUE_KEY)).toBe(String(design().hue));
  });
});
