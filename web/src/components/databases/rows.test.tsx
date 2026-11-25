import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  DATABASE_PATH,
  databasePages,
  DB,
  fakeDatabase,
  fakeFeed,
  FIRST_SHOWN,
  rowPage,
  sampleRows,
} from "@/test/databases";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { fakeWorkspaces } from "@/test/workspaces";
import { WIDE_QUERY } from "@/lib/media";

const ROW_PATH = `/w/design/p/${DB.docs}`;

function setUp(path: string) {
  fakeWorkspaces();
  const pages = fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase();
  fakeFeed();
  return { ...renderApp(path), db, pages };
}

/** A screen wide enough for a row to open beside its database. */
function wideScreen() {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query === WIDE_QUERY,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

describe("a row's page", () => {
  it("shows its id and properties above its content, editable as in the views", async () => {
    const { user, db } = setUp(ROW_PATH);
    const properties = within(
      await screen.findByRole("region", { name: "Properties" }, FIRST_SHOWN),
    );

    expect(await properties.findByText("NW-1")).toBeInTheDocument();
    expect(properties.getByRole("button", { name: /^Status of Write docs/ })).toHaveTextContent(
      "In progress",
    );
    expect(properties.queryByRole("button", { name: /^Name of/ })).toBeNull();
    await user.click(properties.getByRole("button", { name: /^Points of Write docs/ }));
    const points = await screen.findByRole("textbox", { name: "Points of Write docs" });
    await user.clear(points);
    await user.type(points, "5{Enter}");

    await waitFor(() =>
      expect(db.changes.at(-1)).toEqual({
        method: "PATCH",
        path: `/api/v1/rows/${DB.docs}`,
        body: { values: { [DB.points]: 5 } },
      }),
    );
    // The way down to it goes through its database.
    const crumbs = within(screen.getByRole("navigation", { name: "Where this page is" }));
    expect(await crumbs.findByRole("link", { name: "Tasks" })).toHaveAttribute(
      "href",
      DATABASE_PATH,
    );
    // A row stays in its database: its menu does not move it elsewhere.
    await user.click(screen.getByRole("button", { name: "More for Write docs" }));
    expect(await screen.findByRole("menuitem", { name: "Export as markdown" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Move to…" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Import into this page…" })).toBeNull();
  });

  it("orders and edits the database's properties from their names", async () => {
    const { user, db } = setUp(ROW_PATH);
    const properties = within(
      await screen.findByRole("region", { name: "Properties" }, FIRST_SHOWN),
    );

    await user.click(await properties.findByRole("button", { name: "Points, property menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move Points up" }));

    await waitFor(() =>
      expect(db.changes.at(-1)).toEqual({
        method: "POST",
        path: `/api/v1/databases/${DB.tasks}/properties/${DB.points}/move`,
        body: { placement: { before: DB.status } },
      }),
    );
  });

  it("shows what every row has to someone the database is not shared with", async () => {
    fakeWorkspaces();
    const [docs, ...others] = sampleRows();
    if (docs === undefined) {
      throw new Error("the sample has Write docs");
    }
    // Shared on its own, its database above it stays hidden: it comes without a parent,
    // and the row without its database.
    const shared = rowPage(docs);
    delete shared.parent_id;
    fakePages({ pages: [...samplePages(), shared] });
    fakeDatabase({ rows: [{ ...docs, database_id: null }, ...others] });
    renderApp(ROW_PATH);

    const properties = within(
      await screen.findByRole("region", { name: "Properties" }, FIRST_SHOWN),
    );
    expect(await properties.findByText("NW-1")).toBeInTheDocument();
    expect(properties.getByText("Created")).toBeInTheDocument();
    expect(properties.getByText("Last edited")).toBeInTheDocument();
    expect(
      properties.getByText(
        "Its database is not shared with you, so the database's own properties do not show here.",
      ),
    ).toBeInTheDocument();
    expect(properties.queryByRole("button", { name: /^Status of/ })).toBeNull();
    // Nothing leads to the database: the way down to the row starts at the row.
    const crumbs = within(screen.getByRole("navigation", { name: "Where this page is" }));
    expect(crumbs.queryByRole("link", { name: "Tasks" })).toBeNull();
  });
});

describe("opening a row", () => {
  it("opens it as a full page on a narrow screen", async () => {
    const { user, router } = setUp(DATABASE_PATH);
    const table = within(await screen.findByRole("table", { name: "Table" }));

    await user.click(await table.findByRole("button", { name: "Open Write docs" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(ROW_PATH));
    expect(
      await screen.findByRole("region", { name: "Properties" }, FIRST_SHOWN),
    ).toBeInTheDocument();
  });

  it("opens it beside the database on a wide screen, with the way to its own page", async () => {
    wideScreen();
    const { user, router } = setUp(DATABASE_PATH);
    const table = within(await screen.findByRole("table", { name: "Table" }));

    await user.click(await table.findByRole("button", { name: "Open Fix login" }));
    const peek = within(
      await screen.findByRole("dialog", { name: "Fix login, beside the database" }),
    );
    expect(router.state.location.search).toEqual({ peek: DB.login });
    expect(
      await peek.findByRole("region", { name: "Properties" }, FIRST_SHOWN),
    ).toBeInTheDocument();
    expect(peek.getByRole("link", { name: "Open as full page" })).toHaveAttribute(
      "href",
      `/w/design/p/${DB.login}`,
    );

    await user.click(peek.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(router.state.location.search).toEqual({}));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("finds a row by its unique id, in any case", async () => {
    const { user, router } = setUp(DATABASE_PATH);
    const view = within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
    await view.findByText("3 rows");

    await user.click(view.getByRole("button", { name: "Find a row by its ID" }));
    const find = within(await screen.findByRole("dialog", { name: "Find a row by its ID" }));
    await user.type(find.getByLabelText("Unique ID"), "nw-9{Enter}");
    expect(await find.findByText("No row of this database is nw-9.")).toBeInTheDocument();
    await user.clear(find.getByLabelText("Unique ID"));
    await user.type(find.getByLabelText("Unique ID"), "nw-3{Enter}");

    await waitFor(() => expect(router.state.location.pathname).toBe(`/w/design/p/${DB.launch}`));
  });
});
