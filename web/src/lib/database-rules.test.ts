import { describe, expect, it } from "vitest";
import type { Database, ViewRows } from "@/api/databases";
import { DB, properties, row, sampleRows, tasksPage, views } from "@/test/databases";
import { people } from "@/test/workspaces";
import {
  columnsOf,
  conditionCount,
  draftOf,
  filterOf,
  filterSize,
  formatDate,
  formatNumber,
  freshName,
  kindsFor,
  mergedOptions,
  movedCard,
  movedColumn,
  nextHue,
  operatorsFor,
  readNumber,
  shownColumns,
  startingValue,
  valueOf,
  valuesMeeting,
} from "./database-rules";

const database: Database = {
  page: tasksPage(),
  inline: false,
  properties: properties(),
  views: views(),
};

const status = properties()[1];
if (status === undefined) {
  throw new Error("the sample has a status");
}

/** The board's rows, grouped by status as the server lists them. */
function board(): ViewRows {
  const [docs, login, launch] = sampleRows();
  if (docs === undefined || login === undefined || launch === undefined) {
    throw new Error("the sample has three rows");
  }
  return {
    rows: [
      { ...launch, group: null },
      { ...login, group: DB.notStarted },
      { ...docs, group: DB.inProgress },
    ],
    total: 3,
    groups: [
      { key: null, count: 1, hidden: false },
      { key: DB.notStarted, count: 1, hidden: false },
      { key: DB.inProgress, count: 1, hidden: false },
      { key: DB.finished, count: 0, hidden: false },
    ],
  };
}

describe("a card moved on a board", () => {
  it("takes the group's value and joins it last, the counts following", () => {
    const moved = movedCard(
      board(),
      { rowId: DB.docs, from: DB.inProgress, to: DB.finished, before: null },
      status,
    );
    expect(moved.rows.map((entry) => [entry.title, entry.group])).toEqual([
      ["Launch", null],
      ["Fix login", DB.notStarted],
      ["Write docs", DB.finished],
    ]);
    expect(moved.rows.at(-1)?.values[DB.status]).toBe(DB.finished);
    expect(moved.groups?.map((group) => group.count)).toEqual([1, 1, 0, 1]);
  });

  it("goes before the card it was dropped on, and loses its value in the empty group", () => {
    const moved = movedCard(
      board(),
      { rowId: DB.docs, from: DB.inProgress, to: null, before: DB.launch },
      status,
    );
    expect(moved.rows.map((entry) => entry.title)).toEqual(["Write docs", "Launch", "Fix login"]);
    expect(moved.rows[0]?.values).not.toHaveProperty(DB.status);
  });

  it("on a board of people, swaps the person it leaves for the one it joins", () => {
    const owner = properties()[3];
    if (owner === undefined) {
      throw new Error("the sample has an owner");
    }
    const both = row(
      {
        id: DB.docs,
        number: 1,
        title: "Write docs",
        values: { [DB.owner]: [people.ada.id, people.bob.id] },
      },
      "a0",
    );
    const answer: ViewRows = {
      rows: [
        { ...both, group: people.ada.id },
        { ...both, group: people.bob.id },
      ],
      total: 2,
      groups: [
        { key: people.ada.id, count: 1, hidden: false },
        { key: people.bob.id, count: 1, hidden: false },
        { key: people.cy.id, count: 0, hidden: false },
      ],
    };
    const toCy = movedCard(
      answer,
      { rowId: DB.docs, from: people.ada.id, to: people.cy.id, before: null },
      owner,
    );
    expect(toCy.rows.map((entry) => entry.group)).toEqual([people.bob.id, people.cy.id]);
    expect(toCy.rows[0]?.values[DB.owner]).toEqual([people.bob.id, people.cy.id]);

    // Joining a group it is in already, it is listed there once.
    const merged = movedCard(
      answer,
      { rowId: DB.docs, from: people.ada.id, to: people.bob.id, before: null },
      owner,
    );
    expect(merged.rows.map((entry) => entry.group)).toEqual([people.bob.id]);
    expect(merged.total).toBe(1);
    expect(merged.groups?.map((group) => group.count)).toEqual([0, 1, 0]);
  });
});

describe("a filter's draft", () => {
  it("reads a saved filter and writes it back, leaving out what is not ready", () => {
    const saved = {
      or: [
        { property: DB.points, operator: "greater_than" as const, value: 2 },
        {
          and: [
            { property: DB.owner, operator: "contains" as const, value: "me" },
            { property: DB.due, operator: "is_empty" as const },
          ],
        },
      ],
    };
    const draft = draftOf(saved);
    expect(draft.conjunction).toBe("or");
    expect(conditionCount(draft)).toBe(3);
    expect(filterOf(draft)).toEqual(saved);

    const half = {
      ...draft,
      items: [
        ...draft.items,
        ...draftOf({ property: DB.name, operator: "contains", value: " " }).items,
      ],
    };
    expect(filterOf(half)).toEqual(saved);
    expect(filterOf(draftOf(null))).toBeNull();
    expect(filterSize(saved)).toBe(3);
  });

  it("offers each type its operators, and starts each with a value it takes", () => {
    expect(operatorsFor("checkbox")).toEqual(["is"]);
    expect(operatorsFor("status")).toContain("in_group");
    expect(operatorsFor("select")).not.toContain("in_group");
    expect(operatorsFor("person")).toEqual([
      "contains",
      "does_not_contain",
      "is_empty",
      "is_not_empty",
    ]);
    expect(startingValue(status, "in_group")).toBe("to_do");
    expect(startingValue(status, "is")).toBe(DB.notStarted);
    expect(startingValue(status, "is_empty")).toBeUndefined();
  });
});

describe("a view's columns", () => {
  it("list every property in the view's order, the title first among those shown", () => {
    const list = views()[2];
    if (list === undefined) {
      throw new Error("the sample has a list");
    }
    expect(shownColumns(database, list).map((column) => column.property.name)).toEqual([
      "Name",
      "Status",
    ]);
    expect(columnsOf(database, list)).toHaveLength(properties().length);
  });

  it("move before or after another", () => {
    const table = views()[0];
    if (table === undefined) {
      throw new Error("the sample has a table");
    }
    const moved = movedColumn(columnsOf(database, table), DB.ticket, DB.status, "before");
    expect(moved.map((column) => column.property.name).slice(0, 3)).toEqual([
      "Name",
      "ID",
      "Status",
    ]);
  });
});

describe("values", () => {
  it("read computed ones from the row", () => {
    const [docs] = sampleRows();
    const ticket = properties().find((property) => property.type === "unique_id");
    if (docs === undefined || ticket === undefined) {
      throw new Error("the sample has a row and an id");
    }
    expect(valueOf(docs, ticket)).toBe("NW-1");
  });

  it("show numbers in their format", () => {
    expect(formatNumber(0.25, "percent")).toMatch(/25\s?%/);
    expect(formatNumber(1234.567, "integer")).toMatch(/1.?235/);
    expect(formatNumber(12.5, "currency", "EUR")).toMatch(/12[.,]50/);
  });

  it("show days without shifting them, and ranges with an arrow", () => {
    expect(formatDate({ start: "2026-11-22" }, "numeric")).toBe("2026-11-22");
    expect(formatDate({ start: "2026-11-22", end: "2026-11-24" }, "numeric")).toBe(
      "2026-11-22 → 2026-11-24",
    );
  });
});

describe("new things' names and hues", () => {
  it("names a property so no other has its name", () => {
    expect(freshName(database, "Points")).toBe("Points 2");
    expect(freshName(database, "Notes")).toBe("Notes");
  });

  it("offers a unique id type only while the database has none", () => {
    expect(kindsFor(database)).not.toContain("unique_id");
    const ticket = database.properties.find((property) => property.type === "unique_id");
    expect(kindsFor(database, ticket)).toContain("unique_id");
    expect(kindsFor(database, database.properties[0])).toEqual(["title"]);
  });

  it("picks a hue the options do not use yet", () => {
    expect(nextHue([{ hue: 0 }, { hue: 30 }])).toBe(60);
  });
});

describe("a property's options as a change sends them", () => {
  it("keep those added elsewhere meanwhile, and drop those deleted elsewhere", () => {
    const known = new Set([DB.bug, DB.idea]);
    const edited = [
      { id: DB.bug, name: "Defect", hue: 30 },
      { id: DB.idea, name: "Idea", hue: 270 },
      { name: "Spike", hue: 90 },
    ];
    // Idea was deleted and Chore added by someone else while these were edited.
    const live = [
      { id: DB.bug, name: "Bug", hue: 30 },
      { id: DB.chore, name: "Chore", hue: 60 },
    ];
    expect(mergedOptions(edited, known, live)).toEqual([
      { id: DB.bug, name: "Defect", hue: 30 },
      { name: "Spike", hue: 90 },
      { id: DB.chore, name: "Chore", hue: 60 },
    ]);
  });
});

describe("a number as it is typed", () => {
  it("reads signs, fractions and groups of digits", () => {
    expect(readNumber("-3", ".")).toBe(-3);
    expect(readNumber("+2.5", ".")).toBe(2.5);
    expect(readNumber("2.", ".")).toBe(2);
    expect(readNumber(".5", ".")).toBe(0.5);
    expect(readNumber("1e3", ".")).toBe(1000);
    expect(readNumber("1 250 000", ".")).toBe(1250000);
    expect(readNumber("1,250.5", ".")).toBe(1250.5);
    expect(readNumber("1.250,5", ".")).toBe(1250.5);
    expect(readNumber("12,345,678", ".")).toBe(12345678);
    expect(readNumber("1.234.567", ",")).toBe(1234567);
  });

  it("takes a lone comma for the decimal separator unless it groups three digits", () => {
    expect(readNumber("3,75", ".")).toBe(3.75);
    expect(readNumber("1,5", ".")).toBe(1.5);
    expect(readNumber("1234,5", ".")).toBe(1234.5);
    expect(readNumber("1,250", ".")).toBe(1250);
    expect(readNumber("1,250", ",")).toBe(1.25);
    expect(readNumber("1.250", ",")).toBe(1250);
    expect(readNumber("1.250", ".")).toBe(1.25);
  });

  it("refuses what is not a decimal number", () => {
    for (const text of [
      "",
      "-",
      ".",
      "lots",
      "0x1F",
      "0b101",
      "1,2,3",
      "1.2.3",
      "1,5.5,5",
      "1e999",
    ]) {
      expect(readNumber(text, ".")).toBeUndefined();
    }
  });
});

describe("a new row's values in a view that filters", () => {
  const now = new Date(2026, 0, 31);

  it("meet each condition every row must, where one value plainly does", () => {
    expect(
      valuesMeeting(
        database,
        {
          and: [
            { property: DB.status, operator: "in_group", value: "done" },
            { property: DB.tags, operator: "contains", value: DB.bug },
            { and: [{ property: DB.tags, operator: "contains", value: DB.idea }] },
            { property: DB.points, operator: "less_than", value: 1 },
            { property: DB.due, operator: "after", value: "one_month_from_now" },
            { property: DB.name, operator: "starts_with", value: "Fix" },
            { property: DB.link, operator: "contains", value: "example" },
            { property: DB.done, operator: "is", value: false },
          ],
        },
        people.ada.id,
        now,
      ),
    ).toEqual({
      [DB.status]: DB.finished,
      [DB.tags]: [DB.bug, DB.idea],
      [DB.points]: 0,
      [DB.due]: { start: "2026-03-01" },
      [DB.name]: "Fix",
    });
  });

  it("set nothing for an or, a negation, or people for a guest", () => {
    expect(
      valuesMeeting(
        database,
        {
          and: [
            {
              or: [
                { property: DB.status, operator: "is", value: DB.finished },
                { property: DB.done, operator: "is", value: true },
              ],
            },
            { property: DB.points, operator: "not_equals", value: 3 },
            { property: DB.owner, operator: "contains", value: "me" },
            { property: DB.ticket, operator: "equals", value: 4 },
          ],
        },
        null,
        now,
      ),
    ).toEqual({});
    expect(
      valuesMeeting(
        database,
        { property: DB.due, operator: "within", value: "next_week" },
        null,
        now,
      ),
    ).toEqual({ [DB.due]: { start: "2026-01-31" } });
  });
});
