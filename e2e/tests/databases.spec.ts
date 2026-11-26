import type { BrowserContext, Locator, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Wide enough for a board's four columns beside the sidebar, so none needs scrolling to.
test.use({ extraHTTPHeaders: clientAddress(14), viewport: { width: 1600, height: 900 } });

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/**
 * The editable content of a page's editor inside `scope`: the editor's own
 * textbox, which comes before the fields a database block draws inside it.
 */
function content(scope: Page | Locator) {
  return scope.getByRole("region", { name: "Page content" }).getByRole("textbox").first();
}

/** Waits until the page in `scope` is in sync with the server, and so takes typing. */
async function live(scope: Page | Locator) {
  await expect(scope.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(scope)).toHaveAttribute("contenteditable", "true");
}

/** Adds a row from the view's footer and types its title. */
async function addRow(scope: Locator, title: string) {
  await scope.getByRole("button", { name: "Add a row" }).click();
  const field = scope.getByRole("textbox", { name: /^Title of/ });
  await field.fill(title);
  await field.press("Enter");
  await expect(scope.getByRole("rowheader", { name: new RegExp(title) })).toBeVisible();
}

/** Adds a property of a type from the button after the table's last heading. */
async function addProperty(page: Page, scope: Locator, name: string, type: string) {
  await scope.getByRole("button", { name: "Add a property" }).click();
  const dialog = page.getByRole("dialog", { name: "New property" });
  await dialog.getByLabel("Name").fill(name);
  await dialog.getByLabel("Type").selectOption(type);
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toBeHidden();
  await expect(scope.getByRole("button", { name: `${name}, property menu` })).toBeVisible();
}

/** Drags with the mouse from the middle of one element to the middle of another. */
async function drag(page: Page, from: Locator, to: Locator) {
  const start = await from.boundingBox();
  const end = await to.boundingBox();
  if (start === null || end === null) {
    throw new Error("the card or the column is not on the page");
  }
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 12 });
  await page.mouse.up();
}

test("a database is made, filled and moved across its board, live for a second person", async ({
  page: adaPage,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const ada = account("planner", run);
  const bob = account("watcher", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);

    // Ada's workspace, with Bob in it, and so in General.
    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(adaPage.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Boards ${run}` },
      201,
    );
    await callApi(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invitations`,
      { email: bob.email, role: "member" },
      201,
    );
    const [bobContext, bobPage] = await signedIn(browser, bob);
    contexts.push(bobContext);
    await bobPage.setViewportSize({ width: 1600, height: 900 });
    await bobPage.getByRole("button", { name: `Accept: Boards ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    // A new database from General's page: a table and a board, titled as it opens.
    const response = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await response.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    expect(general, "the workspace's General").toBeDefined();
    await adaPage.goto(`/w/${workspace.slug}/teamspaces/${general?.id ?? ""}`);
    await adaPage.getByRole("button", { name: "New database" }).click();
    const title = adaPage.getByRole("textbox", { name: "Page title" });
    await expect(title).toBeFocused();
    await title.fill("Tasks");
    await title.press("Enter");
    const database = adaPage.getByRole("region", { name: "Database Tasks" });
    await expect(database.getByRole("tab", { name: "Table", selected: true })).toBeVisible();
    await expect(
      adaPage.getByRole("complementary").getByRole("link", { name: "Tasks" }),
    ).toBeVisible();

    // A number property and a checkbox, then two rows.
    await addProperty(adaPage, database, "Points", "number");
    await addProperty(adaPage, database, "Shipped", "checkbox");
    await addRow(database, "Write docs");
    await addRow(database, "Fix login");
    await expect(database.getByText("2 rows")).toBeVisible();

    // Cells edit in place: a number, then a status from its options.
    await database.getByRole("button", { name: /^Points of Write docs/ }).click();
    const points = adaPage.getByRole("textbox", { name: "Points of Write docs" });
    await points.fill("3");
    await points.press("Enter");
    await expect(database.getByRole("button", { name: /^Points of Write docs.*3$/ })).toBeVisible();
    await database.getByRole("button", { name: /^Status of Write docs/ }).click();
    await adaPage
      .getByRole("list", { name: "Status of Write docs" })
      .getByRole("button", { name: "In progress" })
      .click();
    await expect(
      database.getByRole("button", { name: /^Status of Write docs.*In progress$/ }),
    ).toBeVisible();
    // The box shows the value the view holds, which the click changes a moment later.
    await database.getByRole("checkbox", { name: "Shipped of Fix login" }).click();
    await expect(database.getByRole("checkbox", { name: "Shipped of Fix login" })).toBeChecked();

    // Bob opens the table as the server keeps it, then both open the board.
    const address = adaPage.url();
    await bobPage.goto(address);
    const bobDatabase = bobPage.getByRole("region", { name: "Database Tasks" });
    await expect(bobDatabase.getByRole("checkbox", { name: "Shipped of Fix login" })).toBeChecked();
    await expect(
      bobDatabase.getByRole("button", { name: /^Points of Write docs.*3$/ }),
    ).toBeVisible();
    for (const scope of [database, bobDatabase]) {
      await scope.getByRole("tab", { name: "Board" }).click();
      await expect(scope.locator("output").filter({ hasText: "Live" })).toBeVisible();
      await expect(
        scope.getByRole("region", { name: "In progress" }).getByRole("article", {
          name: "Write docs",
        }),
      ).toBeVisible();
    }

    // Ada drags the card to Done; Bob sees it move without reloading.
    await drag(
      adaPage,
      database.getByRole("region", { name: "In progress" }).getByRole("article", {
        name: "Write docs",
      }),
      database.getByRole("region", { name: "Done" }),
    );
    await expect(
      database.getByRole("region", { name: "Done" }).getByRole("article", { name: "Write docs" }),
    ).toBeVisible();
    await expect(
      bobDatabase
        .getByRole("region", { name: "Done" })
        .getByRole("article", { name: "Write docs" }),
    ).toBeVisible();
    await expect(
      bobDatabase.getByRole("region", { name: "In progress" }).getByRole("article"),
    ).toHaveCount(0);

    // The card opens the row beside the board: its properties, then its body, which saves.
    await database
      .getByRole("region", { name: "Done" })
      .getByRole("button", { name: "Write docs", exact: true })
      .click();
    const peek = adaPage.getByRole("dialog", { name: "Write docs, beside the database" });
    await expect(peek.getByRole("region", { name: "Properties" })).toContainText("Done");
    await live(peek);
    await content(peek).click();
    await adaPage.keyboard.type("Outline the guide first");
    const rowId = new URL(adaPage.url()).searchParams.get("peek");
    expect(rowId, "the row in the address").not.toBeNull();
    await expect
      .poll(async () => (await adaPage.request.get(`/api/v1/pages/${rowId ?? ""}/export`)).text())
      .toContain("Outline the guide first");
    await peek.getByRole("button", { name: "Close" }).click();
    await expect(peek).toBeHidden();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("a database made inside a page, from the slash menu, shows and edits there", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  const run = String(Date.now());
  const writer = account("inliner", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [writer]);

    await page.goto("/login");
    await signIn(page, writer.email, PASSWORD);
    await expect(page.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      page,
      "POST",
      "/api/v1/workspaces",
      { name: `Inline ${run}` },
      201,
    );
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const host = await callApi<{ id: string }>(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Sprint notes" },
      201,
    );
    await page.goto(`/w/${workspace.slug}/p/${host.id}`);
    await live(page);

    // "/database" offers the block; picking it makes a database inside the page.
    await content(page).click();
    await page.keyboard.type("/database");
    await page.getByRole("option", { name: /^Database/ }).click();
    const body = page.getByRole("region", { name: "Page content" });
    const embedded = body.getByRole("region", { name: /^Database/ });
    await expect(embedded.getByRole("tab", { name: "Table", selected: true })).toBeVisible();
    const name = body.getByRole("textbox", { name: "Database title" });
    await name.fill("Sprint tasks");
    await name.press("Enter");
    await expect(body.getByRole("region", { name: "Database Sprint tasks" })).toBeVisible();
    await addRow(embedded, "Plan the demo");

    // It is a page under the host, in the sidebar, and its rows are its own.
    const sidebar = page.getByRole("complementary");
    await sidebar.getByRole("button", { name: "Expand Sprint notes" }).click();
    const link = sidebar.getByRole("link", { name: "Sprint tasks" });
    await expect(link).toBeVisible();
    const databaseId = ((await link.getAttribute("href")) ?? "").split("/").at(-1) ?? "";

    // Once the server keeps the block, the database's title and its row, they show again.
    await expect
      .poll(async () => (await page.request.get(`/api/v1/pages/${host.id}/export`)).text())
      .toContain(`:::database ${databaseId}`);
    await expect
      .poll(async () => {
        const answer = await callApi<{ rows: { title: string }[] }>(
          page,
          "POST",
          `/api/v1/databases/${databaseId}/query`,
          {},
          200,
        );
        return answer.rows.map((row) => row.title);
      })
      .toEqual(["Plan the demo"]);
    await expect
      .poll(async () => {
        const database = await page.request.get(`/api/v1/pages/${databaseId}`);
        return ((await database.json()) as { title: string }).title;
      })
      .toBe("Sprint tasks");
    await page.reload();
    await live(page);
    const again = page.getByRole("region", { name: "Database Sprint tasks" });
    await expect(again.getByRole("rowheader", { name: /Plan the demo/ })).toBeVisible();

    // And opens as a full page of its own.
    await again.getByRole("button", { name: "Open as full page" }).click();
    await expect(page.getByRole("textbox", { name: "Page title" })).toHaveValue("Sprint tasks");
    await expect(
      page.getByRole("region", { name: "Database Sprint tasks" }).getByRole("rowheader", {
        name: /Plan the demo/,
      }),
    ).toBeVisible();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("a database with four views fits a phone's screen, its tabs scrolling on their own", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const run = String(Date.now());
  const owner = account("pocket", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner]);

    const screen = { width: 390, height: 844 };
    const phone = await browser.newContext({
      viewport: screen,
      deviceScaleFactor: 2,
      isMobile: true,
      hasTouch: true,
    });
    contexts.push(phone);
    const page = await phone.newPage();
    await page.goto("/login");
    await signIn(page, owner.email, PASSWORD);
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeVisible();

    // A database with a cover, four views and a few rows.
    const workspace = await callApi<Workspace>(
      page,
      "POST",
      "/api/v1/workspaces",
      { name: `Pocket ${run}` },
      201,
    );
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const database = await callApi<{ page: { id: string } }>(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/databases`,
      { teamspace_id: general?.id, title: "Sprint" },
      201,
    );
    const at = `/api/v1/databases/${database.page.id}`;
    await callApi(page, "PATCH", `/api/v1/pages/${database.page.id}`, { cover: "sunrise" }, 200);
    for (const [name, layout] of [
      ["Open work", "table"],
      ["List", "list"],
    ] as const) {
      await callApi(page, "POST", `${at}/views`, { name, layout }, 201);
    }
    for (const title of ["Write the demo script", "Rate-limit the public endpoints"]) {
      await callApi(page, "POST", `${at}/rows`, { values: { Name: title } }, 201);
    }

    // Whichever view shows, nothing but the views themselves is wider than the screen.
    await page.goto(`/w/${workspace.slug}/p/${database.page.id}`);
    const views = page.getByRole("region", { name: "Database Sprint" });
    for (const name of ["Table", "Board", "Open work", "List"]) {
      const tab = views.getByRole("tab", { name });
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect(tab).toBeInViewport({ ratio: 1 });
      await expect(views.getByText("2 rows")).toBeVisible();
      // A phone's browser widens its viewport to what overflows it, so the width is the
      // screen's, as the context set it.
      const wide = await page.evaluate(() => document.documentElement.scrollWidth);
      expect(wide, name).toBe(screen.width);
    }
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
