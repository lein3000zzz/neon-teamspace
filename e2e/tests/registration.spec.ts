/**
 * Who may make an account, in each registration mode, with what the stack's mail carries:
 * open sign-ups that verify their address first, sign-ups by an invite's code, closed
 * sign-ups, and requests to join that an admin approves or denies. These tests change the
 * instance's settings, so they run one at a time, apart from the specs that read them, and put
 * each setting back to its default after each test.
 */
import type { Page } from "@playwright/test";
import { account, asAdmin, callApi, clientAddress, PASSWORD, runOf } from "./accounts";
import { ADMIN, type ApiErrorBody, expect, test as base } from "./fixtures";
import { instanceName, linkIn, MAIL, mailTo } from "./mail";

/** The settings these tests change, with what they change them to. */
type Changes = Partial<{
  "auth.registration": "open" | "invite" | "closed";
  "auth.verify_email": boolean;
  "auth.requests": boolean;
}>;

/** Their keys, each back to its default after every test. */
const CHANGED: (keyof Changes)[] = ["auth.registration", "auth.verify_email", "auth.requests"];

/**
 * A browser signed in as the first admin, for the test to change the settings and act as an
 * admin in; once the test is over, it puts the settings it may have changed back to their
 * defaults, whatever became of the test.
 */
const test = base.extend<{ adminPage: Page }>({
  adminPage: async ({ browser }, use) => {
    const context = await browser.newContext();
    try {
      const page = await context.newPage();
      await asAdmin(page);
      await use(page);
      const defaults = Object.fromEntries(CHANGED.map((key) => [key, null]));
      await callApi(page, "PATCH", "/api/v1/admin/settings", { changes: defaults }, 200);
    } finally {
      await context.close();
    }
  },
});

test.skip(
  ADMIN === undefined || MAIL === "",
  "needs the first admin and the Mailpit that make e2e sets up",
);

// Each test waits on mail, and its admin's settings are put back once it is over.
test.describe.configure({ timeout: 60_000 });

// A sign-in before the address is verified answers 403, a wrong invite code and a link used
// already 400; Chromium logs each as a failed load.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(clientAddress(27, testInfo.repeatEachIndex)),
  expectedConsoleError: /the server responded with a status of 40[03]/,
});

/** Changes settings on the admin's settings page, as an admin does, and saves them. */
async function changeSettings(page: Page, changes: Changes) {
  await page.goto("/admin/settings");
  const accounts = page.getByRole("region", { name: "Accounts" });
  for (const [key, value] of Object.entries(changes)) {
    if (typeof value === "boolean") {
      await accounts
        .getByRole("radiogroup", { name: key })
        .getByRole("radio", { name: value ? "On" : "Off" })
        .click();
    } else {
      await accounts.getByLabel(key).selectOption(value);
    }
  }
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText("Saved.", { exact: true })).toBeVisible();
}

/** Fills the sign-up form, all but the invite code. */
async function fillSignUp(page: Page, email: string, name: string) {
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Display name").fill(name);
  await page.getByLabel("Password").fill(PASSWORD);
}

/** Signs in as `email` through the form on the page. */
async function signInAs(page: Page, email: string) {
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
}

const NO_WORKSPACE = { level: 1, name: "You are in no workspace yet" } as const;

test("while anyone may sign up, a newcomer verifies the address with the mailed link, then signs in and out", async ({
  page,
  adminPage,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(page.request);
  const newcomer = account("newcomer", runOf());
  await changeSettings(adminPage, { "auth.registration": "open", "auth.verify_email": true });

  // The sign-in page offers an account to anyone, and the form asks for no invite.
  await page.goto("/login");
  await page.getByRole("link", { name: "Create one" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Create an account" })).toBeVisible();
  await expect(page.getByLabel("Invite code")).toHaveCount(0);
  await fillSignUp(page, newcomer.email, newcomer.name);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Check your inbox" })).toBeVisible();
  await expect(page.getByText(`A link to verify ${newcomer.email} is on its way.`)).toBeVisible();

  // Until the address is verified, the right password does not sign in.
  await page.getByRole("link", { name: "Sign in" }).click();
  await signInAs(page, newcomer.email);
  await expect(
    page.getByText("Verify your email first: open the link mailed to you."),
  ).toBeVisible();

  // The mailed link verifies it, and leads on to sign in with the address filled in.
  const mail = await mailTo(newcomer.email, `Verify your email for ${instance}`);
  const link = linkIn(mail, "Verify my address", origin);
  await page.goto(link);
  await expect(page).toHaveURL(/\/login$/);
  await expect(page.getByText("Your address is verified. Sign in to go on.")).toBeVisible();
  await expect(page.getByLabel("Email")).toHaveValue(newcomer.email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", NO_WORKSPACE)).toBeVisible();
  const sidebar = page.getByRole("complementary");
  await expect(sidebar.getByText(newcomer.email)).toBeVisible();
  await expect(sidebar.getByRole("link", { name: "Admin" })).toHaveCount(0);

  // Signing out ends the session: the app's pages ask to sign in again.
  await sidebar.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
  await page.goto("/account");
  await expect(page).toHaveURL(/\/login\?next=%2Faccount$/);

  // The link works once.
  await page.goto(link);
  await expect(
    page.getByRole("heading", { level: 1, name: "This link does not work" }),
  ).toBeVisible();
});

test("while sign-ups are by invite, the code an admin typed admits one newcomer, and nobody without it", async ({
  page,
  adminPage,
  browser,
}) => {
  const run = runOf();
  const newcomer = account("invited", run);
  const latecomer = account("latecomer", run);
  const code = `e2e-code-${run}`;

  // By invite is the default. The admin makes an invite with a code of their own.
  await adminPage.goto("/admin/invites");
  await expect(adminPage.getByText("People need an invite to create an account.")).toBeVisible();
  const make = adminPage.getByRole("form", { name: "Make an invite" });
  await make.getByLabel("Code").fill(code);
  await expect(make.getByText("Free to take.")).toBeVisible();
  await make.getByRole("button", { name: "Make invite" }).click();
  await expect(adminPage.getByText("Invite made: share its link")).toBeVisible();
  await expect(adminPage.getByLabel("Invite code")).toHaveValue(code);

  // The form asks for the code, refuses a wrong one, and takes the right one in any case.
  await page.goto("/register");
  await expect(
    page.getByText("Accounts here are by invite: you need a code from an admin."),
  ).toBeVisible();
  await fillSignUp(page, newcomer.email, newcomer.name);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByText("Fill this in.")).toBeVisible();
  await page.getByLabel("Invite code").fill(`${code}-wrong`);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByText("Invalid or expired invite.")).toBeVisible();
  await page.getByLabel("Invite code").fill(code.toUpperCase());
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("heading", NO_WORKSPACE)).toBeVisible();
  await expect(page.getByRole("complementary").getByText(newcomer.email)).toBeVisible();

  // It admitted one sign-up, and admits nobody after.
  const context = await browser.newContext();
  try {
    const other = await context.newPage();
    await other.goto("/register");
    await fillSignUp(other, latecomer.email, latecomer.name);
    await other.getByLabel("Invite code").fill(code);
    await other.getByRole("button", { name: "Create account" }).click();
    await expect(other.getByText("Invalid or expired invite.")).toBeVisible();
  } finally {
    await context.close();
  }
});

test("while sign-ups are closed, nobody signs up, even with an invite, and an admin adds the account instead", async ({
  page,
  adminPage,
}) => {
  const newcomer = account("added", runOf());
  // An invite made before the sign-ups closed, which admits nobody now.
  const { code } = await callApi<{ code: string }>(
    adminPage,
    "POST",
    "/api/v1/admin/invites",
    {},
    201,
  );
  await changeSettings(adminPage, { "auth.registration": "closed" });
  await adminPage.goto("/admin/invites");
  await expect(
    adminPage.getByText(
      "Sign-ups are closed, so invites admit nobody until registration is by invite.",
    ),
  ).toBeVisible();

  // The sign-in page offers no account, and the sign-up page says whom to ask.
  await page.goto("/login");
  await expect(page.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Create one" })).toHaveCount(0);
  await page.goto(`/register?invite=${code}`);
  await expect(page.getByRole("heading", { level: 1, name: "Sign-ups are closed" })).toBeVisible();
  await expect(
    page.getByText("Admins add the accounts on this instance. Ask one for access."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Create account" })).toHaveCount(0);
  // Nor does the API take a sign-up, the invite's code and all.
  const refused = await page.request.post("/api/v1/auth/register", {
    headers: { Origin: new URL(page.url()).origin },
    data: { email: newcomer.email, password: PASSWORD, invite_code: code },
  });
  expect(refused.status()).toBe(403);
  expect(((await refused.json()) as ApiErrorBody).error.code).toBe("registration_closed");

  // An admin adds the account, which then signs in.
  await adminPage.goto("/admin");
  await adminPage.getByRole("button", { name: "Add an account" }).click();
  const add = adminPage.getByRole("form", { name: "Add an account" });
  await add.getByLabel("Email").fill(newcomer.email);
  await add.getByLabel("Display name").fill(newcomer.name);
  await add.getByLabel("Password").fill(PASSWORD);
  await add.getByRole("button", { name: "Add account" }).click();
  await expect(adminPage.getByText(`Added ${newcomer.email}.`)).toBeVisible();
  await page.goto("/login");
  await signInAs(page, newcomer.email);
  await expect(page.getByRole("heading", NO_WORKSPACE)).toBeVisible();
});

/** Asks to join on /join, from the sign-in page's link, as `email` with `note`. */
async function askToJoin(page: Page, email: string, note: string) {
  await page.goto("/login");
  await page.getByRole("link", { name: "Ask for one" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Ask for an invite" })).toBeVisible();
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Note for the admins").fill(note);
  await page.getByRole("button", { name: "Ask for an invite" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Check your inbox" })).toBeVisible();
}

/** Opens the link of the mail confirming a request to join, which hands it to the admins. */
async function confirmRequest(page: Page, email: string, instance: string, origin: string) {
  const mail = await mailTo(email, `Confirm your request to join ${instance}`);
  await page.goto(linkIn(mail, "Confirm my request", origin));
  await expect(
    page.getByRole("heading", { level: 1, name: "Your request is with the admins" }),
  ).toBeVisible();
}

test("a request to join, confirmed by mail, is approved from the admin's mail, and its mailed invite makes the account", async ({
  page,
  adminPage,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(page.request);
  const asker = account("asker", runOf());
  const note = `I run the ${asker.name} team.`;
  // Requests to join take sign-ups by invite, the default, and mail.
  await changeSettings(adminPage, { "auth.requests": true });

  await askToJoin(page, asker.email, note);
  await confirmRequest(page, asker.email, instance, origin);

  // The admins hear of it by mail, with the note, and its link opens the request.
  const told = await mailTo(ADMIN?.email ?? "", `${asker.email} asks to join ${instance}`);
  expect(told.text).toContain(`They say: “${note}”`);
  await adminPage.goto(linkIn(told, "Review the request", origin));
  await expect(adminPage).toHaveURL(/\/admin\/join-requests$/);
  const request = adminPage.getByRole("listitem").filter({ hasText: asker.email });
  await expect(request.getByText("Waiting")).toBeVisible();
  await expect(request.getByText(note)).toBeVisible();
  await request.getByRole("button", { name: `Approve: ${asker.email}` }).click();
  await expect(adminPage.getByText(`Invite on its way to ${asker.email}`)).toBeVisible();
  await expect(request.getByText("Approved")).toBeVisible();

  // The invite comes by mail, and works for that address alone.
  const { display_name: adminName } = (await (
    await adminPage.request.get("/api/v1/account/me")
  ).json()) as { display_name: string };
  const invite = await mailTo(asker.email, `${adminName} invited you to ${instance}`);
  await page.goto(linkIn(invite, "Make my account", origin));
  await expect(page.getByRole("heading", { level: 1, name: "Create an account" })).toBeVisible();
  await expect(page.getByLabel("Email")).toHaveValue(asker.email);
  await expect(page.getByLabel("Email")).not.toBeEditable();
  await expect(page.getByText("The invite is for this address alone.")).toBeVisible();
  await page.getByLabel("Display name").fill(asker.name);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("heading", NO_WORKSPACE)).toBeVisible();
  await expect(page.getByRole("complementary").getByText(asker.email)).toBeVisible();
});

test("a request to join an admin denies is told so by mail", async ({
  page,
  adminPage,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(page.request);
  const asker = account("hopeful", runOf());
  await changeSettings(adminPage, { "auth.requests": true });

  await askToJoin(page, asker.email, "Let me in, please.");
  await confirmRequest(page, asker.email, instance, origin);

  await adminPage.goto("/admin/join-requests");
  const request = adminPage.getByRole("listitem").filter({ hasText: asker.email });
  await request.getByRole("button", { name: `Deny: ${asker.email}` }).click();
  const dialog = adminPage.getByRole("alertdialog", { name: `Deny ${asker.email}?` });
  await dialog.getByRole("button", { name: "Deny" }).click();
  await expect(request.getByText(`Denied ${asker.email}.`)).toBeVisible();
  await adminPage.reload();
  await expect(request.getByText("Denied", { exact: true })).toBeVisible();
  await expect(request.getByRole("button", { name: `Approve: ${asker.email}` })).toHaveCount(0);

  // The mail says so, and when they may ask again; it carries no invite.
  const denied = await mailTo(asker.email, `Your request to join ${instance}`);
  expect(denied.text).toContain("did not approve your request to join this time");
  expect(denied.text).toContain("You may ask again in 30 days.");
  expect(denied.text).not.toContain("/register");
});
