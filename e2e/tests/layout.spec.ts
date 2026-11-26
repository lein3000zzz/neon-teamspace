import type { Browser, BrowserContextOptions, Locator, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Each test signs the admin and a person of its own in: an address of the spec's own.
test.use({ extraHTTPHeaders: clientAddress(20) });

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/** A database called Sprint in General: its page, and the ways to it. */
type Sprint = { workspace: Workspace; pageId: string; api: string; path: string };

/**
 * A person of this run, whom the admin makes, signed in on a page of a browser of their
 * own, with a workspace of theirs holding a database: on `page` when given, else on a
 * page of a context made with `options`.
 */
async function sprintOf(
  browser: Browser,
  role: string,
  options: { page?: Page; context?: BrowserContextOptions },
): Promise<Sprint & { page: Page }> {
  const run = String(Date.now());
  const owner = account(role, run);
  const adminContext = await browser.newContext();
  try {
    await makeAccounts(await adminContext.newPage(), [owner]);
  } finally {
    await adminContext.close();
  }
  const page = options.page ?? (await (await browser.newContext(options.context)).newPage());
  await page.goto("/login");
  await signIn(page, owner.email, PASSWORD);
  await expect(page).not.toHaveURL(/\/login/);

  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: `${owner.name} desk` },
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
  return {
    page,
    workspace,
    pageId: database.page.id,
    api: `/api/v1/databases/${database.page.id}`,
    path: `/w/${workspace.slug}/p/${database.page.id}`,
  };
}

test("a database with four views fits a phone's screen, its tabs scrolling on their own", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const screen = { width: 390, height: 844 };
  const { page, pageId, api, path } = await sprintOf(browser, "pocket", {
    context: { viewport: screen, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  });
  try {
    // A cover, four views and a few rows.
    await callApi(page, "PATCH", `/api/v1/pages/${pageId}`, { cover: "sunrise" }, 200);
    for (const [name, layout] of [
      ["Open work", "table"],
      ["List", "list"],
    ] as const) {
      await callApi(page, "POST", `${api}/views`, { name, layout }, 201);
    }
    for (const title of ["Write the demo script", "Rate-limit the public endpoints"]) {
      await callApi(page, "POST", `${api}/rows`, { values: { Name: title } }, 201);
    }

    // Whichever view shows, nothing but the views themselves is wider than the screen.
    await page.goto(path);
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

    // Chosen from the keyboard, each tab, with its menu, scrolls into sight clear of the row's
    // fades.
    await views.getByRole("tab", { name: "List" }).focus();
    for (const name of ["Table", "Board", "Open work", "List"]) {
      await page.keyboard.press("ArrowRight");
      const tab = views.getByRole("tab", { name });
      await expect(tab).toBeFocused();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect
        .poll(
          () =>
            tab.evaluate((element) => {
              const row = element.closest('[role="tablist"]');
              const box = element.parentElement?.getBoundingClientRect();
              if (row === null || box === undefined) {
                return "no row";
              }
              const frame = row.getBoundingClientRect();
              // The row fades out 24px on a side where more tabs are.
              const from = frame.left + (row.scrollLeft > 1 ? 24 : 0);
              const to =
                frame.right - (row.scrollLeft + row.clientWidth < row.scrollWidth - 1 ? 24 : 0);
              return box.left >= from - 1 && box.right <= to + 1
                ? "clear"
                : `${Math.round(box.left)}..${Math.round(box.right)} in ${Math.round(from)}..${Math.round(to)}`;
            }),
          { message: name },
        )
        .toBe("clear");
    }
  } finally {
    await page.context().close();
  }
});

test("a row's properties start on one line, editable or not", async ({ page, browser }) => {
  test.setTimeout(60_000);
  const { api, workspace } = await sprintOf(browser, "aligner", { page });
  for (const property of [
    { name: "ID", type: "unique_id", prefix: "NW" },
    { name: "Due", type: "date" },
    { name: "Created by", type: "created_by" },
  ]) {
    await callApi(page, "POST", `${api}/properties`, property, 201);
  }
  const row = await callApi<{ id: string }>(
    page,
    "POST",
    `${api}/rows`,
    { values: { Name: "Benchmark search", Status: "In progress", Due: { start: "2026-11-28" } } },
    201,
  );

  // The id and who made the row only show; the status and the day are buttons. Each value
  // starts where the others do.
  await page.goto(`/w/${workspace.slug}/p/${row.id}`);
  const properties = page.getByRole("region", { name: "Properties" });
  await expect(properties.getByText("NW-1")).toBeVisible();
  const starts = await properties.evaluate((region) =>
    [...region.querySelectorAll("dd")].map((value) => {
      // The left edge of what shows of the value, inside the box that holds it.
      const shown = [...value.querySelectorAll(":scope > * > *")]
        .filter((element) => !element.classList.contains("sr-only"))
        .map((element) => element.getBoundingClientRect())
        .filter((box) => box.width > 1);
      return Math.round(Math.min(...shown.map((box) => box.left)));
    }),
  );
  expect(starts).toHaveLength(4);
  expect(new Set(starts).size, `the values start at ${starts.join(", ")}`).toBe(1);
});

test("a board's cards lay out from the top, whatever their titles' lengths", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { api, path } = await sprintOf(browser, "carder", { page });
  for (const title of [
    "Write the demo script",
    "Set up the Postgres migrations for teams, their members and their invitations",
  ]) {
    await callApi(
      page,
      "POST",
      `${api}/rows`,
      { values: { Name: title, Status: "In progress" } },
      201,
    );
  }

  await page.goto(path);
  await page.getByRole("tab", { name: "Board" }).click();
  const column = page.getByRole("region", { name: "In progress" });
  await expect(column.getByRole("article")).toHaveCount(2);
  // Where each title's first line starts in its card, and how many lines it takes.
  const laid = await column.evaluate((element) =>
    [...element.querySelectorAll("article")].map((card) => {
      const words = card.querySelector("button > span:last-child");
      if (words === null) {
        return { lines: 0, from: 0 };
      }
      const box = words.getBoundingClientRect();
      const line = Number.parseFloat(getComputedStyle(words).lineHeight);
      return {
        lines: Math.round(box.height / line),
        from: Math.round(box.top - card.getBoundingClientRect().top),
      };
    }),
  );
  const [short, long] = laid;
  expect(short?.lines).toBe(1);
  expect(long?.lines).toBeGreaterThan(1);
  expect(short?.from, "the one-line title starts where the longer one does").toBe(long?.from);
});

test("an agent cut short in a table's column keeps its name, and tells the rest on hover", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { api, path, workspace } = await sprintOf(browser, "namer", { page });
  const scout = await callApi<{ id: string }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/agents`,
    { name: "Scout" },
    201,
  );
  await callApi(page, "POST", `${api}/properties`, { name: "Assignee", type: "person" }, 201);
  await callApi(
    page,
    "POST",
    `${api}/rows`,
    { values: { Name: "Benchmark search", Assignee: [scout.id] } },
    201,
  );

  await page.goto(path);
  const cell = page.getByRole("button", { name: /^Assignee of Benchmark search/ });
  await expect(cell).toHaveAttribute("title", /^Scout · via Namer /);
  // The column is too narrow for all of it: whom the agent acts for gives way, not its name.
  const cut = (element: HTMLElement | SVGElement) => element.scrollWidth > element.clientWidth;
  expect(await cell.getByText(/^· via Namer /).evaluate(cut)).toBe(true);
  expect(await cell.getByText("Scout", { exact: true }).evaluate(cut)).toBe(false);
});

test("the sidebar keeps to its column with a long page title and its emoji, and a teamspace stays folded", async ({
  browser,
}) => {
  test.setTimeout(60_000);
  const { page, workspace } = await sprintOf(browser, "wirer", {
    context: { viewport: { width: 1440, height: 900 } },
  });
  try {
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    await callApi(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Sensor wiring notes", icon: "🔌" },
      201,
    );

    await page.goto("/inbox");
    const sidebar = page.getByRole("complementary");
    const row = sidebar.getByRole("link", { name: "Sensor wiring notes" });
    await expect(row).toBeVisible();
    const widths = () =>
      sidebar.evaluate((element) => ({ scroll: element.scrollWidth, client: element.clientWidth }));
    const fits = async () => {
      const { scroll, client } = await widths();
      expect(scroll, "nothing in the sidebar is wider than it").toBe(client);
    };
    await fits();
    // Its emoji shows whole, inside the box its icon takes.
    const icon = await row.evaluate((link) => {
      const box = link.querySelector("span[aria-hidden]");
      if (box === null) {
        return "no icon";
      }
      const range = document.createRange();
      range.selectNodeContents(box);
      const drawn = range.getBoundingClientRect();
      const frame = box.getBoundingClientRect();
      return drawn.left >= frame.left - 0.5 && drawn.right <= frame.right + 0.5
        ? "whole"
        : `${drawn.left}..${drawn.right} in ${frame.left}..${frame.right}`;
    });
    expect(icon).toBe("whole");
    // The row's actions show on hover, inside the column.
    await row.hover();
    const more = sidebar.getByRole("button", { name: "More for Sensor wiring notes" });
    await expect(more).toBeVisible();
    const column = await sidebar.boundingBox();
    const box = await more.boundingBox();
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(
      (column?.x ?? 0) + (column?.width ?? 0),
    );
    await fits();

    // Folded, General stays folded after a reload.
    await sidebar.getByRole("button", { name: "Collapse General" }).click();
    await expect(row).toBeHidden();
    await page.reload();
    await expect(sidebar.getByRole("button", { name: "Expand General" })).toBeVisible();
    await expect(row).toBeHidden();
  } finally {
    await page.context().close();
  }
});

/**
 * Selects the last word that ends on the first line of the editor's text, or puts the
 * caret before it, as a click there would.
 */
async function lastWordOfFirstLine(content: Locator, select: boolean) {
  await content.getByText("Friday evening", { exact: false }).click();
  await content.evaluate((root, whole) => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const node = walker.nextNode();
    const text = node?.textContent ?? "";
    if (node === null) {
      return;
    }
    const range = document.createRange();
    range.selectNodeContents(node);
    const line = range.getClientRects()[0];
    for (let end = text.length; end > 0; end -= 1) {
      range.setStart(node, end - 1);
      range.setEnd(node, end);
      if (range.getBoundingClientRect().top === line?.top && text[end - 1] !== " ") {
        const start = text.lastIndexOf(" ", end - 1) + 1;
        document.getSelection()?.setBaseAndExtent(node, start, node, whole ? end : start);
        return;
      }
    }
  }, select);
}

test("the editor's toolbar keeps off the comments panel, and its mention menu inside the window", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const screen = { width: 1440, height: 900 };
  const { page, workspace } = await sprintOf(browser, "placer", { context: { viewport: screen } });
  try {
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const plan = await callApi<{ id: string }>(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      {
        teamspace_id: general?.id,
        title: "Launch plan",
        markdown:
          "Friday evening we wire the sensor and get readings into the database. Saturday is " +
          "for the rules worker and the dashboard, and Sunday for the rehearsal.",
      },
      201,
    );
    await page.goto(`/w/${workspace.slug}/p/${plan.id}`);
    const content = page.getByRole("region", { name: "Page content" }).getByRole("textbox");
    await expect(content).toHaveAttribute("contenteditable", "true");

    // A word at the end of the first line, selected with the comments panel open beside it.
    await page.getByRole("button", { name: "Comments" }).click();
    const panel = page.getByRole("complementary", { name: "Comments" });
    await expect(panel).toBeVisible();
    await lastWordOfFirstLine(content, true);
    const toolbar = page.locator(".bn-formatting-toolbar");
    await expect(toolbar).toBeVisible();
    const panelBox = await panel.boundingBox();
    const toolbarBox = await toolbar.boundingBox();
    expect(
      (toolbarBox?.x ?? 0) + (toolbarBox?.width ?? 0),
      "the toolbar ends before the panel starts",
    ).toBeLessThanOrEqual(panelBox?.x ?? 0);

    // With the panel closed, @ typed near the end of the line opens a menu inside the window.
    await panel.getByRole("button", { name: "Close comments" }).click();
    await lastWordOfFirstLine(content, false);
    await page.keyboard.type("@");
    const menu = page.locator(".bn-suggestion-menu");
    await expect(menu).toBeVisible();
    await expect
      .poll(async () => {
        const box = await menu.boundingBox();
        return box === null ? Number.POSITIVE_INFINITY : box.x + box.width;
      })
      .toBeLessThanOrEqual(screen.width - 8);
    await page.keyboard.press("Escape");
  } finally {
    await page.context().close();
  }
});
