import { createFileRoute } from "@tanstack/react-router";
import { WorkspaceLayout } from "@/pages/workspaces/layout";

/** A workspace's pages, by its slug, which the user's list of workspaces resolves. */
export const Route = createFileRoute("/_app/w/$slug")({
  component: WorkspaceLayout,
});
