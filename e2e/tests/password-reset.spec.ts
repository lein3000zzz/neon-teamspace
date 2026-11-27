import type { BrowserContext } from "@playwright/test";
import { account, clientAddress, makeAccounts, PASSWORD, runOf, signedIn } from "./accounts";
import { ADMIN, expect, test } from "./fixtures";
import { instanceName, linkIn, MAIL, mailTo } from "./mail";

test.skip(
  ADMIN === undefined || MAIL === "",
  "needs the first admin and the Mailpit that make e2e sets up",
);

// The old password, refused, answers 401, and the link used already 400; Chromium logs both
// as failed loads.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(clientAddress(28, testInfo.repeatEachIndex)),
  expectedConsoleError: /the server responded with a status of 40[01]/,
});

/** The password the reset sets. */
const NEW_PASSWORD = "another passphrase, longer still";

test("someone who forgot their password sets a new one with the mailed link, which signs them out everywhere", async ({
  page,
  browser,
  baseURL,
}) => {
  test.setTimeout(60_000);
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(page.request);
  const person = account("forgetful", runOf());
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [person]);
    // Signed in on another device, which the reset signs out.
    const [otherContext, otherPage] = await signedIn(browser, person);
    contexts.push(otherContext);

    // On this one, the sign-in page leads to asking for a link, the address carried along.
    await page.goto("/login");
    await page.getByLabel("Email").fill(person.email);
    await page.getByRole("link", { name: "Forgot your password?" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Forgot your password?" }),
    ).toBeVisible();
    await expect(page.getByLabel("Email")).toHaveValue(person.email);
    await page.getByRole("button", { name: "Send the link" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Check your inbox" })).toBeVisible();

    // The mailed link asks for the new password twice.
    const mail = await mailTo(person.email, `Set a new ${instance} password`);
    const link = linkIn(mail, "Set a new password", origin);
    await page.goto(link);
    await expect(page.getByRole("heading", { level: 1, name: "Set a new password" })).toBeVisible();
    await page.getByLabel("New password", { exact: true }).fill(NEW_PASSWORD);
    await page.getByLabel("Repeat the new password").fill(`${NEW_PASSWORD}!`);
    await page.getByRole("button", { name: "Set the password" }).click();
    await expect(page.getByText("The two passwords differ.")).toBeVisible();
    await page.getByLabel("Repeat the new password").fill(NEW_PASSWORD);
    await page.getByRole("button", { name: "Set the password" }).click();

    // Then on to sign in, which the old password no longer does, and the new one does.
    await expect(page).toHaveURL(/\/login$/);
    await expect(
      page.getByText("Your password is set, and every session was signed out. Sign in with it."),
    ).toBeVisible();
    await expect(page.getByLabel("Email")).toHaveValue(person.email);
    await page.getByLabel("Password").fill(PASSWORD);
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByText("Invalid email or password.")).toBeVisible();
    await page.getByLabel("Password").fill(NEW_PASSWORD);
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    await expect(page.getByRole("complementary").getByText(person.email)).toBeVisible();

    // The other device's session ended with the reset.
    await otherPage.reload();
    await expect(otherPage).toHaveURL(/\/login\?next=/);

    // The link works once: opened there again, it sets nothing.
    await otherPage.goto(link);
    await otherPage.getByLabel("New password", { exact: true }).fill(PASSWORD);
    await otherPage.getByLabel("Repeat the new password").fill(PASSWORD);
    await otherPage.getByRole("button", { name: "Set the password" }).click();
    await expect(
      otherPage.getByText(
        "This link does not work any more: it was used already, or it is too old.",
      ),
    ).toBeVisible();
    await expect(otherPage.getByRole("link", { name: "Ask for a new link" })).toBeVisible();

    // A mail tells of the change, with the way to set another should it not have been them.
    const changed = await mailTo(person.email, `Your ${instance} password was changed`);
    expect(linkIn(changed, "Set a new password", origin)).toBe(`${origin}/reset`);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
