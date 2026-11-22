import { createFileRoute } from "@tanstack/react-router";
import { AdminLayout } from "@/pages/admin/layout";

export const Route = createFileRoute("/_app/admin")({
  component: AdminLayout,
});
