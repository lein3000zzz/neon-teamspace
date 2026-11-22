import { expect, test } from "./fixtures";

test("a visitor without a session is sent to sign in, with the way back", async ({ page }) => {
  const response = await page.goto("/account/tokens");

  expect(response?.status()).toBe(200);
  await expect(page).toHaveURL(/\/login\?next=%2Faccount%2Ftokens$/);
  await expect(page.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
});

test("each signed-out page of the app loads from its own address", async ({ page }) => {
  // Without mail set up, which the e2e stack has not, the pages that need it say they are off.
  // /reset without a token asks for a link, as /forgot does.
  for (const [path, heading] of [
    ["/login", "Sign in"],
    ["/register", "Create an account"],
    ["/forgot", "Password resets are off"],
    ["/join", "Requests to join are off"],
    ["/verify", "This link is incomplete"],
    ["/reset", "Password resets are off"],
  ] as const) {
    const response = await page.goto(path);

    expect(response?.status(), path).toBe(200);
    await expect(page.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  }
});

test("the sign-in page names the instance", async ({ page }) => {
  await page.goto("/login");

  const configured = process.env.E2E_INSTANCE_NAME;
  if (configured !== undefined && configured !== "") {
    await expect(page.getByText(configured, { exact: true })).toBeVisible();
  }
});
