import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import {
  DATABASE_PATH,
  databasePages,
  DB,
  fakeDatabase,
  fakeFeed,
  FIRST_SHOWN,
  sampleRows,
} from "@/test/databases";
import { apiError } from "@/test/fixtures";
import { fakePages, samplePages } from "@/test/pages";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { fakeWorkspaces } from "@/test/workspaces";

function openDatabase(path = DATABASE_PATH) {
  fakeWorkspaces();
  fakePages({ pages: databasePages(samplePages()) });
  const db = fakeDatabase();
  fakeFeed();
  return { ...renderApp(path), db };
}

async function database() {
  const view = within(await screen.findByRole("region", { name: "Database Tasks" }, FIRST_SHOWN));
  await view.findByText("3 rows", undefined, FIRST_SHOWN);
  return view;
}

describe("a change the server refuses", () => {
  it("says a row's values would pass 64 KiB, and shows the value as it was", async () => {
    const { user } = openDatabase();
    const view = await database();
    server.use(
      http.patch("/api/v1/rows/:id", () =>
        apiError(
          409,
          "limit_reached",
          "a row's values take at most 64 KiB together, as JSON: this would make them larger",
        ),
      ),
    );

    await user.click(view.getByRole("button", { name: /^Points of Write docs/ }));
    const field = await screen.findByRole("textbox", { name: "Points of Write docs" });
    await user.clear(field);
    await user.type(field, "8{Enter}");

    const alert = within(await screen.findByRole("alert"));
    expect(alert.getByText("The change goes past a limit")).toBeInTheDocument();
    expect(
      alert.getByText(
        "A row's values take at most 64 KiB together, as JSON: this would make them larger.",
      ),
    ).toBeInTheDocument();
    // The rows are read again, so the value shows as the server keeps it.
    await waitFor(() =>
      expect(view.getByRole("button", { name: /^Points of Write docs/ })).toHaveTextContent("3"),
    );
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("says the database is full when a row would pass its 10,000", async () => {
    const { user } = openDatabase();
    const view = await database();
    server.use(
      http.post("/api/v1/databases/:id/rows", () =>
        apiError(409, "too_many_rows", "a database holds at most 10000 rows out of the trash"),
      ),
    );

    await user.click(view.getByRole("button", { name: "Add a row" }));

    const alert = within(await screen.findByRole("alert"));
    expect(alert.getByText("The database is full")).toBeInTheDocument();
    expect(
      alert.getByText("A database holds at most 10000 rows out of the trash."),
    ).toBeInTheDocument();
    expect(alert.getByText("Move some of its rows to the trash to add more.")).toBeInTheDocument();
    expect(view.getByText("3 rows")).toBeInTheDocument();
  });

  it("says a card's move was not allowed, and puts the card back", async () => {
    const { user } = openDatabase(`${DATABASE_PATH}?view=${DB.board}`);
    const view = await database();
    server.use(
      http.post("/api/v1/rows/:id/move", () =>
        apiError(403, "forbidden", "you may not change this row"),
      ),
    );

    const card = view.getByRole("article", { name: "Fix login" });
    await user.click(within(card).getByRole("button", { name: "More for Fix login" }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to Done" }));

    const alert = within(await screen.findByRole("alert"));
    expect(alert.getByText("The change was not saved")).toBeInTheDocument();
    expect(alert.getByText("You are not allowed to do this.")).toBeInTheDocument();
    await waitFor(() =>
      expect(
        within(view.getByRole("region", { name: "Not started" })).getByRole("article", {
          name: "Fix login",
        }),
      ).toBeInTheDocument(),
    );
  });
});

/** The rows as a guest reads them: without the people a person property names. */
function guestRows() {
  return sampleRows().map((entry) => {
    delete entry.values[DB.owner];
    return Object.assign(entry, { created_by: null, last_edited_by: null });
  });
}

/** The database's page, as a guest of the workspace who edits the database opens it. */
function openAsGuest() {
  fakeWorkspaces({ role: "guest" });
  fakePages({ pages: databasePages(samplePages()) });
  fakeDatabase({ rows: guestRows() });
  fakeFeed();
  return renderApp(DATABASE_PATH);
}

describe("a database a guest edits", () => {
  it("shows people without the way to set them, and keeps a person property's type", async () => {
    const { user } = openAsGuest();
    const view = await database();

    // Everything else edits as it does for members.
    expect(view.getByRole("button", { name: /^Points of Write docs/ })).toBeInTheDocument();
    expect(view.queryByRole("button", { name: /^Owner of/ })).toBeNull();

    await user.click(view.getByRole("button", { name: "Owner, property menu" }));
    expect(await screen.findByRole("menuitem", { name: "Rename…" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Change type…" })).toBeNull();
    await user.click(screen.getByRole("menuitem", { name: "Rename…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Owner" }));
    expect(dialog.getByRole("combobox", { name: "Type" })).toBeDisabled();
    expect(
      dialog.getByText(
        "Its type stays: as another type, its people would become names, which guests do not see.",
      ),
    ).toBeInTheDocument();
  });

  it("filters people by the guest alone", async () => {
    const { user } = openAsGuest();
    const view = await database();

    await user.click(view.getByRole("button", { name: "Filter" }));
    const filter = within(await screen.findByRole("dialog", { name: "Filter" }));
    await user.click(filter.getByRole("button", { name: "Add a filter" }));
    await user.selectOptions(filter.getByRole("combobox", { name: "Property" }), "Owner");

    const value = filter.getByRole("combobox", { name: "Value" });
    expect(
      within(value)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["Me"]);
  });
});
