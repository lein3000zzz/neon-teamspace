import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { Agent } from "@/api/agents";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { AGENT_TOKEN, design, fakeWorkspaces, people, scout, teamspaces } from "@/test/workspaces";

const ID = design().id;
const base = `/api/v1/workspaces/${ID}`;

/** An agent's card, by its name. */
async function agentCard(name: string) {
  const region = within(await screen.findByRole("region", { name: "Agents" }));
  return within(await region.findByRole("listitem", { name }));
}

/** Cy's agent, suspended while Cy is out of the workspace. */
const ranger: Agent = {
  ...scout,
  id: "0193a5b2-0000-7000-8000-00000000c0a2",
  name: "Ranger",
  hue: 30,
  owner: { id: people.cy.id, display_name: "Cy", hue: people.cy.hue },
  status: "suspended",
};

describe("the agents of a workspace", () => {
  it("lists each with its owner and whether it works", async () => {
    fakeWorkspaces({ agents: [scout, ranger] });
    renderApp("/w/design/settings/agents");

    const mine = await agentCard("Scout");
    expect(mine.getByText("Owner: Ada")).toBeInTheDocument();
    expect(mine.getByText("Active")).toBeInTheDocument();
    expect(mine.getByText("Yours")).toBeInTheDocument();
    const theirs = await agentCard("Ranger");
    expect(theirs.getByText("Suspended")).toBeInTheDocument();
    expect(
      theirs.getByText("Its owner is out of the workspace or disabled: its tokens are refused."),
    ).toBeInTheDocument();
    expect(theirs.queryByRole("button", { name: "Rename or recolour Ranger" })).toBeNull();
    expect(theirs.getByRole("button", { name: "Delete Ranger" })).toBeInTheDocument();
  });

  it("makes an agent the user owns", async () => {
    const api = fakeWorkspaces();
    const { user } = renderApp("/w/design/settings/agents");

    expect(await screen.findByText("No agents yet.")).toBeInTheDocument();
    const form = within(await screen.findByRole("form", { name: "Make an agent" }));
    await user.click(form.getByRole("button", { name: "Make agent" }));
    expect(form.getByLabelText("Name")).toHaveAccessibleDescription("Fill this in.");
    await user.type(form.getByLabelText("Name"), " Hermes ");
    await user.click(form.getByRole("button", { name: "Make agent" }));

    expect(await screen.findByText("Hermes is made. Make it a token below.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "POST", path: `${base}/agents`, body: { name: "Hermes" } },
    ]);
    expect(await agentCard("Hermes")).toBeTruthy();
  });

  it("says when the workspace is at its agent limit", async () => {
    fakeWorkspaces();
    server.use(
      http.post(`${base}/agents`, () =>
        apiError(409, "agent_limit", "this workspace has reached its limit of 1 agents"),
      ),
    );
    const { user } = renderApp("/w/design/settings/agents");

    const form = within(await screen.findByRole("form", { name: "Make an agent" }));
    await user.type(form.getByLabelText("Name"), "Two");
    await user.click(form.getByRole("button", { name: "Make agent" }));
    expect(await form.findByRole("alert")).toHaveTextContent(
      "This workspace has as many agents and integrations as its limit allows.",
    );
  });

  it("leaves making agents to owners and admins when the workspace says so, and to nobody while off", async () => {
    fakeWorkspaces({ role: "member" });
    server.use(
      http.get(base, () =>
        HttpResponse.json({
          ...design("member"),
          agents: { available: true, enabled: true, creation: "admins", count: 0 },
        }),
      ),
    );
    const first = renderApp("/w/design/settings/agents");
    expect(
      await screen.findByText("In this workspace only owners and admins make agents."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Make an agent" })).toBeNull();
    first.unmount();

    server.use(
      http.get(base, () =>
        HttpResponse.json({
          ...design("owner"),
          agents: { available: false, enabled: true, creation: "members", count: 0 },
        }),
      ),
    );
    renderApp("/w/design/settings/agents");
    expect(await screen.findByText(/Agents are turned off on this instance/)).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Make an agent" })).toBeNull();
  });

  it("renames and recolours an agent for its owner", async () => {
    const api = fakeWorkspaces({ agents: [scout] });
    const { user } = renderApp("/w/design/settings/agents");

    const card = await agentCard("Scout");
    await user.click(card.getByRole("button", { name: "Rename or recolour Scout" }));
    const form = within(card.getByRole("form", { name: "Rename or recolour Scout" }));
    await user.clear(form.getByLabelText("Name"));
    await user.type(form.getByLabelText("Name"), "Pathfinder");
    await user.click(form.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Pathfinder is saved.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "PATCH",
        path: `${base}/agents/${scout.id}`,
        body: { name: "Pathfinder", hue_override: null },
      },
    ]);
  });

  it("hands an agent to someone else once confirmed", async () => {
    const api = fakeWorkspaces({ agents: [ranger] });
    const { user } = renderApp("/w/design/settings/agents");

    const card = await agentCard("Ranger");
    const select = await card.findByRole("combobox", { name: "Hand Ranger to someone else" });
    const options = within(select)
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(options).toEqual(["Choose a person", "Ada", "Bob"]);
    await user.selectOptions(select, people.bob.id);
    await user.click(card.getByRole("button", { name: "Hand over" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Hand Ranger to Bob?" }));
    expect(dialog.getByText(/Its tokens are revoked/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Hand over" }));

    expect(await screen.findByText("Ranger now acts for Bob.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `${base}/agents/${ranger.id}/transfer`,
        body: { owner_id: people.bob.id },
      },
    ]);
    expect(await (await agentCard("Ranger")).findByText("Owner: Bob")).toBeInTheDocument();
  });

  it("deletes an agent once confirmed, which then shows as deleted", async () => {
    const api = fakeWorkspaces({ agents: [scout] });
    const { user } = renderApp("/w/design/settings/agents");

    const card = await agentCard("Scout");
    await user.click(card.getByRole("button", { name: "Delete Scout" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Scout?" }));
    await user.click(dialog.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Scout is deleted.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "DELETE", path: `${base}/agents/${scout.id}`, body: undefined },
    ]);
    const gone = await agentCard("Scout");
    expect(await gone.findByText("Deleted")).toBeInTheDocument();
    expect(gone.queryByRole("button", { name: "Delete Scout" })).toBeNull();
  });

  it("makes a token shown once, lists it without its secret, and revokes it", async () => {
    const api = fakeWorkspaces({ agents: [scout] });
    const { user } = renderApp("/w/design/settings/agents");

    const card = await agentCard("Scout");
    const tokens = within(card.getByRole("region", { name: "Tokens of Scout" }));
    expect(await tokens.findByText("No tokens yet.")).toBeInTheDocument();
    const form = within(tokens.getByRole("form", { name: "Make token" }));
    expect(form.queryByRole("checkbox", { name: /admin/i })).toBeNull();
    await user.type(form.getByLabelText("Token name"), "Hermes");
    await user.click(form.getByRole("checkbox", { name: "Write" }));
    await user.click(form.getByRole("button", { name: "Make token" }));

    expect(await tokens.findByText("Token Hermes is made")).toBeInTheDocument();
    expect(tokens.getByDisplayValue(AGENT_TOKEN)).toHaveAttribute("readonly");
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `${base}/agents/${scout.id}/tokens`,
        body: { name: "Hermes", scopes: ["read"] },
      },
    ]);
    await user.click(tokens.getByRole("button", { name: "Done" }));
    expect(tokens.queryByDisplayValue(AGENT_TOKEN)).toBeNull();
    expect(await tokens.findByText("nw_aaaaaaa…")).toBeInTheDocument();

    await user.click(tokens.getByRole("button", { name: "Revoke the token Hermes" }));
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Revoke the token Hermes?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Revoke" }));
    expect(await tokens.findByText("No tokens yet.")).toBeInTheDocument();
    expect(api.changes.at(-1)).toEqual({
      method: "DELETE",
      path: `${base}/agents/${scout.id}/tokens/0193a5b2-0000-7000-8000-00000000c0b1`,
      body: undefined,
    });
  });

  it("adds its owner's agent to the teamspaces they are in", async () => {
    fakeWorkspaces({
      agents: [scout],
      teamspaces: [teamspaces.general, { ...teamspaces.open, role: "member" }, teamspaces.closed],
      role: "member",
    });
    const added: { teamspace: string; body: unknown }[] = [];
    server.use(
      http.post(`${base}/teamspaces/:teamspace_id/members`, async ({ request, params }) => {
        added.push({ teamspace: String(params["teamspace_id"]), body: await request.json() });
        return HttpResponse.json(
          {
            user: { id: scout.id, kind: "agent", display_name: "Scout", hue: 160 },
            role: "member",
            joined_at: scout.created_at,
          },
          { status: 201 },
        );
      }),
    );
    const { user } = renderApp("/w/design/settings/agents");

    const card = await agentCard("Scout");
    const form = within(card.getByRole("form", { name: "Add to a teamspace: Scout" }));
    const choices = within(await form.findByRole("combobox", { name: "Teamspace" }))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(choices).toEqual(["Choose a teamspace", "Open studio"]);
    await user.selectOptions(form.getByRole("combobox", { name: "Teamspace" }), "Open studio");
    await user.click(form.getByRole("button", { name: "Add" }));

    expect(await screen.findByText("Scout is in Open studio.")).toBeInTheDocument();
    expect(added).toEqual([
      { teamspace: teamspaces.open.id, body: { user_id: scout.id, role: "member" } },
    ]);
  });

  it("shows no Agents tab to a guest", async () => {
    fakeWorkspaces({ role: "guest" });
    renderApp("/w/design/settings");

    const tabs = await screen.findByRole("navigation", { name: "Workspace settings" });
    await waitFor(() => expect(within(tabs).getAllByRole("link")).toHaveLength(1));
    expect(within(tabs).queryByRole("link", { name: "Agents" })).toBeNull();
  });
});
