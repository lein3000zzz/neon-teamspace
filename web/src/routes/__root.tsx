import { createRootRouteWithContext, Outlet } from "@tanstack/react-router";
import { AppShell } from "@/components/shell/app-shell";
import type { RouterContext } from "@/router-context";

export const Route = createRootRouteWithContext<RouterContext>()({
  component: Root,
});

function Root() {
  return (
    <AppShell>
      <Outlet />
    </AppShell>
  );
}
