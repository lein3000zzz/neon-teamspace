import { createFileRoute } from "@tanstack/react-router";
import { WorkspaceSettingsLayout } from "@/pages/workspaces/settings/layout";

export const Route = createFileRoute("/_app/w/$slug/settings")({
  component: WorkspaceSettingsLayout,
});
