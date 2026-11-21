import { QueryClient } from "@tanstack/react-query";

/**
 * The app's query client. A failed query is retried once, a second later, and
 * then reported, so an unreachable server shows as such within about a second
 * rather than after the default three backed-off retries. The Retry buttons,
 * and refetching when the window regains focus or the network returns, take
 * over from there.
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: 1, retryDelay: 1000 } },
  });
}
