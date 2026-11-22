import { createFileRoute } from "@tanstack/react-router";
import { TeamspacesSettingsPage } from "@/pages/workspaces/settings/teamspaces";

export const Route = createFileRoute("/_app/w/$slug/settings/teamspaces")({
  component: TeamspacesSettingsPage,
});
