import { createHmac, timingSafeEqual } from "node:crypto";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

/** The test stack's webhook receiver, as the tests reach it; make e2e sets it. */
const RECEIVER = process.env.E2E_RECEIVER_URL ?? "";

/** The same receiver as the server reaches it, on the stack's network. */
const RECEIVER_ON_STACK = "http://receiver:8080";

test.skip(
  ADMIN === undefined || RECEIVER === "",
  "needs the first admin and the webhook receiver that make e2e sets up",
);

test.use({ extraHTTPHeaders: clientAddress(16) });

type Workspace = { id: string; slug: string };
type Teamspaces = { teamspaces: { id: string; access: string }[] };
type Database = { page: { id: string }; properties: { id: string; name: string }[] };
type Agent = { id: string; name: string };
type Delivery = { path: string; headers: Record<string, string>; body: string };

/** Whether a delivery is signed with `secret`, within five minutes, as the docs check it. */
function signedWith(delivery: Delivery, secret: string): boolean {
  const timestamp = delivery.headers["neon-webhook-timestamp"] ?? "";
  const signature = delivery.headers["neon-webhook-signature"] ?? "";
  if (Math.abs(Date.now() / 1000 - Number(timestamp)) > 300) {
    return false;
  }
  const expected = createHmac("sha256", secret).update(`${timestamp}.${delivery.body}`).digest();
  return signature.split(",").some((entry) => {
    const [scheme, hex] = entry.trim().split("=");
    return (
      scheme === "v1" &&
      hex !== undefined &&
      /^[0-9a-f]{64}$/.test(hex) &&
      timingSafeEqual(Buffer.from(hex, "hex"), expected)
    );
  });
}

test("a row an incoming webhook makes, once assigned to an agent, reaches its webhook signed", async ({
  page,
  playwright,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const lead = account("hooks", run);
  await makeAccounts(page, [lead]);
  await page.context().clearCookies();
  await page.goto("/login");
  await signIn(page, lead.email, PASSWORD);
  await expect(page.getByRole("complementary")).toBeVisible();

  // A workspace with Tasks, a database with an Assignee, and an agent of the lead's.
  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: `Hooks ${run}` },
    201,
  );
  const base = `/api/v1/workspaces/${workspace.id}`;
  const listed = (await (await page.request.get(`${base}/teamspaces`)).json()) as Teamspaces;
  const general = listed.teamspaces.find((teamspace) => teamspace.access === "default");
  expect(general, "General").toBeDefined();
  const tasks = await callApi<Database>(
    page,
    "POST",
    `${base}/databases`,
    { teamspace_id: general?.id, title: "Tasks" },
    201,
  );
  await callApi(
    page,
    "POST",
    `/api/v1/databases/${tasks.page.id}/properties`,
    { name: "Assignee", type: "person" },
    201,
  );
  const scout = await callApi<Agent>(page, "POST", `${base}/agents`, { name: `Scout ${run}` }, 201);

  // In the settings: an outgoing webhook for the agent, to the receiver, and an incoming
  // webhook on Tasks, each secret shown once.
  await page.goto(`/w/${workspace.slug}/settings/webhooks`);
  const outgoing = page.getByRole("region", { name: "Outgoing webhooks" });
  const incoming = page.getByRole("region", { name: "Incoming webhooks" });
  const register = outgoing.getByRole("form", { name: "Register a webhook" });
  await register
    .getByLabel("For", { exact: true })
    .selectOption({ label: `Scout ${run} · via ${lead.name}` });
  await register.getByLabel("URL", { exact: true }).fill(`${RECEIVER_ON_STACK}/scout-${run}`);
  await register.getByRole("button", { name: "Register webhook" }).click();
  const secretField = outgoing.getByRole("textbox", { name: "Signing secret" });
  await expect(secretField).toHaveValue(/^whsec_/);
  const secret = await secretField.inputValue();
  await outgoing.getByRole("button", { name: "Done" }).click();
  await expect(secretField).toBeHidden();

  const make = incoming.getByRole("form", { name: "Make an incoming webhook" });
  await make.getByLabel("Name", { exact: true }).fill("Forms");
  await make
    .getByLabel("Target", { exact: true })
    .selectOption({ label: "Tasks · Database · General" });
  await make.getByRole("button", { name: "Make incoming webhook" }).click();
  const urlField = incoming.getByRole("textbox", { name: "URL", exact: true });
  await expect(urlField).toHaveValue(/\/hooks\/in\/nwh_/);
  const hookUrl = await urlField.inputValue();
  await incoming.getByRole("button", { name: "Done" }).click();

  // A sender posts a row to the hook, through the stack's web origin and its proxy, with no
  // cookie.
  const sender = await playwright.request.newContext();
  const created = await sender.post(hookUrl, {
    data: { version: 1, action: "create_row", values: { Name: `From the form ${run}` } },
  });
  expect(created.status(), await created.text()).toBe(200);
  const answer = (await created.json()) as { row: { id: string; unique_id: string } };
  await sender.dispose();

  // Assigned to the agent by the lead, the row is told to the agent's webhook, signed.
  await callApi(
    page,
    "PATCH",
    `/api/v1/rows/${answer.row.id}`,
    { values: { Assignee: [scout.id] } },
    200,
  );
  const receiver = await playwright.request.newContext({ baseURL: RECEIVER });
  let delivery: Delivery | undefined;
  await expect
    .poll(
      async () => {
        const got = (await (await receiver.get("/deliveries")).json()) as Delivery[];
        delivery = got.find(
          (entry) =>
            entry.path === `/scout-${run}` &&
            entry.headers["neon-webhook-event"] === "row.assigned",
        );
        return delivery !== undefined;
      },
      { timeout: 60_000 },
    )
    .toBe(true);
  await receiver.dispose();
  if (delivery === undefined) {
    throw new Error("no delivery");
  }
  expect(signedWith(delivery, secret), "signed with the webhook's secret").toBe(true);
  expect(signedWith(delivery, `whsec_${"x".repeat(43)}`)).toBe(false);
  const payload = JSON.parse(delivery.body) as {
    type: string;
    identity: { id: string; kind: string };
    row: { id: string; unique_id: string; title: string; database: { title: string } };
    property: { name: string };
  };
  expect(payload.type).toBe("row.assigned");
  expect(payload.identity).toMatchObject({ id: scout.id, kind: "agent" });
  expect(payload.row).toMatchObject({
    id: answer.row.id,
    unique_id: answer.row.unique_id,
    title: `From the form ${run}`,
    database: { title: "Tasks" },
  });
  expect(payload.property.name).toBe("Assignee");

  // Its log shows the delivery, and the incoming webhook's the request.
  await page.reload();
  const card = page.getByRole("listitem", {
    name: `Webhook for Scout ${run} to ${RECEIVER_ON_STACK}/scout-${run}`,
  });
  await card
    .getByRole("button", {
      name: `Delivery log of the webhook to ${RECEIVER_ON_STACK}/scout-${run}`,
    })
    .click();
  await expect(card.getByText("Delivered", { exact: true })).toBeVisible();
  const hook = page.getByRole("listitem", { name: "Forms", exact: true });
  await hook.getByRole("button", { name: "Requests of Forms" }).click();
  await expect(hook.getByText("create_row", { exact: true })).toBeVisible();
});
