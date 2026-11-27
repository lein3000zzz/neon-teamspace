import type { QueryClient } from "@tanstack/react-query";
import {
  createBrowserHistory,
  createRouter,
  parseSearchWith,
  stringifySearchWith,
  type RouterHistory,
} from "@tanstack/react-router";
import { scrollKeyOf, withPanels } from "./lib/panel-history";
import { ErrorPage } from "./pages/error";
import { NotFoundPage } from "./pages/not-found";
import { routeTree } from "./routeTree.gen";

/**
 * One factory for the app and the tests, so both run the same route tree. The history keeps
 * entries for the panels open on a touch screen, which share the scroll position of the page
 * under them.
 */
export function createAppRouter(queryClient: QueryClient, history?: RouterHistory) {
  return createRouter({
    routeTree,
    context: { queryClient },
    defaultPreload: "intent",
    // Fallbacks render inside the shell, so the navigation stays usable.
    defaultErrorComponent: ErrorPage,
    defaultNotFoundComponent: NotFoundPage,
    scrollRestoration: true,
    getScrollRestorationKey: scrollKeyOf,
    // Search values stay the strings they are in the address. The default reads
    // them as JSON, which would turn an invite code such as 12345678 into a number.
    parseSearch: parseSearchWith((value) => value),
    stringifySearch: stringifySearchWith((value: unknown) => String(value)),
    history: withPanels(history ?? createBrowserHistory()),
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
    /** The panels open over the page in this entry, outermost first (lib/panel-history.ts). */
    panels?: string[];
    /** The key of the page's entry under those panels, whose scroll position they keep. */
    panelsOver?: string;
    /** A row's peek was opened from its database's page, the entry before. */
    peeked?: boolean;
  }
}
