import type { QueryClient } from "@tanstack/react-query";
import {
  createRouter,
  parseSearchWith,
  stringifySearchWith,
  type RouterHistory,
} from "@tanstack/react-router";
import { ErrorPage } from "./pages/error";
import { NotFoundPage } from "./pages/not-found";
import { routeTree } from "./routeTree.gen";

/** One factory for the app and the tests, so both run the same route tree. */
export function createAppRouter(queryClient: QueryClient, history?: RouterHistory) {
  return createRouter({
    routeTree,
    context: { queryClient },
    defaultPreload: "intent",
    // Fallbacks render inside the shell, so the navigation stays usable.
    defaultErrorComponent: ErrorPage,
    defaultNotFoundComponent: NotFoundPage,
    scrollRestoration: true,
    // Search values stay the strings they are in the address. The default reads
    // them as JSON, which would turn an invite code such as 12345678 into a number.
    parseSearch: parseSearchWith((value) => value),
    stringifySearch: stringifySearchWith((value: unknown) => String(value)),
    ...(history === undefined ? {} : { history }),
  });
}

export type AppRouter = ReturnType<typeof createAppRouter>;

declare module "@tanstack/react-router" {
  interface Register {
    router: AppRouter;
  }
}

declare module "@tanstack/history" {
  /** What one page hands the next without putting it in the address. */
  interface HistoryState {
    /** An address to fill in on the sign-in and reset forms. */
    email?: string;
    /** Why the sign-in form opened: a link verified the address, or set a password. */
    notice?: "verified" | "password-set";
  }
}
