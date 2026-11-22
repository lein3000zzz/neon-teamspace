import { QueryObserver, type QueryClient } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
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

  it("reads a workspace's data again after a refused read only where the role changed", async () => {
    const client = createQueryClient({ retry: false });
    const roles = { w1: "admin", w2: "member" };
    const list = () => ({
      workspaces: Object.entries(roles).map(([id, role]) => ({ id, role })),
      can_create: true,
    });
    // The sidebar keeps the list on screen, so a refusal reads it again.
    const unsubscribe = new QueryObserver(client, {
      queryKey: keys.workspaces,
      queryFn: () => Promise.resolve(list()),
    }).subscribe(() => undefined);
    await vi.waitFor(() => expect(client.getQueryData(keys.workspaces)).toEqual(list()));
    client.setQueryData(keys.members("w1"), []);
    client.setQueryData(keys.members("w2"), []);

    roles.w1 = "member";
    await failQuery(client, refusal(403, "forbidden"));

    await vi.waitFor(() => expect(client.getQueryData(keys.workspaces)).toEqual(list()));
    await vi.waitFor(() =>
      expect(client.getQueryState(keys.members("w1"))?.isInvalidated).toBe(true),
    );
    expect(client.getQueryState(keys.members("w2"))?.isInvalidated).toBe(false);
    unsubscribe();
  });

  it("reads every workspace's data again after a refused change", async () => {
    const client = createQueryClient({ retry: false });
    client.setQueryData(keys.me, ada);
    client.setQueryData(keys.workspaces, { workspaces: [], can_create: true });
    client.setQueryData(keys.members("w1"), []);
    await client
      .getMutationCache()
      .build(client, { mutationFn: () => Promise.reject(refusal(403, "forbidden")) })
      .execute(undefined)
      .catch(() => undefined);
    expect(client.getQueryState(keys.me)?.isInvalidated).toBe(true);
    expect(client.getQueryState(keys.workspaces)?.isInvalidated).toBe(true);
    expect(client.getQueryState(keys.members("w1"))?.isInvalidated).toBe(true);
  });

  it("reads the list of workspaces again when one answers not_found", async () => {
    const client = createQueryClient({ retry: false });
    client.setQueryData(keys.workspaces, { workspaces: [], can_create: true });
    await client
      .fetchQuery({
        queryKey: keys.members("w1"),
        queryFn: () => Promise.reject(refusal(404, "not_found")),
      })
      .catch(() => undefined);
    expect(client.getQueryState(keys.workspaces)?.isInvalidated).toBe(true);
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
