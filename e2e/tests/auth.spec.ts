import { clientAddress } from "./accounts";
import { ADMIN, type Meta, SERVER_VERSION, expect, loadMoreUntil, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({ extraHTTPHeaders: clientAddress(25) });

/** The first admin, which the skip above makes sure of. */
function admin() {
  if (ADMIN === undefined) {
    throw new Error("E2E_ADMIN_EMAIL and E2E_ADMIN_PASSWORD are unset");
  }
  return ADMIN;
}

test("the first admin signs in, and the shell shows the instance and the admin pages", async ({
  page,
}) => {
  const { email, password } = admin();
  // The sign-in page asks for the instance's name already, and the shell reuses the answer.
  const answered = page.waitForResponse("**/api/v1/meta");
  await page.goto("/");
  await expect(page).toHaveURL(/\/login\?next=%2F$/);

  await signIn(page, email, password);
  await expect(
    page.getByRole("heading", { level: 1, name: "You are in no workspace yet" }),
  ).toBeVisible();
  const meta = (await (await answered).json()) as Meta;

  expect(meta.version).toBe(SERVER_VERSION);
  const sidebar = page.getByRole("complementary");
  const footer = sidebar.getByRole("status");
  await expect(footer).toContainText(meta.name);
  await expect(footer).toContainText(`v${SERVER_VERSION}`);
  await expect(sidebar.getByText(email)).toBeVisible();
  await expect(sidebar.getByRole("link", { name: "Admin" })).toBeVisible();

  // Every page of the shell loads from its own address once signed in.
  for (const [path, heading] of [
    ["/styleguide", "Styleguide"],
    ["/account", "Account"],
    ["/account/appearance", "Account"],
    ["/admin", "Instance"],
    // Where the mail telling admins of a request to join links.
    ["/admin/join-requests", "Instance"],
    ["/no/such/page", "Page not found"],
  ] as const) {
    await page.goto(path);
    await expect(page.getByRole("heading", { level: 1, name: heading }), path).toBeVisible();
  }
});

test("an admin invites someone, who signs up with the link in another browser", async ({
  page,
  browser,
}) => {
  const { email, password } = admin();
  await page.goto("/login?next=%2Fadmin%2Finvites");
  await signIn(page, email, password);
  await expect(page.getByRole("heading", { level: 2, name: "Make an invite" })).toBeVisible();

  await page
    .getByRole("form", { name: "Make an invite" })
    .getByRole("button", { name: "Make invite" })
    .click();
  await expect(page.getByText("Invite made: share its link")).toBeVisible();
  const link = await page.getByLabel("Invite link").inputValue();
  const code = await page.getByLabel("Invite code").inputValue();
  expect(code).toMatch(/^inv_[A-Za-z0-9_-]{43}$/);
  expect(link).toContain(`/register?invite=${code}`);

  // Someone else, in a browser of their own, opens the link.
  const newcomer = `newcomer-${Date.now()}@e2e.example`;
  const context = await browser.newContext();
  try {
    const other = await context.newPage();
    await other.goto(link);
    await expect(other.getByRole("heading", { level: 1, name: "Create an account" })).toBeVisible();
    // The code comes out of the address and stays in the form.
    await expect(other).toHaveURL(/\/register$/);
    await expect(other.getByLabel("Invite code")).toHaveValue(code);

    await other.getByLabel("Email").fill(newcomer);
    await other.getByLabel("Display name").fill("Newcomer");
    await other.getByLabel("Password").fill("a passphrase long enough");
    await other.getByRole("button", { name: "Create account" }).click();

    await expect(
      other.getByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    const sidebar = other.getByRole("complementary");
    await expect(sidebar.getByText(newcomer)).toBeVisible();
    await expect(sidebar.getByRole("link", { name: "Admin" })).toHaveCount(0);

    // A member gets the not-allowed page at an admin address.
    await other.goto("/admin");
    await expect(other.getByRole("heading", { level: 1, name: "Not allowed" })).toBeVisible();

    await sidebar.getByRole("button", { name: "Sign out" }).click();
    await expect(other.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    await other.goto("/account");
    await expect(other).toHaveURL(/\/login\?next=%2Faccount$/);
  } finally {
    await context.close();
  }

  // The invite is used up, and says who signed up with it.
  await page.getByRole("button", { name: "Done" }).click();
  await page.reload();
  // Newest first, so the first is this run's.
  const card = page.getByRole("listitem").filter({ hasText: "1 of 1 sign-up" }).first();
  await expect(card).toBeVisible();
  await expect(card.getByText("Used up")).toBeVisible();
  await card.getByRole("button", { name: "Who signed up (1)" }).click();
  await expect(card.getByText(newcomer)).toBeVisible();

  // And the new account is in the list of users.
  await page.goto("/admin");
  await loadMoreUntil(page, page.getByText(newcomer));
});

test("an admin reads the runtime settings, with what the stack pins", async ({ page }) => {
  const { email, password } = admin();
  await page.goto("/login?next=%2Fadmin%2Fsettings");
  await signIn(page, email, password);

  const accounts = page.getByRole("region", { name: "Accounts" });
  await expect(accounts.getByLabel("auth.registration")).toHaveValue("invite");
  await expect(accounts.getByLabel("auth.min_password_length")).toHaveValue("12");

  // The stack sets the instance's name in the environment, which pins it.
  const instance = page.getByRole("region", { name: "Instance" });
  const name = instance.getByLabel("instance.name");
  await expect(name).toBeDisabled();
  const configured = process.env.E2E_INSTANCE_NAME;
  if (configured !== undefined && configured !== "") {
    await expect(name).toHaveValue(configured);
  }
  await expect(instance.getByText("Pinned")).toBeVisible();
  await expect(instance.getByText("Environment")).toBeVisible();
  await expect(instance.getByText("Change it with NEON_SETTINGS__INSTANCE__NAME.")).toBeVisible();

  // The stack sets mail up in the environment, to its Mailpit, so it can go.
  const mail = page.getByRole("region", { name: "Mail" });
  await expect(mail.getByText("Set up", { exact: true })).toBeVisible();
  await expect(mail.getByLabel("mail.host")).toHaveValue("mailpit");
  await expect(mail.getByLabel("mail.host")).toBeDisabled();
  await expect(mail.getByRole("button", { name: "Send a test mail" })).toBeEnabled();
});
