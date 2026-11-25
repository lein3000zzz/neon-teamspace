import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { keys } from "@/api/keys";
import type { Recorded } from "@/test/workspaces";
import {
  DATABASE_PATH,
  databasePages,
  DB,
  fakeDatabase,
  fakeFeed,
  FIRST_SHOWN,
} from "@/test/databases";
import { apiError } from "@/test/fixtures";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { design, fakeWorkspaces, people } from "@/test/workspaces";

type Setup = { access?: "edit" | "view"; ready?: boolean };

/** The database's page, with the workspace, the pages and the database behind it. */
function openDatabase({ access = "edit", ready = true }: Setup = {}) {
  fakeWorkspaces();
  const pages = fakePages({ pages: databasePages(samplePages(), access) });
  const db = fakeDatabase({ access });
  const feed = fakeFeed({ access, ready });
  return { ...renderApp(DATABASE_PATH), db, feed, pages };
}

/** The live feed's state, as the view says it. */
async function feedState(word: string) {
  return (await database()).findByText(
    (_, element) => element?.tagName === "OUTPUT" && (element.textContent ?? "").startsWith(word),
  );
}

/** The database's views, once they show. */
async function database() {
  return within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
}

/** The table, once its rows came. */
async function table() {
  const view = await database();
  await view.findByRole("rowheader", { name: /Write docs/ }, FIRST_SHOWN);
  return within(view.getByRole("table", { name: "Table" }));
}

/** The changes made to a row, by their values. */
const rowChanges = (changes: Recorded[], rowId: string) =>
  changes
    .filter((change) => change.method === "PATCH" && change.path === `/api/v1/rows/${rowId}`)
    .map((change) => (change.body as { values: unknown }).values);

const viewChanges = (changes: Recorded[]) =>
  changes
    .filter((change) => change.method === "PATCH" && change.path.includes("/views/"))
    .map((change) => change.body);

describe("a database's table", () => {
  it("shows each row with its values, the title first, and counts them", async () => {
    openDatabase();
    const rows = await table();

    expect(rows.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      "Name",
      "Status",
      "Points",
      "Owner",
      "Due",
      "Tags",
      "Shipped",
      "Link",
      "ID",
      "",
    ]);
    expect(rows.getAllByRole("rowheader").map((header) => header.textContent)).toEqual([
      "Write docs",
      "Fix login",
      "Launch",
    ]);
    expect(rows.getByRole("button", { name: /^Status of Write docs/ })).toHaveTextContent(
      "In progress",
    );
    expect(rows.getByRole("button", { name: /^Owner of Write docs/ })).toHaveTextContent("Ada");
    expect(rows.getByRole("button", { name: /^Tags of Fix login/ })).toHaveTextContent("Bug");
    expect(rows.getByRole("checkbox", { name: "Shipped of Fix login" })).toBeChecked();
    expect(rows.getAllByText("NW-2")).toHaveLength(1);
    expect((await database()).getByText("3 rows")).toBeInTheDocument();
    expect(await feedState("Live")).toBeInTheDocument();
  });

  it("edits a number in place and saves it, refusing what is not one", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Points of Fix login/ }));
    const field = await screen.findByRole("textbox", { name: "Points of Fix login" });
    await user.type(field, "lots{Enter}");
    expect(screen.getByText("Enter a number, such as 3 or 2.5.")).toBeInTheDocument();
    expect(rowChanges(db.changes, DB.login)).toEqual([]);

    await user.clear(field);
    await user.type(field, "1,250.5{Enter}");
    await waitFor(() =>
      expect(rowChanges(db.changes, DB.login)).toEqual([{ [DB.points]: 1250.5 }]),
    );
    expect(rows.getByRole("button", { name: /^Points of Fix login/ })).toHaveTextContent(
      /1.?250\.5/,
    );
  });

  it("picks a status, toggles tags and makes a new one from what is typed", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Status of Fix login/ }));
    await user.click(
      within(await screen.findByRole("list", { name: "Status of Fix login" })).getByRole("button", {
        name: "Done",
      }),
    );
    await waitFor(() =>
      expect(rows.getByRole("button", { name: /^Status of Fix login/ })).toHaveTextContent("Done"),
    );

    await user.click(rows.getByRole("button", { name: /^Tags of Fix login/ }));
    const tags = within(await screen.findByRole("list", { name: "Tags of Fix login" }));
    await user.click(tags.getByRole("button", { name: "Idea" }));
    await user.type(screen.getByRole("textbox", { name: "Find or create an option" }), "Chore");
    await user.click(screen.getByRole("button", { name: "Create “Chore”" }));

    await waitFor(() =>
      expect(rowChanges(db.changes, DB.login)).toEqual([
        { [DB.status]: DB.finished },
        { [DB.tags]: [DB.bug, DB.idea] },
        { [DB.tags]: [DB.bug, DB.idea, "0193a5b2-0000-7000-8000-000000000101"] },
      ]),
    );
    const made = db.changes.find((change) => change.path.endsWith(`/properties/${DB.tags}`));
    expect(made?.body).toEqual({
      options: [
        { id: DB.bug, name: "Bug", hue: 30 },
        { id: DB.idea, name: "Idea", hue: 270 },
        { name: "Chore", hue: 0 },
      ],
    });
  });

  it("checks a checkbox in place, and unchecking clears it", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("checkbox", { name: "Shipped of Write docs" }));
    await user.click(rows.getByRole("checkbox", { name: "Shipped of Fix login" }));

    await waitFor(() => expect(rowChanges(db.changes, DB.docs)).toEqual([{ [DB.done]: true }]));
    await waitFor(() => expect(rowChanges(db.changes, DB.login)).toEqual([{ [DB.done]: null }]));
  });

  it("sets a date with the browser's date field, then gives it an end", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Due of Launch/ }));
    fireEvent.change(await screen.findByLabelText("Start"), { target: { value: "2026-12-01" } });
    await user.click(screen.getByRole("checkbox", { name: "End date" }));
    fireEvent.change(screen.getByLabelText("End"), { target: { value: "2026-11-30" } });
    expect(screen.getByText("The end cannot come before the start.")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("End"), { target: { value: "2026-12-03" } });

    await waitFor(() =>
      expect(rowChanges(db.changes, DB.launch)).toEqual([
        { [DB.due]: { start: "2026-12-01" } },
        { [DB.due]: { start: "2026-12-01", end: "2026-12-01" } },
        { [DB.due]: { start: "2026-12-01", end: "2026-12-03" } },
      ]),
    );
  });

  it("picks people from the workspace's members", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Owner of Fix login/ }));
    const list = within(await screen.findByRole("list", { name: "Owner of Fix login" }));
    expect(list.getAllByRole("button").map((button) => button.textContent)).toEqual([
      "Ada",
      "Bob",
      "Cy",
      "Gus",
    ]);
    await user.click(list.getByRole("button", { name: "Bob" }));
    await user.click(list.getByRole("button", { name: "Cy" }));

    await waitFor(() =>
      expect(rowChanges(db.changes, DB.login)).toEqual([
        { [DB.owner]: [people.bob.id] },
        { [DB.owner]: [people.bob.id, people.cy.id] },
      ]),
    );
  });

  it("takes a link with a scheme the server keeps, and saves text as the popover closes", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Link of Launch/ }));
    const link = await screen.findByRole("textbox", { name: "Link of Launch" });
    await user.type(link, "javascript:alert(1){Enter}");
    expect(
      screen.getByText("Enter a link starting with http:, https:, mailto: or tel:."),
    ).toBeInTheDocument();
    await user.clear(link);
    await user.type(link, "https://example.com/launch{Enter}");

    await waitFor(() =>
      expect(rowChanges(db.changes, DB.launch)).toEqual([
        { [DB.link]: "https://example.com/launch" },
      ]),
    );
  });

  it("leaves a value as it was when its editor closes with Escape", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: /^Link of Launch/ }));
    await user.type(
      await screen.findByRole("textbox", { name: "Link of Launch" }),
      "https://x.example",
    );
    await user.keyboard("{Escape}");

    await waitFor(() =>
      expect(screen.queryByRole("textbox", { name: "Link of Launch" })).toBeNull(),
    );
    expect(rowChanges(db.changes, DB.launch)).toEqual([]);
  });

  it("adds a row with its title typed in place", async () => {
    const { user, db } = openDatabase();
    await table();

    await user.click((await database()).getByRole("button", { name: "Add a row" }));
    const title = await screen.findByRole("textbox", { name: "Title of Untitled" });
    await waitFor(() => expect(title).toHaveFocus());
    await user.type(title, "Ship it{Enter}");

    const made = "0193a5b2-0000-7000-8000-000000000101";
    expect(db.changes.filter((change) => change.method === "POST")).toEqual([
      { method: "POST", path: `/api/v1/databases/${DB.tasks}/rows`, body: {} },
    ]);
    await waitFor(() => expect(rowChanges(db.changes, made)).toEqual([{ [DB.name]: "Ship it" }]));
    expect(await (await table()).findByRole("rowheader", { name: /Ship it/ })).toBeInTheDocument();
  });

  it("sorts, hides and adds beside a column from its heading's menu", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: "Points, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Sort descending" }));
    await user.click(rows.getByRole("button", { name: "Due, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Hide in this view" }));
    await waitFor(() => expect(rows.queryByRole("columnheader", { name: /^Due/ })).toBeNull());

    await user.click(rows.getByRole("button", { name: "Points, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Insert right" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New property" }));
    await user.clear(dialog.getByLabelText("Name"));
    await user.type(dialog.getByLabelText("Name"), "Estimate");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(viewChanges(db.changes)).toHaveLength(3));
    const [sorted, hidden, inserted] = viewChanges(db.changes) as {
      sorts?: unknown;
      properties?: { property: string; visible: boolean }[];
    }[];
    expect(sorted).toEqual({ sorts: [{ property: DB.points, direction: "descending" }] });
    expect(hidden?.properties?.find((entry) => entry.property === DB.due)?.visible).toBe(false);
    const order = inserted?.properties?.map((entry) => entry.property) ?? [];
    expect(order.slice(2, 4)).toEqual([DB.points, "0193a5b2-0000-7000-8000-000000000101"]);
    expect(
      db.changes.find((change) => change.path === `/api/v1/databases/${DB.tasks}/properties`)?.body,
    ).toEqual({ name: "Estimate", type: "text", placement: { after: DB.points } });
  });

  it("resizes a column from the keyboard, saving its width once it settles", async () => {
    const { db } = openDatabase();
    const rows = await table();

    const width = rows.getByRole("slider", { name: "Width of Points" });
    expect(width).toHaveValue("180");
    fireEvent.change(width, { target: { value: "240" } });

    await waitFor(() => expect(viewChanges(db.changes)).toHaveLength(1));
    const [change] = viewChanges(db.changes) as {
      properties: { property: string; width?: number }[];
    }[];
    expect(change?.properties.find((entry) => entry.property === DB.points)?.width).toBe(240);
  });

  it("changes a property's type, asking first when values would be lost", async () => {
    const { user, db } = openDatabase();
    let asked = false;
    server.use(
      http.patch("/api/v1/databases/:id/properties/:property_id", async ({ request }) => {
        const body = (await request.clone().json()) as { confirm?: boolean };
        if (body.confirm !== true) {
          asked = true;
          return apiError(409, "lossy_change", "2 rows would lose their value");
        }
        return undefined;
      }),
    );
    const rows = await table();

    await user.click(rows.getByRole("button", { name: "Tags, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Change type…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Tags" }));
    await waitFor(() => expect(dialog.getByLabelText("Type")).toHaveFocus());
    await user.selectOptions(dialog.getByLabelText("Type"), "number");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    const confirm = within(
      await screen.findByRole("alertdialog", { name: "Some values would be lost" }),
    );
    expect(confirm.getByText("2 rows would lose their value.")).toBeInTheDocument();
    await user.click(confirm.getByRole("button", { name: "Change anyway" }));

    await waitFor(() =>
      expect(
        db.changes
          .filter((change) => change.path.endsWith(`/properties/${DB.tags}`))
          .map((change) => change.body),
      ).toEqual([{ type: "number", number_format: "plain", confirm: true }]),
    );
    expect(asked).toBe(true);
  });

  it("deletes a property after asking, but never the title", async () => {
    const { user, db } = openDatabase();
    const rows = await table();

    await user.click(rows.getByRole("button", { name: "Name, property menu" }));
    expect(await screen.findByRole("menuitem", { name: "Rename…" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Delete property" })).toBeNull();
    await user.keyboard("{Escape}");

    await user.click(rows.getByRole("button", { name: "Link, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete property" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: "Delete property",
      }),
    );
    await waitFor(() =>
      expect(db.changes.at(-1)).toEqual({
        method: "DELETE",
        path: `/api/v1/databases/${DB.tasks}/properties/${DB.link}`,
        body: undefined,
      }),
    );
    await waitFor(() => expect(rows.queryByRole("columnheader", { name: /^Link/ })).toBeNull());
  });

  it("shows a viewer the values without the ways to change them", async () => {
    openDatabase({ access: "view" });
    const view = await database();
    const rows = await table();

    expect(rows.queryByRole("button", { name: /^Points of/ })).toBeNull();
    expect(rows.getByRole("checkbox", { name: "Shipped of Fix login" })).toBeDisabled();
    expect(view.queryByRole("button", { name: "Add a row" })).toBeNull();
    expect(view.queryByRole("button", { name: "Filter" })).toBeNull();
    expect(view.queryByRole("button", { name: "Add a view" })).toBeNull();
    expect(rows.getByRole("columnheader", { name: "Points" })).toBeInTheDocument();
  });
});

describe("the table's live changes", () => {
  it("show what others change as the server tells of it", async () => {
    const { db, feed } = openDatabase();
    const rows = await table();
    await waitFor(() => expect(feed.urls).toHaveLength(1));
    expect(feed.urls[0]).toBe(`ws://localhost:3000/ws/databases/${DB.tasks}`);

    const login = db.state.rows.find((entry) => entry.id === DB.login);
    if (login) {
      login.values[DB.points] = 13;
    }
    feed.send({
      type: "row_updated",
      database_id: DB.tasks,
      row_id: DB.login,
      values: { [DB.points]: 13 },
      last_edited_at: "2026-11-22T12:00:00Z",
      last_edited_by: people.bob.id,
    });

    await waitFor(() =>
      expect(rows.getByRole("button", { name: /^Points of Fix login/ })).toHaveTextContent("13"),
    );
  });

  it("reads everything afresh once it is ready, and again once it reconnects", async () => {
    const { feed, db } = openDatabase({ ready: false });
    const rows = await table();
    const points = () => rows.getByRole("button", { name: /^Points of Write docs/ });
    expect(points()).toHaveTextContent("3");
    expect(await feedState("Connecting")).toBeInTheDocument();
    const docs = db.state.rows.find((entry) => entry.id === DB.docs);
    if (docs === undefined) {
      throw new Error("the sample has Write docs");
    }

    // Changed before the feed heard every change: read again once it is ready.
    docs.values[DB.points] = 21;
    feed.ready();
    await waitFor(() => expect(points()).toHaveTextContent("21"));
    expect(await feedState("Live")).toBeInTheDocument();

    // Changed while it was away: read again once it is back and ready.
    feed.close(1012);
    expect(await feedState("Reconnecting")).toBeInTheDocument();
    docs.values[DB.points] = 34;
    await waitFor(() => expect(feed.urls).toHaveLength(2), { timeout: 3000 });
    feed.ready();
    await waitFor(() => expect(points()).toHaveTextContent("34"));
    expect(await feedState("Live")).toBeInTheDocument();
  });

  it("keeps a title typed while the feed connects and reads the page afresh", async () => {
    const { feed, user, pages, queryClient } = openDatabase({ ready: false });
    await table();
    const title = screen.getByRole("textbox", { name: "Page title" });
    await user.clear(title);
    await user.type(title, "Sprint");
    const page = () => queryClient.getQueryState(keys.page(design().id, DB.tasks));
    const before = page()?.dataUpdateCount ?? 0;

    // Ready before the title is saved, the feed reads the page again, with its old title.
    feed.ready();
    await waitFor(() => expect(page()?.dataUpdateCount).toBeGreaterThan(before));
    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(pages.changes.at(-1)).toEqual({
        method: "PATCH",
        path: `/api/v1/pages/${DB.tasks}`,
        body: { title: "Sprint" },
      }),
    );
    await waitFor(() => expect(page()?.data).toEqual(expect.objectContaining({ title: "Sprint" })));
    const sidebar = within(screen.getByRole("complementary"));
    expect(sidebar.getByRole("link", { name: "Sprint" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Database Sprint" })).toBeInTheDocument();
    expect(title).toHaveValue("Sprint");
  });

  it("stops, and says why, once the server ends it for good", async () => {
    const { feed, user } = openDatabase();
    await table();
    await waitFor(() => expect(feed.urls).toHaveLength(1));

    feed.close(4404);
    expect(
      await screen.findByText(
        "You can no longer see this database, so its changes stopped showing.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(feed.urls).toHaveLength(2));
  });

  it("opens afresh once the database shows again, after the server ended it", async () => {
    const { feed, router } = openDatabase();
    await table();
    await waitFor(() => expect(feed.urls).toHaveLength(1));
    feed.close(4404);
    expect(
      await screen.findByText(
        "You can no longer see this database, so its changes stopped showing.",
      ),
    ).toBeInTheDocument();

    await router.navigate({ to: "/account" });
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Database Tasks" })).toBeNull(),
    );
    await router.navigate({
      to: "/w/$slug/p/$pageId",
      params: { slug: "design", pageId: DB.tasks },
    });

    await waitFor(() => expect(feed.urls).toHaveLength(2));
    expect(await feedState("Live")).toBeInTheDocument();
  });
});
