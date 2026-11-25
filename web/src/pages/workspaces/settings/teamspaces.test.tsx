import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import {
  design,
  fakeWorkspaces,
  people,
  scout,
  TEAMSPACE_IDS,
  teamspaces,
} from "@/test/workspaces";

const base = `/api/v1/workspaces/${design().id}/teamspaces`;

/** A teamspace's card in the list, by its name. */
async function card(name: string) {
  const region = within(await screen.findByRole("region", { name: "Teamspaces" }));
  const item = (await region.findByText(name, { selector: "span" })).closest("li");
  if (item === null) {
    throw new Error(`no card for ${name}`);
  }
  return within(item);
}

describe("a workspace's teamspaces", () => {
  it("creates a private teamspace, each access said in words", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/teamspaces");

    const form = within(await screen.findByRole("form", { name: "Create a teamspace" }));
    await user.type(form.getByLabelText("Name"), "Board");
    const access = form.getByRole("radiogroup", { name: "Access" });
    expect(within(access).getByRole("radio", { name: "Open" })).toBeChecked();
    const secret = within(access).getByRole("radio", { name: "Private" });
    expect(secret).toHaveAccessibleDescription(/Only its members see it/);
    await user.click(secret);
    await user.click(form.getByRole("button", { name: "Create teamspace" }));

    expect(await screen.findByText("Board is created.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "POST", path: base, body: { name: "Board", access: "private" } },
    ]);
    expect(await card("Board")).toBeTruthy();
  });

  it("says when members may not create teamspaces here", async () => {
    fakeWorkspaces({ role: "member" });
    server.use(
      http.get("/api/v1/workspaces/:id", () =>
        Response.json({ ...design("member"), members_can_create_teamspaces: false }),
      ),
    );
    renderApp("/w/design/settings/teamspaces");

    expect(
      await screen.findByText("In this workspace only owners and admins create teamspaces."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Create a teamspace" })).toBeNull();
  });

  it("changes a teamspace's name, access and hue, sending only what changed", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Edit Open studio" }));
    const form = within(open.getByRole("form", { name: "Edit Open studio" }));
    await user.clear(form.getByLabelText("Name"));
    await user.type(form.getByLabelText("Name"), "Studio");
    await user.click(form.getByRole("radio", { name: "Closed" }));
    await user.click(form.getByRole("radio", { name: "Pinned" }));
    await user.click(form.getByRole("radio", { name: "Red" }));
    await user.click(form.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Studio is saved.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "PATCH",
        path: `${base}/${TEAMSPACE_IDS.open}`,
        body: { name: "Studio", access: "closed", hue_override: 25 },
      },
    ]);
  });

  it("keeps General's access, and never deletes it", async () => {
    fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/teamspaces");

    const general = await card("General");
    expect(general.queryByRole("button", { name: "Delete General" })).toBeNull();
    await user.click(general.getByRole("button", { name: "Edit General" }));
    const form = within(general.getByRole("form", { name: "Edit General" }));
    expect(form.getByText(/General keeps its access/)).toBeInTheDocument();
    expect(form.queryByRole("radiogroup", { name: "Access" })).toBeNull();
  });

  it("deletes a teamspace once confirmed", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/teamspaces");

    const closed = await card("Closed room");
    await user.click(closed.getByRole("button", { name: "Delete Closed room" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Closed room?" }));
    await user.click(dialog.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Closed room is deleted.")).toBeInTheDocument();
    expect(api.changes.map((change) => `${change.method} ${change.path}`)).toEqual([
      `DELETE ${base}/${TEAMSPACE_IDS.closed}`,
    ]);
    await waitFor(() => expect(screen.queryByText("Closed room", { selector: "span" })).toBeNull());
  });

  it("lists private teamspaces an admin is not in, by name, to join or delete them", async () => {
    const api = fakeWorkspaces({
      role: "admin",
      hidden: [{ id: TEAMSPACE_IDS.private, name: "Secret lab" }],
    });
    const { user } = renderApp("/w/design/settings/teamspaces");

    const hidden = await card("Secret lab");
    expect(hidden.getByText(/only its name shows to you/)).toBeInTheDocument();
    // Its members, settings and access stay closed until the admin is in it.
    expect(hidden.queryByRole("button", { name: "Members of Secret lab" })).toBeNull();
    expect(hidden.queryByRole("button", { name: "Edit Secret lab" })).toBeNull();
    expect(hidden.getByRole("button", { name: "Delete Secret lab" })).toBeInTheDocument();
    await user.click(hidden.getByRole("button", { name: "Add yourself to Secret lab" }));

    await waitFor(() =>
      expect(api.changes.map((change) => change.path)).toEqual([
        `${base}/${TEAMSPACE_IDS.private}/join`,
      ]),
    );
  });

  it("deletes a private teamspace an admin is not in", async () => {
    const api = fakeWorkspaces({
      role: "admin",
      hidden: [{ id: TEAMSPACE_IDS.private, name: "Secret lab" }],
    });
    const { user } = renderApp("/w/design/settings/teamspaces");

    const hidden = await card("Secret lab");
    await user.click(hidden.getByRole("button", { name: "Delete Secret lab" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Secret lab?" }));
    await user.click(dialog.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Secret lab is deleted.")).toBeInTheDocument();
    expect(api.changes.map((change) => `${change.method} ${change.path}`)).toEqual([
      `DELETE ${base}/${TEAMSPACE_IDS.private}`,
    ]);
  });

  it("explains a refusal to add someone who became a guest", async () => {
    fakeWorkspaces();
    server.use(
      http.post("/api/v1/workspaces/:id/teamspaces/:teamspace_id/members", () =>
        apiError(403, "forbidden", "guests are in no teamspace: make them a member first"),
      ),
    );
    const { user } = renderApp("/w/design/settings/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Members of Open studio" }));
    const form = within(await open.findByRole("form", { name: "Add a member" }));
    await user.selectOptions(form.getByLabelText("Person"), people.cy.id);
    await user.click(form.getByRole("button", { name: "Add" }));

    expect(await form.findByRole("alert")).toHaveTextContent(/Guests are in no teamspace/);
  });

  it("adds a member to a teamspace, and changes their teamspace role", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Members of Open studio" }));
    const form = within(await open.findByRole("form", { name: "Add a member" }));
    const person = form.getByLabelText("Person");
    // Guests join no teamspace, so Gus is not offered.
    expect(within(person).queryByRole("option", { name: /Gus/ })).toBeNull();
    await user.selectOptions(person, people.cy.id);
    await user.click(form.getByRole("button", { name: "Add" }));

    expect(await open.findByText("Cy is added.")).toBeInTheDocument();
    const role = await open.findByRole("combobox", { name: "Teamspace role of Cy" });
    await user.selectOptions(role, "owner");

    await waitFor(() =>
      expect(api.changes.map((change) => [change.method, change.path, change.body])).toEqual([
        [
          "POST",
          `${base}/${TEAMSPACE_IDS.open}/members`,
          { user_id: people.cy.id, role: "member" },
        ],
        ["PATCH", `${base}/${TEAMSPACE_IDS.open}/members/${people.cy.id}`, { role: "owner" }],
      ]),
    );
  });

  it("adds an agent as a member alone, named with the person it acts for", async () => {
    const api = fakeWorkspaces({ agents: [scout] });
    const { user } = renderApp("/w/design/settings/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Members of Open studio" }));
    const form = within(await open.findByRole("form", { name: "Add a member" }));
    const person = form.getByLabelText("Person");
    const role = form.getByLabelText("Role");
    expect(within(person).getByRole("option", { name: "Scout · via Ada" })).toBeInTheDocument();
    // Owner chosen for someone first, then the agent: the role goes back to member.
    await user.selectOptions(person, people.cy.id);
    await user.selectOptions(role, "owner");
    await user.selectOptions(person, scout.id);
    expect(
      within(role)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["Member"]);
    expect(role).toHaveValue("member");
    await user.click(form.getByRole("button", { name: "Add" }));

    expect(await open.findByText("Scout is added.")).toBeInTheDocument();
    const added = await open.findByRole("combobox", { name: "Teamspace role of Scout" });
    expect(
      within(added)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["Member"]);
    expect(api.changes.map((change) => [change.method, change.path, change.body])).toEqual([
      ["POST", `${base}/${TEAMSPACE_IDS.open}/members`, { user_id: scout.id, role: "member" }],
    ]);
  });

  it("explains a teamspace's last owner staying one", async () => {
    fakeWorkspaces({
      role: "member",
      teamspaces: [teamspaces.general, { ...teamspaces.open, role: "owner" }],
    });
    server.use(
      http.patch("/api/v1/workspaces/:id/teamspaces/:teamspace_id/members/:user_id", () =>
        apiError(409, "last_teamspace_owner", "a teamspace keeps at least one owner"),
      ),
    );
    const { user } = renderApp("/w/design/settings/teamspaces");

    const open = await card("Open studio");
    await user.click(open.getByRole("button", { name: "Members of Open studio" }));
    await user.selectOptions(
      await open.findByRole("combobox", { name: "Teamspace role of Bob" }),
      "member",
    );
    expect(await open.findByRole("alert")).toHaveTextContent(/keeps at least one owner/);
  });

  it("shows a member only the teamspaces they manage as manageable", async () => {
    fakeWorkspaces({ role: "member" });
    renderApp("/w/design/settings/teamspaces");

    const general = await card("General");
    expect(general.getByRole("button", { name: "Edit General" })).toBeInTheDocument();
    const closed = await card("Closed room");
    expect(closed.queryByRole("button", { name: "Edit Closed room" })).toBeNull();
    expect(await closed.findByText("Closed: ask Bob to add you.")).toBeInTheDocument();
  });
});
