import { type Meta, SERVER_VERSION, expect, test } from "./fixtures";

test("the sidebar shows the instance's name and the server's version from the API", async ({
  page,
}) => {
  const answered = page.waitForResponse("**/api/v1/meta");
  await page.goto("/");
  const response = await answered;
  expect(response.status()).toBe(200);
  const meta = (await response.json()) as Meta;

  expect(meta.version).toBe(SERVER_VERSION);
  const configured = process.env.E2E_INSTANCE_NAME;
  if (configured !== undefined && configured !== "") {
    expect(meta.name).toBe(configured);
  }
  const footer = page.getByRole("complementary").getByRole("status");
  await expect(footer).toContainText(meta.name);
  await expect(footer).toContainText(`v${SERVER_VERSION}`);
});

test("each route of the app loads from its own address", async ({ page }) => {
  for (const [path, heading] of [
    ["/", "Nothing here yet"],
    ["/styleguide", "Styleguide"],
    ["/settings/appearance", "Appearance"],
  ] as const) {
    const response = await page.goto(path);

    expect(response?.status(), path).toBe(200);
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  }
});

test("an address the app has no route for gets its not-found page", async ({ page }) => {
  await page.goto("/no/such/page");

  await expect(page.getByRole("heading", { level: 1, name: "Page not found" })).toBeVisible();
});
