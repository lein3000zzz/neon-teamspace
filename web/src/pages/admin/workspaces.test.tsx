import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { AdminWorkspace, SetLimits } from "@/api/admin";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { people } from "@/test/workspaces";

const design: AdminWorkspace = {
  id: "0193a5b2-0000-7000-8000-00000000a001",
  name: "Design",
  slug: "design",
  owners: [people.ada],
  member_count: 4,
  limits: {
    members: { own: 10, effective: 10 },
    storage_bytes: { own: 2_000_000_000, effective: 2_000_000_000 },
    agents: { effective: 3 },
  },
  usage: { members: 4, storage_bytes: 0, agents: 0 },
  created_at: "2026-11-20T09:00:00Z",
};

const studio: AdminWorkspace = {
  ...design,
  id: "0193a5b2-0000-7000-8000-00000000a002",
  name: "Studio",
  slug: "studio",
  owners: [],
  member_count: 1,
  limits: { members: {}, storage_bytes: {}, agents: {} },
  usage: { members: 1, storage_bytes: 0, agents: 0 },
};

/** The admin's routes for workspaces, a page at a time, recording each change. */
function adminWorkspaces(pages: AdminWorkspace[][]) {
  const limits: (SetLimits & { id: string })[] = [];
  const deleted: string[] = [];
  let all = pages.flat();
  server.use(
    signedInAs(grace),
    http.get("/api/v1/admin/workspaces", ({ request }) => {
      const after = new URL(request.url).searchParams.get("after");
      const index = after === null ? 0 : pages.findIndex((page) => page.at(-1)?.id === after) + 1;
      const page = (pages[index] ?? []).flatMap((entry) => all.filter((w) => w.id === entry.id));
      const next = index + 1 < pages.length ? pages[index]?.at(-1)?.id : undefined;
      return HttpResponse.json({ workspaces: page, ...(next ? { next } : {}) });
    }),
    http.patch("/api/v1/admin/workspaces/:id/limits", async ({ params, request }) => {
      const body = (await request.json()) as SetLimits;
      limits.push({ id: String(params["id"]), ...body });
      const target = all.find((entry) => entry.id === params["id"]);
      if (target === undefined) {
        return apiError(404, "not_found", "no such workspace");
      }
      target.limits.members = { own: body.members ?? null, effective: body.members || null };
      return HttpResponse.json(target);
    }),
    http.delete("/api/v1/admin/workspaces/:id", ({ params }) => {
      deleted.push(String(params["id"]));
      all = all.filter((entry) => entry.id !== params["id"]);
      return new HttpResponse(null, { status: 204 });
    }),
    http.get("/api/v1/admin/settings", () =>
      HttpResponse.json({
        settings: [
          { key: "workspaces.limits.members", value: 0 },
          { key: "workspaces.limits.storage_bytes", value: 5_000_000_000 },
          { key: "workspaces.limits.agents", value: 3 },
        ],
        mail: { ready: false },
      }),
    ),
  );
  return { limits, deleted };
}

async function card(name: string) {
  const item = (await screen.findByText(name, { selector: "p" })).closest("li");
  if (item === null) {
    throw new Error(`no card for ${name}`);
  }
  return within(item);
}

describe("the instance's workspaces", () => {
  it("lists each with its owners, members, limits and usage, and no way inside", async () => {
    adminWorkspaces([[design, studio]]);
    renderApp("/admin/workspaces");

    const first = await card("Design");
    expect(first.getByText("ada@example.com")).toBeInTheDocument();
    expect(first.getByText("4 members")).toBeInTheDocument();
    expect(first.getByText("4 of 10")).toBeInTheDocument();
    expect(first.getByText("0 B of 2 GB")).toBeInTheDocument();
    expect(first.getByText("0 of 3")).toBeInTheDocument();
    expect(first.queryByRole("link")).toBeNull();
    const second = await card("Studio");
    expect(second.getByText("No owner")).toBeInTheDocument();
    expect(second.getByText("1 of no limit")).toBeInTheDocument();
  });

  it("loads the next page", async () => {
    adminWorkspaces([[design], [studio]]);
    const { user } = renderApp("/admin/workspaces");

    await card("Design");
    await user.click(screen.getByRole("button", { name: "Load more" }));
    expect(await card("Studio")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });

  it("sets limits: a number, 0 for none, or blank for the instance default", async () => {
    const api = adminWorkspaces([[design]]);
    const { user } = renderApp("/admin/workspaces");

    const first = await card("Design");
    await user.click(first.getByRole("button", { name: "Edit the limits of Design" }));
    const form = within(first.getByRole("form", { name: "Limits of Design" }));
    expect(form.getByLabelText("Members")).toHaveValue(10);
    expect(form.getByLabelText("Storage")).toHaveValue(2);
    expect(form.getByRole("combobox", { name: "Unit of Storage" })).toHaveValue("GB");
    expect(await form.findByText("Blank: the default, 5 GB.")).toBeInTheDocument();

    await user.clear(form.getByLabelText("Members"));
    await user.type(form.getByLabelText("Members"), "0");
    await user.clear(form.getByLabelText("Storage"));
    await user.type(form.getByLabelText("Storage"), "500");
    await user.selectOptions(form.getByRole("combobox", { name: "Unit of Storage" }), "MB");
    await user.click(form.getByRole("button", { name: "Save limits" }));

    expect(await screen.findByText("Limits of Design saved.")).toBeInTheDocument();
    expect(api.limits).toEqual([
      { id: design.id, members: 0, storage_bytes: 500_000_000, agents: null },
    ]);
  });

  it("refuses a limit that is not a whole number before sending", async () => {
    const api = adminWorkspaces([[design]]);
    const { user } = renderApp("/admin/workspaces");

    const first = await card("Design");
    await user.click(first.getByRole("button", { name: "Edit the limits of Design" }));
    const form = within(first.getByRole("form", { name: "Limits of Design" }));
    await user.clear(form.getByLabelText("Agents"));
    await user.type(form.getByLabelText("Agents"), "-1");
    await user.click(form.getByRole("button", { name: "Save limits" }));

    expect(form.getByLabelText("Agents")).toHaveAttribute("aria-invalid", "true");
    expect(api.limits).toEqual([]);
  });

  it("deletes a workspace only once its name is typed", async () => {
    const api = adminWorkspaces([[design, studio]]);
    const { user } = renderApp("/admin/workspaces");

    const first = await card("Design");
    await user.click(first.getByRole("button", { name: "Delete Design" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Design?" }));
    expect(dialog.getByRole("button", { name: "Delete" })).toBeDisabled();
    await user.type(dialog.getByLabelText("Type Design to confirm"), "Design");
    await user.click(dialog.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Design is deleted.")).toBeInTheDocument();
    expect(api.deleted).toEqual([design.id]);
    await waitFor(() => expect(screen.queryByText("Design", { selector: "p" })).toBeNull());
  });

  it("says when the list cannot load, and tries again", async () => {
    adminWorkspaces([[design, studio]]);
    let fail = true;
    server.use(
      http.get("/api/v1/admin/workspaces", () =>
        fail ? apiError(500, "internal", "internal error") : undefined,
      ),
    );
    const { user } = renderApp("/admin/workspaces");

    const main = within(await screen.findByRole("main"));
    expect(await main.findByRole("alert")).toBeInTheDocument();
    fail = false;
    await user.click(main.getByRole("button", { name: "Try again" }));
    expect(await card("Design")).toBeTruthy();
  });

  it("shows a refused deletion on the workspace's card, which stays", async () => {
    adminWorkspaces([[design, studio]]);
    server.use(
      http.delete("/api/v1/admin/workspaces/:id", () =>
        apiError(403, "forbidden", "deleting a workspace needs a browser session"),
      ),
    );
    const { user } = renderApp("/admin/workspaces");

    const first = await card("Design");
    await user.click(first.getByRole("button", { name: "Delete Design" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Design?" }));
    await user.type(dialog.getByLabelText("Type Design to confirm"), "Design");
    await user.click(dialog.getByRole("button", { name: "Delete" }));

    expect(await first.findByRole("alert")).toHaveTextContent("You are not allowed to do this.");
    expect(screen.queryByText("Design is deleted.")).toBeNull();
    expect((await card("Design")).getByText("4 members")).toBeInTheDocument();
  });

  it("is a tab of the admin pages, which members do not get", async () => {
    renderApp("/admin/workspaces");
    expect(await screen.findByRole("heading", { level: 1, name: "Not allowed" })).toBeVisible();
  });
});
