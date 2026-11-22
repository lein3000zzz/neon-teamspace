import { createFileRoute } from "@tanstack/react-router";
import { WorkspaceHomePage } from "@/pages/workspaces/home";

export const Route = createFileRoute("/_app/w/$slug/")({
  component: WorkspaceHomePage,
});
