import { createFileRoute, Outlet, redirect } from "@tanstack/react-router";
import { meQuery } from "@/api/account";
import { AppShell } from "@/components/shell/app-shell";
import { SignedIn } from "@/components/shell/signed-in";
import { SessionErrorPage } from "@/pages/session-error";

/**
 * Every page in the shell needs someone signed in. Before any of them loads,
 * a visitor without a session is sent to sign in, with the way back.
 */
export const Route = createFileRoute("/_app")({
  beforeLoad: async ({ context, location }) => {
    const me = await context.queryClient.ensureQueryData(meQuery);
    if (me === null) {
      throw redirect({ to: "/login", search: { next: location.href }, replace: true });
    }
  },
  errorComponent: SessionErrorPage,
  component: AppLayout,
});

function AppLayout() {
  return (
    <SignedIn>
      <AppShell>
        <Outlet />
      </AppShell>
    </SignedIn>
  );
}
