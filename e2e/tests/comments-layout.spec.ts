import type { Locator } from "@playwright/test";
import { callApi, clientAddress } from "./accounts";
import { ADMIN, expect, test } from "./fixtures";
import { sprintOf, type Teamspace } from "./sprint";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// How the comments panel and the editor's menus lay out beside the page. Each test signs the
// admin and a person of its own in: an address of the spec's own.
test.use({ extraHTTPHeaders: clientAddress(23) });

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

test("on a phone, the comments cover the whole screen, and neither they nor search show a keyboard's shortcut", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const screen = { width: 390, height: 844 };
  const { page, workspace } = await sprintOf(browser, "thumb", {
    context: { viewport: screen, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  });
  try {
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const plan = await callApi<{ id: string }>(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Launch plan", markdown: "We ship on Friday." },
      201,
    );
    await page.goto(`/w/${workspace.slug}/p/${plan.id}`);
    // The drawer's search names no keys.
    await page.getByRole("button", { name: "Open navigation" }).tap();
    const drawer = page.getByRole("dialog");
    await expect(drawer.getByRole("button", { name: "Search" })).toBeVisible();
    await expect(drawer.getByText(/⌘K|Ctrl K/)).toBeHidden();
    await drawer.getByRole("button", { name: "Close navigation" }).tap();
    await expect(drawer).toBeHidden();

    await page.getByRole("button", { name: "Comments" }).tap();
    const panel = page.getByRole("complementary", { name: "Comments" });
    await expect(panel.getByRole("button", { name: "Comment on the selected text" })).toBeVisible();
    const box = await panel.boundingBox();
    expect(box?.x, "no strip of the page at its left").toBe(0);
    expect(box?.width).toBe(screen.width);
    await expect(panel.getByText(/Ctrl\+Alt\+M/)).toBeHidden();
  } finally {
    await page.context().close();
  }
});

test("an agent's comment keeps its time on the line of its name, as a person's does", async ({
  browser,
  playwright,
}) => {
  test.setTimeout(90_000);
  const { page, workspace } = await sprintOf(browser, "scouted", {
    context: { viewport: { width: 1440, height: 900 } },
  });
  try {
    const base = `/api/v1/workspaces/${workspace.id}`;
    // Named as in the screenshots: Scout, an agent for Ada.
    await callApi(page, "PATCH", "/api/v1/account/me", { display_name: "Ada" }, 200);
    const scout = await callApi<{ id: string }>(
      page,
      "POST",
      `${base}/agents`,
      { name: "Scout" },
      201,
    );
    const { token } = await callApi<{ token: string }>(
      page,
      "POST",
      `${base}/agents/${scout.id}/tokens`,
      { name: "replies", scopes: ["read", "write"] },
      201,
    );
    const answer = await page.request.get(`${base}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const plan = await callApi<{ id: string }>(
      page,
      "POST",
      `${base}/pages`,
      { teamspace_id: general?.id, title: "Launch plan", markdown: "We ship on Friday." },
      201,
    );
    const { thread } = await callApi<{ thread: { id: string } }>(
      page,
      "POST",
      `/api/v1/pages/${plan.id}/comments`,
      { markdown: "Can we freeze at 10 instead?" },
      201,
    );
    const agent = await playwright.request.newContext({
      baseURL: new URL(page.url()).origin,
      extraHTTPHeaders: { Authorization: `Bearer ${token}` },
    });
    try {
      const replied = await agent.post(`/api/v1/threads/${thread.id}/comments`, {
        data: { markdown: "A 10:00 freeze still leaves five hours of rehearsal." },
      });
      expect(replied.status(), await replied.text()).toBe(201);
    } finally {
      await agent.dispose();
    }

    await page.goto(`/w/${workspace.slug}/p/${plan.id}`);
    await page.getByRole("button", { name: "Comments, 1 open" }).click();
    const panel = page.getByRole("complementary", { name: "Comments" });
    const headers = panel.locator("article p:has(time)");
    await expect(headers).toHaveCount(2);
    await expect(headers.nth(1)).toContainText("· via Ada");
    const [person, agentsOwn] = await headers.evaluateAll((all) =>
      all.map((each) => {
        const time = each.querySelector("time")?.getBoundingClientRect();
        const name = each.firstElementChild?.getBoundingClientRect();
        return time !== undefined && name !== undefined && time.top < name.bottom;
      }),
    );
    expect(person, "a person's time beside their name").toBe(true);
    expect(agentsOwn, "an agent's time beside its name").toBe(true);
  } finally {
    await page.context().close();
  }
});
