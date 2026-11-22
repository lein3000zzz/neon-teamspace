import type { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { ada } from "@/test/fixtures";
import { ApiError } from "./errors";
import { keys } from "./keys";
import { createQueryClient } from "./query-client";

const refusal = (status: number, code: string) =>
  new ApiError(status, { error: { code, message: code } });

/** Runs a query that fails with `error`, on a client that already knows who is signed in. */
async function failQuery(client: QueryClient, error: unknown) {
  client.setQueryData(keys.me, ada);
  await client
    .fetchQuery({ queryKey: ["account", "sessions"], queryFn: () => Promise.reject(error) })
    .catch(() => undefined);
}

describe("the query client", () => {
  it("forgets who is signed in when a request says the session ended", async () => {
    const client = createQueryClient({ retry: false });
    await failQuery(client, refusal(401, "unauthenticated"));
    expect(client.getQueryData(keys.me)).toBeNull();
  });

  it("does the same for a mutation", async () => {
    const client = createQueryClient({ retry: false });
    client.setQueryData(keys.me, ada);
    await client
      .getMutationCache()
      .build(client, { mutationFn: () => Promise.reject(refusal(401, "unauthenticated")) })
      .execute(undefined)
      .catch(() => undefined);
    expect(client.getQueryData(keys.me)).toBeNull();
  });

  it("keeps the user signed in after a wrong password", async () => {
    const client = createQueryClient({ retry: false });
    await failQuery(client, refusal(401, "invalid_credentials"));
    expect(client.getQueryData(keys.me)).toEqual(ada);
  });

  it("asks again who is signed in after a 403", async () => {
    const client = createQueryClient({ retry: false });
    await failQuery(client, refusal(403, "forbidden"));
    expect(client.getQueryState(keys.me)?.isInvalidated).toBe(true);
  });

  it("retries a failure to connect once, and an answer the server refused never", () => {
    const retry = createQueryClient().getDefaultOptions().queries?.retry;
    expect(typeof retry).toBe("function");
    const decide = retry as (failures: number, error: unknown) => boolean;
    expect(decide(0, new TypeError("Failed to fetch"))).toBe(true);
    expect(decide(1, new TypeError("Failed to fetch"))).toBe(false);
    expect(decide(0, refusal(429, "rate_limited"))).toBe(false);
    expect(decide(0, refusal(404, "not_found"))).toBe(false);
    expect(decide(0, new ApiError(503, null))).toBe(true);
  });
});
