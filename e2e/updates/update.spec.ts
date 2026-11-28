/**
 * An instance updated from the admin settings, for real: run.ts starts a stack of the older
 * release's images with the updater profile, the release check pointed at a stand-in for
 * GitHub that lists the newer release, and both releases' images in a registry on this
 * machine's loopback. The first admin makes a page, presses Update, and the panel follows the
 * updater through the restart until the page reloads on the newer release.
 *
 * Its own test, rather than a plain Playwright one with the suite's fixture, as that fails on
 * any error in the browser's console: the restart fails the panel's requests on purpose.
 */
import { expect, test } from "@playwright/test";
import { callApi } from "../tests/accounts";
import { ADMIN, signIn, type ApiErrorBody, type Meta } from "../tests/fixtures";
import { generalOf, type Workspace } from "../tests/phone";

/** The release the stack starts on, and the one it is updated to. */
const FROM = process.env.UPDATES_E2E_FROM ?? "";
const TO = process.env.UPDATES_E2E_TO ?? "";

/** How long the update may take, through the pulls and the restart. */
const UPDATE_TAKES_MS = 10 * 60_000;

test.skip(
  ADMIN === undefined || FROM === "" || TO === "",
  "needs the stack and the releases that make updates-e2e sets up",
);

test("an admin updates the instance to a newer release from the admin settings", async ({
  page,
}) => {
  const admin = ADMIN ?? { email: "", password: "" };
  await page.goto("/login");
  await signIn(page, admin.email, admin.password);
  await expect(page.getByRole("complementary")).toBeVisible();

  // A page written on the older release, which the update must keep.
  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: "Before the update" },
    201,
  );
  const made = await callApi<{ id: string }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/pages`,
    {
      teamspace_id: await generalOf(page, workspace),
      title: "Kept across the update",
      markdown: `Written on ${FROM}.`,
    },
    201,
  );
  const pagePath = `/w/${workspace.slug}/p/${made.id}`;
  await page.goto(pagePath);
  await expect(page.getByRole("textbox", { name: "Page title" })).toHaveValue(
    "Kept across the update",
  );

  // The admin settings offer the newer release, which the server's check found as it started;
  // Check now asks again, should the first check have come too early.
  await page.goto("/admin/settings");
  const panel = page.getByRole("region", { name: "Version and updates" });
  await expect(panel.getByText(FROM, { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: "Check now" }).click();
  await expect(panel.getByText(`Neon Teamspace ${TO} is out`)).toBeVisible();
  const update = panel.getByRole("button", { name: `Update to ${TO}` });
  await expect(update).toBeEnabled();

  await update.click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("The server restarts on the new release");
  // No navigation comes between the confirmation and the reload the panel does once the
  // server runs the newer release, so the next load is that one.
  const reloaded = page.waitForEvent("load", { timeout: UPDATE_TAKES_MS });
  await dialog.getByRole("button", { name: "Update" }).click();
  await expect(panel.getByText(`Updating to ${TO}`)).toBeVisible();

  // A second update while this one runs: the updater refuses it, and the server says so.
  const again = await page.request.post("/api/v1/admin/updates/apply", {
    headers: { Origin: new URL(page.url()).origin },
    data: { version: TO },
  });
  expect(again.status(), "a second update while one runs").toBe(409);
  expect(((await again.json()) as ApiErrorBody).error.code).toBe("update_in_progress");

  // The panel follows the update through the restart, and the page reloads on the newer
  // release.
  await reloaded;
  await expect(panel.getByText(TO, { exact: true }).first()).toBeVisible();
  await expect(panel.getByText("This is the newest release.")).toBeVisible({ timeout: 60_000 });
  const meta = (await (await page.request.get("/api/v1/meta")).json()) as Meta;
  expect(meta.version).toBe(TO);

  // The page made on the older release is still there, as it was.
  await page.goto(pagePath);
  await expect(page.getByRole("textbox", { name: "Page title" })).toHaveValue(
    "Kept across the update",
  );
  await expect(page.getByRole("region", { name: "Page content" })).toContainText(
    `Written on ${FROM}.`,
  );
});
