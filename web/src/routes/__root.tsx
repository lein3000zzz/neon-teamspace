import { createRootRouteWithContext, Outlet } from "@tanstack/react-router";
import type { RouterContext } from "@/router-context";

/**
 * Two layouts hang off the root: the shell, for signed-in pages (_app), and
 * the centred card, for the pages of signing in and up (_auth).
 */
export const Route = createRootRouteWithContext<RouterContext>()({
  component: Outlet,
});
