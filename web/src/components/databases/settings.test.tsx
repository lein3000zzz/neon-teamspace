import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Database } from "@/api/databases";
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
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { design, fakeWorkspaces, people } from "@/test/workspaces";

function openDatabase(view: string = DB.table) {
  fakeWorkspaces();
  fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase();
  const feed = fakeFeed();
  return { ...renderApp(`${DATABASE_PATH}?view=${view}`), db, feed };
}

/** Adds Chore to Tags on the server, as someone else does. */
function addChore(db: ReturnType<typeof fakeDatabase>) {
  const tags = db.state.properties.find((property) => property.id === DB.tags);
  if (tags === undefined) {
    throw new Error("the sample has Tags");
  }
  tags.options = [...(tags.options ?? []), { id: DB.chore, name: "Chore", hue: 60 }];
  return tags;
}

async function database() {
  const view = within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
  await view.findByText("3 rows", undefined, FIRST_SHOWN);
  return view;
}

/** A panel of the view's settings, opened from its button. */
async function panel(name: RegExp | string) {
  return within(await screen.findByRole("dialog", { name }));
}

const bodies = (changes: Recorded[], path: string, method = "PATCH") =>
  changes
    .filter((change) => change.method === method && change.path.endsWith(path))
    .map((change) => change.body);

describe("a view's filter", () => {
  it("is built of conditions and groups, and saved with the view once edits pause", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Filter" }));
    const filter = await panel("Filter");
    expect(filter.getByText("No filters: every row shows.")).toBeInTheDocument();
    await user.click(filter.getByRole("button", { name: "Add a filter" }));
    await user.selectOptions(filter.getByRole("combobox", { name: "Property" }), "Points");
    // By its value: the option reads ">", which its markup escapes.
    await user.selectOptions(filter.getByRole("combobox", { name: "Condition" }), "greater_than");
    await user.clear(filter.getByRole("textbox", { name: "Value" }));
    await user.type(filter.getByRole("textbox", { name: "Value" }), "2");

    await user.click(filter.getByRole("button", { name: "Add a filter group" }));
    const group = within(filter.getByRole("group", { name: "Filter group" }));
    await user.selectOptions(group.getByRole("combobox", { name: "Property" }), "Owner");
    expect(group.getByRole("combobox", { name: "Value" })).toHaveDisplayValue("Me");
    await user.click(group.getByRole("button", { name: "Add a filter" }));
    const [, second] = group.getAllByRole("combobox", { name: "Property" });
    if (second === undefined) {
      throw new Error("the group has two conditions");
    }
    await user.selectOptions(second, "Due");
    await user.selectOptions(
      group.getAllByRole("combobox", { name: "Condition" })[1] as HTMLElement,
      "is within",
    );
    await user.selectOptions(
      group.getByRole("combobox", { name: "How these filters combine" }),
      "Or",
    );
    await user.selectOptions(
      filter.getAllByRole("combobox", { name: "How these filters combine" })[0] as HTMLElement,
      "Or",
    );

    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: {
          or: [
            { property: DB.points, operator: "greater_than", value: 2 },
            {
              or: [
                { property: DB.owner, operator: "contains", value: "me" },
                { property: DB.due, operator: "within", value: "past_week" },
              ],
            },
          ],
        },
      }),
    );
    await waitFor(() =>
      expect(view.getByRole("button", { name: "Filter, 3 conditions" })).toBeInTheDocument(),
    );
  });

  it("takes a decimal or a negative number typed key by key", async () => {
    const { user, db } = openDatabase();
    const view = await database();
    await user.click(view.getByRole("button", { name: "Filter" }));
    const filter = await panel("Filter");
    await user.click(filter.getByRole("button", { name: "Add a filter" }));
    await user.selectOptions(filter.getByRole("combobox", { name: "Property" }), "Points");
    const value = filter.getByRole("textbox", { name: "Value" });

    await user.clear(value);
    await user.type(value, "2.5");
    expect(value).toHaveValue("2.5");
    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: { and: [{ property: DB.points, operator: "equals", value: 2.5 }] },
      }),
    );

    await user.clear(value);
    await user.type(value, "-3");
    expect(value).toHaveValue("-3");
    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: { and: [{ property: DB.points, operator: "equals", value: -3 }] },
      }),
    );
  });

  it("compares a date with a day relative to today, or an exact one", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Filter" }));
    const filter = await panel("Filter");
    await user.click(filter.getByRole("button", { name: "Add a filter" }));
    await user.selectOptions(filter.getByRole("combobox", { name: "Property" }), "Due");
    await user.selectOptions(filter.getByRole("combobox", { name: "Condition" }), "is before");
    await user.selectOptions(filter.getByRole("combobox", { name: "Value" }), "One week from now");
    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: { and: [{ property: DB.due, operator: "before", value: "one_week_from_now" }] },
      }),
    );
    await user.selectOptions(filter.getByRole("combobox", { name: "Value" }), "An exact date");
    // An exact date starts at today, in the reader's time zone.
    const day = (filter.getByLabelText("Day") as HTMLInputElement).value;
    expect(day).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: { and: [{ property: DB.due, operator: "before", value: day }] },
      }),
    );
  });

  it("starts with a condition on the column its menu's Filter by it came from", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Status, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Filter by it" }));
    const filter = await panel("Filter");
    expect(filter.getByRole("combobox", { name: "Property" })).toHaveDisplayValue("Status");
    await user.selectOptions(
      filter.getByRole("combobox", { name: "Condition" }),
      "is in the group",
    );
    await user.selectOptions(filter.getByRole("combobox", { name: "Value" }), "Done");

    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`).at(-1)).toEqual({
        filter: { and: [{ property: DB.status, operator: "in_group", value: "done" }] },
      }),
    );
  });

  it("is removed when its last condition goes", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Filter" }));
    const filter = await panel("Filter");
    await user.click(filter.getByRole("button", { name: "Add a filter" }));
    await user.selectOptions(filter.getByRole("combobox", { name: "Property" }), "Shipped");
    await waitFor(() => expect(bodies(db.changes, `/views/${DB.table}`)).toHaveLength(1));
    await user.click(filter.getByRole("button", { name: "Remove this filter" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`)).toEqual([
        { filter: { and: [{ property: DB.done, operator: "is", value: true }] } },
        { filter: null },
      ]),
    );
  });
});

describe("a view's sorts", () => {
  it("are added, turned, reordered and removed, each change saved at once", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Sort" }));
    const sort = await panel("Sort");
    await user.click(sort.getByRole("button", { name: "Add a sort" }));
    await user.selectOptions(
      await sort.findByRole("combobox", { name: "Property of sort 1" }),
      "Points",
    );
    await user.selectOptions(
      sort.getByRole("combobox", { name: "Direction of sort 1" }),
      "Descending",
    );
    await user.click(sort.getByRole("button", { name: "Add a sort" }));
    await user.click(await sort.findByRole("button", { name: "Move sort 2 up" }));
    await user.click(sort.getByRole("button", { name: "Remove sort 2" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`)).toEqual([
        { sorts: [{ property: DB.name, direction: "ascending" }] },
        { sorts: [{ property: DB.points, direction: "ascending" }] },
        { sorts: [{ property: DB.points, direction: "descending" }] },
        {
          sorts: [
            { property: DB.points, direction: "descending" },
            { property: DB.name, direction: "ascending" },
          ],
        },
        {
          sorts: [
            { property: DB.name, direction: "ascending" },
            { property: DB.points, direction: "descending" },
          ],
        },
        { sorts: [{ property: DB.name, direction: "ascending" }] },
      ]),
    );
  });
});

describe("a view's grouping and properties", () => {
  it("groups a table by a property and hides one of its groups", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Group" }));
    const group = await panel("Group");
    await user.selectOptions(group.getByLabelText("Group by"), "Owner");
    expect(await view.findByRole("columnheader", { name: /^Ada/ })).toBeInTheDocument();
    await user.click(await group.findByRole("checkbox", { name: /^Ada/ }));

    await waitFor(() =>
      expect(bodies(db.changes, `/views/${DB.table}`)).toEqual([
        { group: { property: DB.owner } },
        { group: { property: DB.owner, order: [], hidden: [people.ada.id] } },
      ]),
    );
  });

  it("shows, hides and orders the properties a view shows, but never hides the title", async () => {
    const { user, db } = openDatabase(DB.list);
    const view = await database();

    await user.click(view.getByRole("button", { name: "Properties" }));
    const picker = await panel("Properties");
    expect(picker.getByRole("checkbox", { name: "Show Name" })).toBeDisabled();
    await user.click(picker.getByRole("checkbox", { name: "Show Points" }));
    await user.click(picker.getByRole("button", { name: "Move Points up" }));

    await waitFor(() => expect(bodies(db.changes, `/views/${DB.list}`)).toHaveLength(2));
    const [shown, moved] = bodies(db.changes, `/views/${DB.list}`) as {
      properties: { property: string; visible: boolean }[];
    }[];
    expect(shown?.properties.find((entry) => entry.property === DB.points)?.visible).toBe(true);
    expect(moved?.properties.slice(0, 3).map((entry) => entry.property)).toEqual([
      DB.name,
      DB.points,
      DB.status,
    ]);
  });
});

describe("a property's settings", () => {
  it("edits a select's options, their colours and order", async () => {
    const { user, db } = openDatabase(DB.list);
    const view = await database();

    await user.click(view.getByRole("button", { name: "Properties" }));
    await user.click((await panel("Properties")).getByRole("button", { name: "Edit Tags" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Tags" }));
    await user.click(dialog.getByRole("button", { name: "Add an option" }));
    await user.type(dialog.getByRole("textbox", { name: "Name of option 3" }), "Chore");
    await user.click(dialog.getByRole("button", { name: "Colour of Chore" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "Colour of Chore" })).getByRole("button", {
        name: "Teal",
      }),
    );
    await user.click(dialog.getByRole("button", { name: "Move Chore up" }));
    await user.click(dialog.getByRole("button", { name: "Remove Bug" }));
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/properties/${DB.tags}`)).toEqual([
        {
          options: [
            { name: "Chore", hue: 180 },
            { id: DB.idea, name: "Idea", hue: 270 },
          ],
        },
      ]),
    );
  });

  it("leaves the options out of a rename, so one added elsewhere meanwhile stays", async () => {
    const { user, db, feed, queryClient } = openDatabase();
    const view = await database();
    await user.click(view.getByRole("button", { name: "Tags, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Tags" }));

    const tags = addChore(db);
    feed.send({ type: "schema_changed", database_id: DB.tasks });
    await waitFor(() =>
      expect(
        queryClient
          .getQueryData<Database>(keys.database(design().id, DB.tasks))
          ?.properties.find((property) => property.id === DB.tags)?.options,
      ).toHaveLength(3),
    );
    await user.clear(dialog.getByLabelText("Name"));
    await user.type(dialog.getByLabelText("Name"), "Labels");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/properties/${DB.tags}`)).toEqual([{ name: "Labels" }]),
    );
    expect(tags.options?.map((option) => option.name)).toEqual(["Bug", "Idea", "Chore"]);
  });

  it("keeps an option added elsewhere while the options were edited", async () => {
    const { user, db } = openDatabase();
    const view = await database();
    await user.click(view.getByRole("button", { name: "Tags, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Tags" }));

    // Not yet told of: the options sent are made from those the server has as they go.
    addChore(db);
    const bug = dialog.getByRole("textbox", { name: "Name of option 1" });
    await user.clear(bug);
    await user.type(bug, "Defect");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/properties/${DB.tags}`)).toEqual([
        {
          options: [
            { id: DB.bug, name: "Defect", hue: 30 },
            { id: DB.idea, name: "Idea", hue: 270 },
            { id: DB.chore, name: "Chore", hue: 60 },
          ],
        },
      ]),
    );
  });

  it("makes a status with its three groups, each option in one", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Add a property" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New property" }));
    await user.selectOptions(dialog.getByLabelText("Type"), "status");
    expect(dialog.getByLabelText("Name")).toHaveValue("Status 2");
    await user.selectOptions(
      dialog.getByRole("combobox", { name: "Group of In progress" }),
      "Done",
    );
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(bodies(db.changes, `/databases/${DB.tasks}/properties`, "POST")).toEqual([
        {
          name: "Status 2",
          type: "status",
          options: [
            { name: "Not started", group: "to_do" },
            { name: "In progress", hue: 240, group: "done" },
            { name: "Done", hue: 150, group: "done" },
          ],
        },
      ]),
    );
  });

  it("sets a number's format and a unique id's prefix, which come with their type", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Points, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    let dialog = within(await screen.findByRole("dialog", { name: "Edit Points" }));
    await user.selectOptions(dialog.getByLabelText("Number format"), "Currency");
    await user.clear(dialog.getByLabelText("Currency"));
    await user.type(dialog.getByLabelText("Currency"), "usd");
    await user.click(dialog.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit Points" })).toBeNull());

    await user.click(view.getByRole("button", { name: "ID, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    dialog = within(await screen.findByRole("dialog", { name: "Edit ID" }));
    await user.clear(dialog.getByLabelText("Prefix"));
    await user.type(dialog.getByLabelText("Prefix"), "tk");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect([
        ...bodies(db.changes, `/properties/${DB.points}`),
        ...bodies(db.changes, `/properties/${DB.ticket}`),
      ]).toEqual([
        { type: "number", number_format: "currency", currency: "USD" },
        { type: "unique_id", prefix: "TK" },
      ]),
    );
  });

  it("shows dates in the format the reader picks, which stays in this browser", async () => {
    const { user, db } = openDatabase();
    const view = await database();
    const launch = db.state.rows.find((entry) => entry.id === DB.launch);
    if (launch === undefined) {
      throw new Error("the sample has Launch");
    }
    await user.click(view.getByRole("button", { name: /^Due of Launch/ }));
    await user.type(await screen.findByLabelText("Start"), "2026-12-01");
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(view.getByRole("button", { name: /^Due of Launch/ })).toHaveTextContent("2026"),
    );

    await user.click(view.getByRole("button", { name: "Due, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Due" }));
    await user.selectOptions(dialog.getByLabelText("Date format"), "2026-11-22");
    await user.click(dialog.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(view.getByRole("button", { name: /^Due of Launch/ })).toHaveTextContent("2026-12-01"),
    );
    expect(bodies(db.changes, `/properties/${DB.due}`)).toEqual([]);
    expect(window.localStorage.getItem(`neon.date-format.${DB.due}`)).toBe("numeric");
  });
});

describe("a database's views", () => {
  it("are tabs the arrow keys move between", async () => {
    const { user } = openDatabase();
    const view = await database();

    view.getByRole("tab", { name: "Table" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(view.getByRole("tab", { name: "Board" })).toHaveAttribute("aria-selected", "true");
    expect(view.getByRole("tab", { name: "Board" })).toHaveFocus();
    await user.keyboard("{End}");
    expect(view.getByRole("tab", { name: "List" })).toHaveAttribute("aria-selected", "true");
    expect(await view.findByRole("tabpanel", { name: "List" })).toBeInTheDocument();
  });

  it("are added as boards grouped by status, renamed, duplicated, moved and deleted", async () => {
    const { user, db } = openDatabase();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Add a view" }));
    await user.click(await screen.findByRole("menuitem", { name: "Board" }));
    expect(await view.findByRole("tab", { name: "Board 2", selected: true })).toBeInTheDocument();

    await user.click(view.getByRole("button", { name: "More for the view Board 2" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const name = view.getByRole("textbox", { name: "Rename Board 2" });
    await user.clear(name);
    await user.type(name, "Sprint{Enter}");
    expect(await view.findByRole("tab", { name: "Sprint" })).toBeInTheDocument();

    await user.click(view.getByRole("button", { name: "More for the view Sprint" }));
    await user.click(await screen.findByRole("menuitem", { name: "Duplicate" }));
    expect(
      await view.findByRole("tab", { name: "Sprint (copy)", selected: true }),
    ).toBeInTheDocument();

    await user.click(view.getByRole("button", { name: "More for the view Sprint (copy)" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move left" }));
    await waitFor(() =>
      expect(view.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
        "Table",
        "Board",
        "List",
        "Sprint (copy)",
        "Sprint",
      ]),
    );

    await user.click(view.getByRole("button", { name: "More for the view Sprint (copy)" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() =>
      expect(view.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
        "Table",
        "Board",
        "List",
        "Sprint",
      ]),
    );
    expect(bodies(db.changes, `/databases/${DB.tasks}/views`, "POST")).toEqual([
      { name: "Board 2", layout: "board", group: { property: DB.status } },
    ]);
  });

  it("shows a view as another layout, a board grouping by the first property it can", async () => {
    const { user, db } = openDatabase(DB.list);
    const view = await database();

    await user.click(view.getByRole("button", { name: "More for the view List" }));
    await user.click(await screen.findByRole("menuitem", { name: "Show as board" }));

    expect(await view.findByRole("region", { name: "No Status" })).toBeInTheDocument();
    expect(bodies(db.changes, `/views/${DB.list}`)).toEqual([
      { layout: "board", group: { property: DB.status } },
    ]);
  });
});
