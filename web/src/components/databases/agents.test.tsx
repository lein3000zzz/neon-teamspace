import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Agent } from "@/api/agents";
import type { View } from "@/api/databases";
import {
  DATABASE_PATH,
  databasePages,
  DB,
  fakeDatabase,
  fakeFeed,
  FIRST_SHOWN,
  properties,
  row,
  views,
} from "@/test/databases";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeWorkspaces, people, scout } from "@/test/workspaces";

/** An integration of Ada's, which delivers what a webhook posts and takes no work. */
const hook: Agent = {
  ...scout,
  id: "0193a5b2-0000-7000-8000-00000000c0a3",
  kind: "integration",
  name: "GitHub",
};

/** An agent of Bob's, deleted since it made a row. */
const former: Agent = {
  ...scout,
  id: "0193a5b2-0000-7000-8000-00000000c0a4",
  name: "Courier",
  owner: { id: people.bob.id, display_name: "Bob", hue: people.bob.hue },
  status: "deleted",
  deleted_at: "2026-11-21T09:00:00Z",
};

const rows = () => [
  row({ id: DB.docs, number: 1, title: "Write docs", values: { [DB.owner]: [scout.id] } }, "a0"),
  { ...row({ id: DB.login, number: 2, title: "Fix login" }, "a1"), created_by: former.id },
];

function open(options: { views?: View[] } = {}) {
  fakeWorkspaces({ agents: [scout, hook, former] });
  fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase({ rows: rows(), ...options });
  fakeFeed();
  return { ...renderApp(DATABASE_PATH), db };
}

async function database() {
  return within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
}

describe("agents in a database", () => {
  it("shows an agent a value names with its badge and the person it acts for", async () => {
    open();
    const view = await database();
    const table = within(await view.findByRole("table", { name: "Table" }, FIRST_SHOWN));

    const cell = await table.findByRole("button", { name: /^Owner of Write docs/ });
    await waitFor(() => expect(cell).toHaveTextContent("ScoutAgent· via Ada"));
  });

  it("names the agents deleted since by what they made", async () => {
    const createdBy = "0193a5b2-0000-7000-8000-0000000000ea";
    const withCreator = views().map((view) =>
      view.id === DB.table
        ? Object.assign(view, {
            properties: [...view.properties, { property: createdBy, visible: true }],
          })
        : view,
    );
    fakeWorkspaces({ agents: [scout, hook, former] });
    fakePages({ pages: databasePages(samplePages()) });
    fakeFeed();
    fakeDatabase({
      rows: rows(),
      views: withCreator,
      properties: [...properties(), { id: createdBy, name: "Created by", type: "created_by" }],
    });
    renderApp(DATABASE_PATH);
    const view = await database();
    const table = within(await view.findByRole("table", { name: "Table" }, FIRST_SHOWN));

    const made = await table.findByRole("cell", { name: /Courier/ });
    await waitFor(() => expect(made).toHaveTextContent("CourierAgent· via Bob(deleted)"));
  });

  it("offers people and agents to assign, never integrations or the deleted", async () => {
    const { user, db } = open();
    const view = await database();
    const table = within(await view.findByRole("table", { name: "Table" }, FIRST_SHOWN));

    await user.click(await table.findByRole("button", { name: /^Owner of Fix login/ }));
    const list = within(await screen.findByRole("list", { name: "Owner of Fix login" }));
    expect(list.getAllByRole("button").map((button) => button.textContent)).toEqual([
      "Ada",
      "Bob",
      "Cy",
      "Gus",
      "ScoutAgent· via Ada",
    ]);
    await user.type(screen.getByRole("textbox", { name: "Find someone" }), "via ada");
    expect(list.getAllByRole("button").map((button) => button.textContent)).toEqual([
      "ScoutAgent· via Ada",
    ]);
    await user.click(list.getByRole("button", { name: /Scout/ }));
    await waitFor(() =>
      expect(
        db.changes
          .filter(
            (change) => change.method === "PATCH" && change.path === `/api/v1/rows/${DB.login}`,
          )
          .map((change) => change.body),
      ).toEqual([{ values: { [DB.owner]: [scout.id] } }]),
    );
  });

  it("groups a board by an agent, named with whom it acts for", async () => {
    const byOwner = views().map((view) =>
      view.id === DB.board
        ? Object.assign(view, { group: { property: DB.owner, order: [], hidden: [] } })
        : view,
    );
    const { user } = open({ views: byOwner });
    const view = await database();

    await user.click(await view.findByRole("tab", { name: "Board" }, FIRST_SHOWN));
    const column = await view.findByRole("region", { name: /Scout · via Ada/ }, FIRST_SHOWN);
    expect(within(column).getByText("Write docs")).toBeInTheDocument();
  });
});
