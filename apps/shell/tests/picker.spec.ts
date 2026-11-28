import { expect, test, type Page } from "@playwright/test";
import { calls, fakeBackend, opened, type FakeConfig } from "./fake-backend";

const ACME = { origin: "https://neon.example.com", name: "Acme", version: "0.1.0" };
const LAB = { origin: "http://192.168.1.20:3000", name: "Lab", version: "0.2.0" };

const NETWORK: FakeConfig["network"] = {
  [ACME.origin]: { name: ACME.name, version: ACME.version },
  [LAB.origin]: { name: LAB.name, version: LAB.version },
  "https://gitea.example.com": {
    code: "not_neon",
    message:
      'https://gitea.example.com answered with JSON that is not a Neon Teamspace server\'s: it says it is "gitea".',
  },
};

/** The picker with the fake's state, as the app shows it when it starts. */
async function launch(page: Page, config: Partial<FakeConfig> = {}): Promise<string[]> {
  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      violations.push(message.text());
    }
  });
  await page.addInitScript(() => {
    document.addEventListener("securitypolicyviolation", (event) => {
      console.error(`CSP: ${event.violatedDirective} ${event.blockedURI}`);
    });
  });
  await fakeBackend(page, { servers: [], last: null, open: false, network: NETWORK, ...config });
  await page.goto("/");
  await expect(page.locator("main")).toHaveAttribute("aria-busy", "false");
  return violations;
}

function server(page: Page, name: string) {
  return page.getByRole("listitem").filter({ has: page.getByRole("button", { name }) });
}

test("the first launch asks for an address, under the app's policy, without violations", async ({
  page,
}) => {
  const violations = await launch(page);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Connect to a server");
  await expect(page.getByLabel("Server address")).toBeFocused();
  await expect(page.getByRole("button", { name: "Connect" })).toBeVisible();
  expect(violations).toEqual([]);
});

test("an address is checked, saved and opened", async ({ page }) => {
  await launch(page);
  await page.getByLabel("Server address").fill("  neon.example.com  ");
  await page.getByLabel("Server address").press("Enter");
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  expect(await opened(page)).toEqual([ACME.origin]);
  expect((await calls(page)).filter((call) => call.cmd === "connect")).toEqual([
    { cmd: "connect", args: { address: "  neon.example.com  ", acceptInsecure: false } },
  ]);
});

test("an address that is not a Neon Teamspace server is refused with its reason", async ({
  page,
}) => {
  await launch(page);
  const field = page.getByLabel("Server address");
  await field.fill("gitea.example.com");
  await page.getByRole("button", { name: "Connect" }).click();
  const error = page.getByRole("alert");
  await expect(error).toContainText('it says it is "gitea"');
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(field).toBeFocused();
  await expect(field).toHaveAccessibleDescription(/gitea/);
  expect(await opened(page)).toEqual([]);

  // Unreachable, then fixed: the error goes once the address works.
  await field.fill("down.example.com");
  await field.press("Enter");
  await expect(page.getByRole("alert")).toContainText("Could not reach https://down.example.com");
  await field.fill("neon.example.com");
  await field.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("plain http waits for the warning to be accepted", async ({ page }) => {
  await launch(page);
  await page.getByLabel("Server address").fill("http://192.168.1.20:3000");
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByText("This connection is not encrypted")).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("your password included");
  const accept = page.getByRole("button", { name: "Connect anyway" });
  await expect(accept).toBeFocused();
  expect(await opened(page)).toEqual([]);

  // Cancelling goes back to the field, with nothing opened.
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByLabel("Server address")).toBeFocused();
  await expect(accept).toHaveCount(0);

  await page.getByLabel("Server address").press("Enter");
  await page.getByRole("button", { name: "Connect anyway" }).press("Enter");
  await expect(page.getByRole("status")).toHaveText("Opening Lab…");
  expect(await opened(page)).toEqual([LAB.origin]);
  expect((await calls(page)).filter((call) => call.cmd === "connect").map((c) => c.args)).toEqual([
    { address: "http://192.168.1.20:3000", acceptInsecure: false },
    { address: "http://192.168.1.20:3000", acceptInsecure: false },
    { address: "http://192.168.1.20:3000", acceptInsecure: true },
  ]);
});

test("editing the address after the http warning takes the warning back", async ({ page }) => {
  await launch(page);
  const field = page.getByLabel("Server address");
  await field.fill("http://192.168.1.20:3000");
  await field.press("Enter");
  await expect(page.getByRole("button", { name: "Connect anyway" })).toBeFocused();

  // A key typed into the field: the warning goes, the field keeps the focus and the caret.
  // The caret at the end, which the End key does not do on macOS.
  await field.focus();
  await field.evaluate((input: HTMLInputElement) =>
    input.setSelectionRange(input.value.length, input.value.length),
  );
  await page.keyboard.type("01");
  await expect(page.getByRole("button", { name: "Connect anyway" })).toHaveCount(0);
  await expect(page.getByText("This connection is not encrypted")).toHaveCount(0);
  await expect(field).toBeFocused();
  await expect(field).toHaveValue("http://192.168.1.20:300001");

  // Connect checks the address as the field shows it, never the one the warning was for.
  await field.fill("neon.example.com");
  await field.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  expect(await opened(page)).toEqual([ACME.origin]);
  expect((await calls(page)).filter((call) => call.cmd === "connect").map((c) => c.args)).toEqual([
    { address: "http://192.168.1.20:3000", acceptInsecure: false },
    { address: "neon.example.com", acceptInsecure: false },
  ]);
});

test("servers whose addresses differ only in punctuation keep apart", async ({ page }) => {
  const alpha = { origin: "https://neon-example.com", name: "Alpha", version: "0.1.0" };
  const beta = { origin: "https://neon.example.com", name: "Beta", version: "0.2.0" };
  await launch(page, { servers: [alpha, beta], last: alpha.origin });
  const ids = await page.evaluate(() =>
    [...document.querySelectorAll("[id]")].map((element) => element.id),
  );
  expect(new Set(ids).size).toBe(ids.length);
  await expect(page.getByRole("button", { name: /^Beta/ })).toHaveAccessibleDescription(
    "Neon Teamspace 0.2.0",
  );
  await expect(page.getByRole("button", { name: /^Alpha/ })).toHaveAccessibleDescription(
    "Neon Teamspace 0.1.0 · last used",
  );

  await page.getByRole("button", { name: "Rename Beta" }).click();
  await page.getByLabel("Name for https://neon.example.com").fill("Work");
  await page.getByLabel("Name for https://neon.example.com").press("Enter");
  await expect(page.getByRole("button", { name: /^Work/ })).toBeFocused();
});

test("the saved servers are listed, and one opens on a click", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: LAB.origin });
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Choose a server");
  await expect(page.getByRole("list", { name: "Saved servers" }).getByRole("listitem")).toHaveCount(
    2,
  );
  await expect(server(page, "Acme")).toContainText("https://neon.example.com");
  await expect(server(page, "Acme")).toContainText("Neon Teamspace 0.1.0");
  await expect(server(page, "Acme")).not.toContainText("Not encrypted");
  await expect(server(page, "Lab")).toContainText("Not encrypted");
  await expect(server(page, "Lab")).toContainText("last used");
  await expect(page.getByRole("button", { name: /^Acme/ })).toHaveAccessibleDescription(
    /Neon Teamspace 0\.1\.0/,
  );

  await page.getByRole("button", { name: /^Acme/ }).click();
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  expect(await opened(page)).toEqual([ACME.origin]);
});

test("a screen reader reads the badge apart from the details before it", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: LAB.origin });
  await expect(page.getByRole("button", { name: /^Lab/ })).toHaveAccessibleDescription(
    "Neon Teamspace 0.2.0 · last used · Not encrypted",
  );
  await expect(page.getByRole("button", { name: /^Acme/ })).toHaveAccessibleDescription(
    "Neon Teamspace 0.1.0",
  );
});

test("the last server opens as the app starts, and a failure shows on its row", async ({
  page,
}) => {
  await launch(page, {
    servers: [ACME, { origin: "https://gone.example", name: "Gone", version: "0.1.0" }],
    last: "https://gone.example",
    open: true,
  });
  await expect(server(page, "Gone").getByRole("alert")).toContainText(
    "Could not reach https://gone.example",
  );
  await expect(page.getByRole("button", { name: /^Gone/ })).toBeFocused();
  expect((await calls(page)).map((call) => call.cmd)).toEqual(["startup", "open_server"]);

  // Coming back to the picker later shows the list and opens nothing.
  await page.reload();
  await expect(page.locator("main")).toHaveAttribute("aria-busy", "false");
  expect((await calls(page)).map((call) => call.cmd)).toEqual(["startup"]);
});

test("opening a server can be stopped while its page loads", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: null });
  await page.getByRole("button", { name: /^Acme/ }).click();
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  const stop = page.getByRole("button", { name: "Stop" });
  await expect(stop).toBeFocused();
  expect(await opened(page)).toEqual([ACME.origin]);

  // The app stops the page by loading the picker again, which says so.
  await stop.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Stopped opening Acme.");
  await expect(page.getByRole("button", { name: /^Acme/ })).toBeFocused();
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect((await calls(page)).map((call) => call.cmd)).toEqual(["startup"]);
});

test("a server whose page does not arrive in time is given up on, with the reason", async ({
  page,
}) => {
  await launch(page, { servers: [ACME, LAB], last: null, pageTimeout: 100 });
  await page.getByRole("button", { name: /^Lab/ }).click();
  await expect(server(page, "Lab").getByRole("alert")).toContainText(
    "its page did not start to arrive within 30 seconds",
  );
  await expect(page.getByRole("button", { name: /^Lab/ })).toBeFocused();
  await expect(page.locator("main")).toHaveAttribute("aria-busy", "false");
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
});

test("a check that keeps the picker waiting is stopped with Escape", async ({ page }) => {
  const slow = { origin: "https://slow.example.com", name: "Slow", version: "0.1.0" };
  await launch(page, {
    servers: [slow, ACME],
    last: null,
    network: { ...NETWORK, [slow.origin]: { ...slow, waits: true } },
  });
  await page.getByRole("button", { name: /^Slow/ }).click();
  await expect(page.getByRole("status")).toHaveText("Opening Slow…");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("status")).toHaveText("Stopped opening Slow.");
  await expect(page.getByRole("button", { name: /^Slow/ })).toBeFocused();
  expect(await opened(page)).toEqual([]);

  // Another server opens, and the check that was cancelled, answering late, changes nothing.
  await page.getByRole("button", { name: /^Acme/ }).click();
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  await page.evaluate(() => window.__answerChecks());
  await expect(page.getByRole("status")).toHaveText("Opening Acme…");
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(await opened(page)).toEqual([ACME.origin]);
  expect((await calls(page)).map((call) => call.cmd)).toEqual([
    "startup",
    "open_server",
    "cancel_opening",
    "open_server",
  ]);
});

test("checking a new address can be stopped, keeping what was typed", async ({ page }) => {
  await launch(page, {
    network: {
      ...NETWORK,
      "https://slow.example.com": { name: "Slow", version: "0.1.0", waits: true },
    },
  });
  const field = page.getByLabel("Server address");
  await field.fill("slow.example.com");
  await field.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Checking slow.example.com…");
  // The add form's Cancel, inert meanwhile, closes the form; Stop stops the check.
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped opening slow.example.com.");
  await expect(field).toBeFocused();
  await expect(field).toHaveValue("slow.example.com");
  await page.evaluate(() => window.__answerChecks());
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Connect to a server");
  expect(await opened(page)).toEqual([]);
});

test("a server is renamed, and a blank name brings back the instance's", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: null });
  await page.getByRole("button", { name: "Rename Acme" }).click();
  const field = page.getByLabel("Name for https://neon.example.com");
  await expect(field).toBeFocused();
  await expect(field).toHaveAttribute("placeholder", "Acme");
  await field.fill("Work");
  await field.press("Enter");
  await expect(page.getByRole("button", { name: /^Work/ })).toBeFocused();
  await expect(page.getByRole("status")).toHaveText("Renamed to Work.");
  await expect(server(page, "Work")).toContainText("Acme");

  // Escape leaves the name as it was.
  await page.getByRole("button", { name: "Rename Work" }).click();
  await page.getByLabel("Name for https://neon.example.com").fill("Something else");
  await page.getByLabel("Name for https://neon.example.com").press("Escape");
  await expect(page.getByRole("button", { name: /^Work/ })).toBeFocused();

  await page.getByRole("button", { name: "Rename Work" }).click();
  await page.getByLabel("Name for https://neon.example.com").fill("   ");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("button", { name: /^Acme/ })).toBeVisible();
  expect(
    (await calls(page)).filter((call) => call.cmd === "rename_server").map((c) => c.args),
  ).toEqual([
    { origin: ACME.origin, label: "Work" },
    { origin: ACME.origin, label: "   " },
  ]);
});

test("a name the app refuses shows its reason", async ({ page }) => {
  await launch(page, { servers: [ACME], last: null });
  await page.getByRole("button", { name: "Rename Acme" }).click();
  await page.getByLabel("Name for https://neon.example.com").evaluate((input, value) => {
    (input as HTMLInputElement).removeAttribute("maxlength");
    (input as HTMLInputElement).value = value;
  }, "n".repeat(81));
  await page.getByRole("button", { name: "Save" }).click();
  await expect(server(page, "Acme").getByRole("alert")).toContainText("at most 80 characters");
  await expect(page.getByLabel("Name for https://neon.example.com")).toBeFocused();
});

test("removing asks first, and focus moves on to the next server", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: ACME.origin });
  await page.getByRole("button", { name: "Remove Acme" }).click();
  await expect(page.getByRole("button", { name: "Keep it" })).toBeFocused();
  await expect(page.getByRole("group", { name: /Remove Acme from this app/ })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: /^Acme/ })).toBeFocused();

  await page.getByRole("button", { name: "Remove Acme" }).click();
  await page.getByRole("button", { name: "Remove Acme" }).last().click();
  await expect(page.getByRole("status")).toHaveText("Removed Acme.");
  await expect(page.getByRole("button", { name: /^Lab/ })).toBeFocused();
  await expect(page.getByRole("listitem")).toHaveCount(1);

  // The last one gone, the picker asks for an address again.
  await page.getByRole("button", { name: "Remove Lab" }).click();
  await page.getByRole("button", { name: "Remove Lab" }).last().click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Connect to a server");
  await expect(page.getByLabel("Server address")).toBeFocused();
  expect(
    (await calls(page)).filter((call) => call.cmd === "remove_server").map((c) => c.args),
  ).toEqual([{ origin: ACME.origin }, { origin: LAB.origin }]);
});

test("a second server is added with the keyboard alone", async ({ page, browserName }) => {
  await launch(page, { servers: [ACME], last: ACME.origin });
  // WebKit moves focus to buttons with Tab only when told to, as Safari's setting does.
  const tab = browserName === "webkit" ? "Alt+Tab" : "Tab";
  await page.getByRole("heading", { level: 1 }).focus();
  const order: string[] = [];
  for (let step = 0; step < 4; step += 1) {
    await page.keyboard.press(tab);
    order.push(
      await page.evaluate(
        () =>
          document.activeElement?.getAttribute("aria-label") ?? document.activeElement?.id ?? "",
      ),
    );
  }
  expect(order).toEqual([
    "open-https_3a__2f__2f_neon_2e_example_2e_com",
    "Rename Acme",
    "Remove Acme",
    "add-server",
  ]);

  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Add a server" })).toBeVisible();
  await expect(page.getByLabel("Server address")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Add a server" })).toBeFocused();

  await page.keyboard.press("Enter");
  await page.keyboard.type("http://192.168.1.20:3000");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Connect anyway" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Opening Lab…");
  expect(await opened(page)).toEqual([LAB.origin]);
});

test("the palette is chosen and kept, and the mode follows the system", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await launch(page, { servers: [ACME], last: null });
  const root = page.locator("html");
  await expect(root).toHaveAttribute("data-palette", "nothing");
  await expect(root).toHaveAttribute("data-mode", "light");
  await expect(page.getByRole("radio", { name: "Nothing" })).toBeChecked();

  await page.getByRole("radio", { name: "Warm" }).check();
  await expect(root).toHaveAttribute("data-palette", "warm");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(root).toHaveAttribute("data-mode", "dark");

  await page.reload();
  await expect(root).toHaveAttribute("data-palette", "warm");
  await expect(root).toHaveAttribute("data-mode", "dark");
  await expect(page.getByRole("radio", { name: "Warm" })).toBeChecked();
  const background = await page.evaluate(
    () => getComputedStyle(document.documentElement).backgroundColor,
  );
  expect(background).toBe("rgb(26, 24, 21)");
});

test("every control is at least a 44px target", async ({ page }) => {
  await launch(page, { servers: [ACME, LAB], last: null });
  await page.getByRole("button", { name: "Rename Acme" }).click();
  const controls = page.locator("button, a, input:not([type=radio]), label:has(input[type=radio])");
  const count = await controls.count();
  expect(count).toBeGreaterThan(8);
  for (let at = 0; at < count; at += 1) {
    const box = await controls.nth(at).boundingBox();
    expect(box, `control ${at}`).not.toBeNull();
    expect(box?.height ?? 0, `control ${at}`).toBeGreaterThanOrEqual(44);
  }
});

test("the picker fits a phone's width without scrolling sideways", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 640 });
  await launch(page, {
    servers: [{ ...ACME, label: "A name long enough to wrap onto a second line on a phone" }],
    last: null,
  });
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBe(0);
});

test("the picker links to the third-party notices of the packages and the crates", async ({
  page,
}) => {
  await launch(page, { servers: [ACME], last: null });
  await page.getByRole("link", { name: "Third-party licenses" }).click();
  await expect(page).toHaveURL(/\/THIRD_PARTY_NOTICES\.txt$/);
  const notices = page.locator("body");
  await expect(notices).toContainText("Third-party notices");
  // The picker's one package, and the crates of the webview on each platform.
  await expect(notices).toContainText(/@tauri-apps\/api \d/);
  for (const crate of ["tauri", "wry", "webkit2gtk", "webview2-com", "objc2-web-kit", "jni"]) {
    await expect(notices).toContainText(new RegExp(`\\n  ${crate} \\d`));
  }
});

test("the picker offers no check for updates where the app has a menu, or none to make", async ({
  page,
}) => {
  await launch(page, { servers: [ACME], last: null });
  await expect(page.getByRole("button", { name: "Check for updates" })).toHaveCount(0);
  await expect(page.locator("#updates")).toBeHidden();
});

test("on Android, a check for updates says the app is the latest", async ({ page }) => {
  await launch(page, {
    servers: [ACME],
    last: null,
    updates: [{ state: "current", version: "0.1.0", waits: true }],
  });
  // The status is a live region in the page before the first press, empty, so that a screen
  // reader announces its first message.
  const status = page.locator("#update-status");
  await expect(status).toBeVisible();
  await expect(status).toHaveText("");
  await expect(page.getByRole("status").and(status)).toHaveCount(1);
  // From the keyboard, which keeps the focus on the button in every engine: WebKit focuses no
  // button a click presses.
  const check = page.getByRole("button", { name: "Check for updates" });
  await check.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#update-status")).toHaveText("Checking for updates…");
  await expect(check).toHaveAttribute("aria-disabled", "true");
  await expect(check).toBeFocused();
  // A second press while the first is under way asks nothing more.
  await page.keyboard.press("Enter");
  await page.evaluate(() => window.__answerChecks());
  await expect(page.locator("#update-status")).toHaveText(
    "Neon Teamspace 0.1.0 is the latest version.",
  );
  await expect(check).not.toHaveAttribute("aria-disabled");
  await expect(check).toBeFocused();
  await expect(check).toHaveAccessibleDescription("Neon Teamspace 0.1.0 is the latest version.");
  expect((await calls(page)).filter((call) => call.cmd === "check_for_updates")).toEqual([
    { cmd: "check_for_updates", args: {} },
  ]);
});

test("on Android, a newer release is named while the app's dialog offers it", async ({ page }) => {
  await launch(page, {
    servers: [],
    last: null,
    updates: [{ state: "available", version: "0.2.0", current: "0.1.0" }],
  });
  await page.getByRole("button", { name: "Check for updates" }).click();
  await expect(page.locator("#update-status")).toHaveText(
    "Neon Teamspace 0.2.0 is available. You have 0.1.0.",
  );
  // The check is the footer's: the address field keeps what it had, and nothing opened.
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Connect to a server");
  expect(await opened(page)).toEqual([]);
});

test("on Android, a check that fails says why, and can be made again", async ({ page }) => {
  await launch(page, {
    servers: [ACME],
    last: null,
    updates: [
      {
        code: "unreachable",
        message:
          "The app could not reach GitHub, where its updates are published. Check the connection, then try again.",
      },
      { state: "current", version: "0.1.0" },
    ],
  });
  const check = page.getByRole("button", { name: "Check for updates" });
  await check.click();
  await expect(page.locator("#update-error")).toHaveText(/could not reach GitHub/);
  await expect(page.getByRole("alert")).toHaveCount(1);
  await expect(page.locator("#update-status")).toHaveText("");
  await check.click();
  await expect(page.locator("#update-status")).toHaveText(
    "Neon Teamspace 0.1.0 is the latest version.",
  );
  await expect(page.locator("#update-error")).toHaveText("");
});

test("on Android, the check for updates is a 44px target that fits a phone", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 640 });
  await launch(page, {
    servers: [ACME, LAB],
    last: null,
    updates: [{ state: "current", version: "0.1.0" }],
  });
  const check = page.getByRole("button", { name: "Check for updates" });
  await check.click();
  await expect(page.locator("#update-status")).not.toHaveText("");
  const box = await check.boundingBox();
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
  expect(box?.x ?? -1).toBeGreaterThanOrEqual(0);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBe(0);
});
