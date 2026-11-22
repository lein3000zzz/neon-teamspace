import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { fakeWorkspaces } from "@/test/workspaces";

describe("creating a workspace", () => {
  it("creates one and opens it", async () => {
    const api = fakeWorkspaces({ workspaces: [] });
    const { user, router } = renderApp("/workspaces/new");

    const form = within(await screen.findByRole("form", { name: "Create a workspace" }));
    await user.type(form.getByLabelText("Name"), "  Launch crew ");
    await user.type(form.getByLabelText("Icon"), "🛰️");
    await user.click(form.getByRole("button", { name: "Create workspace" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/launch-crew"));
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: "/api/v1/workspaces",
        body: { name: "Launch crew", icon: "🛰️" },
      },
    ]);
    expect(
      await screen.findByRole("heading", { level: 1, name: "Nothing here yet" }),
    ).toBeVisible();
  });

  it("checks the name, icon and address before sending", async () => {
    const api = fakeWorkspaces({ workspaces: [] });
    const { user } = renderApp("/workspaces/new");

    const form = within(await screen.findByRole("form", { name: "Create a workspace" }));
    await user.type(form.getByLabelText("Icon"), "x".repeat(17));
    await user.type(form.getByLabelText("Address"), "Not a slug!");
    await user.click(form.getByRole("button", { name: "Create workspace" }));

    expect(form.getByLabelText("Name")).toHaveAccessibleDescription("Fill this in.");
    expect(form.getByLabelText("Name")).toHaveFocus();
    expect(form.getByLabelText("Icon")).toHaveAttribute("aria-invalid", "true");
    expect(form.getByLabelText("Address")).toHaveAccessibleDescription(
      /lowercase letters, digits and single dashes/,
    );
    expect(api.changes).toEqual([]);
  });

  it("says when another workspace has the address", async () => {
    fakeWorkspaces({ workspaces: [] });
    server.use(
      http.post("/api/v1/workspaces", () =>
        apiError(409, "slug_taken", "another workspace has this slug"),
      ),
    );
    const { user } = renderApp("/workspaces/new");

    const form = within(await screen.findByRole("form", { name: "Create a workspace" }));
    await user.type(form.getByLabelText("Name"), "Design");
    await user.type(form.getByLabelText("Address"), "design");
    await user.click(form.getByRole("button", { name: "Create workspace" }));

    await waitFor(() =>
      expect(form.getByLabelText("Address")).toHaveAccessibleDescription(
        expect.stringContaining("Another workspace has this address."),
      ),
    );
  });

  it("says so where only instance admins create workspaces", async () => {
    fakeWorkspaces({ workspaces: [], canCreate: false });
    renderApp("/workspaces/new");

    expect(await screen.findByText(/Only instance admins create workspaces/)).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Create a workspace" })).toBeNull();
  });

  it("explains a refusal that arrives after the page loaded", async () => {
    fakeWorkspaces({ workspaces: [] });
    server.use(
      http.post("/api/v1/workspaces", () =>
        apiError(403, "forbidden", "only instance admins create workspaces here"),
      ),
    );
    const { user } = renderApp("/workspaces/new");

    const form = within(await screen.findByRole("form", { name: "Create a workspace" }));
    await user.type(form.getByLabelText("Name"), "Design");
    await user.click(form.getByRole("button", { name: "Create workspace" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /Only instance admins create workspaces/,
    );
  });
});
