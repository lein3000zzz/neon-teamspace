import type { QueryClient } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createQueryClient } from "@/api/query-client";
import { App } from "@/app";
import { createAppRouter } from "@/router";

/**
 * Renders the whole app, shell and routes included, at the given path. Queries
 * fail on the first error unless a test passes the app's own client; the
 * session handling is the app's either way.
 */
export function renderApp(
  path = "/",
  queryClient: QueryClient = createQueryClient({
    retry: false,
    gcTime: Number.POSITIVE_INFINITY,
  }),
) {
  const router = createAppRouter(queryClient, createMemoryHistory({ initialEntries: [path] }));
  const user = userEvent.setup();
  return {
    user,
    router,
    queryClient,
    ...render(<App queryClient={queryClient} router={router} />),
  };
}
