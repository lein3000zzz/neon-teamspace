import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import {
  DATABASE_PATH,
  databasePages,
  DB,
  fakeDatabase,
  fakeFeed,
  FIRST_SHOWN,
  row,
  views,
} from "@/test/databases";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeWorkspaces, people } from "@/test/workspaces";

/** The database's page, open on one of its views. */
function openView(view: string) {
  fakeWorkspaces();
  fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase();
  const feed = fakeFeed();
  return { ...renderApp(`${DATABASE_PATH}?view=${view}`), db, feed };
}

async function database() {
  return within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
}

/** A column of the board, by its group's name. */
async function column(name: string) {
  return within(await (await database()).findByRole("region", { name }));
}

const moves = (changes: { method: string; path: string; body: unknown }[]) =>
  changes.filter((change) => change.path.endsWith("/move")).map((change) => change.body);

describe("a database's board", () => {
  it("has a column per status, in the options' order, each with its cards and count", async () => {
    openView(DB.board);
    const view = await database();
    await view.findByRole("article", { name: "Write docs" });

    const columns = view.getAllByRole("region");
    expect(columns.map((entry) => entry.getAttribute("aria-label"))).toEqual([
      "No Status",
      "Not started",
      "In progress",
      "Done",
    ]);
    expect((await column("In progress")).getByRole("article")).toHaveAccessibleName("Write docs");
    expect((await column("No Status")).getByRole("article")).toHaveAccessibleName("Launch");
    // Cards show the values the view picks, here the points and the owner.
    const card = within((await column("In progress")).getByRole("article"));
    expect(card.getByText("3")).toBeInTheDocument();
    expect(card.getByText("Ada")).toBeInTheDocument();
    // Columns of options with a hue are drawn in it.
    expect(view.getByRole("region", { name: "In progress" })).toHaveAttribute("data-hue");
    expect(view.getByRole("region", { name: "No Status" })).not.toHaveAttribute("data-hue");
  });

  it("moves a card to another column from its menu, the keyboard's and touch's way", async () => {
    const { user, db } = openView(DB.board);
    const progress = await column("In progress");
    await progress.findByRole("article", { name: "Write docs" });

    await user.click(progress.getByRole("button", { name: "More for Write docs" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to Done" }));

    expect(
      await (await column("Done")).findByRole("article", { name: "Write docs" }),
    ).toBeVisible();
    expect(progress.queryByRole("article")).toBeNull();
    expect(screen.getByText("Moved Write docs to Done.")).toBeInTheDocument();
    await waitFor(() =>
      expect(moves(db.changes)).toEqual([
        { group: { view_id: DB.board, from: DB.inProgress, to: DB.finished } },
      ]),
    );
  });

  it("drags a card into another column, before the card it is dropped over", async () => {
    const { db } = openView(DB.board);
    const view = await database();
    const card = await view.findByRole("article", { name: "Write docs" });
    const target = view.getByRole("region", { name: "Not started" });
    const below = within(target).getByRole("article", { name: "Fix login" });
    // jsdom lays nothing out: the point lands on the column, above Fix login's middle.
    vi.spyOn(document, "elementsFromPoint").mockReturnValue([below, target]);
    vi.spyOn(below, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 100, 200, 40));

    fireEvent.pointerDown(card, {
      button: 0,
      pointerId: 7,
      pointerType: "mouse",
      clientX: 10,
      clientY: 10,
    });
    fireEvent.pointerMove(window, { pointerId: 7, pointerType: "mouse", clientX: 40, clientY: 90 });
    expect(await screen.findByText("Moving Write docs")).toBeInTheDocument();
    fireEvent.pointerUp(window, { pointerId: 7, pointerType: "mouse", clientX: 40, clientY: 90 });

    const cards = await (await column("Not started")).findAllByRole("article");
    expect(cards.map((entry) => entry.getAttribute("aria-label"))).toEqual([
      "Write docs",
      "Fix login",
    ]);
    await waitFor(() =>
      expect(moves(db.changes)).toEqual([
        {
          placement: { before: DB.login },
          group: { view_id: DB.board, from: DB.inProgress, to: DB.notStarted },
        },
      ]),
    );
  });

  it("offers no place within a card's own column when the view sorts", async () => {
    fakeWorkspaces();
    fakePages({ pages: databasePages(samplePages()) });
    const sorted = views();
    for (const entry of sorted) {
      if (entry.id === DB.board) {
        entry.sorts = [{ property: DB.points, direction: "ascending" }];
      }
    }
    fakeDatabase({ views: sorted });
    fakeFeed();
    renderApp(`${DATABASE_PATH}?view=${DB.board}`);
    const view = await database();
    const card = await view.findByRole("article", { name: "Write docs" });
    const own = view.getByRole("region", { name: "In progress" });
    const other = view.getByRole("region", { name: "Done" });
    const over = vi.spyOn(document, "elementsFromPoint").mockReturnValue([own]);

    fireEvent.pointerDown(card, {
      button: 0,
      pointerId: 5,
      pointerType: "mouse",
      clientX: 10,
      clientY: 10,
    });
    fireEvent.pointerMove(window, { pointerId: 5, pointerType: "mouse", clientX: 20, clientY: 40 });
    expect(await screen.findByText("Moving Write docs")).toBeInTheDocument();
    expect(own).not.toHaveClass("outline-primary");

    over.mockReturnValue([other]);
    fireEvent.pointerMove(window, { pointerId: 5, pointerType: "mouse", clientX: 30, clientY: 40 });
    await waitFor(() => expect(other).toHaveClass("outline-primary"));
    fireEvent.keyDown(window, { key: "Escape" });
  });

  it("leaves a card where it was when a drag is called off with Escape", async () => {
    const { db } = openView(DB.board);
    const view = await database();
    const card = await view.findByRole("article", { name: "Write docs" });
    vi.spyOn(document, "elementsFromPoint").mockReturnValue([
      view.getByRole("region", { name: "Done" }),
    ]);

    fireEvent.pointerDown(card, {
      button: 0,
      pointerId: 3,
      pointerType: "mouse",
      clientX: 10,
      clientY: 10,
    });
    fireEvent.pointerMove(window, { pointerId: 3, pointerType: "mouse", clientX: 60, clientY: 60 });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(window, { pointerId: 3, pointerType: "mouse", clientX: 60, clientY: 60 });

    expect(
      within(view.getByRole("region", { name: "In progress" })).getByRole("article"),
    ).toHaveAccessibleName("Write docs");
    expect(moves(db.changes)).toEqual([]);
  });

  it("adds a card to a column, in its group, its title typed in place", async () => {
    const { user, db } = openView(DB.board);
    const done = await column("Done");

    await user.click(done.getByRole("button", { name: "Add a card to Done" }));
    const title = await done.findByRole("textbox", { name: "Title of Untitled" });
    await user.type(title, "Celebrate{Enter}");

    expect(await done.findByRole("article", { name: "Celebrate" })).toBeInTheDocument();
    expect(db.changes[0]).toEqual({
      method: "POST",
      path: `/api/v1/databases/${DB.tasks}/rows`,
      body: { group: { view_id: DB.board, key: DB.finished } },
    });
  });

  it("hides a column, keeping its count among the hidden groups, and shows it again", async () => {
    const { user, db } = openView(DB.board);
    const view = await database();
    await view.findByRole("article", { name: "Write docs" });

    await user.click(view.getByRole("button", { name: "More for the group Not started" }));
    await user.click(await screen.findByRole("menuitem", { name: "Hide group" }));
    const hidden = within(await view.findByRole("region", { name: "Hidden groups" }));
    expect(hidden.getByText("Not started")).toBeInTheDocument();
    await waitFor(() => expect(view.queryByRole("region", { name: "Not started" })).toBeNull());

    await user.click(hidden.getByRole("button", { name: "Show Not started" }));
    expect(await view.findByRole("region", { name: "Not started" })).toBeInTheDocument();
    const groups = db.changes
      .filter((change) => change.method === "PATCH")
      .map((change) => (change.body as { group: { hidden: unknown } }).group.hidden);
    expect(groups).toEqual([[DB.notStarted], []]);
  });

  it("places a column from its menu, which the view keeps", async () => {
    const { user, db } = openView(DB.board);
    const view = await database();
    await view.findByRole("article", { name: "Write docs" });

    await user.click(view.getByRole("button", { name: "More for the group Done" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move left" }));

    await waitFor(() =>
      expect(view.getAllByRole("region").map((entry) => entry.getAttribute("aria-label"))).toEqual([
        "No Status",
        "Not started",
        "Done",
        "In progress",
      ]),
    );
    expect(db.changes.at(-1)?.body).toEqual({
      group: {
        property: DB.status,
        order: [null, DB.notStarted, DB.finished, DB.inProgress],
        hidden: [],
      },
    });
  });

  it("shows a card's move that someone else made, as the server tells of it", async () => {
    const { db, feed } = openView(DB.board);
    await (await column("In progress")).findByRole("article", { name: "Write docs" });
    await waitFor(() => expect(feed.urls).toHaveLength(1));

    const docs = db.state.rows.find((entry) => entry.id === DB.docs);
    if (docs) {
      docs.values[DB.status] = DB.finished;
    }
    feed.send({
      type: "row_moved",
      database_id: DB.tasks,
      row_id: DB.docs,
      values: { [DB.status]: DB.finished },
      position: "a0",
      last_edited_at: "2026-11-22T12:00:00Z",
      last_edited_by: people.bob.id,
    });

    expect(
      await (await column("Done")).findByRole("article", { name: "Write docs" }),
    ).toBeVisible();
    await waitFor(async () =>
      expect((await column("In progress")).queryByRole("article")).toBeNull(),
    );
  });
});

describe("a long board", () => {
  it("draws only the cards near the screen in each column, and counts them all", async () => {
    // jsdom lays nothing out: each card measures as a card does on a screen.
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(72);
    const many = Array.from({ length: 500 }, (_, index) =>
      row(
        {
          id: `0193a5b2-0000-7000-8000-${String(index + 1).padStart(12, "0")}`,
          number: index + 1,
          title: `Task ${index + 1}`,
          values: { [DB.status]: DB.inProgress },
        },
        `a${index}`,
      ),
    );
    fakeWorkspaces();
    fakePages({ pages: databasePages(samplePages()) });
    fakeDatabase({ rows: many });
    fakeFeed();
    renderApp(`${DATABASE_PATH}?view=${DB.board}`);
    const progress = await column("In progress");
    await progress.findByRole("article", { name: "Task 1" }, FIRST_SHOWN);

    expect(progress.getByText("500")).toBeInTheDocument();
    const drawn = progress.getAllByRole("article");
    expect(drawn.length).toBeGreaterThan(5);
    expect(drawn.length).toBeLessThan(40);
    expect(progress.queryByRole("article", { name: "Task 500" })).toBeNull();
  });
});

describe("a database's list", () => {
  // The page loads the editor on demand; loaded once here, so no test waits on it. Left to
  // the first page a test opens, the import would run on into the tests after it.
  beforeAll(async () => {
    await import("@/components/editor/page-editor");
  });

  it("lists each row with the values the view picks, and opens one as its page", async () => {
    const { user, router } = openView(DB.list);
    const view = await database();
    const rows = await view.findAllByRole("listitem");

    expect(rows.map((line) => within(line).getAllByRole("button")[0]?.textContent)).toEqual([
      "Write docs",
      "Fix login",
      "Launch",
    ]);
    expect(within(rows[0] as HTMLElement).getByText("In progress")).toBeInTheDocument();
    expect(within(rows[1] as HTMLElement).queryByText("Bug")).toBeNull();

    await user.click(view.getByRole("button", { name: "Fix login" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/w/design/p/${DB.login}`));
    expect(await screen.findByRole("region", { name: "Properties" })).toBeInTheDocument();
  });
});
