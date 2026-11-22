import { createFileRoute } from "@tanstack/react-router";
import { WorkspacesAdminPage } from "@/pages/admin/workspaces";

export const Route = createFileRoute("/_app/admin/workspaces")({
  component: WorkspacesAdminPage,
});
