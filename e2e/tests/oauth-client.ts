import { createHash } from "node:crypto";
import type { Page } from "@playwright/test";
import { PASSWORD } from "./accounts";
import { expect, signIn } from "./fixtures";

/** Where the clients of the specs listen, as a CLI's would: nothing does, the test answers. */
export const REDIRECT = "http://127.0.0.1:43110/callback";

/** What the token endpoint issues. */
export type Tokens = {
  access_token: string;
  refresh_token: string;
  scope: string;
  expires_in: number;
};

/** A PKCE verifier and its S256 challenge, low in entropy: no test secret looks real. */
export function pkce(): { verifier: string; challenge: string } {
  const verifier = "e".repeat(48);
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return { verifier, challenge };
}

/**
 * Sends the consent page to the authorization endpoint with `query` and allows the client
 * named `app` as the agent named `agent`, signing in as `email` first when given. Answers
 * the address the browser was sent back to, which the test answers in the client's place.
 */
export async function allowAsAgent(
  consent: Page,
  query: URLSearchParams,
  app: string,
  agent: string,
  email?: string,
): Promise<URL> {
  const back = new Promise<URL>((resolve) => {
    void consent.route(`${REDIRECT}**`, async (route) => {
      resolve(new URL(route.request().url()));
      await route.fulfill({ status: 200, contentType: "text/plain", body: "connected" });
    });
  });
  await consent.goto(`/api/v1/oauth/authorize?${query.toString()}`);
  if (email !== undefined) {
    await expect(consent.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    await signIn(consent, email, PASSWORD);
  }
  const card = consent.getByRole("region", { name: `Connect ${app}?` });
  await expect(card).toBeVisible();
  await expect(card.getByText("Then you go back to http://127.0.0.1:43110.")).toBeVisible();
  await card.getByRole("radio", { name: `${agent}, my agent` }).check();
  await expect(card.getByRole("checkbox", { name: "Write" })).toBeChecked();
  await card.getByRole("button", { name: "Allow" }).click();
  const answer = await back;
  await consent.unroute(`${REDIRECT}**`);
  return answer;
}
