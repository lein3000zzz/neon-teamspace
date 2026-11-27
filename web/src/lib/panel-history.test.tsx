import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { fakeComments } from "@/test/comments";
import { fakePages, PAGE_IDS } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeSearch } from "@/test/search";
import { fakeSync } from "@/test/sync";
import { fakeWorkspaces } from "@/test/workspaces";
import { COARSE_QUERY } from "./media";

/** A screen whose main pointer is a finger, or a mouse. */
function pointer(kind: "touch" | "mouse") {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: kind === "touch" && query === COARSE_QUERY,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

type Router = ReturnType<typeof renderApp>["router"];

/** Where the history is: its entry's place, and the panels the entry lists. */
function entry(router: Router) {
  const { state } = router.history.location;
  return { index: state["__TSR_index"], panels: state.panels ?? [] };
}

async function openDrawer(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(await screen.findByRole("button", { name: "Open navigation" }));
  return screen.findByRole("dialog", { name: "Main" });
}

describe("on a touch screen, an open panel", () => {
  it("has an entry of its own, which back leaves, closing the panel on the same page", async () => {
    pointer("touch");
    const { user, router } = renderApp("/");
    await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" });
    const start = entry(router).index;

    await openDrawer(user);
    await waitFor(() =>
      expect(entry(router)).toEqual({ index: start + 1, panels: [expect.any(String)] }),
    );
    expect(router.history.location.pathname).toBe("/");

    act(() => router.history.back());
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(entry(router)).toEqual({ index: start, panels: [] });
    expect(router.history.location.pathname).toBe("/");
  });

  it("takes its entry back as it closes another way, so back then leaves the page", async () => {
    pointer("touch");
    const { user, router } = renderApp("/");
    await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" });
    const start = entry(router).index;

    await openDrawer(user);
    await waitFor(() => expect(entry(router).index).toBe(start + 1));
    await user.keyboard("{Escape}");

    await waitFor(() => expect(entry(router)).toEqual({ index: start, panels: [] }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens again as forward returns to its entry", async () => {
    pointer("touch");
    const { user, router } = renderApp("/");
    await openDrawer(user);
    await waitFor(() => expect(entry(router).panels).toHaveLength(1));

    act(() => router.history.back());
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    act(() => router.history.forward());

    expect(await screen.findByRole("dialog", { name: "Main" })).toBeInTheDocument();
    expect(entry(router).panels).toHaveLength(1);
  });

  it("that does not open again as forward returns to its entry steps back off it", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    const { user, router } = renderApp("/w/design/trash");
    const list = within(await screen.findByRole("list", { name: "Trash" }));
    const start = entry(router).index;

    await user.click(list.getByRole("button", { name: "Delete Old plans for good" }));
    await screen.findByRole("alertdialog");
    await waitFor(() => expect(entry(router).index).toBe(start + 1));
    act(() => router.history.back());
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());

    // Its question went with it, so it stays closed, and the history goes back off its entry.
    act(() => router.history.forward());
    await waitFor(() => expect(entry(router)).toEqual({ index: start, panels: [] }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("leaves its entry to the page a link in it opens, so back goes to the page under it", async () => {
    pointer("touch");
    const { user, router } = renderApp("/");
    await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" });
    const start = entry(router).index;

    const drawer = within(await openDrawer(user));
    await waitFor(() => expect(entry(router).index).toBe(start + 1));
    await user.click(drawer.getByRole("link", { name: "Account" }));

    await waitFor(() => expect(router.history.location.pathname).toBe("/account"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(entry(router)).toEqual({ index: start + 1, panels: [] });

    act(() => router.history.back());
    await waitFor(() => expect(router.history.location.pathname).toBe("/"));
    expect(entry(router)).toEqual({ index: start, panels: [] });
  });

  it("that opens another as it closes gives its entry to the other", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    fakeSearch();
    const { user, router } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });
    const start = entry(router).index;

    const drawer = within(await openDrawer(user));
    await user.click(drawer.getByRole("button", { name: /Search/ }));
    const search = await screen.findByRole("dialog", { name: "Search" });

    await waitFor(() =>
      expect(entry(router)).toEqual({ index: start + 1, panels: [expect.any(String)] }),
    );
    act(() => router.history.back());
    await waitFor(() => expect(search).not.toBeInTheDocument());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(entry(router)).toEqual({ index: start, panels: [] });
    expect(router.history.location.pathname).toBe("/w/design");
  });

  it("over another closes alone, then the one under it", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    const { user, router } = renderApp("/w/design");
    await screen.findByRole("heading", { level: 1, name: "Design" });
    const start = entry(router).index;

    const drawer = within(await openDrawer(user));
    await user.click(await drawer.findByRole("button", { name: "More for Roadmap" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to…" }));
    const move = await screen.findByRole("dialog", { name: "Move Roadmap" });
    await waitFor(() => expect(entry(router).panels).toHaveLength(2));
    expect(entry(router).index).toBe(start + 2);

    act(() => router.history.back());
    await waitFor(() => expect(move).not.toBeInTheDocument());
    expect(screen.getByRole("dialog", { name: "Main" })).toBeInTheDocument();
    expect(entry(router)).toEqual({ index: start + 1, panels: [expect.any(String)] });

    act(() => router.history.back());
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(entry(router)).toEqual({ index: start, panels: [] });
  });

  it("includes the comments, which back closes over the page", async () => {
    pointer("touch");
    fakeWorkspaces();
    fakePages();
    fakeSync();
    fakeComments();
    const path = `/w/design/p/${PAGE_IDS.roadmap}`;
    const { user, router } = renderApp(path);
    await user.click(await screen.findByRole("button", { name: /Comments/ }));
    const panel = await screen.findByRole("complementary", { name: "Comments" });
    await waitFor(() => expect(entry(router).panels).toHaveLength(1));

    act(() => router.history.back());
    await waitFor(() => expect(panel).not.toBeInTheDocument());
    expect(router.history.location.pathname).toBe(path);
    expect(entry(router).panels).toEqual([]);
  });
});

describe("with a mouse, an open panel", () => {
  it("has no entry, so back leaves the page under it as before", async () => {
    pointer("mouse");
    const { user, router } = renderApp("/");
    await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" });
    const start = entry(router).index;
    await user.click(within(await openDrawer(user)).getByRole("link", { name: "Account" }));
    await waitFor(() => expect(router.history.location.pathname).toBe("/account"));
    expect(entry(router).index).toBe(start + 1);

    await openDrawer(user);
    act(() => router.history.back());

    await waitFor(() => expect(router.history.location.pathname).toBe("/"));
    expect(entry(router)).toEqual({ index: start, panels: [] });
  });
});
