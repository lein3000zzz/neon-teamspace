import { createFileRoute, Outlet } from "@tanstack/react-router";
import { AuthLayout } from "@/components/auth/auth-layout";

/** The pages of signing in and up, open to anyone. */
export const Route = createFileRoute("/_auth")({
  component: () => (
    <AuthLayout>
      <Outlet />
    </AuthLayout>
  ),
});
