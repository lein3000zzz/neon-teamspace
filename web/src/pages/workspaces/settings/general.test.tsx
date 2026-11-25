import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { design, fakeWorkspaces } from "@/test/workspaces";

const ID = design().id;

describe("a workspace's general settings", () => {
  it("turns the workspace's agents off, and leaves making them to owners and admins", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings");

    const form = within(await screen.findByRole("form", { name: "Agents" }));
    const enabled = form.getByRole("checkbox", { name: /Agents are on in this workspace/ });
    expect(enabled).toBeChecked();
    await user.click(enabled);
    await user.selectOptions(form.getByLabelText("Who makes agents"), "admins");
    await user.click(form.getByRole("button", { name: "Save" }));

    expect(await form.findByText("Saved.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "PATCH",
        path: `/api/v1/workspaces/${ID}`,
        body: { agents_enabled: false, agent_creation: "admins" },
      },
    ]);
  });

  it("shows the agent switches to owners and admins alone", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp("/w/design/settings");

    await screen.findByRole("region", { name: "Workspace" });
    expect(screen.queryByRole("form", { name: "Agents" })).toBeNull();
  });

  it("renames it, keeping its address", async () => {
    const api = fakeWorkspaces();
    const { user, router } = renderApp("/w/design/settings");

    const form = within(await screen.findByRole("form", { name: "Workspace" }));
    const name = form.getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Design team");
    await user.click(form.getByRole("button", { name: "Save" }));

    expect(await form.findByText("Saved.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "PATCH",
        path: `/api/v1/workspaces/${ID}`,
        body: { name: "Design team", icon: "🚀", members_can_create_teamspaces: true },
      },
    ]);
    expect(router.state.location.pathname).toBe("/w/design/settings");
  });

  it("moves to the new address when the slug changes", async () => {
    const api = fakeWorkspaces();
    const { user, router } = renderApp("/w/design/settings");

    const form = within(await screen.findByRole("form", { name: "Workspace" }));
    const slug = form.getByLabelText("Address");
    await user.clear(slug);
    await user.type(slug, "Studio-2");
    await user.click(form.getByRole("checkbox", { name: /Members can create teamspaces/ }));
    await user.click(form.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/w/studio-2/settings"));
    expect(api.changes[0]?.body).toEqual({
      name: "Design",
      icon: "🚀",
      members_can_create_teamspaces: false,
      slug: "studio-2",
    });
    expect(
      await screen.findByRole("heading", { level: 1, name: "Workspace settings" }),
    ).toBeVisible();
  });

  it("says when another workspace has the address", async () => {
    fakeWorkspaces();
    server.use(
      http.patch("/api/v1/workspaces/:id", () =>
        apiError(409, "slug_taken", "another workspace has this slug"),
      ),
    );
    const { user } = renderApp("/w/design/settings");

    const form = within(await screen.findByRole("form", { name: "Workspace" }));
    await user.clear(form.getByLabelText("Address"));
    await user.type(form.getByLabelText("Address"), "taken");
    await user.click(form.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(form.getByLabelText("Address")).toHaveAccessibleDescription(
        expect.stringContaining("Another workspace has this address."),
      ),
    );
  });

  it("pins a hue, and goes back to the hashed one", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings");

    const colour = within(await screen.findByRole("region", { name: "Colour" }));
    expect(colour.getByText(/degrees for this one/)).toBeInTheDocument();
    await user.click(colour.getByRole("radio", { name: "Pinned" }));
    expect(colour.getByRole("radio", { name: "Pinned" })).toBeChecked();
    await user.click(colour.getByRole("radio", { name: "Green" }));
    await user.click(colour.getByRole("button", { name: "Save colour" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ hue_override: 155 }));

    await user.click(colour.getByRole("radio", { name: "From its id" }));
    await user.click(colour.getByRole("button", { name: "Save colour" }));
    await waitFor(() => expect(api.changes.at(-1)?.body).toEqual({ hue_override: null }));
  });

  it("shows a member what the workspace is, with nothing to change", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp("/w/design/settings");

    const region = within(await screen.findByRole("region", { name: "Workspace" }));
    expect(region.getByText("/w/design")).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Workspace" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Colour" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Danger zone" })).toBeNull();
    expect(screen.getByRole("region", { name: "Leave workspace" })).toBeInTheDocument();
  });

  it("tells a guest the member limit, but not how many are in the workspace", async () => {
    fakeWorkspaces({ role: "guest" });
    server.use(
      http.get("/api/v1/workspaces/:id", () =>
        Response.json({ ...design("guest"), limits: { members: 10 } }),
      ),
    );
    renderApp("/w/design/settings");

    const facts = within(await screen.findByRole("region", { name: "Workspace" }));
    expect(facts.getByText("/w/design")).toBeInTheDocument();
    expect(facts.queryByText("People")).toBeNull();
    const limits = within(screen.getByRole("region", { name: "Limits" }));
    expect(limits.getByText("10")).toBeInTheDocument();
    expect(limits.queryByText(/ of /)).toBeNull();
  });

  it("explains why the last owner of one of its teamspaces cannot leave", async () => {
    fakeWorkspaces({ role: "member" });
    server.use(
      http.post("/api/v1/workspaces/:id/leave", () =>
        apiError(409, "last_teamspace_owner", "a teamspace keeps at least one owner"),
      ),
    );
    const { user } = renderApp("/w/design/settings");

    const leave = within(await screen.findByRole("region", { name: "Leave workspace" }));
    await user.click(leave.getByRole("button", { name: "Leave workspace" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Leave Design?" }));
    await user.click(dialog.getByRole("button", { name: "Leave workspace" }));

    expect(await leave.findByRole("alert")).toHaveTextContent(
      /last owner of one of its teamspaces/,
    );
  });

  it("explains why its last owner cannot leave", async () => {
    fakeWorkspaces();
    server.use(
      http.post("/api/v1/workspaces/:id/leave", () =>
        apiError(409, "last_owner", "a workspace keeps at least one owner"),
      ),
    );
    const { user } = renderApp("/w/design/settings");

    const leave = within(await screen.findByRole("region", { name: "Leave workspace" }));
    await user.click(leave.getByRole("button", { name: "Leave workspace" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Leave Design?" }));
    await user.click(dialog.getByRole("button", { name: "Leave workspace" }));

    expect(await leave.findByRole("alert")).toHaveTextContent(/You are its last owner/);
  });

  it("leaves, then opens the start page", async () => {
    fakeWorkspaces({ role: "member" });
    const { user, router } = renderApp("/w/design/settings");

    const leave = within(await screen.findByRole("region", { name: "Leave workspace" }));
    await user.click(leave.getByRole("button", { name: "Leave workspace" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Leave Design?" }));
    await user.click(dialog.getByRole("button", { name: "Leave workspace" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/"));
    expect(
      await screen.findByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
  });

  it("deletes the workspace only once its name is typed", async () => {
    const api = fakeWorkspaces();
    const { user, router } = renderApp("/w/design/settings");

    const danger = within(await screen.findByRole("region", { name: "Danger zone" }));
    await user.click(danger.getByRole("button", { name: "Delete workspace" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Design?" }));
    const confirm = dialog.getByRole("button", { name: "Delete workspace" });
    expect(confirm).toBeDisabled();
    await user.type(dialog.getByLabelText("Type Design to confirm"), "design");
    expect(confirm).toBeDisabled();
    await user.clear(dialog.getByLabelText("Type Design to confirm"));
    await user.type(dialog.getByLabelText("Type Design to confirm"), "Design");
    await user.click(confirm);

    await waitFor(() => expect(router.state.location.pathname).toBe("/"));
    expect(api.changes).toEqual([
      { method: "DELETE", path: `/api/v1/workspaces/${ID}`, body: { confirm_name: "Design" } },
    ]);
  });

  it("shows only the general tab to a guest", async () => {
    fakeWorkspaces({ role: "guest" });
    renderApp("/w/design/settings");

    const tabs = within(await screen.findByRole("navigation", { name: "Workspace settings" }));
    expect(tabs.getAllByRole("link").map((link) => link.textContent)).toEqual(["General"]);
  });
});
