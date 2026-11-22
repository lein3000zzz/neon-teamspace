import { MutationCache, QueryCache, QueryClient, type DefaultOptions } from "@tanstack/react-query";
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
 * - a 403 `forbidden` may mean the user's role changed, so who they are is
 *   asked again, and pages they may no longer see say so.
 *
 * Tests pass their own query defaults; the session handling and the
 * mutation defaults are the same.
 */
export function createQueryClient(queries: DefaultOptions["queries"] = APP_QUERY_DEFAULTS) {
  const client: QueryClient = new QueryClient({
    queryCache: new QueryCache({ onError: (error) => watchSession(client, error) }),
    mutationCache: new MutationCache({ onError: (error) => watchSession(client, error) }),
    defaultOptions: { queries, mutations: APP_MUTATION_DEFAULTS },
  });
  return client;
}

function watchSession(client: QueryClient, error: unknown) {
  if (!(error instanceof ApiError)) {
    return;
  }
  if (error.sessionEnded) {
    client.setQueryData(keys.me, null);
  } else if (error.status === 403 && error.code === "forbidden") {
    void client.invalidateQueries({ queryKey: keys.me });
  }
}
