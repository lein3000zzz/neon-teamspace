/**
 * What the specs that use the app on a phone share: their client addresses, a person of
 * their own with a workspace and a page, and the page's editor and its toolbar.
 */
import type { Browser, BrowserContext, Locator, Page, TestInfo } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, runOf } from "./accounts";
import { expect, signIn, test } from "./fixtures";

/** The phones the specs run in, by the names playwright.config.ts gives their projects. */
export const PHONES = ["iphone", "android"];

/**
 * The address a phone spec's browsers say they come from: the spec's own, apart for each phone
 * it runs in side by side, and for each repetition of it.
 */
export function phoneAddress(spec: number, info: TestInfo): Record<string, string> {
  const phone = Math.max(0, PHONES.indexOf(info.project.name));
  return clientAddress(spec, info.repeatEachIndex * PHONES.length + phone);
}

export type Workspace = { id: string; slug: string; name: string };
type Teamspace = { id: string; access: string };

/** The General teamspace of a workspace, where its members' pages go. */
export async function generalOf(page: Page, workspace: Workspace): Promise<string> {
  const response = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
  const { teamspaces } = (await response.json()) as { teamspaces: Teamspace[] };
  const found = teamspaces.find((teamspace) => teamspace.access === "default");
  expect(found, "the workspace's General").toBeDefined();
  return found?.id ?? "";
}

/**
 * A browser of a computer's screen and pointer, whose sidebar shows beside the page, for
 * what a test does as someone at a desk: a new context takes the phone's settings otherwise.
 */
export function desk(browser: Browser): Promise<BrowserContext> {
  return browser.newContext({
    viewport: { width: 1280, height: 720 },
    isMobile: false,
    hasTouch: false,
  });
}

/**
 * A person of this run, whom the admin makes at a desk, signed in on `page` through the form,
 * with a workspace of theirs. The phone is in their name, as each test runs in both at once.
 */
export async function personOnPhone(browser: Browser, page: Page, role: string) {
  const person = account(`${role}-${test.info().project.name}`, runOf());
  const adminContext = await desk(browser);
  try {
    await makeAccounts(await adminContext.newPage(), [person]);
  } finally {
    await adminContext.close();
  }
  await page.goto("/login");
  await signIn(page, person.email, PASSWORD);
  await expect(page.getByRole("button", { name: "Open navigation" })).toBeVisible();
  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: `${person.name} desk` },
    201,
  );
  return { person, workspace };
}

/** A page in a workspace's General, holding `markdown`, and where it opens. */
export async function pageIn(page: Page, workspace: Workspace, title: string, markdown: string) {
  const made = await callApi<{ id: string }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/pages`,
    { teamspace_id: await generalOf(page, workspace), title, markdown },
    201,
  );
  return { id: made.id, path: `/w/${workspace.slug}/p/${made.id}` };
}

/** The editable content of the page in view. */
export function content(page: Page): Locator {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox").first();
}

/** Waits until the page in view is in sync with the server, and so takes typing. */
export async function live(page: Page) {
  await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", "true");
}

/** The editing toolbar above the keyboard, while the content has focus. */
export function toolbar(page: Page): Locator {
  return page.getByRole("toolbar", { name: "Editing tools" });
}

/** The page as markdown, as the server keeps it. */
export async function markdownOf(page: Page, pageId: string): Promise<string> {
  return (await page.request.get(`/api/v1/pages/${pageId}/export`)).text();
}

/**
 * Selects `text` in the page's content, as a long press and the selection's handles do on
 * a phone: the browser tells the editor, which takes it as its selection.
 */
export async function selectText(page: Page, text: string) {
  await page.evaluate((wanted) => {
    const root = document.querySelector('[aria-label="Page content"] [contenteditable]');
    const walker = document.createTreeWalker(root ?? document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const at = node.textContent?.indexOf(wanted) ?? -1;
      if (at >= 0) {
        document.getSelection()?.setBaseAndExtent(node, at, node, at + wanted.length);
        return;
      }
    }
    throw new Error(`no "${wanted}" in the page`);
  }, text);
}

/**
 * Taps the page's content just after `text`, at the end of its line, which puts the caret
 * after it, once the text is where it stays: a block changing type eases into its new size.
 */
export async function tapAfter(page: Page, text: string) {
  const at = await page.evaluate(async (wanted) => {
    const root = document.querySelector('[aria-label="Page content"] [contenteditable]');
    await Promise.all(
      (root?.getAnimations({ subtree: true }) ?? []).map((animation) => animation.finished),
    );
    const walker = document.createTreeWalker(root ?? document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const start = node.textContent?.indexOf(wanted) ?? -1;
      if (start >= 0) {
        const range = document.createRange();
        range.setStart(node, start + wanted.length - 1);
        range.setEnd(node, start + wanted.length);
        const box = range.getBoundingClientRect();
        // Past the last letter, on its line: no nearer letter to put the caret before.
        return { x: box.right + 6, y: box.top + box.height / 2 };
      }
    }
    throw new Error(`no "${wanted}" in the page`);
  }, text);
  await page.touchscreen.tap(at.x, at.y);
}

/** Opens the sidebar, a drawer on a phone, from the button at the top of the screen. */
export async function openDrawer(page: Page): Promise<Locator> {
  await page.getByRole("button", { name: "Open navigation" }).tap();
  const drawer = page.getByRole("dialog", { name: "Main" });
  await expect(drawer).toBeVisible();
  return drawer;
}
