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
  const controls = page.locator("button, input:not([type=radio]), label:has(input[type=radio])");
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
