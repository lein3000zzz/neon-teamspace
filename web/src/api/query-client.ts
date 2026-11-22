import {
  MutationCache,
  QueryCache,
  QueryClient,
  type DefaultOptions,
  type QueryKey,
} from "@tanstack/react-query";
import type { Schemas } from "./client";
import { ApiError } from "./errors";
import { keys } from "./keys";

/**
 * A failed query is retried once, a second later, and then reported, so an
 * unreachable server shows as such within about a second rather than after
 * the default three backed-off retries. The Retry buttons, and refetching
 * when the window regains focus or the network returns, take over from there.
 * An answer the server gave on purpose, any 4xx, is never retried: asking
 * again after a 401 or a 404 changes nothing, and after a 429 makes it worse.
 */
const APP_QUERY_DEFAULTS: NonNullable<DefaultOptions["queries"]> = {
  retry: (failures, error) => failures < 1 && !isRefusal(error),
  retryDelay: 1000,
};

/**
 * A mutation's variables and answer can hold a password, a new token's secret
 * or an invite code, so it leaves the cache as soon as no component shows it,
 * rather than five minutes later. Nothing reads finished mutations back.
 */
const APP_MUTATION_DEFAULTS: NonNullable<DefaultOptions["mutations"]> = { gcTime: 0 };

function isRefusal(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500;
}

/**
 * The app's query client. It also watches every failed request for what it
 * says about the session:
 *
 * - a 401 `unauthenticated` means the session ended (it expired, or was
 *   revoked elsewhere), so nobody is signed in any more; the signed-in layout
 *   sees that and sends the user to sign in again, back to where they were.
 * - a 403 `forbidden` may mean the user's role changed, on the instance or in
 *   a workspace, so who they are and their list of workspaces, which holds
 *   their roles, are asked again, and pages and controls they may no longer use
 *   say so. A refused change also reads every workspace's data again. A refused
 *   read does that only for a workspace where the role did change: reading
 *   again what was refused, while nothing changed, would be refused again, and
 *   that refusal would start the same again, without end.
 * - a 404 `not_found` from a workspace's data may mean the user is no longer
 *   in it, so their list of workspaces is read again.
 *
 * None of these cancels a request already on its way, which would only ask
 * the same again: a burst of refusals joins the one read of the list.
 *
 * Tests pass their own query defaults; the session handling and the
 * mutation defaults are the same.
 */
export function createQueryClient(queries: DefaultOptions["queries"] = APP_QUERY_DEFAULTS) {
  const client: QueryClient = new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => watchSession(client, error, query.queryKey),
    }),
    mutationCache: new MutationCache({ onError: (error) => watchSession(client, error) }),
    defaultOptions: { queries, mutations: APP_MUTATION_DEFAULTS },
  });
  return client;
}

function watchSession(client: QueryClient, error: unknown, queryKey?: QueryKey) {
  if (!(error instanceof ApiError)) {
    return;
  }
  if (error.sessionEnded) {
    client.setQueryData(keys.me, null);
  } else if (error.status === 403 && error.code === "forbidden") {
    void client.invalidateQueries({ queryKey: keys.me }, JOIN_RUNNING);
    if (queryKey === undefined) {
      void client.invalidateQueries({ queryKey: WORKSPACES_ROOT }, JOIN_RUNNING);
    } else {
      void refreshChangedRoles(client);
    }
  } else if (
    error.status === 404 &&
    error.code === "not_found" &&
    queryKey?.[0] === WORKSPACES_ROOT[0] &&
    queryKey[1] !== keys.workspaces[1]
  ) {
    void client.invalidateQueries({ queryKey: keys.workspaces }, JOIN_RUNNING);
  }
}

const WORKSPACES_ROOT = [keys.workspaces[0]] as const;

/** A read already on its way is waited for rather than cancelled and sent again. */
const JOIN_RUNNING = { cancelRefetch: false } as const;

/**
 * Reads the list of workspaces again, then all the data of each workspace in
 * it where the user's role is not what it was.
 */
async function refreshChangedRoles(client: QueryClient) {
  const before = new Map(
    client
      .getQueryData<Schemas["WorkspaceList"]>(keys.workspaces)
      ?.workspaces.map((workspace) => [workspace.id, workspace.role]),
  );
  await client.invalidateQueries({ queryKey: keys.workspaces }, JOIN_RUNNING);
  const after = client.getQueryData<Schemas["WorkspaceList"]>(keys.workspaces);
  for (const workspace of after?.workspaces ?? []) {
    if (before.get(workspace.id) !== workspace.role) {
      void client.invalidateQueries({ queryKey: keys.workspace(workspace.id) }, JOIN_RUNNING);
    }
  }
}
