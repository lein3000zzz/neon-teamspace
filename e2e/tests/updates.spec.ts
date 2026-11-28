/**
 * The server's version and updates in the admin settings, against the stack's receiver, which
 * stands in for GitHub's releases and for the updater: its newest release is 99.0.0, beside a
 * pre-release and a draft, and its updater starts an update and then says it failed. The
 * second test saves the updater's token, an instance setting, so these tests run one at a
 * time, apart from the specs that read the settings, and put the token back afterwards.
 */
import type { Page } from "@playwright/test";
import { asAdmin, callApi, clientAddress } from "./accounts";
import { ADMIN, SERVER_VERSION, expect, test as base } from "./fixtures";

/** The token the receiver's updater takes, obviously not a secret: e2e/receiver/server.ts. */
const UPDATER_TOKEN = "e2e-updater-token-not-a-secret-at-all";

/** The newest release the receiver lists that counts: neither its pre-release nor its draft. */
const RELEASE = "99.0.0";

const RECEIVER = process.env.E2E_RECEIVER_URL ?? "";

type Delivery = { path: string; headers: Record<string, string>; body: string };

/**
 * A browser signed in as the first admin; once the test is over, the updater's token goes
 * back to its default, none, whatever became of the test.
 */
const test = base.extend<{ adminPage: Page }>({
  adminPage: async ({ browser }, use) => {
    const context = await browser.newContext();
    try {
      const page = await context.newPage();
      await asAdmin(page);
      await use(page);
      await callApi(
        page,
        "PATCH",
        "/api/v1/admin/settings",
        { changes: { "updates.updater_token": null } },
        200,
      );
    } finally {
      await context.close();
    }
  },
});

test.skip(
  ADMIN === undefined || RECEIVER === "",
  "needs the first admin and the receiver that make e2e sets up",
);

test.use({ extraHTTPHeaders: clientAddress(31) });

/** Opens the admin settings and has the server check for releases now. */
async function checkNow(page: Page) {
  await page.goto("/admin/settings");
  const panel = page.getByRole("region", { name: "Version and updates" });
  await expect(panel.getByText(SERVER_VERSION, { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: "Check now" }).click();
  await expect(panel.getByText(`Neon Teamspace ${RELEASE} is out`)).toBeVisible();
  return panel;
}

test("an admin checks for releases and is given the commands to update by hand", async ({
  adminPage: page,
}) => {
  const panel = await checkNow(page);

  await expect(
    panel.getByRole("link", { name: `Release notes of ${RELEASE}, on GitHub` }),
  ).toHaveAttribute(
    "href",
    `https://github.com/lein3000zzz/neon-teamspace/releases/tag/v${RELEASE}`,
  );
  // Neither the pre-release, while updates.prereleases is off, nor the draft counts.
  await expect(panel.getByText("99.1.0-rc.1")).toHaveCount(0);
  await expect(panel.getByText("100.0.0")).toHaveCount(0);
  await expect(panel.getByLabel("deployments/.env")).toHaveText(`NEON_VERSION=${RELEASE}`);
  await expect(panel.getByLabel("Commands")).toContainText(
    "docker compose -f deployments/docker-compose.yml up -d --wait",
  );
  await expect(panel.getByRole("button", { name: `Update to ${RELEASE}` })).toHaveCount(0);
  await expect(
    page.getByRole("complementary").getByRole("link", { name: "Admin, an update is available" }),
  ).toBeVisible();

  // The settings of updates are listed with the rest; the stack pins where the updater is.
  const settings = page.getByRole("region", { name: "Updates", exact: true });
  await expect(settings.getByRole("radiogroup", { name: "updates.check" })).toBeVisible();
  await expect(settings.getByRole("radiogroup", { name: "updates.prereleases" })).toBeVisible();
  await expect(settings.getByLabel("updates.updater_url")).toHaveValue(
    "http://receiver:8080/updater",
  );
  await expect(settings.getByLabel("updates.updater_url")).toBeDisabled();
  await expect(settings.getByLabel("updates.updater_token")).toBeEnabled();
});

test("with the updater set up, an admin starts an update and follows it until it fails", async ({
  adminPage: page,
  playwright,
}) => {
  await page.goto("/admin/settings");
  const settings = page.getByRole("region", { name: "Updates", exact: true });
  await settings.getByLabel("updates.updater_token").fill(UPDATER_TOKEN);
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText("Saved.", { exact: true })).toBeVisible();

  const panel = await checkNow(page);
  await panel.getByRole("button", { name: `Update to ${RELEASE}` }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("The server restarts on the new release");
  await dialog.getByRole("button", { name: "Update" }).click();

  await expect(panel.getByText(`Updating to ${RELEASE}`)).toBeVisible();
  await expect(panel.getByLabel("What the updater did")).toContainText(
    `Pulling server, web at ${RELEASE}`,
  );
  await expect(
    panel.getByText(
      `The update to ${RELEASE} failed: the end-to-end tests' receiver stands in for the updater, and updates nothing.`,
    ),
  ).toBeVisible({ timeout: 20_000 });
  await expect(panel.getByRole("button", { name: `Update to ${RELEASE}` })).toBeEnabled();

  // The server asked the updater for the release, with the token.
  const receiver = await playwright.request.newContext({ baseURL: RECEIVER });
  try {
    const got = (await (await receiver.get("/deliveries")).json()) as Delivery[];
    const asked = got.filter((entry) => entry.path === "/updater/v1/update");
    expect(asked.at(-1)?.headers.authorization).toBe(`Bearer ${UPDATER_TOKEN}`);
    expect(JSON.parse(asked.at(-1)?.body ?? "{}")).toEqual({ version: RELEASE });
  } finally {
    await receiver.dispose();
  }
});
