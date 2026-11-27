import { ADMIN, expect, test } from "./fixtures";
import {
  tapAfter,
  content,
  live,
  markdownOf,
  pageIn,
  personOnPhone,
  phoneAddress,
  selectText,
  toolbar,
} from "./phone";
import { png } from "./png";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(31, testInfo)),
});

test("a page is written, formatted and rearranged from the toolbar above the keyboard", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  const { workspace } = await personOnPhone(browser, page, "writer");
  const made = await pageIn(
    page,
    workspace,
    "Field notes",
    "Sunrise at the ridge.\n\nPack the tent.\n\nCheck the weather.",
  );
  await page.goto(made.path);
  await live(page);

  // A tap in the text brings the toolbar, across the bottom of the screen; no block handles
  // show for a finger.
  const bar = toolbar(page);
  await expect(bar).toBeHidden();
  await content(page).getByText("Pack the tent.").tap();
  await expect(bar).toBeVisible();
  const screen = page.viewportSize();
  const box = await bar.boundingBox();
  expect(box?.width).toBe(screen?.width);
  expect(Math.round((box?.y ?? 0) + (box?.height ?? 0))).toBe(screen?.height);
  await expect(page.locator(".bn-side-menu")).toHaveCount(0);
  for (const button of await bar.getByRole("button").all()) {
    const size = await button.boundingBox();
    expect(Math.min(size?.width ?? 0, size?.height ?? 0)).toBeGreaterThanOrEqual(44);
  }

  // A divider goes in after the block, and comes out again with Undo.
  await bar.getByRole("button", { name: "Insert" }).tap();
  await bar.getByRole("button", { name: "Divider" }).tap();
  await expect(content(page).locator("hr")).toHaveCount(1);
  await bar.getByRole("button", { name: "Undo" }).tap();
  await expect(content(page).locator("hr")).toHaveCount(0);
  await expect(bar.getByRole("button", { name: "Redo" })).toBeEnabled();

  // The block becomes a heading, and what is typed next is bold.
  await content(page).getByText("Pack the tent.").tap();
  await bar.getByRole("button", { name: "Block type: Text" }).tap();
  await bar.getByRole("button", { name: "Heading 2" }).tap();
  await expect(content(page).getByRole("heading", { level: 2 })).toHaveText("Pack the tent.");
  await tapAfter(page, "Pack the tent.");
  await page.keyboard.type(" ");
  await bar.getByRole("button", { name: "Bold" }).tap();
  await expect(bar.getByRole("button", { name: "Bold" })).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.type("Tonight");
  await expect(content(page).locator("strong")).toHaveText("Tonight");

  // Its menu moves it up, above the first.
  await bar.getByRole("button", { name: "Block menu" }).tap();
  await bar.getByRole("button", { name: "Move up" }).tap();
  await expect(content(page).locator(".bn-block-content").first()).toHaveText(
    "Pack the tent. Tonight",
  );
  await expect(bar.getByRole("button", { name: "Move up" })).toBeDisabled();
  await bar.getByRole("button", { name: "Back" }).tap();

  // A word linked from the toolbar's own field, whose address needs no scheme.
  await selectText(page, "ridge");
  await expect(bar.getByRole("button", { name: "Link" })).toBeEnabled();
  await bar.getByRole("button", { name: "Link" }).tap();
  const address = bar.getByRole("textbox", { name: "Link address" });
  await expect(address).toBeFocused();
  await address.fill("example.com/ridge");
  await bar.getByRole("button", { name: "Apply link" }).tap();
  await expect(content(page).getByRole("link", { name: "ridge" })).toHaveAttribute(
    "href",
    "https://example.com/ridge",
  );
  await expect(content(page)).toBeFocused();

  // The slash menu, from Insert, stays clear of the toolbar, and a tap picks from it.
  await tapAfter(page, "Check the weather.");
  // The editor has the caret there, out of the link, once the toolbar says so.
  await expect(bar.getByRole("button", { name: "Link" })).toBeDisabled();
  await page.keyboard.press("Enter");
  await bar.getByRole("button", { name: "Insert" }).tap();
  await bar.getByRole("button", { name: "Other blocks" }).tap();
  const menu = page.getByRole("listbox");
  await expect(menu).toBeVisible();
  const menuBox = await menu.boundingBox();
  const barBox = await bar.boundingBox();
  expect((menuBox?.y ?? 0) + (menuBox?.height ?? 0)).toBeLessThanOrEqual(barBox?.y ?? 0);
  await menu.getByRole("option", { name: /Bullet List/ }).tap();
  await page.keyboard.type("Water filter for ");
  await expect(content(page).locator('[data-content-type="bulletListItem"]')).toHaveText(
    "Water filter for",
  );

  // So does the mention menu, filtered as the phone's keyboard types.
  await bar.getByRole("button", { name: "Insert" }).tap();
  await bar.getByRole("button", { name: "Mention" }).tap();
  await page.keyboard.type("Field");
  const mention = page.getByRole("option", { name: /Field notes/ });
  await expect(mention).toBeVisible();
  const mentionBox = await mention.boundingBox();
  expect((mentionBox?.y ?? 0) + (mentionBox?.height ?? 0)).toBeLessThanOrEqual(barBox?.y ?? 0);
  await mention.tap();
  await expect(content(page).getByRole("link", { name: "@Field notes" })).toBeVisible();

  // Done: the keyboard goes, and the toolbar with it.
  await bar.getByRole("button", { name: "Done editing" }).tap();
  await expect(bar).toBeHidden();
  await expect(content(page)).not.toBeFocused();

  // Everything reached the server, in its order.
  await expect
    .poll(() => markdownOf(page, made.id))
    .toMatch(
      /^# Field notes\s+## Pack the tent\. \*\*Tonight\*\*\s+Sunrise at the \[ridge\]\(https:\/\/example\.com\/ridge\)\.\s+Check the weather\.\s+[-*] Water filter for @\[Field notes\]\(page:[^)]+\)(&#32;)?\s*$/,
    );
});

test("an image uploaded from the toolbar is selected with a tap, for its caption and its moves", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { workspace } = await personOnPhone(browser, page, "photographer");
  const made = await pageIn(page, workspace, "Trip photos", "The summit at dawn.\n\nThe way down.");
  await page.goto(made.path);
  await live(page);

  // An image picked on the phone goes in after the block the caret is in.
  const bar = toolbar(page);
  await content(page).getByText("The summit at dawn.").tap();
  await expect(bar.getByRole("button", { name: "Block type: Text" })).toBeEnabled();
  await bar.getByRole("button", { name: "Insert" }).tap();
  const chooser = page.waitForEvent("filechooser");
  await bar.getByRole("button", { name: "Image" }).tap();
  await (await chooser).setFiles({ name: "summit.png", mimeType: "image/png", buffer: png(96) });
  const image = content(page).locator('[data-content-type="image"]');
  await expect(image.locator("img")).toBeVisible();

  // A tap on it selects it, where the browser would put the caret in the text nearest: its
  // menu writes its caption, and moves it past the text after it.
  await image.locator("img").tap();
  await bar.getByRole("button", { name: "Block menu" }).tap();
  await bar.getByRole("button", { name: "Caption" }).tap();
  const caption = bar.getByRole("textbox", { name: "Caption" });
  await expect(caption).toBeFocused();
  await caption.fill("Sunrise from the top");
  await bar.getByRole("button", { name: "Set caption" }).tap();
  await expect(image).toContainText("Sunrise from the top");
  await expect(content(page)).toBeFocused();
  await bar.getByRole("button", { name: "Block menu" }).tap();
  await bar.getByRole("button", { name: "Move down" }).tap();
  await expect(bar.getByRole("button", { name: "Move down" })).toBeDisabled();

  await expect
    .poll(() => markdownOf(page, made.id))
    .toMatch(
      /^# Trip photos\s+The summit at dawn\.\s+The way down\.\s+!\[summit\.png\]\(\S+ "Sunrise from the top"\)\s*$/,
    );
});

test("the line being typed stays above the toolbar, and the last line can scroll up past it", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { workspace } = await personOnPhone(browser, page, "scroller");
  const lines = Array.from({ length: 40 }, (_, index) => `Line ${index + 1} of the log.`);
  const made = await pageIn(page, workspace, "Long log", lines.join("\n\n"));
  await page.goto(made.path);
  await live(page);

  // The line at the bottom of the screen, which the toolbar covers once it shows.
  const height = page.viewportSize()?.height ?? 0;
  const tapped = await content(page)
    .locator(".bn-inline-content")
    .evaluateAll((elements, screenHeight) => {
      const line = elements.find((element) => {
        const box = element.getBoundingClientRect();
        return box.top > screenHeight - 50 && box.bottom <= screenHeight;
      });
      const box = line?.getBoundingClientRect();
      return line === undefined || box === undefined
        ? null
        : { text: line.textContent ?? "", x: box.left + 4, y: box.top + box.height / 2 };
    }, height);
  expect(tapped, "a line at the bottom of the screen").not.toBeNull();
  await page.touchscreen.tap(tapped?.x ?? 0, tapped?.y ?? 0);
  const bar = toolbar(page);
  await expect(bar).toBeVisible();

  /** Whether the line of that text is wholly above the toolbar. */
  const clear = async (text: string) => {
    const top = (await bar.boundingBox())?.y ?? 0;
    const line = await content(page).getByText(text, { exact: true }).boundingBox();
    return line !== null && line.y + line.height <= top;
  };
  // It scrolls clear of it.
  await expect.poll(() => clear(tapped?.text ?? "")).toBe(true);

  // The last line too, typed on at the end of the page.
  const last = content(page).getByText("Line 40 of the log.");
  await last.scrollIntoViewIfNeeded();
  await tapAfter(page, "Line 40 of the log.");
  await page.keyboard.type(" The end.");
  await expect.poll(() => clear("Line 40 of the log. The end.")).toBe(true);
});

test("a time typed on a phone opens no emoji menu, and a name after a colon opens it above the toolbar", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { workspace } = await personOnPhone(browser, page, "timekeeper");
  const made = await pageIn(page, workspace, "Meeting", "Agenda.");
  await page.goto(made.path);
  await live(page);
  const bar = toolbar(page);
  await tapAfter(page, "Agenda.");
  await expect(bar).toBeVisible();

  // The colon of a time opens nothing: no menu marks it, and none covers the page.
  await page.keyboard.press("Enter");
  await page.keyboard.type("We meet at 15:00.");
  await expect(content(page)).toContainText("We meet at 15:00.");
  await expect(content(page).locator(".bn-suggestion-decorator")).toHaveCount(0);
  await expect(page.locator(".bn-frimousse-picker")).toHaveCount(0);

  // A name after a colon that starts a word opens it, clear of the toolbar, and a tap picks.
  await page.keyboard.type(" Bring a :rocket");
  const picker = page.locator(".bn-frimousse-picker");
  const rocket = picker.getByText("🚀", { exact: true });
  await expect(rocket).toBeVisible();
  const pickerBox = await picker.boundingBox();
  const barBox = await bar.boundingBox();
  expect((pickerBox?.y ?? 0) + (pickerBox?.height ?? 0)).toBeLessThanOrEqual(barBox?.y ?? 0);
  await rocket.tap();
  await expect(picker).toHaveCount(0);
  await expect(content(page)).toContainText("We meet at 15:00. Bring a 🚀");
});

test("the toolbar's rows show at which end more of them is off the screen", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  const { workspace } = await personOnPhone(browser, page, "swiper");
  const made = await pageIn(page, workspace, "Notes", "A line to edit.");
  await page.goto(made.path);
  await live(page);
  await content(page).getByText("A line to edit.").tap();
  const bar = toolbar(page);
  await expect(bar).toBeVisible();
  const start = bar.locator('[data-more="start"]');
  const end = bar.locator('[data-more="end"]');

  // The main row is wider than a phone: its end fades, with an arrow that way.
  await expect(end).toBeVisible();
  await expect(start).toHaveCount(0);

  // Scrolled to its last button, the fade is at the start instead.
  await bar.getByRole("button", { name: "Redo" }).scrollIntoViewIfNeeded();
  await expect(start).toBeVisible();
  await expect(end).toHaveCount(0);

  // So is the Insert row's end, from its first button.
  await bar.getByRole("button", { name: "Insert" }).tap();
  await expect(bar.getByRole("button", { name: "Mention" })).toBeVisible();
  await expect(end).toBeVisible();
  await expect(start).toHaveCount(0);
});
