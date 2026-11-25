import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DB, properties, row, sampleRows, tasksPage, views } from "@/test/databases";
import { people } from "@/test/workspaces";
import { afterFeedClose, applyFeedEvent, feedServer } from "./database-feed";
import { REFRESH_MS, type Database, type Row, type ViewRows } from "./databases";
import { keys } from "./keys";

const WORKSPACE = "0193a5b2-0000-7000-8000-00000000a001";
const database: Database = {
  page: tasksPage(),
  inline: false,
  properties: properties(),
  views: views(),
};

/** A client holding the database, its table's and board's rows, and one row's page. */
function seeded() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const rows = sampleRows();
  client.setQueryData(keys.database(WORKSPACE, DB.tasks), database);
  client.setQueryData<ViewRows>(keys.viewRows(WORKSPACE, DB.tasks, DB.table), {
    rows,
    groups: null,
    total: rows.length,
  });
  client.setQueryData<ViewRows>(keys.viewRows(WORKSPACE, DB.tasks, DB.board), {
    rows: rows.map((entry) => ({ ...entry, group: (entry.values[DB.status] as string) ?? null })),
    groups: [
      { key: null, count: 1, hidden: false },
      { key: DB.notStarted, count: 1, hidden: false },
      { key: DB.inProgress, count: 1, hidden: false },
      { key: DB.finished, count: 0, hidden: false },
    ],
    total: 3,
  });
  client.setQueryData(keys.row(WORKSPACE, DB.docs), rows[0]);
  return client;
}

const table = (client: QueryClient) =>
  client.getQueryData<ViewRows>(keys.viewRows(WORKSPACE, DB.tasks, DB.table));
const board = (client: QueryClient) =>
  client.getQueryData<ViewRows>(keys.viewRows(WORKSPACE, DB.tasks, DB.board));

describe("a database's live feed", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("connects to its database on this origin, over wss: from https:", () => {
    expect(feedServer({ protocol: "https:", host: "teams.example" })).toBe(
      "wss://teams.example/ws/databases",
    );
    expect(feedServer({ protocol: "http:", host: "localhost:5173" })).toBe(
      "ws://localhost:5173/ws/databases",
    );
  });

  it("stops for a session or access that ended, pauses when asked, and else tries again", () => {
    expect(afterFeedClose(4401)).toBe("signedOut");
    expect(afterFeedClose(4404)).toBe("accessLost");
    expect(afterFeedClose(4408)).toBe("later");
    expect(afterFeedClose(4429)).toBe("later");
    expect(afterFeedClose(1013)).toBe("later");
    expect(afterFeedClose(1009)).toBe("again");
    expect(afterFeedClose(4400)).toBe("again");
    expect(afterFeedClose(1012)).toBe("again");
    expect(afterFeedClose(1006)).toBe("again");
  });

  it("writes a row's changed values into every view and its own page at once, then reads the views again", async () => {
    const client = seeded();
    const read = vi.spyOn(client, "invalidateQueries");
    applyFeedEvent(client, WORKSPACE, DB.tasks, {
      type: "row_updated",
      database_id: DB.tasks,
      row_id: DB.docs,
      values: { [DB.points]: 8, [DB.owner]: null, [DB.name]: "Write the docs" },
      title: "Write the docs",
      last_edited_at: "2026-11-22T12:00:00Z",
      last_edited_by: people.bob.id,
    });
    const changed = table(client)?.rows[0];
    expect(changed?.title).toBe("Write the docs");
    expect(changed?.values[DB.points]).toBe(8);
    expect(changed?.values).not.toHaveProperty(DB.owner);
    expect(changed?.last_edited_by).toBe(people.bob.id);
    expect(client.getQueryData<Row>(keys.row(WORKSPACE, DB.docs))?.values[DB.points]).toBe(8);
    expect(read).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(read).toHaveBeenCalledWith({ queryKey: keys.databaseRows(WORKSPACE, DB.tasks) });
  });

  it("adds a new row to the views that show every row, in its group on a board", () => {
    const client = seeded();
    const made = row(
      {
        id: "0193a5b2-0000-7000-8000-000000000101",
        number: 4,
        title: "Ship it",
        values: { [DB.status]: DB.finished },
      },
      "a3",
    );
    const { access: _access, ...fed } = made;
    applyFeedEvent(client, WORKSPACE, DB.tasks, {
      type: "row_created",
      database_id: DB.tasks,
      row: fed,
    });
    expect(table(client)?.rows.map((entry) => entry.title)).toContain("Ship it");
    expect(table(client)?.total).toBe(4);
    const done = board(client)?.rows.filter((entry) => entry.group === DB.finished);
    expect(done?.map((entry) => entry.title)).toEqual(["Ship it"]);
    expect(board(client)?.groups?.find((group) => group.key === DB.finished)?.count).toBe(1);
  });

  it("takes a trashed row out of every view, and its group's count with it", () => {
    const client = seeded();
    applyFeedEvent(client, WORKSPACE, DB.tasks, {
      type: "row_trashed",
      database_id: DB.tasks,
      row_id: DB.docs,
    });
    expect(table(client)?.rows.map((entry) => entry.id)).not.toContain(DB.docs);
    expect(board(client)?.groups?.find((group) => group.key === DB.inProgress)?.count).toBe(0);
  });

  it("reads the database again when its schema changes, and a view's rows when it changes", () => {
    const client = seeded();
    const read = vi.spyOn(client, "invalidateQueries");
    applyFeedEvent(client, WORKSPACE, DB.tasks, { type: "schema_changed", database_id: DB.tasks });
    expect(read).toHaveBeenCalledWith({ queryKey: keys.database(WORKSPACE, DB.tasks) });
    applyFeedEvent(client, WORKSPACE, DB.tasks, {
      type: "view_changed",
      database_id: DB.tasks,
      view_id: DB.board,
      deleted: true,
    });
    expect(board(client)).toBeUndefined();
  });
});
