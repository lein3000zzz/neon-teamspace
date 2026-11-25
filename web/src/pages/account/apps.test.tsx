import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { ConnectedApp } from "@/api/oauth";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { asPerson, design, people, scout } from "@/test/workspaces";

/** Claude Code, connected by Ada as herself in Design. */
const asAda: ConnectedApp = {
  id: "0193a5b2-0000-7000-8000-00000000e001",
  client: { id: "0193a5b2-0000-7000-8000-00000000d001", name: "Claude Code" },
  workspace: { id: design().id, name: "Design", slug: "design" },
  identity: people.ada,
  connected_by: { id: people.ada.id, display_name: "Ada" },
  scopes: ["read", "write"],
  resource: "mcp",
  created_at: "2026-11-20T09:00:00Z",
  updated_at: "2026-11-20T09:00:00Z",
  last_used_at: "2026-11-21T09:00:00Z",
};

/** Cursor, connected by Ada as her agent Scout, never used since. */
const asScout: ConnectedApp = {
  ...asAda,
  id: "0193a5b2-0000-7000-8000-00000000e002",
  client: { id: "0193a5b2-0000-7000-8000-00000000d002", name: "Cursor" },
  identity: asPerson(scout),
  scopes: ["read"],
  last_used_at: null,
};

/** The account's connected apps on the server, which disconnecting removes. */
function apps(initial: ConnectedApp[]) {
  let list = initial;
  const removed: string[] = [];
  server.use(
    http.get("/api/v1/account/connected-apps", () => HttpResponse.json({ apps: list })),
    http.delete("/api/v1/account/connected-apps/:id", ({ params }) => {
      removed.push(String(params["id"]));
      list = list.filter((app) => app.id !== params["id"]);
      return new HttpResponse(null, { status: 204 });
    }),
  );
  return removed;
}

/** The apps' cards, each named after its app. */
async function cards() {
  const region = within(await screen.findByRole("region", { name: "Connected apps" }));
  return region.findAllByRole("listitem", { name: /./ });
}

describe("connected apps", () => {
  it("lists each app with its workspace, whom it acts as, its scopes and its last use", async () => {
    apps([asAda, asScout]);
    renderApp("/account/apps");

    const [claude, cursor, ...rest] = await cards();
    expect(rest).toEqual([]);
    const first = within(claude as HTMLElement);
    expect(first.getByText("Claude Code")).toBeInTheDocument();
    expect(first.getByText("Design")).toBeInTheDocument();
    expect(first.getByText("You")).toBeInTheDocument();
    expect(first.getByText("Read")).toBeInTheDocument();
    expect(first.getByText("Write")).toBeInTheDocument();
    const second = within(cursor as HTMLElement);
    expect(second.getByText("Scout")).toBeInTheDocument();
    expect(second.getByText("· via Ada")).toBeInTheDocument();
    expect(second.getByText("Never")).toBeInTheDocument();
    expect(second.queryByText("Write")).toBeNull();
  });

  it("disconnects one once confirmed", async () => {
    const removed = apps([asAda, asScout]);
    const { user } = renderApp("/account/apps");

    await user.click(await screen.findByRole("button", { name: "Disconnect Cursor from Design" }));
    const dialog = within(await screen.findByRole("alertdialog"));
    expect(dialog.getByText("Disconnect Cursor?")).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Disconnect" }));

    await waitFor(async () => expect(await cards()).toHaveLength(1));
    expect(removed).toEqual([asScout.id]);
    expect(await screen.findByText("Cursor is disconnected.")).toBeInTheDocument();
  });

  it("says when none is connected", async () => {
    apps([]);
    renderApp("/account/apps");

    expect(await screen.findByText("No apps are connected.")).toBeInTheDocument();
  });

  it("is a tab of the account", async () => {
    apps([]);
    const { user, router } = renderApp("/account");

    await user.click(await screen.findByRole("link", { name: "Connected apps" }));

    expect(router.state.location.pathname).toBe("/account/apps");
  });
});
