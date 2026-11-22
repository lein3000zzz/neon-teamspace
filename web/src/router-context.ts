import type { QueryClient } from "@tanstack/react-query";

/** What every route's loader and component can reach through the router. */
export interface RouterContext {
  queryClient: QueryClient;
}
