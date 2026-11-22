import { createFileRoute } from "@tanstack/react-router";
import { BrowseTeamspacesPage } from "@/pages/workspaces/teamspaces";

export const Route = createFileRoute("/_app/w/$slug/teamspaces/")({
  component: BrowseTeamspacesPage,
});
